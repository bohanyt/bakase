use super::*;
use axum::{response::IntoResponse, routing::post};
use std::sync::Mutex;
use tokio::sync::Notify;

const RECAP: &str = "Ringkasan uji: target 12 sampel. Jadwal pengukuran masih terbuka.";
const BUDGET: usize = 2048;

#[tokio::test]
#[ignore = "pengukuran ACL dengan riwayat dan indeks sintetis"]
async fn history_acl_200_turns_stay_fast() {
    let (state, _fixture) = docs_state_with(100).await;
    let app = router(state.clone());
    let admin = admin_cookie(&app).await;
    let (user_id, _) = active_user(&app, &admin, "ukur-ringkasan@example.com").await;
    let turn = conversations::begin_turn(&state, user_id, None, "Awal uji".into(), vec![])
        .await
        .unwrap();
    let id = turn.id;
    let seed_id = id.clone();
    state.db.call(move |c| {
        let tx = c.transaction()?;
        let ids: Vec<i64> = tx.prepare("SELECT id FROM documents_v4 WHERE rel_path LIKE 'm16-%' ORDER BY id")?
            .query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        assert_eq!(ids.len(), 100);
        for n in 0..200 {
            let doc = ids[n % ids.len()];
            let sources = json!([{
                "n": 1, "chunk_id": format!("uji-{doc}"), "doc_id": doc, "title": "Uji",
                "source":"Review", "kind":"txt", "page":null,
            }]).to_string();
            tx.execute("INSERT INTO messages (conversation_id, role, content, created_at) VALUES (?1, 'user', 'Siapkan uji', 0)", [&seed_id])?;
            tx.execute("INSERT INTO messages (conversation_id, role, content, created_at, sources) VALUES (?1, 'assistant', 'Siap [1]', 0, ?2)", rusqlite::params![seed_id, sources])?;
        }
        tx.commit()?;
        Ok(())
    }).await.unwrap();
    let mut elapsed = Vec::new();
    for n in 0..8 {
        let start = std::time::Instant::now();
        let turn = conversations::begin_turn(
            &state,
            user_id,
            Some(id.clone()),
            "Lanjut uji".into(),
            vec![],
        )
        .await
        .unwrap();
        assert!(turn.first_revoked.is_none());
        if n > 0 {
            elapsed.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    elapsed.sort_by(f64::total_cmp);
    println!(
        "ACL 200 giliran, 100 dokumen: median {:.2} ms, min {:.2} ms, max {:.2} ms",
        elapsed[elapsed.len() / 2],
        elapsed[0],
        elapsed[elapsed.len() - 1]
    );
}

struct Mockmodel {
    requests: Arc<Mutex<Vec<Value>>>,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
}

#[derive(Default)]
struct SummaryGate {
    stage: usize,
    received: Notify,
    release: Notify,
}

impl Mockmodel {
    async fn start(state: &mut Arc<AppState>, failure: bool) -> Self {
        Self::start_with_gate(state, failure, None).await
    }

    async fn start_with_gate(
        state: &mut Arc<AppState>,
        failure: bool,
        gate: Option<Arc<SummaryGate>>,
    ) -> Self {
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let collected = requests.clone();
        let service = Router::new().route(
            "/openai/v1/responses",
            post(move |Json(body): Json<Value>| {
                let collected = collected.clone();
                let gate = gate.clone();
                async move {
                    let streaming = body["stream"] == true;
                    let stage = {
                        let mut requests = collected.lock().unwrap();
                        let stage = requests.iter().filter(|request| request["stream"] == false).count();
                        requests.push(body);
                        stage
                    };
                    if streaming {
                        let delta = json!({"type": "response.output_text.delta", "delta": "Pengukuran siap."});
                        let done = json!({"type": "response.completed", "response": {
                            "output": [], "usage": {"input_tokens": 11, "output_tokens": 3}
                        }});
                        return (
                            [(header::CONTENT_TYPE, "text/event-stream")],
                            format!("data: {delta}\n\ndata: {done}\n\ndata: [DONE]\n\n"),
                        ).into_response();
                    }
                    if let Some(gate) = gate && stage == gate.stage {
                        gate.received.notify_one();
                        gate.release.notified().await;
                    }
                    Json(json!({
                        "status": if failure { "incomplete" } else { "completed" },
                        "output": [{"type": "message", "content": [{"type": "output_text", "text": RECAP}]}],
                        "usage": {"input_tokens": 18, "output_tokens": 6}
                    })).into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = Arc::get_mut(state).unwrap();
        state.context_tokens = BUDGET;
        state.llm = Llm::new(&config::AzureConfig {
            endpoint: format!("http://{address}"),
            deployment: "sintetis".into(),
            api_version: String::new(),
            api_key: "sintetis".into(),
            context_tokens: BUDGET,
        });
        let server = tokio::spawn(async move { axum::serve(listener, service).await });
        Self { requests, server }
    }

    fn take(&self) -> Vec<Value> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}

impl Drop for Mockmodel {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn seed_turn(
    state: &AppState,
    user_id: i64,
    id: Option<String>,
    question: &str,
    answer: String,
    sources: Vec<agent::SourceRef>,
) -> (String, i64) {
    let turn = conversations::begin_turn(state, user_id, id, question.into(), Vec::new())
        .await
        .unwrap();
    let answer_id = conversations::save_answer(
        state,
        turn.id.clone(),
        turn.question_id,
        answer,
        1,
        conversations::AnswerExtras {
            sources,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    (turn.id, answer_id)
}

async fn chat_events(
    app: &Router,
    cookie: &str,
    id: &str,
    message: String,
) -> Vec<(String, Value)> {
    read_events(chat_response(app, cookie, id, message).await).await
}

async fn chat_response(
    app: &Router,
    cookie: &str,
    id: &str,
    message: String,
) -> axum::response::Response {
    let req = Request::post("/api/chat")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie)
        .body(Body::from(
            json!({"conversation_id": id, "message": message}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    res
}

async fn read_events(res: axum::response::Response) -> Vec<(String, Value)> {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let events: Vec<_> = std::str::from_utf8(&bytes)
        .unwrap()
        .split("\n\n")
        .filter_map(|event| {
            let (name, data) = event.strip_prefix("event: ")?.split_once("\ndata: ")?;
            Some((name.to_owned(), serde_json::from_str(data).unwrap()))
        })
        .collect();
    assert_eq!(events.last().unwrap().0, "done");
    events
}

#[tokio::test]
async fn chat_compacts_at_seventy_percent_and_reuses_persisted_summary() {
    let mut state = app_state();
    let vision = Mockmodel::start(&mut state, false).await;
    let app = router(state.clone());
    let admin = admin_cookie(&app).await;
    let (user, cookie) = active_user(&app, &admin, "ringkasan@example.com").await;
    let (id, boundary) =
        seed_turn(&state, user, None, "Target awal", "x".repeat(4700), vec![]).await;
    seed_turn(
        &state,
        user,
        Some(id.clone()),
        "Topik aktif",
        "Periksa ulang".into(),
        vec![],
    )
    .await;

    let under = chat_events(&app, &cookie, &id, "y".repeat(500)).await;
    assert!(!under.iter().any(|(name, _)| name == "compaction"));
    let before = vision.take();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0]["input"][0]["content"], "Target awal");

    let over = chat_events(&app, &cookie, &id, "z".repeat(700)).await;
    assert_eq!(
        over[1],
        (
            "compaction".into(),
            json!({"running": true, "summary": null})
        )
    );
    let summary = json!({"through_message_id": boundary, "content": RECAP});
    assert_eq!(
        over[2],
        (
            "compaction".into(),
            json!({"running": false, "summary": summary})
        )
    );
    let requests = vision.take();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["stream"], false);
    assert_eq!(requests[0]["store"], false);
    assert!(requests[0]["tools"].is_null());
    assert_eq!(requests[0]["input"].as_array().unwrap().len(), 2);
    assert_eq!(requests[0]["input"][0]["content"], "Target awal");
    assert_eq!(requests[1]["instructions"], before[0]["instructions"]);
    let input = requests[1]["input"].as_array().unwrap();
    assert_eq!(input[0]["role"], "assistant");
    assert!(input[0]["content"].as_str().unwrap().ends_with(RECAP));
    assert_eq!(input[1]["content"], "Topik aktif");
    assert_eq!(input.last().unwrap()["content"], "z".repeat(700));
    let detail = send(
        &app,
        "GET",
        &format!("/api/conversations/{id}"),
        None,
        Some(&cookie),
    )
    .await;
    assert_eq!(detail.body["summary"], summary);
    assert_eq!(detail.body["messages"][0]["content"], "Target awal");
    assert_eq!(detail.body["messages"][1]["content"], "x".repeat(4700));

    let next = chat_events(&app, &cookie, &id, "Kapan diperiksa?".into()).await;
    assert_eq!(next[0].1["summary"], summary);
    assert!(!next.iter().any(|(name, _)| name == "compaction"));
    let next_request = vision.take();
    assert_eq!(next_request.len(), 1);
    assert_eq!(next_request[0]["instructions"], requests[1]["instructions"]);
    assert_eq!(next_request[0]["input"][0], input[0]);
    let usage = state
        .db
        .call(move |c| {
            Ok(c.query_row(
                "SELECT input_tokens, output_tokens, requests, reported_requests
             FROM usage_log WHERE user_id = ?1 AND requests = 2 AND kind = 'chat'",
                [user],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                },
            )?)
        })
        .await
        .unwrap();
    assert_eq!(usage, (29, 9, 2, 2));
}

#[tokio::test]
async fn unfinished_summary_falls_back_to_old_trimming() {
    let mut state = app_state();
    let vision = Mockmodel::start(&mut state, true).await;
    let app = router(state.clone());
    let admin = admin_cookie(&app).await;
    let (user, cookie) = active_user(&app, &admin, "gagal-ringkasan@example.com").await;
    let (id, _) = seed_turn(&state, user, None, "Target awal", "x".repeat(5000), vec![]).await;
    let (id, _) = seed_turn(
        &state,
        user,
        Some(id),
        "Topik aktif",
        "Periksa ulang".into(),
        vec![],
    )
    .await;
    let events = chat_events(&app, &cookie, &id, "z".repeat(700)).await;
    assert_eq!(events[1].1["running"], true);
    assert_eq!(events[2].1, json!({"running": false, "summary": null}));
    assert_eq!(
        events[3],
        ("delta".into(), json!({"text": "Pengukuran siap."}))
    );
    let requests = vision.take();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1]["input"].as_array().unwrap().len(), 3);
    assert_eq!(requests[1]["input"][0]["content"], "Topik aktif");
    let detail = send(
        &app,
        "GET",
        &format!("/api/conversations/{id}"),
        None,
        Some(&cookie),
    )
    .await;
    assert!(detail.body["summary"].is_null());
    assert_eq!(detail.body["messages"][0]["content"], "Target awal");
    let next = chat_events(&app, &cookie, &id, "Coba lagi".into()).await;
    assert!(!next.iter().any(|(name, _)| name == "compaction"));
    let requests = vision.take();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["stream"], true);
}

#[tokio::test]
async fn active_question_takes_priority_over_a_summary_that_no_longer_fits() {
    let mut state = app_state();
    let vision = Mockmodel::start(&mut state, false).await;
    Arc::get_mut(&mut state).unwrap().context_tokens = 4096;
    let app = router(state.clone());
    let admin = admin_cookie(&app).await;
    let (user, cookie) = active_user(&app, &admin, "pertanyaan-aktif@example.com").await;
    let (id, boundary) = seed_turn(
        &state,
        user,
        None,
        "Pertanyaan awal",
        "Jawaban awal".into(),
        vec![],
    )
    .await;
    let summary = format!("[7] {}", "x".repeat(4092));
    assert_eq!(summary.len(), 4096);
    let (stored_id, stored_summary) = (id.clone(), summary.clone());
    state.db.call(move |c| {
        c.execute(
            "INSERT INTO conversation_compactions (conversation_id, through_message_id, content) VALUES (?1, ?2, ?3)",
            rusqlite::params![stored_id, boundary, stored_summary],
        )?;
        Ok(())
    }).await.unwrap();

    let question = "z".repeat(7500);
    let events = chat_events(&app, &cookie, &id, question.clone()).await;
    assert_eq!(events[0].1["summary"]["content"], summary);
    let requests = vision.take();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]["input"],
        json!([{"role": "user", "content": question}])
    );
    let detail = send(
        &app,
        "GET",
        &format!("/api/conversations/{id}"),
        None,
        Some(&cookie),
    )
    .await;
    assert_eq!(detail.body["summary"]["content"], summary);
}

#[tokio::test]
async fn failed_extension_keeps_the_previous_summary_and_recent_tail() {
    let mut state = app_state();
    let vision = Mockmodel::start(&mut state, true).await;
    let app = router(state.clone());
    let admin = admin_cookie(&app).await;
    let (user, cookie) = active_user(&app, &admin, "ringkasan-lama@example.com").await;
    let (id, old_boundary) = seed_turn(
        &state,
        user,
        None,
        "Pesan tercakup",
        "Jawaban awal".into(),
        vec![],
    )
    .await;
    seed_turn(
        &state,
        user,
        Some(id.clone()),
        "Pesan yang terlalu besar",
        "x".repeat(5100),
        vec![],
    )
    .await;
    let saved_id = id.clone();
    state.db.call(move |c| {
        c.execute(
            "INSERT INTO conversation_compactions (conversation_id, through_message_id, content) VALUES (?1, ?2, ?3)",
            rusqlite::params![saved_id, old_boundary, RECAP],
        )?;
        Ok(())
    }).await.unwrap();

    let recent = "z".repeat(700);
    let events = chat_events(&app, &cookie, &id, recent.clone()).await;
    assert_eq!(events[2].1["summary"]["through_message_id"], old_boundary);
    let requests = vision.take();
    assert_eq!(requests.len(), 2);
    let input = requests[1]["input"].as_array().unwrap();
    assert_eq!(input.len(), 2);
    assert!(input[0]["content"].as_str().unwrap().ends_with(RECAP));
    assert_eq!(input[1]["content"], recent);
    let next = chat_events(&app, &cookie, &id, "Lanjut".into()).await;
    assert!(!next.iter().any(|(name, _)| name == "compaction"));
    let requests = vision.take();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["input"][0], input[0]);
}

#[tokio::test]
async fn third_stage_timeout_reuses_two_saved_stages_on_the_next_attempt() {
    let mut state = app_state();
    let gate = Arc::new(SummaryGate {
        stage: 2,
        ..Default::default()
    });
    let vision = Mockmodel::start_with_gate(&mut state, false, Some(gate.clone())).await;
    let app = router(state.clone());
    let admin = admin_cookie(&app).await;
    let (user, cookie) = active_user(&app, &admin, "lanjut-tahap@example.com").await;
    let mut conversation = None;
    let mut boundaries = Vec::new();
    for n in 1..=3 {
        let (id, boundary) = seed_turn(
            &state,
            user,
            conversation,
            &format!("Giliran {n}"),
            "x".repeat(5100),
            vec![],
        )
        .await;
        conversation = Some(id);
        boundaries.push(boundary);
    }
    let id = conversation.unwrap();
    let recent = "z".repeat(700);
    let pending = chat_response(&app, &cookie, &id, recent.clone()).await;
    gate.received.notified().await;
    let check_id = id.clone();
    let saved = state
        .db
        .call(move |c| Ok(compaction::load(c, &check_id)?))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.through_message_id, boundaries[1]);

    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(21)).await;
    tokio::time::resume();
    let events = read_events(pending).await;
    gate.release.notify_one();
    assert_eq!(events[2].1["summary"]["through_message_id"], boundaries[1]);
    let requests = vision.take();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0]["input"][0]["content"], "Giliran 1");
    assert_eq!(requests[1]["input"][1]["content"], "Giliran 2");
    assert_eq!(requests[2]["input"][1]["content"], "Giliran 3");
    assert!(
        requests[3]["input"][0]["content"]
            .as_str()
            .unwrap()
            .ends_with(RECAP)
    );
    assert_eq!(
        requests[3]["input"].as_array().unwrap().last().unwrap()["content"],
        recent
    );

    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(601)).await;
    tokio::time::resume();
    let events = chat_events(&app, &cookie, &id, "Teruskan".into()).await;
    assert_eq!(events[2].1["summary"]["through_message_id"], boundaries[2]);
    let next = vision.take();
    assert_eq!(next.len(), 2);
    assert_eq!(next[0]["input"], requests[2]["input"]);
    assert_eq!(next[1]["stream"], true);
}

#[tokio::test]
async fn concurrent_tabs_reuse_the_summary_after_waiting_for_its_lock() {
    let mut state = app_state();
    let gate = Arc::new(SummaryGate::default());
    let vision = Mockmodel::start_with_gate(&mut state, false, Some(gate.clone())).await;
    let app = router(state.clone());
    let admin = admin_cookie(&app).await;
    let (user, cookie) = active_user(&app, &admin, "dua-tab@example.com").await;
    let (id, boundary) =
        seed_turn(&state, user, None, "Target awal", "x".repeat(5100), vec![]).await;
    let first = chat_response(&app, &cookie, &id, "y".repeat(700)).await;
    gate.received.notified().await;
    let second = chat_response(&app, &cookie, &id, "Lanjut dari tab kedua".into()).await;
    gate.release.notify_one();
    let (first, second) = tokio::join!(read_events(first), read_events(second));
    assert_eq!(first[2].1["summary"]["through_message_id"], boundary);
    assert!(second[0].1["summary"].is_null());
    assert_eq!(second[1].1["summary"]["through_message_id"], boundary);
    assert!(
        !second
            .iter()
            .any(|(name, data)| name == "compaction" && data["running"] == true)
    );
    let requests = vision.take();
    assert_eq!(
        requests.iter().filter(|req| req["stream"] == false).count(),
        1
    );
    assert_eq!(
        requests.iter().filter(|req| req["stream"] == true).count(),
        2
    );
    for req in requests.iter().filter(|req| req["stream"] == true) {
        assert!(
            req["input"][0]["content"]
                .as_str()
                .unwrap()
                .ends_with(RECAP)
        );
    }
}

#[tokio::test]
async fn compaction_covers_old_middle_and_recent_messages_in_one_megabyte_chat() {
    let mut state = app_state();
    let vision = Mockmodel::start(&mut state, false).await;
    Arc::get_mut(&mut state).unwrap().context_tokens = 128_000;
    let app = router(state.clone());
    let admin = admin_cookie(&app).await;
    let (user, cookie) = active_user(&app, &admin, "riwayat-besar@example.com").await;
    let turn = conversations::begin_turn(&state, user, None, "Giliran 000".into(), vec![])
        .await
        .unwrap();
    let id = turn.id;
    let seed_id = id.clone();
    state.db.call(move |c| {
        let tx = c.transaction()?;
        let answer = "x".repeat(10_000);
        for n in 0..100 {
            if n > 0 {
                tx.execute("INSERT INTO messages (conversation_id, role, content, created_at) VALUES (?1, 'user', ?2, 0)",
                    rusqlite::params![seed_id, format!("Giliran {n:03}")])?;
            }
            tx.execute("INSERT INTO messages (conversation_id, role, content, created_at) VALUES (?1, 'assistant', ?2, 0)",
                rusqlite::params![seed_id, answer])?;
        }
        tx.commit()?;
        Ok(())
    }).await.unwrap();

    let events = chat_events(&app, &cookie, &id, "Teruskan".into()).await;
    let boundary = events[2].1["summary"]["through_message_id"]
        .as_i64()
        .unwrap();
    let requests = vision.take();
    assert!(requests.len() >= 3);
    assert!(
        requests[..requests.len() - 1]
            .iter()
            .all(|req| req["stream"] == false)
    );
    assert_eq!(requests.last().unwrap()["stream"], true);
    let mut expected: Vec<_> = (0..100).map(|n| format!("Giliran {n:03}")).collect();
    expected.push("Teruskan".into());
    let observed: Vec<_> = requests
        .iter()
        .flat_map(|req| req["input"].as_array().unwrap())
        .filter(|message| message["role"] == "user")
        .map(|message| message["content"].as_str().unwrap())
        .collect();
    assert_eq!(observed, expected);
    assert!(
        requests[1]["input"][0]["content"]
            .as_str()
            .unwrap()
            .ends_with(RECAP)
    );
    let following: String = state.db.call(move |c| {
        Ok(c.query_row("SELECT content FROM messages WHERE conversation_id = ?1 AND id > ?2 ORDER BY id LIMIT 1",
            rusqlite::params![id, boundary], |r| r.get(0))?)
    }).await.unwrap();
    assert_eq!(requests.last().unwrap()["input"][1]["content"], following);
}

#[tokio::test]
async fn revoked_text_source_blocks_summarization_and_disclosure() {
    let (mut state, base) = docs_state_with(0).await;
    let vision = Mockmodel::start(&mut state, false).await;
    let app = router(state.clone());
    let admin = admin_cookie(&app).await;
    let (user, cookie) = active_user(&app, &admin, "akses-ringkasan@example.com").await;
    let (doc_id, metadata_id): (i64, i64) = state
        .db
        .call(|c| {
            let find = |path| {
                c.query_row(
                    "SELECT id FROM documents_v4 WHERE rel_path = ?1",
                    [path],
                    |r| r.get(0),
                )
            };
            Ok((find("torsi.txt")?, find("cuti.txt")?))
        })
        .await
        .unwrap();
    let source = agent::SourceRef {
        n: 7,
        chunk_id: "uji-potongan-torsi".into(),
        doc_id,
        title: "Torsi Uji".into(),
        display_title: String::new(),
        source: "Review".into(),
        kind: "txt".into(),
        page: None,
        sheet: None,
        image: None,
        evidence_only: false,
    };
    let answer = format!("Contoh jawaban [7]. {}", "x".repeat(4900));
    let metadata = agent::SourceRef {
        n: 8,
        doc_id: metadata_id,
        chunk_id: String::new(),
        evidence_only: true,
        ..source.clone()
    };
    let (id, answer_id) = seed_turn(
        &state,
        user,
        None,
        "Pertanyaan lama",
        answer.clone(),
        vec![source.clone(), metadata],
    )
    .await;
    let (blocked, _) = seed_turn(&state, user, None, "Pertanyaan lama", answer, vec![source]).await;
    const FILE_ID: &str = "11111111111111111111111111111111";
    let for_file = id.clone();
    state.db.call(move |c| {
        c.execute("INSERT INTO files (id, user_id, conversation_id, message_id, name, kind, size, spec, created_at)
                   VALUES (?1, ?2, ?3, ?4, 'hasil.xlsx', 'xlsx', 0, '{}', 0)",
            rusqlite::params![FILE_ID, user, for_file, answer_id])?;
        Ok(())
    }).await.unwrap();

    let events = chat_events(&app, &cookie, &id, "z".repeat(800)).await;
    assert_eq!(events[2].1["summary"]["content"], RECAP);
    let requests = vision.take();
    assert_eq!(requests.len(), 2);
    let old = requests[0]["input"][1]["content"].as_str().unwrap();
    assert!(old.starts_with("Contoh jawaban [1]."));
    assert!(old.contains(&format!("dokumen {doc_id}, sitasi uji-potongan-torsi")));
    let references = |request: &Value| {
        let text = request["input"][0]["content"].as_str().unwrap();
        serde_json::from_str::<Value>(
            text.split_once("[Referensi tersimpan: ")
                .unwrap()
                .1
                .strip_suffix(']')
                .unwrap(),
        )
        .unwrap()
    };
    let expected_docs: std::collections::BTreeSet<_> = [doc_id, metadata_id].into_iter().collect();
    let expected = json!({"id_dokumen": expected_docs, "sitasi": [[doc_id, "uji-potongan-torsi"]], "id_file_buatan": [FILE_ID]});
    assert_eq!(references(&requests[1]), expected);
    chat_events(&app, &cookie, &id, "Buka lagi hasil uji".into()).await;
    let requests = vision.take();
    assert_eq!(requests.len(), 1);
    assert_eq!(references(&requests[0]), expected);

    state
        .db
        .call(move |c| {
            c.execute(
                "UPDATE documents_v4 SET visibility = '{\"type\":\"admin\"}' WHERE id = ?1",
                [doc_id],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let detail = send(
        &app,
        "GET",
        &format!("/api/conversations/{id}"),
        None,
        Some(&cookie),
    )
    .await;
    assert!(detail.body["summary"].is_null());
    assert_eq!(detail.body["messages"][1]["sources"], json!([]));
    assert!(
        detail.body["messages"][1]["content"]
            .as_str()
            .unwrap()
            .starts_with("Contoh jawaban")
    );
    for id in [id, blocked] {
        let events = chat_events(&app, &cookie, &id, "z".repeat(800)).await;
        assert!(events[0].1["summary"].is_null());
        assert!(!events.iter().any(|(name, _)| name == "compaction"));
        let requests = vision.take();
        assert_eq!(requests.len(), 1);
        assert!(!requests[0]["input"].to_string().contains(RECAP));
    }
    drop((app, state));
    let _ = std::fs::remove_dir_all(base);
}
