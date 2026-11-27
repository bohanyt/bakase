use super::*;
use axum::routing::post;

#[tokio::test]
async fn table_is_offered_to_capable_panels_and_saved_for_calculation() {
    let mock = Router::new().route(
        "/openai/v1/responses",
        post(async |Json(body): Json<Value>| {
            let calls = body["input"].as_array().unwrap();
            let result = |id: &str| {
                calls.iter().find(|item| item["type"] == "function_call_output" && item["call_id"] == id)
                    .map(|item| serde_json::from_str::<Value>(item["output"].as_str().unwrap()).unwrap())
            };
            let names = body["tools"].as_array().unwrap();
            let offered = |name| names.iter().any(|tool| tool["name"] == name);
            assert!(offered("gulir"));
            let call = |name: &str, id: &str, args: Value| {
                json!({"type":"function_call","name":name,"call_id":id,"arguments":args.to_string()})
            };
            let (event, output) = if let Some(calculation) = result("sql") {
                assert_eq!(calculation["hasil"], json!([[2, 12, "=1+1", "1.234,50"]]));
                (json!({"type":"response.output_text.delta","delta":"Excel siap."}), json!([]))
            } else if let Some(table) = result("tabel") {
                assert_eq!(table["baris"], 2);
                assert_eq!(table["lengkap"], false);
                assert_eq!(table["contoh"][0][2], "1.234,50");
                assert!(!table["alasan"].as_array().unwrap().is_empty());
                let item = call("hitung_xlsx", "sql", json!({
                    "file_id": table["file_id"], "sheet": "Tabel",
                    "sql": "SELECT COUNT(*), SUM(CAST(B AS INTEGER)), MIN(A), MIN(C) FROM data"
                }));
                (json!({"type":"response.output_item.added","item":item}), json!([item]))
            } else if offered("ambil_tabel") {
                let item = call("ambil_tabel", "tabel", json!({"ref":"f2e1"}));
                (json!({"type":"response.output_item.added","item":item}), json!([item]))
            } else {
                (json!({"type":"response.output_text.delta","delta":"Panel lama."}), json!([]))
            };
            let done = json!({"type":"response.completed","response":{"output":output}});
            ([(header::CONTENT_TYPE, "text/event-stream")], format!("data: {event}\n\ndata: {done}\n\ndata: [DONE]\n\n"))
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
    let app = router(state);
    let owner = admin_cookie(&app).await;
    let (_, other) = active_user(&app, &owner, "tabel-uji@example.com").await;
    let upload = || {
        uploaded(
            &app,
            &owner,
            "Halaman - Uji.md",
            "# Tabel uji\n\n- grid [f2e1]".as_bytes().to_vec(),
        )
    };
    let origin = "chrome-extension://ponmlkjihgfedcbaponmlkjihgfedcba";
    let id = upload().await;
    let (_, old) = chat(
        &app,
        &owner,
        json!({
            "message":"Ambil tabel", "attachments":[id],
            "page":{"origin":origin,"attachment_id":id}
        }),
    )
    .await;
    assert!(old.contains("Panel lama.") && !old.contains("event: page_action"));

    let id = upload().await;
    let req = Request::builder()
        .method("POST")
        .uri("/api/chat")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, &owner)
        .body(Body::from(
            json!({
                "message":"Ambil tabel", "attachments":[id],
                "page":{"origin":origin,"attachment_id":id,"ambil_tabel":true}
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let mut stream = res.into_body();
    let mut sse = String::new();
    let action_id = loop {
        let data = stream.frame().await.unwrap().unwrap().into_data().unwrap();
        sse.push_str(std::str::from_utf8(&data).unwrap());
        if let Some(line) = sse.split_once("event: page_action\ndata: ") {
            let command: Value = serde_json::from_str(line.1.lines().next().unwrap()).unwrap();
            assert_eq!(command["action"], "ambil_tabel");
            assert_eq!(command["ref"], "f2e1");
            assert!(command["remaining_ms"].as_u64().unwrap() <= 65_000);
            break command["id"].as_str().unwrap().to_owned();
        }
    };
    let endpoint = "/api/chat/page-action";
    let reply = json!({"id": action_id,"reply":{"status":"table","table":{
        "title":"Tabel uji","columns":["Kode","Nilai","Tampilan"],
        "rows":[["=1+1","5","1.234,50"],["abc","7","2,345.60"]],
        "issues":["missing_rows"]
    }}});
    assert_eq!(
        send(&app, "POST", endpoint, Some(reply.clone()), None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&app, "POST", endpoint, Some(reply.clone()), Some(&other))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let oversized = json!({"id": action_id,"reply":"x".repeat(page::MAX_COMPLETION_BYTES)});
    assert_eq!(
        send(&app, "POST", endpoint, Some(oversized), Some(&owner))
            .await
            .status,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let progress = json!({"id": action_id,"progress":2});
    assert_eq!(
        send(&app, "POST", endpoint, Some(progress), Some(&owner))
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    let data = stream.frame().await.unwrap().unwrap().into_data().unwrap();
    assert!(
        std::str::from_utf8(&data)
            .unwrap()
            .contains("Mengambil tabel: 2 baris")
    );
    assert_eq!(
        send(&app, "POST", endpoint, Some(reply), Some(&owner))
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    let rest = stream.collect().await.unwrap().to_bytes();
    sse.push_str(std::str::from_utf8(&rest).unwrap());
    server.abort();

    assert!(sse.contains("Excel siap.") && sse.contains("Tabel belum lengkap (2 baris)"));
    let file: Value = serde_json::from_str(
        sse.split_once("event: file\ndata: ")
            .unwrap()
            .1
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(file["name"], "Tabel - Tabel uji.xlsx");
    let url = format!("/api/files/{}", file["id"].as_str().unwrap());
    assert_eq!(get_raw(&app, &url, &owner).await.0, StatusCode::OK);
    assert_eq!(get_raw(&app, &url, &other).await.0, StatusCode::NOT_FOUND);
    let detail = send(
        &app,
        "GET",
        &format!("/api/conversations/{}", conversation_id(&sse)),
        None,
        Some(&owner),
    )
    .await;
    assert_eq!(detail.body["messages"][1]["files"][0], file);
}
