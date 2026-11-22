use super::*;
use axum::routing::post;

#[tokio::test]
async fn pptx_tool_persists_file_with_owner_only_read_preview_and_download() {
    let input = json!({
        "filename":"Uji PowerPoint",
        "slides":[
            {"type":"chart","title":"Uji grafik","chart_type":"bar","unit":"kg",
                "categories":["A","B"],"series":[{"name":"Contoh","values":[12.0,15.0]}]},
            {"type":"table","title":"Uji presisi","columns":["Kode","Nilai","Status","Catatan"],
                "rows":[[9007199254740993u64,0.0004,true,null]]}
        ]
    });
    let arguments = input.to_string();
    let mock = Router::new().route(
        "/openai/v1/responses",
        post(move |Json(body): Json<Value>| {
            let arguments = arguments.clone();
            async move {
                let result = body["input"].as_array().unwrap().iter()
                    .find(|item| item["type"] == "function_call_output");
                let (event, output) = if let Some(result) = result {
                    let result: Value = serde_json::from_str(result["output"].as_str().unwrap()).unwrap();
                    assert_eq!(result["status"], "ok");
                    (json!({"type":"response.output_text.delta","delta":"Presentasi siap."}), json!([]))
                } else {
                    assert!(body["tools"].as_array().unwrap().iter()
                        .any(|tool| tool["name"] == "create_pptx" && tool["strict"] == true));
                    let call = json!({"type":"function_call","name":"create_pptx","call_id":"pptx-1","arguments":arguments});
                    (json!({"type":"response.output_item.added","item":call}), json!([call]))
                };
                let done = json!({"type":"response.completed","response":{"output":output}});
                ([(header::CONTENT_TYPE, "text/event-stream")],
                    format!("data: {event}\n\ndata: {done}\n\ndata: [DONE]\n\n"))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, mock).await });
    let mut state = app_state();
    Arc::get_mut(&mut state).unwrap().llm = Llm::new(&config::AzureConfig {
        endpoint: format!("http://{address}"),
        deployment: "sintetis".into(),
        api_version: String::new(),
        api_key: "sintetis".into(),
        context_tokens: 128_000,
    });
    let app = router(state.clone());
    let admin = admin_cookie(&app).await;
    let (owner_id, owner) = active_user(&app, &admin, "presentasi-1@example.com").await;
    let (other_id, other) = active_user(&app, &admin, "presentasi-2@example.com").await;
    let (status, sse) = chat(&app, &owner, json!({"message":"Buat presentasi uji"})).await;
    server.abort();

    assert_eq!(status, StatusCode::OK);
    assert!(sse.contains("Membuat presentasi PowerPoint…"));
    assert!(sse.contains("Presentasi siap."));
    let file: Value = serde_json::from_str(
        sse.split_once("event: file\ndata: ")
            .unwrap()
            .1
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(file["name"], "Uji PowerPoint.pptx");
    assert_eq!(file["kind"], "pptx");
    let id = file["id"].as_str().unwrap();
    let conversation = conversation_id(&sse);
    let url = format!("/api/files/{id}");
    let preview_url = format!("{url}/preview");
    let detail = send(
        &app,
        "GET",
        &format!("/api/conversations/{conversation}"),
        None,
        Some(&owner),
    )
    .await;
    assert_eq!(detail.body["messages"][1]["files"][0], file);
    let preview = send(&app, "GET", &preview_url, None, Some(&owner)).await;
    assert_eq!(preview.status, StatusCode::OK);
    assert_eq!(preview.body["spec"]["filename"], input["filename"]);
    let mut preview_slides = input["slides"].clone();
    preview_slides[1]["rows"][0] = json!(["9007199254740993", "0.0004", true, null]);
    assert_eq!(preview.body["spec"]["slides"], preview_slides);
    let (status, mime, bytes) = get_raw(&app, &url, &owner).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        mime.as_deref(),
        Some("application/vnd.openxmlformats-officedocument.presentationml.presentation")
    );
    assert_eq!(bytes.len() as u64, file["size"].as_u64().unwrap());
    assert!(bytes.starts_with(b"PK"));
    for route in [url, preview_url] {
        assert_eq!(
            send(&app, "GET", &route, None, Some(&other)).await.status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(&app, "GET", &route, None, None).await.status,
            StatusCode::UNAUTHORIZED
        );
    }

    let reader = |user_id, conversation_id| agent::ToolContext {
        files: files::FileStore::User {
            db: state.db.clone(),
            dir: state.files_dir.clone(),
            user_id,
            conversation_id,
        },
        docs: None,
    };
    let read = agent::ToolCall {
        id: "read-1".into(),
        name: "read_pptx".into(),
        arguments: json!({"file_id":id,"from_slide":1}).to_string(),
    };
    let owner_context = reader(owner_id, conversation.clone());
    let result = agent::run_tool(&read, &owner_context, 4000, &mut Vec::new()).await;
    assert!(result.ok, "{}", result.output);
    let page: Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(page["slides"], input["slides"]);
    for context in [
        reader(other_id, conversation),
        reader(owner_id, "percakapan-lain".into()),
    ] {
        let denied = agent::run_tool(&read, &context, 4000, &mut Vec::new()).await;
        assert!(!denied.ok && denied.output.contains("File tidak ditemukan"));
    }
    let attachment_read = agent::ToolCall {
        name: "read_attachment".into(),
        ..read
    };
    let result = agent::run_tool(&attachment_read, &owner_context, 4000, &mut Vec::new()).await;
    assert!(!result.ok && result.output.contains("read_pptx"));
}
