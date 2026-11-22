use super::*;
use rust_xlsxwriter::Workbook;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;

pub(super) struct Temp(pub PathBuf);

impl Temp {
    pub(super) fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("rag-hub-eval-{:016x}", rand::random::<u64>()));
        fs::create_dir(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn export_fixture(dir: &Path, files: Value, answers: Value) {
    let relations = dir.join("answers");
    fs::create_dir_all(&relations).unwrap();
    fs::write(
        dir.join("files.json"),
        json!({ "files": files }).to_string(),
    )
    .unwrap();
    fs::write(
        relations.join("references.json"),
        json!({ "metadata": {}, "answers": answers }).to_string(),
    )
    .unwrap();
}

#[test]
fn metrics_use_first_expected_document_and_fixed_cutoffs() {
    let doc = |n: usize| ("Uji".into(), format!("{n}.txt"));
    let ranked: Vec<_> = (1..=12).map(doc).collect();
    let mut metrics = Metrics::default();
    let both = first_rank(&HashSet::from([doc(11), doc(6)]), &ranked);
    assert_eq!(both, Some(6));
    for rank in [Some(1), both, Some(10), Some(11), None] {
        metrics.add(rank);
    }
    assert_eq!(metrics.count, 5);
    assert_eq!(metrics.hits, [1, 2, 3]);
    assert!((metrics.reciprocal_sum / 5.0 - (1.0 + 1.0 / 6.0 + 0.1) / 5.0).abs() < 1e-12);
}

#[test]
fn expected_keys_normalize_excel_edits() {
    let temp = Temp::new();
    let path = temp.0.join(SET);
    let mut book = Workbook::new();
    let sheet = book.add_worksheet();
    sheet
        .write_row(0, 0, HEADERS)
        .unwrap()
        .write_row(
            1,
            0,
            [
                "0123456789abcdef",
                "ya",
                "Uji",
                "soal buatan saja",
                "  Uji  ›  Folder\\file.pdf  \nUji › Folder/file.pdf",
            ],
        )
        .unwrap();
    book.save(&path).unwrap();
    assert_eq!(
        read_set(&path).unwrap()[0].expected,
        HashSet::from([("Uji".into(), "Folder/file.pdf".into())])
    );
    book.worksheet_from_index(0)
        .unwrap()
        .write_string(1, 4, "Uji ›   ")
        .unwrap();
    book.save(&path).unwrap();
    assert!(read_set(&path).is_err());
}

#[tokio::test]
async fn distinct_ranking_stops_before_a_document_beyond_the_ceiling() {
    let (state, base) = crate::tests::docs_state_files(
        0,
        &[(
            "review/batas.txt",
            b"Isi dokumen buatan untuk uji.".to_vec(),
        )],
    )
    .await;
    let _base = Temp(base);
    let index = state.docs.as_ref().unwrap();
    let table = index.table();
    let (leading, trailing) = state
        .db
        .call(move |conn| {
            let id = |path: &str| {
                conn.query_row(
                    &format!("SELECT id FROM {table} WHERE rel_path = ?1"),
                    [path],
                    |row| row.get::<_, i64>(0),
                )
            };
            Ok((id("torsi.txt")?, id("batas.txt")?))
        })
        .await
        .unwrap();
    let mut writer = index.writer().unwrap();
    let acl = ["umum".to_string()];
    for (doc_id, rel_path, title, chunks) in [
        (leading, "torsi.txt", "ujiunik", 10_000),
        (trailing, "batas.txt", "batas", 1),
    ] {
        for n in 0..chunks {
            index
                .add_chunk(
                    &writer,
                    search::NewChunk {
                        doc_id,
                        n,
                        title,
                        rel_path,
                        source: "Review",
                        page: None,
                        text: "ujiunik",
                        acl: &acl,
                        file_type: "txt",
                        years: &[],
                    },
                )
                .unwrap();
        }
    }
    writer.commit().unwrap();
    writer.wait_merging_threads().unwrap();
    index.reload().unwrap();
    let beyond = index
        .search_in(
            &index.snapshot(),
            "ujiunik",
            &principal(),
            &["Review".into()],
            10_000,
            1,
            false,
            &SearchFilters::default(),
        )
        .unwrap();
    assert_eq!(beyond[0].doc_id, trailing);
    let ranked = Mode::Kata
        .rank(index, &state.db, &state.sources, "ujiunik", 2, None)
        .await
        .unwrap();
    assert_eq!(ranked, [("Review".into(), "torsi.txt".into())]);
    drop(state);
}

#[tokio::test]
async fn prepares_and_evaluates_a_synthetic_index_without_a_key() {
    const CONTENT: &str = "Torsi baut flange kelas 150 adalah 45 Nm untuk diameter M16.";
    let (state, base) = crate::tests::docs_state_files(
        0,
        &[
            ("review/salinan.txt", CONTENT.as_bytes().to_vec()),
            ("review/torsi.bin", b"format yang tidak dibaca".to_vec()),
            (
                "review/torsi-panjang.txt",
                "torsi baut flange\n".repeat(400).into_bytes(),
            ),
        ],
    )
    .await;
    let base = Temp(base);
    let data = base.0.join("data");
    fs::write(base.0.join("review/belum-diindeks.txt"), CONTENT).unwrap();
    let cfg = base.0.join("eval-test.toml");
    fs::write(
        &cfg,
        r#"
data_dir = "data"
[azure]
endpoint = "http://127.0.0.1:9"
deployment = "uji"
[[sources]]
name = "Review"
path = "review"
visibility = { type = "departemen", departments = ["Review"] }
[[sources]]
name = "Umum"
path = "umum"
visibility = { type = "umum" }
"#,
    )
    .unwrap();

    let export = data.join("eval/lama/Review");
    export_fixture(
        &export,
        json!([
            { "document_id": "asli", "bytes": CONTENT.len(),
              "sha256": hex::encode(Sha256::digest(CONTENT)) },
            { "document_id": "nama", "bytes": 24,
              "sha256": hex::encode(Sha256::digest(b"format yang tidak dibaca")) }
        ]),
        json!([
            { "question": "torsi baut flange", "question_number": 1, "chat_id": "uji",
              "question_message_id": "1", "ui_document_reference_cards": [
                { "match_status": "matched_unique", "document_ids": ["asli", "nama", "hilang"] }
              ]
            },
            { "question": "pertanyaan yang terlewat", "question_number": 2, "chat_id": "uji",
              "question_message_id": "2", "ui_document_reference_cards": [
                { "match_status": "matched_unique", "document_ids": ["asli"] }
              ]
            },
            { "question": "hanya nama dokumen", "question_number": 3, "chat_id": "uji",
              "question_message_id": "3", "ui_document_reference_cards": [
                { "match_status": "matched_unique", "document_ids": ["nama"] }
              ]
            }
        ]),
    );
    let cfg_arg = cfg.to_str().unwrap().to_owned();
    cli(
        "eval-siapkan",
        &[
            "--config".into(),
            cfg_arg.clone(),
            export.to_str().unwrap().into(),
        ],
    )
    .await
    .unwrap();
    let set = data.join("eval").join(SET);
    let mut book: Xlsx<_> = open_workbook(&set).unwrap();
    let sheet = book.worksheet_range_at(0).unwrap().unwrap();
    assert_eq!(
        sheet.get((1, 6)).and_then(DataType::as_string).as_deref(),
        Some("2 referensi lain tidak ditemukan di index")
    );
    assert!(sheet.get((2, 6)).is_none_or(DataType::is_empty));
    let questions = read_set(&set).unwrap();
    assert_eq!(questions.len(), 2);
    for question in &questions {
        assert_eq!(
            question.expected,
            HashSet::from([
                ("Review".into(), "salinan.txt".into()),
                ("Review".into(), "torsi.txt".into()),
            ])
        );
    }

    let db = Db::open_read_only(&data).unwrap();
    let index = state.docs.as_ref().unwrap();
    let chunks = documents::search(
        index.clone(),
        &db,
        &principal(),
        &state.sources,
        "torsi baut flange".into(),
        20,
        2,
        false,
        SearchFilters::default(),
    )
    .await
    .unwrap();
    assert!(
        chunks
            .iter()
            .filter(|(_, row)| row.rel_path == "torsi-panjang.txt")
            .count()
            > 1
    );
    let ranked = Mode::Kata
        .rank(index, &db, &state.sources, "torsi baut flange", 10, None)
        .await
        .unwrap();
    assert_eq!(ranked.iter().collect::<HashSet<_>>().len(), ranked.len());
    assert!(ranked.contains(&("Review".into(), "torsi.bin".into())));
    let expect_rank = first_rank(&questions[0].expected, &ranked).unwrap();
    let mut edited = Workbook::new();
    let sheet = edited.add_worksheet();
    sheet.write_row(0, 0, HEADERS).unwrap();
    for (row, question) in (1..).zip(&questions) {
        sheet
            .write_row(
                row,
                0,
                [
                    question.id.as_str(),
                    "ya",
                    question.origin.as_str(),
                    question.text.as_str(),
                    "Review › salinan.txt\nReview › torsi.txt",
                ],
            )
            .unwrap();
    }
    sheet
        .write_row(
            3,
            0,
            [
                "0123456789abcdef",
                "ya",
                "Review",
                "torsi baut flange",
                "Review › salinan.txt\nReview › dipindah.txt",
            ],
        )
        .unwrap();
    edited.save(&set).unwrap();
    cli("eval", std::slice::from_ref(&cfg_arg)).await.unwrap();
    let output = fs::read_dir(set.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap())
        .find(|entry| entry.file_name().to_string_lossy().ends_with("-kata.json"))
        .unwrap()
        .path();
    let details: Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(
        details,
        json!([
            { "id": questions[0].id, "rank": expect_rank, "k": 10 },
            { "id": questions[1].id, "rank": null, "k": 10 }
        ])
    );
    fs::write(
        base.0.join("app-model.toml"),
        "name = 'uji'\npooling = 'cls'\ndim = 2\nnormalize = true\n",
    )
    .unwrap();
    crate::embed::write_test_artifacts(&base.0);
    let model = Model::load(&base.0).unwrap();
    crate::embed::write_test_store(&data, &model, index).unwrap();
    for mode in [Mode::Makna, Mode::Gabungan] {
        let mut semantic = load_ranker(mode, &data, &model, index, &db, &state.sources, || {
            Ok(Box::new(crate::embed::FakeEmbedder))
        })
        .await
        .unwrap();
        let ranked = mode
            .rank(
                index,
                &db,
                &state.sources,
                "pengencangan",
                10,
                Some(&mut semantic),
            )
            .await
            .unwrap();
        assert_eq!(ranked.len(), ranked.iter().collect::<HashSet<_>>().len());
        assert!(ranked.contains(&("Review".into(), "torsi-panjang.txt".into())));
        assert!(ranked.contains(&("Review".into(), "torsi.bin".into())));
        run(
            (mode, Some(&model.name), Some(semantic)),
            index,
            &db,
            &state.sources,
            &set,
            10,
        )
        .await
        .unwrap();
        let output = fs::read_dir(set.parent().unwrap())
            .unwrap()
            .flatten()
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .ends_with(&format!("-{}-uji.json", mode.name()))
            })
            .unwrap();
        let details: Value = serde_json::from_slice(&fs::read(output.path()).unwrap()).unwrap();
        assert_eq!(details.as_array().unwrap().len(), 2);
        assert!(details[0]["rank"].as_u64().unwrap() <= 10);
    }
    let sheet = edited.worksheet_from_index(0).unwrap();
    sheet.write_string(1, 1, "tidak").unwrap();
    sheet.write_string(2, 1, "tidak").unwrap();
    edited.save(&set).unwrap();
    let err = cli("eval", std::slice::from_ref(&cfg_arg))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Semua baris set uji dilewati"));

    let empty_export = base.0.join("Production");
    export_fixture(&empty_export, json!([]), json!([]));
    let prepare_args = [
        "--timpa".into(),
        "--config".into(),
        cfg_arg.clone(),
        empty_export.to_str().unwrap().into(),
    ];
    let before = fs::read(&set).unwrap();
    let err = cli("eval-siapkan", &prepare_args).await.unwrap_err();
    assert!(err.to_string().contains("set uji tidak ditulis"));
    assert_eq!(fs::read(&set).unwrap(), before);
    let sheet = edited.worksheet_from_index(0).unwrap();
    sheet.write_string(3, 1, "tidak").unwrap();
    edited.save(&set).unwrap();
    let err = cli("eval", std::slice::from_ref(&cfg_arg))
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("Tidak ada baris set uji bertanda ya")
    );

    assert!(Mode::parse("lain").is_err());
    drop(db);
    drop(state);

    let (shared, index) = open_index(&data).unwrap();
    let exclusive = search::version_lock(&data).unwrap();
    assert!(matches!(
        exclusive.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    drop(index);
    drop(shared);
    exclusive.try_lock().unwrap();
    for (command, args) in [
        ("eval", std::slice::from_ref(&cfg_arg)),
        ("eval-siapkan", prepare_args.as_slice()),
    ] {
        let err = cli(command, args).await.unwrap_err();
        assert!(err.to_string().contains("Index sedang dikunci oleh ingest"));
    }
}
