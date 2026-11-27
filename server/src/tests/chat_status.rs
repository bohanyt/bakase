use super::{
    chat_trace::{Mock, after_partial, answer, completed},
    *,
};

async fn detail(app: &Router, id: &str, cookie: &str) -> Value {
    send(
        app,
        "GET",
        &format!("/api/conversations/{id}"),
        None,
        Some(cookie),
    )
    .await
    .body
}

#[tokio::test]
async fn a_cut_answer_is_marked_in_history_and_for_the_model() {
    let mut state = app_state();
    let vision = Mock::start(
        &mut state,
        vec![
            answer("Setengah jawaban."),
            answer("Sudah lengkap.") + &completed(json!([])),
        ],
        false,
    )
    .await;
    let app = router(state);
    let cookie = admin_cookie(&app).await;
    let (_, sse) = chat(&app, &cookie, json!({ "message": "Uji terpotong" })).await;
    assert!(sse.contains("event: error") && sse.contains("event: done"));
    let id = conversation_id(&sse);
    let (_, sse) = chat(
        &app,
        &cookie,
        json!({ "conversation_id": id, "message": "Lanjutkan" }),
    )
    .await;
    assert!(!sse.contains("event: error"));

    let messages = detail(&app, &id, &cookie).await["messages"].clone();
    assert_eq!(messages[1]["content"], "Setengah jawaban.");
    assert_eq!(messages[1]["partial"], true);
    assert_eq!(messages[3]["content"], "Sudah lengkap.");
    assert!(messages[3].get("partial").is_none());

    let requests = vision.requests.lock().unwrap();
    let history = requests[1]["input"].to_string();
    assert!(
        history.contains("[Jawaban ini berhenti sebelum selesai.]"),
        "{history}"
    );
}

#[tokio::test]
async fn an_answer_that_cannot_be_saved_is_reported() {
    let mut state = app_state();
    let _vision = Mock::start(
        &mut state,
        vec![answer("Jawaban uji.") + &completed(json!([]))],
        false,
    )
    .await;
    state
        .db
        .call(|c| {
            Ok(c.execute_batch(
                "CREATE TRIGGER tolak BEFORE INSERT ON messages WHEN NEW.role = 'assistant'
                 BEGIN SELECT RAISE(ABORT, 'uji'); END;",
            )?)
        })
        .await
        .unwrap();
    let app = router(state);
    let cookie = admin_cookie(&app).await;
    let (_, sse) = chat(&app, &cookie, json!({ "message": "Uji simpan" })).await;
    assert!(sse.contains("Jawaban uji."));
    assert!(sse.contains("Jawaban tidak tersimpan."), "{sse}");
    assert!(sse.contains("event: done"));
}

#[tokio::test]
async fn a_repeated_send_is_stored_once_and_returns_its_conversation() {
    let mut state = app_state();
    let _vision = Mock::start(
        &mut state,
        vec![
            answer("Jawaban pertama.") + &completed(json!([])),
            answer("Jawaban kedua.") + &completed(json!([])),
        ],
        false,
    )
    .await;
    let app = router(state);
    let admin = admin_cookie(&app).await;
    let (_, other) = active_user(&app, &admin, "alex@example.com").await;
    let body = json!({ "message": "Uji kirim ulang", "turn_id": "0123456789abcdef" });
    let (_, sse) = chat(&app, &admin, body.clone()).await;
    let id = conversation_id(&sse);
    let (status, again) = chat(&app, &admin, body.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(conversation_id(&again), id);
    assert!(again.contains("sudah diterima"), "{again}");
    assert!(again.contains("\"question_id\":"));
    assert!(!again.contains("event: done"));
    assert_eq!(
        detail(&app, &id, &admin).await["messages"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // ID yang sama dari user lain adalah giliran lain.
    let (_, sse) = chat(&app, &other, body).await;
    assert_ne!(conversation_id(&sse), id);
    assert!(sse.contains("Jawaban kedua."));
    for bad in ["pendek", "0123456789abcdef; DROP", &"a".repeat(65)] {
        let (status, _) = chat(&app, &admin, json!({ "message": "x", "turn_id": bad })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
}

#[tokio::test]
async fn a_changed_department_stops_the_running_turn_before_the_next_search() {
    let (mut state, base) = docs_state().await;
    let search = |id: &str| {
        completed(
            json!([{"type": "function_call", "call_id": id, "name": "search_documents",
            "arguments": "{\"query\":\"torsi\"}"}]),
        )
    };
    let gate = Arc::new(tokio::sync::Notify::new());
    let _vision = Mock::gated(
        &mut state,
        vec![
            answer("Potongan uji.") + &search("satu"),
            search("dua"),
            answer("Tidak boleh sampai sini.") + &completed(json!([])),
        ],
        false,
        Some(gate.clone()),
    )
    .await;
    let app = router(state.clone());
    let admin = admin_cookie(&app).await;
    let (id, cookie) = active_user(&app, &admin, "alex@example.com").await;
    let (conversation, body) =
        after_partial(&app, &cookie, json!({ "message": "Cari torsi" })).await;
    let changed = send(
        &app,
        "POST",
        &format!("/api/admin/users/{id}"),
        Some(json!({ "status": "active", "department": "Operasional" })),
        Some(&admin),
    )
    .await;
    assert!(changed.status.is_success(), "{:?}", changed.body);
    gate.notify_one();
    let rest = body.collect().await.unwrap().to_bytes();
    let rest = std::str::from_utf8(&rest).unwrap();
    assert!(rest.contains("Akses akunmu berubah."), "{rest}");
    assert!(!rest.contains("Tidak boleh sampai sini."));
    state.traces.flush().await;
    let reason: String = state
        .db
        .call(move |c| {
            Ok(c.query_row(
                "SELECT t.reason FROM turn_traces t JOIN messages m ON m.id = t.question_id
                 WHERE m.conversation_id = ?1",
                [conversation],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(reason, "access_changed");
    drop((app, state));
    let _ = std::fs::remove_dir_all(base);
}

#[tokio::test]
async fn a_tab_showing_another_account_cannot_write() {
    let app = app();
    let admin = admin_cookie(&app).await;
    let admin_id = user_id(&app, &admin, ADMIN_EMAIL).await;
    let (alex, _) = active_user(&app, &admin, "alex@example.com").await;
    let post = |path: &'static str, tab_user: Option<String>| {
        let (app, cookie) = (app.clone(), admin.clone());
        async move {
            let mut req = Request::post(path)
                .header(header::COOKIE, cookie)
                .header(header::CONTENT_TYPE, "application/json");
            if let Some(id) = tab_user {
                req = req.header("X-app-User", id);
            }
            let body = Body::from(json!({ "message": "x" }).to_string());
            app.oneshot(req.body(body).unwrap()).await.unwrap().status()
        }
    };
    for path in ["/api/chat", "/api/attachments"] {
        for wrong in [alex.to_string(), "bukan-angka".into()] {
            let status = post(path, Some(wrong)).await;
            assert_eq!(status, StatusCode::CONFLICT, "{path}");
        }
    }
    assert_eq!(
        post("/api/chat", Some(admin_id.to_string())).await,
        StatusCode::OK
    );
    assert_eq!(post("/api/chat", None).await, StatusCode::OK);
}
