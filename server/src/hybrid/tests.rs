use super::*;
use crate::{
    AppState,
    acl::Role,
    agent::{self, DocAccess, ToolCall, ToolContext},
    embed::{FakeEmbedder, write_test_artifacts, write_test_store},
    files::FileStore,
    ingest::status,
    search::NewChunk,
    tests::docs_state_files,
};
use std::{fs, time::Instant};

const MODEL: &str =
    "name = 'uji'\npooling = 'cls'\ndim = 2\nnormalize = true\nquery_prefix = 'pengencangan: '\n";

fn model(folder: &Path) -> Model {
    write_test_artifacts(folder);
    fs::write(folder.join("app-model.toml"), MODEL).unwrap();
    Model::load(folder).unwrap()
}

fn person() -> AuthUser {
    AuthUser {
        id: 1,
        name: "Uji".into(),
        email: String::new(),
        departments: vec!["Operasional".into()],
        role: Role::User,
    }
}

fn publish(loader: &mut Loader, live: &Live, base: &Path, index: &DocIndex) {
    loader.publish(
        live,
        &base.join("data"),
        base,
        index,
        Store::open_generation,
        |_| Ok(Box::new(FakeEmbedder)),
    );
}

async fn search(
    state: &AppState,
    live: Option<&Live>,
    filters: SearchFilters,
    limit: usize,
) -> Vec<String> {
    chat(
        live,
        state.docs.clone().unwrap(),
        &state.db,
        &person(),
        &state.sources,
        "pengencangan".into(),
        limit,
        filters,
    )
    .await
    .unwrap()
    .into_iter()
    .map(|(_, row)| row.rel_path)
    .collect()
}

#[tokio::test]
async fn semantic_filters_access_before_ranking_and_cites_semantic_only_content() {
    let (state, base) = docs_state_files(
        0,
        &[
            ("umum/2021/manual.txt", b"Torsi panduan sintetis.".to_vec()),
            ("umum/panduan.txt", b"Pengencangan baut sintetis.".to_vec()),
            (
                "umum/petunjuk.txt",
                b"Pengencangan pengencangan untuk baut sintetis.".to_vec(),
            ),
            ("umum/pending.txt", b"Torsi sintetis pending.".to_vec()),
            ("umum/mixed.txt", b"Torsi sintetis bercampur.".to_vec()),
            ("umum/torsi-a.bin", vec![1]),
            ("umum/torsi-b.bin", vec![1]),
            ("umum/torsi-c.bin", vec![1]),
        ],
    )
    .await;
    let index = state.docs.as_ref().unwrap();
    let mixed = state
        .db
        .call(|c| {
            c.execute(
                &format!("UPDATE {INDEX_TABLE} SET status=? WHERE rel_path='pending.txt'"),
                [status::PENDING],
            )?;
            Ok(c.query_row(
                &format!("SELECT id FROM {INDEX_TABLE} WHERE rel_path='mixed.txt'"),
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    let add_hidden = |range: std::ops::Range<usize>| {
        let mut writer = index.writer().unwrap();
        for n in range {
            index
                .add_chunk(
                    &writer,
                    NewChunk {
                        doc_id: mixed,
                        n,
                        source: "Umum",
                        acl: &["dept:review".into()],
                        title: "Torsi pengencangan",
                        rel_path: "mixed.txt",
                        page: None,
                        text: "Torsi pengencangan baut tersembunyi",
                        file_type: "teks",
                        years: &[2021],
                    },
                )
                .unwrap();
        }
        writer.commit().unwrap();
        writer.wait_merging_threads().unwrap();
        index.reload().unwrap();
    };
    let keyword_ranks = || {
        let snapshot = index.snapshot();
        let (allowed, _) = index
            .semantic_addresses(
                &snapshot,
                &person(),
                &["Umum".into()],
                &SearchFilters::default(),
            )
            .unwrap();
        let eligible = allowed
            .into_iter()
            .filter(|&address| {
                let hit = index.hit_at(&snapshot, address).unwrap();
                hit.doc_id != mixed && hit.title != "pending"
            })
            .collect();
        let mut scores = index
            .keyword_scores(&snapshot, "pengencangan baut", &eligible)
            .unwrap()
            .into_iter()
            .map(|(score, address)| (index.hit_at(&snapshot, address).unwrap().chunk_id, score))
            .collect::<Vec<_>>();
        scores.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        scores
    };
    let baseline = keyword_ranks();
    assert!(baseline.len() >= 2);
    add_hidden(99..100);
    let model = model(&base);
    let live = Arc::new(Live::default());
    let mut loader = Loader::default();
    let refresh = |loader: &mut Loader| {
        write_test_store(&base.join("data"), &model, index).unwrap();
        publish(loader, &live, &base, index);
        publish(loader, &live, &base, index);
    };
    refresh(&mut loader);
    let before = search(&state, Some(&live), SearchFilters::default(), 10).await;
    add_hidden(100..220);
    assert_eq!(keyword_ranks(), baseline);
    refresh(&mut loader);
    let all = search(&state, Some(&live), SearchFilters::default(), 10).await;
    assert_eq!(all, before);
    assert_eq!(
        search(&state, Some(&live), SearchFilters::default(), 1).await,
        before[..1]
    );
    for path in ["mixed.txt", "pending.txt", "torsi.txt"] {
        assert!(!all.iter().any(|hit| hit == path), "{all:?}");
    }
    assert_eq!(all.len(), 6, "{all:?}");
    assert_eq!(all.iter().filter(|path| path.ends_with(".bin")).count(), 2);
    assert!(all.contains(&"2021/manual.txt".to_string()));
    for (filters, expected) in [
        (
            SearchFilters {
                source: Some("Review".into()),
                ..Default::default()
            },
            Vec::new(),
        ),
        (
            SearchFilters {
                file_type: Some("Word".into()),
                ..Default::default()
            },
            Vec::new(),
        ),
        (
            SearchFilters {
                source: Some("Umum".into()),
                file_type: Some("teks".into()),
                year: Some(2021),
            },
            vec!["2021/manual.txt"],
        ),
    ] {
        assert_eq!(search(&state, Some(&live), filters, 1).await, expected);
    }
    let ctx = ToolContext {
        files: FileStore::Local { dir: base.clone() },
        docs: Some(DocAccess {
            semantic: Some(live),
            images: None,
            index: index.clone(),
            db: state.db.clone(),
            user: person(),
            sources: state.sources.clone(),
            session: None,
        }),
    };
    let call = ToolCall {
        id: "uji".into(),
        name: "search_documents".into(),
        arguments: serde_json::json!({"query":"kencangkan","year":2021}).to_string(),
    };
    let mut citations = Vec::new();
    let result = agent::run_tool(&call, &ctx, 120_000, &mut citations).await;
    assert!(
        result.ok && result.output.contains("[1] manual"),
        "{}",
        result.output
    );
    assert_eq!(citations.len(), 1);
    assert_eq!(citations[0].n, 1);
}

#[tokio::test]
async fn unavailable_or_incompatible_store_preserves_keyword_and_pointer_reload_recovers() {
    let (state, base) = docs_state_files(0, &[]).await;
    let index = state.docs.as_ref().unwrap();
    let data = base.join("data");
    let live = Live::default();
    let filters = SearchFilters::default();
    let user = person();
    let kw = || {
        chat(
            None,
            index.clone(),
            &state.db,
            &user,
            &state.sources,
            "cuti".into(),
            6,
            filters.clone(),
        )
    };
    let expected = serde_json::to_value(kw().await.unwrap()).unwrap();
    assert_ne!(expected, serde_json::json!([]));
    for reason in ["model", "store", "index", "config"] {
        let mut loader = Loader::default();
        let model = (reason != "model").then(|| model(&base));
        if matches!(reason, "index" | "config") {
            let model = model.as_ref().unwrap();
            write_test_store(&data, model, index).unwrap();
            let dir = model.store_dir(&data);
            let manifest = Store::current(&dir).unwrap().join("manifest.json");
            let mut json: serde_json::Value =
                serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
            let (field, value) = if reason == "index" {
                ("index_version", serde_json::json!(-1))
            } else {
                ("config_sha256", serde_json::json!("other"))
            };
            json[field] = value;
            fs::write(manifest, serde_json::to_vec(&json).unwrap()).unwrap();
        }
        publish(&mut loader, &live, &base, index);
        publish(&mut loader, &live, &base, index);
        assert!(live.ready.read().unwrap().is_none(), "{reason}");
        let hits = chat(
            Some(&live),
            index.clone(),
            &state.db,
            &person(),
            &state.sources,
            "cuti".into(),
            6,
            filters.clone(),
        )
        .await
        .unwrap();
        assert_eq!(serde_json::to_value(hits).unwrap(), expected, "{reason}");
        if matches!(reason, "index" | "config") {
            fs::remove_file(model.unwrap().store_dir(&data).join("current")).unwrap();
        }
    }
    let mut loader = Loader::default();
    let model = model(&base);
    write_test_store(&data, &model, index).unwrap();
    publish(&mut loader, &live, &base, index);
    assert!(live.ready.read().unwrap().is_none());
    publish(&mut loader, &live, &base, index);
    let first = live.ready.read().unwrap().clone().unwrap();
    index.reload().unwrap();
    publish(&mut loader, &live, &base, index);
    assert!(Arc::ptr_eq(
        &first,
        live.ready.read().unwrap().as_ref().unwrap()
    ));
    let old = Store::current(&model.store_dir(&data)).unwrap();
    let mut writer = index.writer().unwrap();
    index.delete_document(
        &writer,
        first
            .chunks
            .values()
            .find(|chunk| chunk.source == "Review")
            .unwrap()
            .doc_id,
    );
    let public_id = first
        .chunks
        .values()
        .find(|chunk| chunk.source == "Umum")
        .unwrap()
        .doc_id;
    index
        .add_chunk(
            &writer,
            NewChunk {
                doc_id: public_id,
                n: 99,
                source: "Umum",
                acl: &["umum".into()],
                title: "Cuti",
                rel_path: "cuti.txt",
                page: None,
                text: "Limau baru",
                file_type: "teks",
                years: &[2021],
            },
        )
        .unwrap();
    writer.commit().unwrap();
    writer.wait_merging_threads().unwrap();
    index.reload().unwrap();
    publish(&mut loader, &live, &base, index);
    assert!(live.ready.read().unwrap().is_none());
    assert!(loader.ready.is_none());
    publish(&mut loader, &live, &base, index);
    let stale = live.ready.read().unwrap().clone().unwrap();
    assert!(Arc::ptr_eq(&first.encoder, &stale.encoder));
    assert!(stale.chunks.values().all(|chunk| chunk.source != "Review"));
    assert_eq!(
        stale
            .chunks
            .values()
            .filter(|chunk| chunk.vector.is_none())
            .count(),
        1
    );
    let hits = chat(
        Some(&live),
        index.clone(),
        &state.db,
        &person(),
        &state.sources,
        "limau".into(),
        10,
        SearchFilters::default(),
    )
    .await
    .unwrap();
    assert!(hits.iter().any(|(hit, _)| hit.text == "Limau baru"));
    fs::remove_file(model.store_dir(&data).join("current")).unwrap();
    publish(&mut loader, &live, &base, index);
    assert!(Arc::ptr_eq(
        &stale,
        live.ready.read().unwrap().as_ref().unwrap()
    ));
    write_test_store(&data, &model, index).unwrap();
    assert_ne!(Store::current(&model.store_dir(&data)).unwrap(), old);
    publish(&mut loader, &live, &base, index);
    let updated = live.ready.read().unwrap().clone().unwrap();
    assert!(!Arc::ptr_eq(&first.store, &updated.store));
    assert!(Arc::ptr_eq(&first.encoder, &updated.encoder));
}

#[tokio::test]
async fn encoder_failure_is_terminal_and_never_reads_the_store() {
    let (state, base) = docs_state_files(0, &[]).await;
    let index = state.docs.as_ref().unwrap();
    let data = base.join("data");
    write_test_store(&data, &model(&base), index).unwrap();
    let live = Live::default();
    let mut loader = Loader::default();
    let mut attempts = 0;
    for _ in 0..4 {
        loader.publish(
            &live,
            &data,
            &base,
            index,
            |_, _, _| panic!("store opened after encoder failure"),
            |_| {
                attempts += 1;
                anyhow::bail!("synthetic encoder failure")
            },
        );
    }
    assert_eq!(attempts, 1);
    assert!(live.ready.read().unwrap().is_none());
}

#[tokio::test]
async fn loader_restart_keeps_the_encoder_result() {
    let (state, base) = docs_state_files(0, &[]).await;
    let index = state.docs.as_ref().unwrap();
    let data = base.join("data");
    write_test_store(&data, &model(&base), index).unwrap();
    for fail in [false, true] {
        let live = Live::default();
        let mut loader = Loader::default();
        loader.publish(&live, &data, &base, index, Store::open_generation, |_| {
            anyhow::ensure!(!fail, "synthetic encoder failure");
            Ok(Box::new(FakeEmbedder))
        });
        publish(&mut loader, &live, &base, index);
        let before = live.ready.read().unwrap().clone();
        assert_eq!(before.is_none(), fail);
        loader.run(&live, |_| panic!("synthetic loader panic"));
        assert!(live.ready.read().unwrap().is_none());
        for _ in 0..2 {
            loader.publish(&live, &data, &base, index, Store::open_generation, |_| {
                panic!("encoder reopened after loader panic")
            });
        }
        match (before, live.ready.read().unwrap().as_ref()) {
            (Some(before), Some(after)) => assert!(Arc::ptr_eq(&before.encoder, &after.encoder)),
            (None, None) if fail => {}
            _ => panic!("encoder result changed after loader panic"),
        }
    }
}

#[tokio::test]
async fn delete_only_commit_invalidates_ready_without_reopening_store() {
    let (_state, base) = docs_state_files(0, &[]).await;
    let index = DocIndex::in_memory().unwrap();
    let mut writer = index.writer().unwrap();
    writer.set_merge_policy(Box::new(tantivy::merge_policy::NoMergePolicy));
    for doc_id in 1..=3 {
        index
            .add_chunk(
                &writer,
                NewChunk {
                    doc_id,
                    n: 0,
                    source: "Uji",
                    acl: &["umum".into()],
                    title: "Torsi",
                    rel_path: "uji.txt",
                    page: None,
                    text: "Potongan sintetis",
                    file_type: "teks",
                    years: &[],
                },
            )
            .unwrap();
    }
    writer.commit().unwrap();
    index.reload().unwrap();
    let model = model(&base);
    let data = base.join("data");
    write_test_store(&data, &model, &index).unwrap();
    let dir = model.store_dir(&data);
    let generation = Store::current(&dir).unwrap();
    let live = Live::default();
    let mut loader = Loader::default();
    publish(&mut loader, &live, &base, &index);
    publish(&mut loader, &live, &base, &index);
    let before = live.ready.read().unwrap().clone().unwrap();
    let deleted = before
        .chunks
        .iter()
        .find(|(address, _)| {
            before
                .snapshot
                .segment_reader(address.segment_ord)
                .num_docs()
                > 1
        })
        .unwrap()
        .1
        .doc_id;
    index.delete_document(&writer, deleted);
    writer.commit().unwrap();
    writer.wait_merging_threads().unwrap();
    index.reload().unwrap();
    let snapshot = index.snapshot();
    let old_segments = before.snapshot.generation().segments();
    let new_segments = snapshot.generation().segments();
    assert!(old_segments.keys().eq(new_segments.keys()));
    assert_ne!(old_segments, new_segments);
    assert!(!before.matches(snapshot.generation()));

    let tick = |loader: &mut Loader| {
        loader.publish(
            &live,
            &data,
            &base,
            &index,
            |_, _, _| panic!("unchanged store generation reopened"),
            |_| panic!("encoder reopened"),
        );
    };
    tick(&mut loader);
    assert!(live.ready.read().unwrap().is_none());
    tick(&mut loader);
    let after = live.ready.read().unwrap().clone().unwrap();
    assert!(after.matches(snapshot.generation()));
    assert!(Arc::ptr_eq(&before.store, &after.store));
    assert_eq!(Store::current(&dir).unwrap(), generation);
    assert_eq!(after.chunks.len(), 2);
    assert!(after.chunks.values().all(|chunk| chunk.doc_id != deleted));
}

fn chunk(i: usize) -> Chunk {
    Chunk {
        id: format!("{i:05}"),
        doc_id: i as i64,
        source: "Uji".into(),
        name_only: false,
        vector: Some(i),
    }
}

#[test]
fn fusion_scores_chunks_at_one_based_ranks() {
    let [a, b, c] = [1, 2, 3].map(|i| DocAddress::new(0, i));
    let chunks = [a, b, c]
        .into_iter()
        .enumerate()
        .map(|(i, address)| (address, chunk(i)))
        .collect();
    assert_eq!(
        fusion([vec![a, a, b], vec![b, c]], &chunks),
        [
            (b, 1.0 / 62.0 + 1.0 / 61.0),
            (a, 1.0 / 61.0),
            (c, 1.0 / 62.0)
        ]
    );
}

#[test]
#[ignore = "synthetic 200 MiB benchmark"]
fn fake_query_and_50k_vectors() {
    const N: usize = 50_000;
    const DIM: usize = 1024;
    struct Synthetic;
    impl Embedder for Synthetic {
        fn embed(&mut self, _: &[String]) -> Result<Vec<Vec<f32>>> {
            Ok(vec![vec![1.0 / 32.0; DIM]])
        }
    }
    let encoder: Encoder = Arc::new(Mutex::new(Box::new(Synthetic)));
    let mut vectors = vec![1.0 / 32.0; N * DIM];
    for (i, row) in vectors.chunks_exact_mut(DIM).enumerate() {
        row[..i % DIM].iter_mut().for_each(|x| *x = -*x);
    }
    let store = Store::synthetic(vectors, DIM);
    let addresses = (0..N)
        .map(|i| (i, DocAddress::new(0, i as u32)))
        .collect::<Vec<_>>();
    let chunks = addresses
        .iter()
        .map(|&(i, address)| (address, chunk(i)))
        .collect();
    let mut times = Vec::new();
    for _ in 0..11 {
        let candidates = addresses.clone();
        let started = Instant::now();
        let query = embed_query(&encoder, "pertanyaan sintetis".into(), DIM).unwrap();
        let embedded = started.elapsed().as_secs_f64() * 1e6;
        let scores = score_vectors(&store, &query, candidates);
        let top = top_scores(scores, &chunks, 100);
        assert_eq!(top.len(), 100);
        times.push((started.elapsed().as_secs_f64() * 1e3, embedded));
    }
    times.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    println!(
        "50,000 x 1,024 | 11 queries | median {:.2} ms | max {:.2} ms | fake embed at median {:.1} us",
        times[5].0, times[10].0, times[5].1
    );
}
