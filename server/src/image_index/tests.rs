use super::*;
use crate::acl::Role;

pub(super) struct Temp(pub(super) PathBuf);

impl Temp {
    pub(super) fn new() -> Self {
        let path = std::env::temp_dir().join(format!("app-test-gambar-{}", rand::random::<u64>()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn resume_commits_whole_files_and_cleans_last_reference() {
    let dir = Temp::new();
    let path = dir.0.join("uji.sqlite");
    let hash: Hash = [21; 32];
    let first: Hash = [1; 32];
    let second: Hash = [2; 32];
    {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.begin(1, &first).unwrap(), FileState::Work);
    }
    let mut store = Store::open(&path).unwrap();
    store
        .reconcile(&HashMap::from([(1, first), (2, second)]))
        .unwrap();
    assert_eq!(store.begin(1, &first).unwrap(), FileState::Work);
    assert!(!store.has(&hash).unwrap());
    assert!(store::document(&path, 1, &first).unwrap().is_empty());
    store
        .finish(
            1,
            Vectors::from([(hash, (vec!["halaman:1".into()], vec![0.0; eg2::DIM]))]),
        )
        .unwrap();
    assert_eq!(store.begin(1, &first).unwrap(), FileState::Done);
    store.begin(2, &second).unwrap();
    store
        .finish(
            2,
            Vectors::from([(hash, (vec!["xl/media/foto.png".into()], Vec::new()))]),
        )
        .unwrap();
    assert_eq!(store.counts().unwrap(), (1, 2));
    assert_eq!(store::document(&path, 1, &first).unwrap().len(), 1);
    assert!(store::document(&path, 1, &second).unwrap().is_empty());
    store.reconcile(&HashMap::from([(2, second)])).unwrap();
    assert_eq!(store.counts().unwrap(), (1, 1));
    store.reconcile(&HashMap::new()).unwrap();
    assert!(!store.has(&hash).unwrap());
}

#[test]
fn failed_and_interrupted_files_are_retried_only_after_a_change() {
    let dir = Temp::new();
    let path = dir.0.join("uji.sqlite");
    let first = [1; 32];
    let changed = [2; 32];
    {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.begin(1, &first).unwrap(), FileState::Work);
        store.fail(1).unwrap();
        for _ in 0..3 {
            assert_eq!(store.begin(2, &first).unwrap(), FileState::Work);
        }
    }
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.begin(1, &first).unwrap(), FileState::Failed);
    assert_eq!(store.begin(2, &first).unwrap(), FileState::Failed);
    assert_eq!(store.begin(3, &first).unwrap(), FileState::Work);
    store
        .reconcile(&HashMap::from([(1, changed), (2, changed)]))
        .unwrap();
    assert_eq!(store.begin(1, &changed).unwrap(), FileState::Work);
    assert_eq!(store.begin(2, &changed).unwrap(), FileState::Work);
}

fn model_artifacts(folder: &Path) -> Tokens {
    fs::create_dir(folder).unwrap();
    for file in ["model.onnx", "vision_encoder.onnx"] {
        fs::write(folder.join(file), []).unwrap();
    }
    fs::write(
        folder.join("config.json"),
        r#"{
        "text_config":{"bos_token_id":0,"embedding_dim":768},
        "boi_token_id":1,"image_token_id":2,"eoi_token_id":3
    }"#,
    )
    .unwrap();
    fs::write(
        folder.join("processor_config.json"),
        r#"{"image_processor":{
        "do_convert_rgb":true,"do_resize":true,"do_rescale":true,"do_normalize":false,
        "resample":3,"patch_size":16,"pooling_kernel_size":3,"max_soft_tokens":280,
        "rescale_factor":0.00392156862745098
    }}"#,
    )
    .unwrap();
    let vocab = ["<bos>", "<|image>", "<|image|>", "<image|>"]
        .into_iter()
        .enumerate()
        .map(|(i, value)| (value.into(), i as u32))
        .collect();
    let model = tokenizers::models::wordlevel::WordLevel::builder()
        .vocab(vocab)
        .build()
        .unwrap();
    tokenizers::Tokenizer::new(model)
        .save(folder.join("tokenizer.json"), false)
        .unwrap();
    Tokens::load(folder).unwrap()
}

#[test]
fn stale_artifact_store_is_removed_without_touching_current_or_other_files() {
    let dir = Temp::new();
    let current = dir.0.join(format!("{}.sqlite", "a".repeat(64)));
    for name in [
        current.clone(),
        dir.0.join(format!("{}.sqlite-wal", "b".repeat(64))),
        dir.0.join(format!("{}.sqlite", "b".repeat(64))),
        dir.0.join("other.sqlite"),
    ] {
        fs::write(name, []).unwrap();
    }
    remove_old_stores(&current).unwrap();
    assert!(current.is_file());
    assert!(dir.0.join("other.sqlite").is_file());
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
}

#[cfg(windows)]
#[test]
fn publication_keeps_a_generation_open_in_the_other_slot() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = Temp::new();
    let old = dir.0.join(format!("{}.sqlite", "a".repeat(64)));
    let new = dir.0.join(format!("{}.sqlite", "b".repeat(64)));
    let old_wal = old.with_extension("sqlite-wal");
    fs::write(&old, []).unwrap();
    fs::write(&old_wal, []).unwrap();
    let serving = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&old)
        .unwrap();
    fs::write(building_marker(&new), []).unwrap();
    fs::write(&new, []).unwrap();
    assert!(published(&old));
    assert!(!published(&new));
    fs::remove_file(building_marker(&new)).unwrap();
    assert!(published(&new));
    remove_old_stores(&new).unwrap();
    assert!(old.is_file() && old_wal.is_file());
    drop(serving);
    remove_old_stores(&new).unwrap();
    assert!(!old.exists() && !old_wal.exists());
}

#[test]
fn processing_continues_after_panic_and_discards_stale_file_results() {
    let dir = Temp::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let make = |name: &str, id| {
        let path = dir.0.join(name);
        RgbImage::from_pixel(128, 128, image::Rgb([id as u8, 40, 60]))
            .save(&path)
            .unwrap();
        let (size, mtime) = ingest::file_stamp(&fs::metadata(&path).unwrap());
        Target {
            id,
            path,
            stamp: stamp("uji", name, size, mtime),
            size,
            mtime,
            kind: Kind::Direct,
        }
    };
    let broken = make("rusak.png", 1);
    let changing = make("berubah.png", 2);
    let good = make("valid.png", 3);
    let modified = changing.path.clone();
    let mut invocations = 0;
    let mut work = Work {
        store: Store::open(&dir.0.join("uji.sqlite")).unwrap(),
        capacity: 2,
        encode: move |batch: Vec<Prepared>| {
            invocations += 1;
            match invocations {
                1 => panic!("decoder sintetis panik"),
                2 => fs::write(&modified, [0]).unwrap(),
                _ => {}
            }
            Ok(vec![vec![0.0; eg2::DIM]; batch.len()])
        },
        pending: Vec::new(),
        vectors: Vectors::new(),
    };
    assert!(matches!(
        work.process(&runtime, &broken).unwrap(),
        Outcome::Processed { failed: true, .. }
    ));
    assert!(matches!(
        work.process(&runtime, &broken).unwrap(),
        Outcome::Failed
    ));
    assert!(matches!(
        work.process(&runtime, &changing).unwrap(),
        Outcome::Changed
    ));
    assert!(matches!(
        work.process(&runtime, &good).unwrap(),
        Outcome::Processed {
            embedded: 1,
            failed: false,
            ..
        }
    ));
    assert_eq!(work.store.counts().unwrap(), (1, 1));
    assert!(matches!(
        work.process(&runtime, &good).unwrap(),
        Outcome::Unchanged
    ));

    let same = RgbImage::new(128, 128);
    assert!(!work.add("satu".into(), same.clone()).unwrap());
    assert!(work.add("dua".into(), same).unwrap());
    assert_eq!(work.pending.len(), 1);
    work.flush().unwrap();
    assert_eq!(work.vectors.values().next().unwrap().0, ["satu", "dua"]);
}

#[tokio::test]
async fn image_reference_uses_source_document_access() {
    let temp = Temp::new();
    let config = temp.0.join("uji.toml");
    fs::write(
        &config,
        r#"
data_dir = "data"
[azure]
endpoint = "http://127.0.0.1:1"
deployment = "uji"
[gambar]
model = "model"
[[sources]]
name = "Dokumen Uji"
path = "sumber"
visibility = { type = "admin" }
"#,
    )
    .unwrap();
    let restricted = Config::load(&config).unwrap();
    let mut cfg = restricted.clone();
    let root = cfg.sources[0].path.clone();
    fs::create_dir(&root).unwrap();
    let name = "uji.png";
    let path = root.join(name);
    RgbImage::from_pixel(128, 128, image::Rgb([20, 40, 60]))
        .save(&path)
        .unwrap();
    fs::write(root.join("contoh.txt"), "Torsi sintetis Contoh").unwrap();
    ingest::run(&restricted, false, None).await.unwrap();
    cfg.sources[0].visibility = Visibility::Departemen(vec!["Contoh".into()]);
    assert_eq!(targets(&cfg).unwrap().len(), 1);
    let db = Db::open_read_only(&cfg.data_dir).unwrap();
    let id: i64 = db
        .call(|conn| {
            Ok(conn.query_row(
                &format!("SELECT id FROM {INDEX_TABLE} WHERE rel_path = 'uji.png'"),
                [],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap();
    let model = cfg.gambar.as_ref().unwrap();
    let tokens = model_artifacts(&model.model);
    let output = store_path(&cfg.data_dir, model, tokens).unwrap();
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    let mut store = Store::open(&output).unwrap();
    let (size, mtime) = ingest::file_stamp(&fs::metadata(&path).unwrap());
    let root = root.canonicalize().unwrap();
    let stamp = stamp(&root.to_string_lossy(), name, size, mtime);
    store.begin(id, &stamp).unwrap();
    store
        .finish(
            id,
            Vectors::from([([42; 32], (vec![String::new()], vec![0.0; eg2::DIM]))]),
        )
        .unwrap();
    let (_lock, index) = crate::eval::open_index(&cfg.data_dir).unwrap();
    let index = std::sync::Arc::new(index);
    let mut user = AuthUser {
        id: 1,
        email: "uji@example.test".into(),
        name: "Uji".into(),
        role: Role::User,
        departments: vec!["Operasional".into()],
    };
    assert!(
        for_document(&cfg, &db, &index, &user, id)
            .await
            .unwrap()
            .is_empty()
    );
    user.departments.push("Contoh".into());
    let assert_index_access = |expected_count| {
        let names = ["Dokumen Uji".to_string()];
        assert_eq!(
            index
                .search("torsi", &user, &names, 0, 10, false)
                .unwrap()
                .len(),
            expected_count
        );
        let snapshot = index.snapshot();
        let (allowed, _) = index
            .semantic_addresses(
                &snapshot,
                &user,
                &names,
                &crate::search::SearchFilters::default(),
            )
            .unwrap();
        assert_eq!(allowed.len(), expected_count * 2);
    };
    assert_index_access(0);
    assert!(
        for_document(&cfg, &db, &index, &user, id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(ingest::run(&cfg, false, None).await.unwrap().read, 2);
    index.reload().unwrap();
    assert_index_access(1);
    let allowed = for_document(&cfg, &db, &index, &user, id).await.unwrap();
    assert_eq!(allowed.len(), 1);
    assert_eq!(allowed[0].location, "");
    assert_eq!(allowed[0].vector.len(), eg2::DIM);
    assert!(
        for_document(&restricted, &db, &index, &user, id)
            .await
            .unwrap()
            .is_empty()
    );
    let mut moved = cfg.clone();
    moved.sources[0].path = temp.0.join("sumber-baru");
    fs::create_dir(&moved.sources[0].path).unwrap();
    assert!(targets(&moved).unwrap().is_empty());
    assert!(
        for_document(&moved, &db, &index, &user, id)
            .await
            .unwrap()
            .is_empty()
    );

    let write_db = Db::open(&cfg.data_dir).unwrap();
    let public = serde_json::to_string(&Visibility::Umum).unwrap();
    write_db
        .call(move |conn| {
            conn.execute(&format!("UPDATE {INDEX_TABLE} SET visibility=?"), [public])?;
            Ok(())
        })
        .await
        .unwrap();
    let mut public_cfg = cfg.clone();
    public_cfg.sources[0].visibility = Visibility::Umum;
    user.departments = vec!["Operasional".into()];
    assert!(
        for_document(&public_cfg, &db, &index, &user, id)
            .await
            .unwrap()
            .is_empty()
    );
    write_db
        .call(move |conn| {
            conn.execute(
                &format!("UPDATE {INDEX_TABLE} SET status=?"),
                [ingest::status::PENDING],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    user.departments = vec!["Contoh".into()];
    assert!(
        for_document(&cfg, &db, &index, &user, id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(targets(&cfg).unwrap().is_empty());
    write_db
        .call(move |conn| {
            conn.execute(
                &format!("UPDATE {INDEX_TABLE} SET status=?, version=?"),
                rusqlite::params![ingest::status::SCAN, ingest::NAME_ONLY_VERSION],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        for_document(&cfg, &db, &index, &user, id)
            .await
            .unwrap()
            .len(),
        1
    );
    fs::remove_file(path).unwrap();
    assert!(
        for_document(&cfg, &db, &index, &user, id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test(start_paused = true)]
async fn same_image_hash_does_not_reveal_a_private_reference_or_thumbnail() {
    use axum::{
        body::Body,
        http::{Request, StatusCode, header},
    };
    use http_body_util::BodyExt;
    use rusqlite::params;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tower::ServiceExt;

    // Database replies arrive from outside Tokio. Keep the paused clock from
    // advancing to a request deadline while a reply is in flight.
    let (_hold_clock, release_clock) = std::sync::mpsc::channel::<()>();
    tokio::task::spawn_blocking(move || {
        let _ = release_clock.recv();
    });

    struct SyntheticText(Vec<f32>);
    impl crate::embed::Embedder for SyntheticText {
        fn embed(&mut self, input: &[String]) -> Result<Vec<Vec<f32>>> {
            assert_eq!(input, ["task: search result | query: lingkaran"]);
            Ok(vec![self.0.clone()])
        }
    }

    let temp = Temp::new();
    let config = temp.0.join("uji.toml");
    fs::write(
        &config,
        r#"
data_dir = "data"
[azure]
endpoint = "http://127.0.0.1:1"
deployment = "uji"
[gambar]
model = "model"
[[sources]]
name = "Sumber Terbatas"
path = "terbatas"
visibility = { type = "departemen", departments = ["Operasional"] }
[[sources]]
name = "Sumber Review"
path = "review"
visibility = { type = "departemen", departments = ["Review"] }
"#,
    )
    .unwrap();
    let cfg = Config::load(&config).unwrap();
    let image = RgbImage::from_pixel(128, 128, image::Rgb([20, 40, 60]));
    let index = Arc::new(DocIndex::in_memory_v4().unwrap());
    let db = Db::open_in_memory().unwrap();
    let mut writer = index.writer().unwrap();
    let mut docs = Vec::new();
    for (n, (source, name)) in cfg
        .sources
        .iter()
        .zip(["rahasia.png", "contoh.png"])
        .enumerate()
    {
        fs::create_dir(&source.path).unwrap();
        let path = source.path.join(name);
        image.save(&path).unwrap();
        let (size, mtime) = ingest::file_stamp(&fs::metadata(&path).unwrap());
        let root = source
            .path
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let id = n as i64 + 1;
        index
            .add_chunk(
                &writer,
                crate::search::NewChunk {
                    doc_id: id,
                    n: 0,
                    title: name,
                    rel_path: name,
                    source: &source.name,
                    page: None,
                    text: "",
                    acl: &crate::search::acl_labels(&source.visibility),
                    file_type: "gambar/scan",
                    years: &[],
                },
            )
            .unwrap();
        docs.push(Target {
            id,
            path,
            size,
            mtime,
            stamp: stamp(&root, name, size, mtime),
            kind: Kind::Direct,
        });
        let source = source.clone();
        db.call(move |c| {
            c.execute(&format!("INSERT INTO {INDEX_TABLE}
                (id, source, rel_path, title, kind, status, size, mtime, visibility, version, indexed_at, root)
                VALUES (?, ?, ?, ?, 'png', 'ok', ?, ?, ?, 0, 0, ?)"),
                params![id, source.name, name, name, size, mtime, serde_json::to_string(&source.visibility)?, root])?;
            Ok(())
        }).await.unwrap();
    }
    writer.commit().unwrap();
    writer.wait_merging_threads().unwrap();
    index.reload().unwrap();
    let [private, public] = docs.as_slice() else {
        panic!("dua dokumen uji diperlukan");
    };
    assert!(private.path.ends_with("rahasia.png"));
    let model = cfg.gambar.as_ref().unwrap();
    let tokens = model_artifacts(&model.model);
    let store_path = store_path(&cfg.data_dir, model, tokens).unwrap();
    fs::create_dir_all(store_path.parent().unwrap()).unwrap();
    let mut store = Store::open(&store_path).unwrap();
    let hash = image_hash(&image);
    let mut vector = vec![0.0; eg2::DIM];
    vector[0] = 1.0;
    for (doc, values) in [(private, vector.clone()), (public, Vec::new())] {
        store.begin(doc.id, &doc.stamp).unwrap();
        store
            .finish(
                doc.id,
                Vectors::from([(hash, (vec![String::new()], values))]),
            )
            .unwrap();
    }
    let mut state = crate::tests::state_with(db.clone(), Some(index), cfg.sources);
    let live = Live::new(model.clone(), cfg.data_dir);
    live.test_text(SyntheticText(vector));
    Arc::get_mut(&mut state).unwrap().images = Some(live.clone());
    let app = crate::router(state.clone());
    let admin = crate::tests::admin_cookie(&app).await;
    let (user_id, review) = crate::tests::active_user(&app, &admin, "uji-gambar@example.com").await;
    let request = |uri, cookie: String, json: Option<Value>| {
        let router = app.clone();
        async move {
            let mut request = Request::builder().uri(uri).header(header::COOKIE, cookie);
            let body = if let Some(json) = json {
                request = request
                    .method("POST")
                    .header(header::CONTENT_TYPE, "application/json");
                Body::from(json.to_string())
            } else {
                Body::empty()
            };
            router.oneshot(request.body(body).unwrap()).await.unwrap()
        }
    };
    let key = hex::encode(hash);
    let private_url = format!("/api/documents/{}/images/{key}/thumbnail", private.id);
    let public_url = format!(
        "/api/documents/{}/images/{key}/thumbnail?location=rahasia",
        public.id
    );
    let old = request(private_url.clone(), admin, None).await;
    assert_eq!(old.status(), StatusCode::OK);
    assert_eq!(old.headers()[header::CACHE_CONTROL], "private, no-store");
    let denied = request(private_url, review.clone(), None).await;
    assert_eq!(denied.status(), StatusCode::NOT_FOUND);
    let allowed = request(public_url, review.clone(), None).await;
    assert_eq!(allowed.status(), StatusCode::OK);
    assert_eq!(allowed.headers()[header::CONTENT_TYPE], "image/jpeg");

    let found = request(
        "/api/documents/search".into(),
        review.clone(),
        Some(json!({"query":"lingkaran"})),
    )
    .await;
    assert_eq!(found.status(), StatusCode::OK);
    let json: Value =
        serde_json::from_slice(&found.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let images = json["images"].as_array().unwrap();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0]["doc_id"], public.id);
    assert!(!json.to_string().contains("Sumber Terbatas"));
    mock_vision_with_images(&state, live, user_id, &temp.0).await;
    let mut evidence = images[0].clone();
    evidence["n"] = json!(1);
    evidence["image"]["evidence_only"] = json!(true);
    db.call(move |c| {
        c.execute(
            "INSERT INTO conversations VALUES ('uji-gambar', ?, 'Temuan uji', 0, 0)",
            [user_id],
        )?;
        c.execute(
            "INSERT INTO messages (conversation_id, role, content, created_at, sources)
             VALUES ('uji-gambar', 'assistant', 'Ringkasan rahasia sintetis.', 0, ?)",
            params![json!([evidence]).to_string()],
        )?;
        c.execute(
            "INSERT INTO conversation_compactions (conversation_id, through_message_id, content)
             VALUES ('uji-gambar', ?1, 'Ringkasan obrolan dengan bukti gambar sintetis.')",
            [c.last_insert_rowid()],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let detail = "/api/conversations/uji-gambar".to_string();
    let allowed = request(detail.clone(), review.clone(), None).await;
    let allowed = allowed.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&allowed).contains("Ringkasan rahasia sintetis."));
    assert!(
        String::from_utf8_lossy(&allowed)
            .contains("Ringkasan obrolan dengan bukti gambar sintetis.")
    );
    let search = request(
        "/api/conversations/search".into(),
        review.clone(),
        Some(json!({"q":"rahasia sintetis"})),
    )
    .await;
    let result: Value =
        serde_json::from_slice(&search.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(result[0]["snippet"], "Ringkasan rahasia sintetis.");
    db.call(move |c| {
        let tx = c.transaction()?;
        crate::access::replace(&tx, user_id, &["Umum".into()])?;
        tx.commit()?;
        Ok(())
    })
    .await
    .unwrap();
    let login = request(
        "/api/auth/login".into(),
        String::new(),
        Some(json!({"email": "uji-gambar@example.com", "password": crate::tests::ACTIVE_USER_PW})),
    )
    .await;
    assert_eq!(login.status(), StatusCode::OK);
    let review = login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let blocked = request(detail, review.clone(), None).await;
    assert_eq!(blocked.status(), StatusCode::OK);
    let blocked = blocked.into_body().collect().await.unwrap().to_bytes();
    let blocked = String::from_utf8_lossy(&blocked);
    assert!(!blocked.contains("Ringkasan rahasia sintetis."));
    assert!(!blocked.contains("Ringkasan obrolan dengan bukti gambar sintetis."));
    assert!(!blocked.contains("Sumber Review"));
    let search = request(
        "/api/conversations/search".into(),
        review,
        Some(json!({"q":"rahasia sintetis"})),
    )
    .await;
    assert_eq!(search.into_body().collect().await.unwrap().to_bytes(), "[]");
}

async fn mock_vision_with_images(
    state: &std::sync::Arc<crate::AppState>,
    images: std::sync::Arc<Live>,
    user_id: i64,
    directory: &Path,
) {
    use crate::{
        agent,
        files::FileStore,
        llm::{ChatMessage, Llm},
        page,
    };
    use axum::{Json, Router, http::header, routing::post};
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};

    let requests = Arc::new(Mutex::new(Vec::new()));
    let collected = requests.clone();
    let service = Router::new().route(
        "/openai/v1/responses",
        post(move |Json(body): Json<Value>| {
            let collected = collected.clone();
            async move {
                let mut requests = collected.lock().unwrap();
                requests.push(body);
                let turn = requests.len();
                let output = match turn {
                    1 | 3 => json!([{
                        "type": "function_call", "name":"search_documents",
                        "call_id":format!("uji_{turn}"),
                        "arguments":json!({"query":"lingkaran"}).to_string(),
                    }]),
                    2 => json!([{
                        "type": "function_call", "name":"sorot", "call_id":"uji_sorot",
                        "arguments":json!({"ref":"e1"}).to_string(),
                    }]),
                    _ => json!([]),
                };
                let tokens = [41, 101, 43, 57].get(turn - 1).copied().unwrap_or(0);
                let done = json!({"type":"response.completed","response":{
                    "output":output, "usage":{"input_tokens":tokens,"output_tokens":8}
                }});
                let content = if turn == 4 {
                    format!(
                        "data: {}\n\n",
                        json!({"type":"response.output_text.delta","delta":"Contoh gambar [1]."})
                    )
                } else {
                    String::new()
                };
                (
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    format!("{content}data: {done}\n\ndata: [DONE]\n\n"),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, service).await });
    let llm = Llm::new(&crate::config::AzureConfig {
        endpoint: format!("http://{address}"),
        deployment: "sintetis".into(),
        api_version: "2024-10-21".into(),
        api_key: "sintetis".into(),
        context_tokens: 4096,
    });
    let user = AuthUser {
        id: user_id,
        email: "uji@example.test".into(),
        name: "Uji".into(),
        role: Role::User,
        departments: vec!["Review".into()],
    };
    let context = agent::ToolContext {
        files: FileStore::Local {
            dir: directory.into(),
        },
        docs: Some(agent::DocAccess {
            semantic: None,
            images: Some(images),
            db: state.db.clone(),
            index: state.docs.as_ref().unwrap().clone(),
            sources: state.sources.clone(),
            user: user.clone(),
            session: None,
        }),
    };
    let page_request = serde_json::from_value(json!({
        "origin": format!("chrome-extension://{}", "a".repeat(32)),
        "attachment_id": "b".repeat(32),
        "text": "Panel sintetis [e1]",
    }))
    .unwrap();
    let page_turn = page::Turn::new(state.page_actions.clone(), user_id, page_request);
    let (send, mut receive) = tokio::sync::mpsc::channel(32);
    let run = async move {
        agent::run(
            &llm,
            "",
            &[ChatMessage {
                role: "user".into(),
                content: "lingkaran".into(),
                attachments: Vec::new(),
            }],
            &context,
            &send,
            Some(page_turn),
            &mut crate::turn_trace::Turn::default(),
        )
        .await
    };
    let browser = async {
        let mut saw_action = false;
        while let Some(event) = receive.recv().await {
            if let agent::Event::PageAction(action) = event {
                let action: Value = serde_json::from_str(&action.into_data()).unwrap();
                assert_eq!(action["action"], "sorot");
                assert_eq!(action["ref"], "e1");
                let completion = serde_json::from_value(json!({
                    "id": action["id"], "reply": { "status": "highlighted" },
                }))
                .unwrap();
                page::complete(
                    axum::extract::State(state.clone()),
                    user.clone(),
                    Json(completion),
                )
                .await
                .unwrap();
                saw_action = true;
            }
        }
        assert!(saw_action, "aksi halaman tidak terkirim");
    };
    let (answer, ()) = tokio::join!(run, browser);
    server.abort();
    assert!(answer.error.is_none(), "{:?}", answer.error);
    assert_eq!(answer.usage.input, 242);
    assert_eq!(answer.usage.reported, 4);
    assert!(
        answer
            .steps
            .iter()
            .any(|step| step.label == "Elemen disorot" && !step.failed)
    );
    assert_eq!(answer.sources.len(), 1);
    assert!(answer.sources[0].visible());
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0]["parallel_tool_calls"], false);
    let tools = requests[0]["tools"].as_array().unwrap();
    for name in ["search_documents", "sorot", "gulir"] {
        assert!(tools.iter().any(|tool| tool["name"] == name));
    }
    assert!(
        requests[0]["input"]
            .to_string()
            .contains("Panel sintetis [e1]")
    );
    let content = requests[1]["input"].as_array().unwrap().last().unwrap()["output"]
        .as_array()
        .unwrap();
    let mut photos = content.iter().filter(|item| item["type"] == "input_image");
    let photo = photos.next().unwrap();
    assert_eq!(photo["detail"], "low");
    assert!(
        photo["image_url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/jpeg;base64,")
    );
    assert!(photos.next().is_none());
    assert!(
        requests[2]["input"]
            .to_string()
            .contains("Elemen disorot sementara.")
    );
    assert!(
        requests[2..]
            .iter()
            .all(|req| !req["input"].to_string().contains("data:image/jpeg;base64,"))
    );
    assert!(!requests[1]["input"].to_string().contains("Sumber Terbatas"));
}
