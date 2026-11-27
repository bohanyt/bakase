use super::*;
use axum::response::IntoResponse;
use std::sync::Mutex;

const ARGUMENTS: &str = r#"{ "query" : "torsi", "limit": 1 }"#;
const EXCERPT: &str = "Torsi baut flange kelas 150 adalah 45 Nm untuk diameter M16.";

#[tokio::test]
async fn failed_round_keeps_partials_prompt_and_exact_model_input() {
    let mut state = app_state();
    let vision = Mock::start(&mut state, vec![
        answer("Putaran awal.") + &event("response.completed", json!({"response":{
            "id":"resp-awal", "usage":{"input_tokens":34,"output_tokens":8,"total_tokens":42,
                "input_tokens_details":{"cached_tokens":13,"encrypted_content":"opaque-sintetis"},"output_tokens_details":{"reasoning_tokens":3}},
            "output":[
                {"type":"reasoning", "encrypted_content":"opaque-sintetis"},
                {"type":"function_call", "call_id":"satu", "name":"read_attachment", "arguments":"{}"}
            ]
        }})),
        event("response.created", json!({"response":{"id":"resp-kedua","usage":null}})) +
        &answer("Putaran kedua.") +
            &event("response.reasoning_summary_text.delta", json!({"delta":"Ringkasan kedua."})) +
            &event("response.output_item.added", json!({"output_index":2,"item":{"type":"function_call","id":"item-2","call_id":"dua","name":"read_attachment","arguments":""}})) +
            &event("response.function_call_arguments.delta", json!({"output_index":2,"item_id":"item-2","delta":"{\"file_id\":"})),
    ], false).await;
    let app = router(state.clone());
    let cookie = admin_cookie(&app).await;
    let attachment = uploaded(
        &app,
        &cookie,
        "Halaman - Uji.md",
        b"Isi fixture halaman".to_vec(),
    )
    .await;
    let (_, sse) = chat(
        &app,
        &cookie,
        json!({
            "message":"Uji beberapa putaran.", "attachments":[attachment],
            "page":{"origin":"chrome-extension://ponmlkjihgfedcbaponmlkjihgfedcba",
                    "attachment_id":attachment, "text":"Snapshot lanjutan sintetis."}
        }),
    )
    .await;
    let (_, _, _, detail) = trace(&state, &conversation_id(&sse)).await;
    let first = &detail["entries"][0];
    let last = &detail["entries"][2];
    assert_eq!(first["text"], "Putaran awal.");
    assert_eq!(last["text"], "Putaran kedua.");
    assert_eq!(last["reasoning"], "Ringkasan kedua.");
    assert_eq!(last["partial_calls"][0]["arguments"], "{\"file_id\":");
    assert_eq!(last["status"], "failed");
    assert_eq!(first["response_id"], "resp-awal");
    assert_eq!(
        first["usage"],
        json!({"input_tokens":34,"output_tokens":8,"total_tokens":42,
        "input_tokens_details":{"cached_tokens":13},"output_tokens_details":{"reasoning_tokens":3}})
    );
    assert_eq!(last["response_id"], "resp-kedua");
    assert!(last["usage"].is_null());
    assert_eq!(first["prompt_sha256"], last["prompt_sha256"]);
    let (input, prompt) = {
        let requests = vision.requests.lock().unwrap();
        (
            requests[0]["input"].clone(),
            requests[0]["instructions"].as_str().unwrap().to_owned(),
        )
    };
    assert_eq!(first["input"], input);
    assert!(
        first["input"]
            .to_string()
            .contains("Snapshot lanjutan sintetis.")
    );
    assert!(last["input"].is_null());
    let key = last["prompt_sha256"].as_str().unwrap().to_owned();
    let (saved_prompt, forbidden) = state
        .db
        .call(move |c| {
            let prompt: String =
                c.query_row("SELECT text FROM trace_texts WHERE sha256=?1", [key], |r| {
                    r.get(0)
                })?;
            let forbidden: i64 = c.query_row(
                "SELECT count(*) FROM (
                    SELECT text AS data FROM trace_texts UNION ALL SELECT data FROM trace_events
                 ) WHERE data LIKE '%opaque-sintetis%' OR data LIKE '%sintetis-kunci%'",
                [],
                |r| r.get(0),
            )?;
            Ok((prompt, forbidden))
        })
        .await
        .unwrap();
    assert_eq!(saved_prompt, prompt);
    assert_eq!(forbidden, 0);
    std::fs::remove_dir_all(&state.files_dir).unwrap();
}

#[tokio::test]
async fn trace_keeps_header_like_free_text_and_an_uncut_final_answer() {
    let mut state = app_state();
    let args = r#"{ "query": "header: kolom A" }"#;
    let full = format!("header: kolom A\n{}", "測 ".repeat(34_379));
    let _vision = Mock::start(&mut state, vec![
        completed(json!([{"type":"function_call", "call_id":"uji", "name":"search_documents", "arguments":args}])),
        answer(&full) + &completed(json!([])),
    ], false).await;
    let app = router(state.clone());
    let cookie = admin_cookie(&app).await;
    let (_, sse) = chat(&app, &cookie, json!({"message":"Uji teks persis."})).await;
    let (_, _, _, detail) = trace(&state, &conversation_id(&sse)).await;
    assert_eq!(detail["entries"][1]["arguments"]["text"], args);
    assert_eq!(
        detail["answer"]["text"].as_str().map(str::len),
        Some(full.len())
    );
    assert_eq!(detail["answer"]["text"], full);
}

pub(super) struct Mock {
    pub(super) requests: Arc<Mutex<Vec<Value>>>,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
}

fn event(kind: &str, fields: Value) -> String {
    let mut event = fields.as_object().unwrap().clone();
    event.insert("type".into(), json!(kind));
    format!("data: {}\n\n", Value::Object(event))
}

pub(super) fn answer(text: &str) -> String {
    event("response.output_text.delta", json!({"delta": text}))
}

pub(super) fn completed(output: Value) -> String {
    event(
        "response.completed",
        json!({"response": {"model": "vision-sintetis", "reasoning": {"effort": "medium"}, "output": output}}),
    ) + "data: [DONE]\n\n"
}

impl Mock {
    pub(super) async fn start(
        state: &mut Arc<AppState>,
        rounds: Vec<String>,
        hold_last: bool,
    ) -> Self {
        Self::gated(state, rounds, hold_last, None).await
    }

    pub(super) async fn gated(
        state: &mut Arc<AppState>,
        rounds: Vec<String>,
        hold_last: bool,
        gate: Option<Arc<tokio::sync::Notify>>,
    ) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = Arc::get_mut(state).unwrap();
        state.llm = Llm::new(&config::AzureConfig {
            endpoint: format!("http://{address}"),
            deployment: "deployment-sintetis".into(),
            api_version: String::new(),
            api_key: "sintetis-kunci".into(),
            context_tokens: state.context_tokens,
        });
        state.traces = turn_trace::Store::in_memory(state.db.clone());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = requests.clone();
        let app = Router::new().route(
            "/openai/v1/responses",
            post(move |Json(request): Json<Value>| {
                let (index, count, output) = {
                    let mut requests = observed.lock().unwrap();
                    let index = requests.len();
                    requests.push(request);
                    (index, rounds.len(), rounds[index].clone())
                };
                let gate = gate.clone();
                async move {
                    let stream = async_stream::stream! {
                        if index == 1 && let Some(gate) = gate {
                            gate.notified().await;
                        }
                        yield Ok::<_, Infallible>(output);
                        if hold_last && index + 1 == count {
                            std::future::pending::<()>().await;
                        }
                    };
                    (
                        [(header::CONTENT_TYPE, "text/event-stream")],
                        Body::from_stream(stream),
                    )
                        .into_response()
                }
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        Self { requests, server }
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn trace(state: &AppState, id: &str) -> (i64, String, Option<String>, Value) {
    let id = id.to_owned();
    state.traces.flush().await;
    state.db.call(move |c| {
        let (question, status, reason): (i64, String, Option<String>) = c.query_row(
            "SELECT t.question_id, t.status, t.reason FROM turn_traces t
             JOIN messages m ON m.id=t.question_id WHERE m.conversation_id=?1 ORDER BY m.id LIMIT 1",
            [id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        Ok((question, status, reason, read_question(c, question)?))
    }).await.unwrap()
}

pub(super) fn read_question(c: &rusqlite::Connection, q: i64) -> anyhow::Result<Value> {
    let id: String = c.query_row(
        "SELECT id FROM turn_traces WHERE question_id=?1",
        [q],
        |r| r.get(0),
    )?;
    read_detail(c, &id)
}

#[tokio::test]
async fn traced_excel_fill_calculation_cites_the_worksheet() {
    let mut book = rust_xlsxwriter::Workbook::new();
    let sheet = book.add_worksheet().set_name("Data").unwrap();
    let gray = rust_xlsxwriter::Format::new().set_background_color("#BFBFBF");
    sheet.write(0, 0, "Nilai").unwrap();
    sheet.write_with_format(1, 0, 7, &gray).unwrap();
    let (mut state, base) =
        docs_state_files(0, &[("review/Rekap.xlsx", book.save_to_buffer().unwrap())]).await;
    let _model = Mock::start(&mut state, vec![
        completed(json!([{"type":"function_call", "call_id":"cari", "name":"search_documents", "arguments":"{\"query\":\"Rekap\",\"limit\":1}"}])),
        completed(json!([{"type":"function_call", "call_id":"hitung", "name":"hitung_xlsx", "arguments":json!({"sitasi":1,"sql":"SELECT count(*) FROM warna WHERE nama = 'abu-abu'"}).to_string()}])),
        answer("Satu sel berwarna abu-abu [2].") + &completed(json!([])),
    ], false).await;
    let app = router(state.clone());
    let cookie = admin_cookie(&app).await;
    let id = doc_id(&state, "Rekap.xlsx").await;
    let (status, sse) = chat(
        &app,
        &cookie,
        json!({"message":"Berapa sel berwarna abu-abu?"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, status, _, detail) = trace(&state, &conversation_id(&sse)).await;
    assert_eq!(status, "ok");
    assert_eq!(detail["answer"]["text"], "Satu sel berwarna abu-abu [1].");
    let tool = &detail["entries"][3];
    assert_eq!(tool["name"]["text"], "hitung_xlsx");
    assert_eq!(tool["status"], "ok");
    let result: Value = serde_json::from_str(tool["output"]["text"].as_str().unwrap()).unwrap();
    assert_eq!(result["hasil"], json!([[1]]));
    assert_eq!(result["sitasi"], json!(["[2]"]));
    let calculated = &detail["excerpts"][1];
    assert_eq!(calculated["model_number"], 2);
    assert_eq!(calculated["doc_id"], id);
    assert_eq!(calculated["sheet"], "Data");
    assert_eq!(calculated["cited_as"], 1);
    drop(app);
    drop(state);
    let _ = std::fs::remove_dir_all(&base);
}

fn read_detail(c: &rusqlite::Connection, id: &str) -> anyhow::Result<Value> {
    fn push(target: &mut Value, text: &str) {
        *target = json!(target.as_str().unwrap_or_default().to_owned() + text);
    }
    let (deployment, version): (String, String) = c.query_row(
        "SELECT deployment, server_version FROM turn_traces WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let mut detail = json!({"deployment":{"text":deployment}, "server_version":version});
    let mut entries = Vec::<Value>::new();
    let mut excerpts = Vec::<Value>::new();
    let mut stmt = c.prepare(
        "SELECT span, at_ms, kind, item, data FROM trace_events WHERE turn_id=?1 ORDER BY seq",
    )?;
    let mut rows = stmt.query([id])?;
    while let Some(row) = rows.next()? {
        let span = row.get::<_, u32>(0)? as usize;
        let at: i64 = row.get(1)?;
        let kind: String = row.get(2)?;
        let item = row.get::<_, Option<i64>>(3)?.map(|item| item as u64);
        let data: String = row.get(4)?;
        let parse = || serde_json::from_str::<Value>(&data).unwrap();
        if kind == "model" || kind == "tool" {
            assert_eq!(span, entries.len() + 1);
            let mut entry = parse();
            entry["kind"] = json!(kind);
            entry["started_at_ms"] = json!(at);
            entry["status"] = json!("running");
            entry["name"] = json!({"text":entry["name"]});
            entry["effort"] = json!({"text":entry["effort"]});
            if let Some(tools) = entry["tool_spans"].as_array() {
                for tool in tools {
                    let tool = &mut entries[tool.as_u64().unwrap() as usize - 1];
                    if tool["submitted_in"].is_null() {
                        tool["submitted_in"] = json!(span);
                    }
                }
            }
            entries.push(entry);
            continue;
        }
        if span == 0 {
            if kind == "answer" || kind == "reasoning" {
                push(&mut detail[&kind]["text"], &data);
            }
            continue;
        }
        let entry = &mut entries[span - 1];
        match kind.as_str() {
            "input" => push(&mut entry["input"], &data),
            "end" => {
                let end = parse();
                entry["status"] = end["status"].clone();
                entry["duration_ms"] = end["duration_ms"].clone();
            }
            "model_metadata" => {
                let metadata = parse();
                for key in ["response_id", "usage"] {
                    if !metadata[key].is_null() {
                        entry[key] = metadata[key].clone();
                    }
                }
                for key in ["model", "effort"] {
                    if !metadata[key].is_null() {
                        entry[key] = json!({"text":metadata[key]});
                    }
                }
            }
            "model_text" => push(&mut entry["text"], &data),
            "model_reasoning" => push(&mut entry["reasoning"], &data),
            "model_call" => {
                if entry["partial_calls"].is_null() {
                    entry["partial_calls"] = json!([]);
                }
                entry["partial_calls"].as_array_mut().unwrap().push(parse());
            }
            "model_arguments" => {
                if let Some(call) = entry["partial_calls"]
                    .as_array_mut()
                    .and_then(|calls| calls.iter_mut().find(|call| call["index"].as_u64() == item))
                {
                    push(&mut call["arguments"], &data);
                }
            }
            "tool_arguments" => push(&mut entry["arguments"]["text"], &data),
            "tool_output" => push(&mut entry["output"]["text"], &data),
            "excerpt" => {
                let mut excerpt = parse();
                excerpt["source"] = json!({"text":excerpt["source"]});
                excerpts.push(excerpt);
            }
            "excerpt_text" => push(&mut excerpts.last_mut().unwrap()["text"]["text"], &data),
            "citations" => {
                let mapping: Vec<(Value, Value)> = serde_json::from_str(&data)?;
                for excerpt in &mut excerpts {
                    if let Some((_, new)) = mapping
                        .iter()
                        .find(|(old, _)| *old == excerpt["model_number"])
                    {
                        excerpt["cited_as"] = new.clone();
                    }
                }
            }
            _ => (),
        }
    }
    for entry in &mut entries {
        if let Some(input) = entry["input"].as_str() {
            entry["input"] = serde_json::from_str::<Value>(input)?;
        }
    }
    detail["entries"] = json!(entries);
    detail["excerpts"] = json!(excerpts);
    Ok(detail)
}

#[tokio::test]
async fn ordered_trace_preserves_dispatched_tool_results_and_cited_text_after_reindex() {
    let (mut state, base) = docs_state().await;
    let vision = Mock::start(
        &mut state,
        vec![
            completed(json!([
                {"type": "reasoning", "encrypted_content": "terenkripsi-sintetis", "summary": []},
                {"type": "function_call", "call_id": "tool-1", "name": "search_documents", "arguments": ARGUMENTS},
                {"type": "function_call", "call_id": "tool-2", "name": "read_attachment", "arguments":"{}"},
            ])),
            answer("Besarnya 45 Nm [1].") + &completed(json!([])),
            answer("Jawaban susulan.") + &completed(json!([])),
        ],
        false,
    ).await;
    let app = router(state.clone());
    let cookie = admin_cookie(&app).await;
    let (status, sse) = chat(&app, &cookie, json!({"message":"Berapa torsi?"})).await;
    assert_eq!(status, StatusCode::OK);
    let id = conversation_id(&sse);
    let (question, status, reason, detail) = trace(&state, &id).await;
    assert_eq!(status, "ok");
    assert!(reason.is_none());
    assert_eq!(detail["deployment"]["text"], "deployment-sintetis");
    assert_eq!(detail["server_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(detail["answer"]["text"], "Besarnya 45 Nm [1].");
    assert!(!detail.to_string().contains("terenkripsi-sintetis"));
    let entries = detail["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0]["kind"], "model");
    assert_eq!(entries[0]["model"]["text"], "vision-sintetis");
    assert_eq!(entries[0]["effort"]["text"], "medium");
    assert_eq!(entries[0]["prompt_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(entries[1]["name"]["text"], "search_documents");
    assert_eq!(entries[1]["arguments"]["text"], ARGUMENTS);
    assert_eq!(entries[1]["status"], "ok");
    assert_eq!(entries[2]["name"]["text"], "read_attachment");
    assert_eq!(entries[2]["status"], "failed");
    assert_eq!(entries[3]["kind"], "model");
    for step in entries {
        assert!(step["started_at_ms"].as_i64().unwrap() > 0);
        assert!(step["duration_ms"].as_i64().unwrap() >= 0);
    }
    {
        let requests = vision.requests.lock().unwrap();
        let results: Vec<_> = requests[1]["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["type"] == "function_call_output")
            .collect();
        for (step, result) in entries[1..=2].iter().zip(results) {
            assert_eq!(step["submitted_in"], 4);
            assert_eq!(step["output"]["text"], result["output"]);
        }
    }
    let excerpt = &detail["excerpts"][0];
    assert_eq!(excerpt["cited_as"], 1);
    assert_eq!(excerpt["source"]["text"], "Review");
    assert_eq!(excerpt["text"]["text"], EXCERPT);
    assert_eq!(excerpt["text_sha256"].as_str().unwrap().len(), 64);
    let chunk = excerpt["chunk_id"].as_str().unwrap();
    let chunk_route = format!("/api/documents/chunks/{chunk}");
    assert_eq!(get_raw(&app, &chunk_route, &cookie).await.0, StatusCode::OK);
    let index = state.docs.as_ref().unwrap();
    let mut writer = index.writer().unwrap();
    index.delete_document(&writer, excerpt["doc_id"].as_i64().unwrap());
    writer.commit().unwrap();
    writer.wait_merging_threads().unwrap();
    index.reload().unwrap();
    assert_eq!(
        get_raw(&app, &chunk_route, &cookie).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(trace(&state, &id).await.3["excerpts"][0], *excerpt);

    let path = format!("/api/conversations/{id}");
    let (_, second) = chat(
        &app,
        &cookie,
        json!({"conversation_id":id,"message":"tanya lagi"}),
    )
    .await;
    assert!(!second.contains(EXCERPT));
    assert!(
        !vision.requests.lock().unwrap()[2]
            .to_string()
            .contains(EXCERPT)
    );
    let conversation = send(&app, "GET", &path, None, Some(&cookie)).await;
    let answer_id = conversation.body["messages"][1]["id"].as_i64().unwrap();
    let reply_to = state
        .db
        .call(move |c| {
            Ok(c.query_row(
                "SELECT reply_to FROM messages WHERE id = ?1",
                [answer_id],
                |r| r.get::<_, i64>(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(reply_to, question);
    assert!(!conversation.body.to_string().contains(EXCERPT));
    assert!(!conversation.body.to_string().contains("prompt_sha256"));
    let search = send(
        &app,
        "POST",
        "/api/conversations/search",
        Some(json!({"q":"diameter M16"})),
        Some(&cookie),
    )
    .await;
    assert_eq!(search.body, json!([]));
    assert_eq!(
        get_raw(&app, &format!("/api/turn-traces/{question}"), &cookie)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            &app,
            "DELETE",
            &format!("{path}/archive-v1"),
            None,
            Some(&cookie)
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    state.traces.flush().await;
    let remaining = state
        .db
        .call(|c| {
            Ok(c.query_row(
                "SELECT count(*) FROM turn_traces t JOIN messages m ON m.id=t.question_id
                 JOIN conversations c ON c.id=m.conversation_id WHERE c.user_id<>-1",
                [],
                |r| r.get::<_, i64>(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(remaining, 0);
    let archived = state
        .db
        .call(move |c| read_question(c, question))
        .await
        .unwrap();
    assert_eq!(archived["excerpts"][0], *excerpt);
    drop((app, state));
    let _ = std::fs::remove_dir_all(base);
}

pub(super) async fn after_partial(app: &Router, cookie: &str, request: Value) -> (String, Body) {
    let request = Request::post("/api/chat")
        .header(header::COOKIE, cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(request.to_string()))
        .unwrap();
    let mut body = app.clone().oneshot(request).await.unwrap().into_body();
    let mut text = String::new();
    while !text.contains("Potongan uji.") {
        let frame = body.frame().await.unwrap().unwrap();
        if let Some(data) = frame.data_ref() {
            text.push_str(std::str::from_utf8(data).unwrap());
        }
    }
    (conversation_id(&text), body)
}

#[tokio::test]
async fn open_turn_can_read_its_attachment_after_archive() {
    open_turn_after_archive(false).await;
}

#[tokio::test]
async fn open_turn_finishes_after_its_local_account_is_archived_and_deleted() {
    open_turn_after_archive(true).await;
}

async fn open_turn_after_archive(delete_account: bool) {
    let db_dir = std::env::temp_dir().join(format!("app-relay-{}", rand::random::<u64>()));
    let mut state = state_with(db::Db::open(&db_dir).unwrap(), None, vec![]);
    let seed_app = router(state.clone());
    let admin = admin_cookie(&seed_app).await;
    let (owner, cookie) = if delete_account {
        active_user(&seed_app, &admin, "lokal-uji@example.com").await
    } else {
        (user_id(&seed_app, &admin, ADMIN_EMAIL).await, admin.clone())
    };
    let attachment = uploaded(
        &seed_app,
        &cookie,
        "Uji.txt",
        b"Isi lampiran setelah arsip".to_vec(),
    )
    .await;
    drop(seed_app);
    let gate = Arc::new(tokio::sync::Notify::new());
    let vision = Mock::gated(&mut state, vec![
        answer("Potongan uji.") + &completed(json!([
            {"type":"function_call","call_id":"awal","name":"read_attachment","arguments":"{}"}
        ])),
        completed(json!([
            {"type":"function_call","call_id":"baca","name":"read_attachment",
             "arguments":json!({"file_id":attachment}).to_string()}
        ])),
        answer("Kelanjutan privat.") + &completed(json!([])),
    ], false, Some(gate.clone())).await;
    let app = router(state.clone());
    let (id, body) = after_partial(
        &app,
        &cookie,
        json!({"message":"Mulai.","attachments":[attachment]}),
    )
    .await;
    let peer = router(state_with(db::Db::open(&db_dir).unwrap(), None, vec![]));
    let path = format!("/api/conversations/{id}");
    if delete_account {
        let config = db_dir.join("uji.toml");
        std::fs::write(
            &config,
            "data_dir = \".\"\n[azure]\nendpoint = \"http://127.0.0.1:9\"\ndeployment = \"uji\"\n",
        )
        .unwrap();
        archive_accounts::cli(&[
            "--semua-lokal".into(),
            "--terapkan".into(),
            "--config".into(),
            config.to_string_lossy().into_owned(),
        ])
        .await
        .unwrap();
        assert_eq!(
            send(&peer, "GET", &path, None, Some(&cookie)).await.status,
            StatusCode::UNAUTHORIZED
        );
        let store = files::FileStore::User {
            db: state.db.clone(),
            dir: state.files_dir.clone(),
            user_id: owner,
            conversation_id: id.clone(),
        };
        let file = store
            .save("Uji", "txt", b"File yang terlambat".to_vec(), &json!({}))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(files::path_of(&state.files_dir, owner, &file.id, "txt")).unwrap(),
            b"File yang terlambat"
        );
        assert_eq!(
            get_raw(&peer, &format!("/api/files/{}", file.id), &admin)
                .await
                .0,
            StatusCode::NOT_FOUND
        );
    } else {
        assert_eq!(
            send(
                &peer,
                "DELETE",
                &format!("{path}/archive-v1"),
                None,
                Some(&cookie)
            )
            .await
            .status,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(
        send(&peer, "GET", &path, None, Some(&admin)).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get_raw(&peer, &format!("/api/files/{attachment}"), &admin)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get_raw(&peer, &format!("/api/files/{attachment}/preview"), &admin)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    gate.notify_one();
    let remainder = body.collect().await.unwrap().to_bytes();
    let remainder = std::str::from_utf8(&remainder).unwrap();
    assert!(remainder.contains("Kelanjutan privat."));
    {
        let requests = vision.requests.lock().unwrap();
        let output = requests[2]["input"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "function_call_output" && item["call_id"] == "baca")
            .unwrap();
        assert!(
            output["output"]
                .as_str()
                .unwrap()
                .contains("Isi lampiran setelah arsip"),
            "{output}"
        );
    }
    state.traces.flush().await;
    let saved = state
        .db
        .call(move |c| {
            Ok(c.query_row(
        "SELECT m.content FROM messages m JOIN archived_conversations a ON a.conversation_id=m.conversation_id
         WHERE m.conversation_id=?1 AND m.role='assistant'",
        [id], |r| r.get::<_, String>(0),
    )?)
        })
        .await
        .unwrap();
    assert_eq!(saved, "Potongan uji.\n\nKelanjutan privat.");
    std::fs::remove_dir_all(&state.files_dir).unwrap();
    let traces = state.traces.clone();
    drop((app, peer, state));
    traces.close_for_test().await;
    std::fs::remove_dir_all(db_dir).unwrap();
}

#[tokio::test]
async fn trace_commit_cannot_interrupt_sending_or_removing_feedback() {
    use rusqlite::{Connection, ErrorCode, functions::FunctionFlags};

    let dir = std::env::temp_dir().join(format!("app-feedback-lock-{}", rand::random::<u64>()));
    let state = state_with(db::Db::open(&dir).unwrap(), None, vec![]);
    let app = router(state.clone());
    let cookie = admin_cookie(&app).await;
    let user = user_id(&app, &cookie, ADMIN_EMAIL).await;
    let turn = conversations::begin_turn(&state, user, None, "Awal".into(), vec![])
        .await
        .unwrap();
    let answer_id = conversations::save_answer(
        &state,
        turn.id.clone(),
        turn.question_id,
        "Jawaban".into(),
        0,
        Default::default(),
    )
    .await
    .unwrap();
    let feedback = format!("/api/messages/{answer_id}/feedback");
    assert_eq!(
        send(
            &app,
            "POST",
            &feedback,
            Some(json!({"rating":"up"})),
            Some(&cookie)
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );

    let attempts = Arc::new(Mutex::new(Vec::new()));
    let observed = attempts.clone();
    let path = dir.join("rag-hub.sqlite");
    const WRITE_TRACE: &str = "INSERT INTO trace_events (turn_id,span,at_ms,kind,data)
         VALUES ('probe',0,0,'answer','sintetis')";
    state.db.call(move |c| {
        c.execute(
            "INSERT INTO turn_traces (id,guest_question,owner,deployment,server_version,started_at_ms,status)
             VALUES ('probe','uji','uji','uji','tes',0,'running')",
            [],
        )?;
        let writer = Connection::open(path)?;
        writer.busy_timeout(std::time::Duration::ZERO)?;
        c.create_scalar_function("trace_during_ownership", 1, FunctionFlags::SQLITE_UTF8, move |_| {
            observed.lock().unwrap().push(writer.execute(WRITE_TRACE, []).map_err(|err| err.sqlite_error_code()));
            Ok(true)
        })?;
        // Panggil penulis pada SELECT yang benar-benar dipakai untuk cek kepemilikan.
        c.execute_batch(
            "ALTER TABLE conversations RENAME TO fixture_conversations;
             CREATE TEMP VIEW conversations AS SELECT * FROM fixture_conversations WHERE trace_during_ownership(id);
             CREATE TEMP TRIGGER forward_chat_update INSTEAD OF UPDATE ON conversations
             BEGIN
                UPDATE fixture_conversations SET updated_at=NEW.updated_at WHERE id=OLD.id;
             END;",
        )?;
        Ok(())
    }).await.unwrap();

    let next =
        conversations::begin_turn(&state, user, Some(turn.id), "Lanjut".into(), vec![]).await;
    let cleared = send(
        &app,
        "POST",
        &feedback,
        Some(json!({"rating":null})),
        Some(&cookie),
    )
    .await;
    let writes = std::mem::take(&mut *attempts.lock().unwrap());
    assert!(next.is_ok());
    assert_eq!(cleared.status, StatusCode::NO_CONTENT, "{writes:?}");
    assert!(writes.len() >= 2);
    assert!(
        writes
            .iter()
            .all(|attempt| *attempt == Err(Some(ErrorCode::DatabaseBusy)))
    );

    let docs_table = search::INDEX_TABLE;
    state.db.call(move |c| {
        c.execute("INSERT INTO usage_log (created_at,kind) VALUES (?1,'chat')", [db::now()])?;
        c.execute_batch(
            "ALTER TABLE usage_log RENAME TO fixture_usage_log;
             CREATE TEMP VIEW usage_log AS SELECT * FROM fixture_usage_log WHERE trace_during_ownership(id);",
        )?;
        c.execute_batch(&format!(
            "INSERT INTO {docs_table} (id,source,rel_path,title,kind,status,size,mtime,visibility,version,indexed_at)
             VALUES (1,'uji','uji.txt','Uji','txt','ready',0,0,'{{}}',1,0);
             ALTER TABLE {docs_table} RENAME TO fixture_docs;
             CREATE TEMP VIEW {docs_table} AS SELECT * FROM fixture_docs WHERE trace_during_ownership(id);",
        ))?;
        Ok(())
    }).await.unwrap();
    assert_eq!(
        send(&app, "GET", "/api/admin/usage", None, Some(&cookie))
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        documents::rows(&state.db, docs_table, vec![1])
            .await
            .unwrap()
            .len(),
        1
    );
    let reads = std::mem::take(&mut *attempts.lock().unwrap());
    assert!(
        !reads.is_empty() && reads.iter().all(|attempt| *attempt == Ok(1)),
        "{reads:?}"
    );

    let writer = Connection::open(dir.join("rag-hub.sqlite")).unwrap();
    assert_eq!(writer.execute(WRITE_TRACE, []).unwrap(), 1);
    assert_eq!(
        writer
            .query_row("SELECT count(*) FROM feedback", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop((writer, app, state));
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn stop_and_shutdown_keep_partial_answer_and_running_model_span() {
    for shutdown in [false, true] {
        let mut state = app_state();
        let _vision = Mock::start(
            &mut state,
            vec![
                event(
                    "response.created",
                    json!({"response":{"model":"vision-uji"}}),
                ) + &answer("Potongan uji."),
            ],
            true,
        )
        .await;
        let (stop, receiver) = tokio::sync::watch::channel(false);
        Arc::get_mut(&mut state).unwrap().chat_shutdown = receiver;
        let app = router(state.clone());
        let cookie = admin_cookie(&app).await;
        let (id, body) = after_partial(&app, &cookie, json!({"message":"Mulai."})).await;
        if shutdown {
            stop.send(true).unwrap();
            body.collect().await.unwrap();
        } else {
            drop(body);
        }
        let (_, status, reason, detail) = trace(&state, &id).await;
        assert_eq!(status, "stopped");
        assert_eq!(
            reason.as_deref(),
            Some(if shutdown {
                "shutdown"
            } else {
                "client_disconnected"
            })
        );
        assert_eq!(detail["entries"][0]["status"], "stopped");
        assert_eq!(detail["entries"][0]["model"]["text"], "vision-uji");
        assert_eq!(detail["answer"]["text"], "Potongan uji.");
        let observed = send(
            &app,
            "GET",
            &format!("/api/conversations/{id}"),
            None,
            Some(&cookie),
        )
        .await;
        assert_eq!(observed.body["messages"][1]["content"], "Potongan uji.");
    }
}

#[tokio::test]
async fn stream_error_and_empty_failure_are_saved_privately() {
    let mut state = app_state();
    let _vision = Mock::start(
        &mut state,
        vec![
            answer("Potongan uji."),
            event(
                "response.failed",
                json!({"response":{"id":"resp-gagal","error":{"code":"uji"},"usage":{"input_tokens":2}}}),
            ),
        ],
        false,
    )
    .await;
    let app = router(state.clone());
    let cookie = admin_cookie(&app).await;
    for (message, expected) in [
        ("Sebagian", json!("Potongan uji.")),
        ("Kosong", Value::Null),
    ] {
        let (_, sse) = chat(&app, &cookie, json!({"message":message})).await;
        assert!(sse.contains("event: error"));
        let id = conversation_id(&sse);
        let (_, status, reason, detail) = trace(&state, &id).await;
        assert_eq!(status, "failed");
        assert_eq!(reason.as_deref(), Some("model_or_stream_error"));
        assert_eq!(detail["entries"][0]["status"], "failed");
        assert_eq!(detail["answer"]["text"], expected);
        if expected.is_null() {
            assert_eq!(detail["entries"][0]["response_id"], "resp-gagal");
            assert_eq!(detail["entries"][0]["usage"], json!({"input_tokens":2}));
        }
        let observed = send(
            &app,
            "GET",
            &format!("/api/conversations/{id}"),
            None,
            Some(&cookie),
        )
        .await;
        assert_eq!(
            observed.body["messages"].as_array().unwrap().len(),
            if expected.is_null() { 1 } else { 2 }
        );
    }
}

#[tokio::test]
async fn trace_write_failure_leaves_stream_and_answer_storage_working() {
    let mut state = app_state();
    let _vision = Mock::start(
        &mut state,
        vec![answer("Jawaban uji.") + &completed(json!([]))],
        false,
    )
    .await;
    state
        .db
        .call(|c| Ok(c.execute_batch("DROP TABLE turn_traces")?))
        .await
        .unwrap();
    let app = router(state.clone());
    let cookie = admin_cookie(&app).await;
    let (_, sse) = chat(&app, &cookie, json!({"message":"Cek."})).await;
    assert!(sse.contains("Jawaban uji.") && sse.contains("event: done"));
    assert!(!sse.contains("event: error"));
    let id = conversation_id(&sse);
    let detail = send(
        &app,
        "GET",
        &format!("/api/conversations/{id}"),
        None,
        Some(&cookie),
    )
    .await;
    assert_eq!(detail.body["messages"][1]["content"], "Jawaban uji.");
    state.traces.flush().await;
}

#[tokio::test]
async fn long_reply_is_split_into_bounded_append_only_events() {
    let mut state = app_state();
    let huge = "測 ".repeat(34_000);
    let _vision = Mock::start(
        &mut state,
        vec![answer(&huge) + &completed(json!([]))],
        false,
    )
    .await;
    let app = router(state.clone());
    let cookie = admin_cookie(&app).await;
    let (_, sse) = chat(&app, &cookie, json!({"message":"Cek batas kejadian."})).await;
    let (q, status, _, detail) = trace(&state, &conversation_id(&sse)).await;
    assert_eq!(status, "ok");
    assert_eq!(detail["answer"]["text"], huge);
    let (chunks, max, total): (i64, i64, i64) = state.db.call(move |c| Ok(c.query_row(
        "SELECT count(*), max(length(CAST(e.data AS BLOB))), sum(length(CAST(e.data AS BLOB)))
         FROM trace_events e JOIN turn_traces t ON t.id=e.turn_id WHERE t.question_id=?1 AND e.kind='answer'",
        [q], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?)).await.unwrap();
    assert!(chunks > 1 && max <= 8192);
    assert_eq!(total, huge.len() as i64);
}

async fn guest_request(app: &Router, path: &str, question: &str) -> String {
    let req = Request::post(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::USER_AGENT, "AgenSyntetis")
        .header("x-forwarded-for", "192.0.2.17")
        .body(Body::from(json!({"message":question}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
}

#[tokio::test]
async fn guest_archive_requires_new_notice_and_keeps_failures_without_device_identifiers() {
    let mut state = app_state();
    Arc::get_mut(&mut state).unwrap().guest.enabled = true;
    let _vision = Mock::start(
        &mut state,
        vec![
            answer("Web lama.") + &completed(json!([])),
            answer("Jawaban baru.") + &completed(json!([])),
            answer("Potongan tamu."),
        ],
        false,
    )
    .await;
    let app = router(state.clone());
    assert!(
        guest_request(&app, "/api/guest/chat", "Tanya lama.")
            .await
            .contains("event: done")
    );
    assert!(
        guest_request(&app, "/api/guest/chat/archive-v1", "Tanya baru.")
            .await
            .contains("event: done")
    );
    assert!(
        guest_request(&app, "/api/guest/chat/archive-v1", "Tanya gagal.")
            .await
            .contains("event: error")
    );
    state.traces.flush().await;
    state
        .db
        .call(|c| {
            let mut stmt = c.prepare(
                "SELECT guest_question, status, id, duration_ms, started_at_ms, owner
                 FROM turn_traces WHERE guest_question IS NOT NULL ORDER BY status DESC",
            )?;
            let mut rows = stmt.query([])?;
            for (expected_question, expected_status, expected_answer, effort) in [
                ("Tanya baru.", "ok", "Jawaban baru.", "medium"),
                ("Tanya gagal.", "failed", "Potongan tamu.", "low"),
            ] {
                let row = rows.next()?.unwrap();
                let question: String = row.get(0)?;
                let status: String = row.get(1)?;
                let id: String = row.get(2)?;
                let detail = read_detail(c, &id)?;
                let owner: String = row.get(5)?;
                let combined = format!("{question}{detail}{owner}");
                assert!(!combined.contains("192.0.2.17") && !combined.contains("AgenSyntetis"));
                assert_eq!(question, expected_question);
                assert_eq!(status, expected_status);
                assert!(row.get::<_, i64>(3)? >= 0);
                assert!(row.get::<_, i64>(4)? > 0);
                assert_eq!(detail["answer"]["text"], expected_answer);
                assert_eq!(detail["entries"][0]["effort"]["text"], effort);
                assert_eq!(
                    detail["entries"][0]["prompt_sha256"]
                        .as_str()
                        .unwrap()
                        .len(),
                    64
                );
            }
            assert!(rows.next()?.is_none());
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn a_failed_guest_archive_write_keeps_the_answer_stream() {
    let mut state = app_state();
    Arc::get_mut(&mut state).unwrap().guest.enabled = true;
    let _vision = Mock::start(
        &mut state,
        vec![answer("Tetap menjawab.") + &completed(json!([]))],
        false,
    )
    .await;
    state
        .db
        .call(|c| Ok(c.execute_batch("DROP TABLE trace_events")?))
        .await
        .unwrap();
    let app = router(state.clone());
    let sse = guest_request(&app, "/api/guest/chat/archive-v1", "Uji gagal menyimpan.").await;
    assert!(sse.contains("Tetap menjawab.") && sse.contains("event: done"));
    state.traces.flush().await;
}
