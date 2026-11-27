use super::*;
use crate::{acl, agent, files::FileStore, search::DocIndex};
use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::post};
use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

fn completed(text: &str) -> Value {
    json!({
        "status":"completed","usage":{"input_tokens":11,"output_tokens":5},
        "output":[{"type":"message","content":[{"type":"output_text","text":text}]}]
    })
}

#[tokio::test]
async fn label_scan_resumes_handles_http_retry_and_keeps_archive_text_and_access() {
    let base = std::env::temp_dir().join(format!("rag-hub-label-test-{}", rand::random::<u64>()));
    let source = base.join("sumber");
    let sidecar = source.join(".extracted-text");
    fs::create_dir_all(&sidecar).unwrap();
    let path = source.join("123abcd0_lembar-sintetis.pdf");
    fs::write(&path, ingest::minimal_pdf(&["", ""])).unwrap();
    fs::write(
        sidecar.join("123abcd0.md"),
        "Zirkon sintetis hanya untuk tes.",
    )
    .unwrap();
    let mut cfg: Config = toml::from_str(&format!(
        r#"
        data_dir = {:?}
        [azure]
        endpoint = "http://127.0.0.1:9"
        deployment = "sintetis"
        [[sources]]
        name = "Uji"
        path = {:?}
        visibility = {{ type = "departemen", departments = ["Review"] }}
    "#,
        base.join("data").to_string_lossy(),
        source.to_string_lossy()
    ))
    .unwrap();
    ingest::run(&cfg, false, None).await.unwrap();
    let db = Db::open(&cfg.data_dir).unwrap();
    let old_version: i64 = db
        .call(|c| Ok(c.pragma_query_value(None, "user_version", |r| r.get(0))?))
        .await
        .unwrap();
    let id: i64 = db
        .call(|c| Ok(c.query_row("SELECT id FROM documents_v4", [], |r| r.get(0))?))
        .await
        .unwrap();
    let dry = label_scans(&cfg, &db, true, 10, async |_, _| {
        panic!("--coba mengirim permintaan")
    })
    .await
    .unwrap();
    assert_eq!((dry.dokumen, dry.halaman, dry.berlabel), (1, 2, 0));

    let index = Arc::new(DocIndex::open(&cfg.data_dir).unwrap());
    let search = |dept: &str| {
        let ctx = agent::ToolContext {
            files: FileStore::Local { dir: base.clone() },
            docs: Some(agent::DocAccess {
                index: index.clone(),
                db: db.clone(),
                sources: Arc::new(cfg.sources.clone()),
                semantic: None,
                images: None,
                user: acl::AuthUser {
                    id: 1,
                    email: String::new(),
                    name: String::new(),
                    departments: vec![dept.into()],
                    role: acl::Role::User,
                },
                session: None,
            }),
        };
        async move {
            let mut sources = Vec::new();
            let result = agent::run_tool(
                &agent::ToolCall {
                    id: "s1".into(),
                    name: "search_documents".into(),
                    arguments: r#"{"query":"zirkon"}"#.into(),
                },
                &ctx,
                120_000,
                &mut sources,
            )
            .await;
            (result.output, sources)
        }
    };
    let (before, sources) = search("Review").await;
    assert!(before.contains("hasil scan, belum dicek tulisan tangannya"));
    assert_eq!(sources[0].handwriting, Some(Handwriting::Unchecked));

    let attempts = Arc::new(AtomicUsize::new(0));
    let calls = attempts.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    cfg.azure.endpoint = format!("http://{}", listener.local_addr().unwrap());
    let llm = crate::llm::Llm::new(&cfg.azure);
    let server = tokio::spawn(async move {
        let router =
            Router::new().route(
                "/openai/v1/responses",
                post(move |Json(request): Json<Value>| {
                    let calls = calls.clone();
                    async move {
                        assert_eq!(request["reasoning"]["effort"], "low");
                        assert_eq!(
                            request["text"]["format"]["schema"]["required"],
                            json!(["jenis"])
                        );
                        assert_eq!(request["store"], false);
                        let rate_limited =
                            || {
                                (
                    StatusCode::TOO_MANY_REQUESTS, [("retry-after-ms", "0")],
                    Json(json!({"error":{"message":"uji","type":"rate_limit_error"}})),
                ).into_response()
                            };
                        match calls.fetch_add(1, Ordering::SeqCst) {
                            0 => rate_limited(),
                            1 => (StatusCode::SERVICE_UNAVAILABLE, [("retry-after-ms", "0")])
                                .into_response(),
                            2 => Json(completed(
                                r#"{"jenis":["paraf","cap","tanda_tangan","centang"]}"#,
                            ))
                            .into_response(),
                            3 => Json(completed(r#"{"jenis":["jenis_tidak_dikenal"]}"#))
                                .into_response(),
                            _ => Json(completed(r#"{"jenis":["angka","angka"]}"#)).into_response(),
                        }
                    }
                }),
            );
        axum::serve(listener, router).await.unwrap();
    });
    let run_batch = |max| label_scans(&cfg, &db, false, max, async |a, b| llm.respond(a, b).await);
    let first = run_batch(1).await.unwrap();
    assert_eq!(
        (
            first.halaman,
            first.berlabel,
            first.tulisan_tangan,
            first.panggilan
        ),
        (1, 1, 0, 1)
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    let failed = run_batch(10).await.unwrap();
    assert_eq!((failed.halaman, failed.berlabel, failed.gagal), (1, 0, 1));
    let partial = search("Review").await;
    assert_eq!(partial.1[0].handwriting, Some(Handwriting::Unchecked));
    let resumed = run_batch(10).await.unwrap();
    assert_eq!(
        serde_json::to_value(resumed).unwrap(),
        json!({
        "dokumen":1, "dokumen_gagal":0, "halaman":1, "berlabel":1, "dilewati":0, "gagal":0, "tulisan_tangan":1,
            "panggilan":1, "token_masuk":11, "token_keluar":5,
        })
    );
    assert_eq!(run_batch(10).await.unwrap().halaman, 0);
    assert_eq!(attempts.load(Ordering::SeqCst), 5);
    server.abort();
    let _ = server.await;

    let (found, sources) = search("Review").await;
    assert!(found.contains("hasil scan, tulisan tangan di hal. 2"));
    assert!(found.contains("Zirkon sintetis hanya untuk tes."));
    assert_eq!(sources[0].page, None);
    assert_eq!(
        serde_json::to_value(&sources[0]).unwrap()["handwriting"],
        json!([2])
    );
    // Kutipan lama mengikuti label baru hanya selama filenya masih revisi yang sama.
    let mut cited = sources;
    cited[0].handwriting = Some(Handwriting::Unchecked);
    refresh_citations(&db, INDEX_TABLE, cited.iter_mut())
        .await
        .unwrap();
    assert_eq!(cited[0].handwriting, Some(Handwriting::Pages(vec![2])));
    cited[0].scan_rev = Some("revisi-lain".into());
    refresh_citations(&db, INDEX_TABLE, cited.iter_mut())
        .await
        .unwrap();
    assert_eq!(cited[0].handwriting, None);
    let (blocked, sources) = search("Operasional").await;
    assert!(sources.is_empty());
    assert!(!blocked.contains("hasil scan") && !blocked.contains("hal. 2"));

    // Startup kedua mempertahankan skema lama untuk rollback dan label yang sudah tersimpan.
    let reopened = Db::open(&cfg.data_dir).unwrap();
    let docs = documents::rows(&reopened, INDEX_TABLE, vec![id])
        .await
        .unwrap();
    let read = labels(&reopened, INDEX_TABLE, docs.values()).await.unwrap();
    assert_eq!(read[&id].handwriting(Some(1)), Some(vec![]));
    assert_eq!(read[&id].handwriting(Some(2)), Some(vec![2]));
    reopened
        .call(move |c| {
            assert_eq!(
                c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?,
                old_version
            );
            let mut stmt = c.prepare(
                "SELECT count(*), sum(input_tokens) FROM usage_log WHERE kind='label_scan'",
            )?;
            assert_eq!(
                stmt.query_row([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?,
                (3, 33)
            );
            drop(stmt);
            // Simulasi SQL ingest lama, yang tidak menulis ke scan_pages.
            c.execute(
                "UPDATE documents_v4 SET size = size + 1 WHERE id = ?1",
                [id],
            )?;
            assert!(!write_page(
                c,
                id,
                1,
                2,
                &vec![Kind::Angka],
                Some(&Stamp::from_doc(&docs[&id]))
            )?);
            assert_eq!(
                c.query_row("SELECT count(*) FROM scan_pages", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
            save_page(c, id, 1, 2, &vec![Kind::Angka])?;
            c.execute("DELETE FROM documents_v4 WHERE id = ?1", [id])?;
            assert_eq!(
                c.query_row("SELECT count(*) FROM scan_pages", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
            Ok(())
        })
        .await
        .unwrap();
    drop((db, reopened, index));
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn unrenderable_pages_do_not_use_up_the_page_limit() {
    let jpx = crate::image_index::pdf_with_image(128, 128, "JPXDecode", &[1, 2, 3]);
    let pages = pdf_pages(jpx, HashSet::new(), 1, true).await.ok().unwrap();
    assert!(pages.images.iter().all(|(_, image)| image.is_none()));
    let pdf = ingest::minimal_pdf(&["satu", "dua", "tiga"]);
    let pages = pdf_pages(pdf, HashSet::from([1]), 1, true)
        .await
        .ok()
        .unwrap();
    assert_eq!(
        pages
            .images
            .iter()
            .map(|(page, _)| *page)
            .collect::<Vec<_>>(),
        [2]
    );
}

#[test]
fn notes_describe_the_scan_for_the_model() {
    assert_eq!(note(false, None), "");
    assert_eq!(note(true, None), ", hasil scan");
    assert_eq!(
        note(true, Some(&Handwriting::Pages(vec![]))),
        ", hasil scan"
    );
    assert_eq!(
        note(true, Some(&Handwriting::Pages(vec![2, 5]))),
        ", hasil scan, tulisan tangan di hal. 2, 5"
    );
    assert_eq!(
        note(true, Some(&Handwriting::Unchecked)),
        ", hasil scan, belum dicek tulisan tangannya"
    );
    // Teks instruksi tidak membawa sisa tanda dari penyuntingan.
    assert!(!MARKS.contains('+') && !TRANSCRIBE.contains('+'));
}
