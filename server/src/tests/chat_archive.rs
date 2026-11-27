use super::*;
use rusqlite::{Connection, OptionalExtension, params};

const QUESTION: &str = "catatan-uji-privat";
const FILE: &[u8] = b"lampiran sintetis";

fn legacy(name: &str) -> &'static str {
    include_str!("ba88633.sql")
        .split("\n-- ")
        .find_map(|part| {
            let (tag, sql) = part.split_once('\n')?;
            (tag == name).then(|| sql.trim_end())
        })
        .unwrap_or_else(|| panic!("SQL ba88633 tidak ditemukan: {name}"))
}

#[tokio::test]
async fn ba88633_finishes_an_inflight_answer_and_normalizes_files_after_archive() {
    let dir = std::env::temp_dir().join(format!("app-legacy-slot-{}", rand::random::<u64>()));
    let state = state_with(db::Db::open(&dir).unwrap(), None, vec![]);
    let app = router(state.clone());
    let cookie = admin_cookie(&app).await;
    let user = user_id(&app, &cookie, ADMIN_EMAIL).await;
    let turn = conversations::begin_turn(&state, user, None, QUESTION.into(), vec![])
        .await
        .unwrap();
    let id = turn.id;
    let q = turn.question_id;

    let old = Connection::open(dir.join("rag-hub.sqlite")).unwrap();
    old.pragma_update(None, "foreign_keys", true).unwrap();
    let version: i64 = old
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 10);
    let archive = format!("/api/conversations/{id}/archive-v1");
    assert_eq!(
        send(&app, "DELETE", &archive, None, Some(&cookie))
            .await
            .status,
        StatusCode::NO_CONTENT
    );

    let file = "c".repeat(32);
    let physical = files::path_of(&state.files_dir, user, &file, "txt");
    std::fs::create_dir_all(physical.parent().unwrap()).unwrap();
    std::fs::write(&physical, FILE).unwrap();
    old.execute(
        legacy("create_file"),
        params![file, user, id, "Uji.txt", "txt", FILE.len() as u32, "{}", 0],
    )
    .unwrap();
    let lookup = |uid| {
        old.query_row(legacy("download"), params![file, uid], |r| {
            r.get::<_, String>(0)
        })
        .optional()
        .unwrap()
    };
    assert_eq!(lookup(user), None);
    assert_eq!(lookup(archive::SYSTEM_ID).as_deref(), Some("Uji.txt"));
    let cleaned = old
        .prepare(legacy("cleanup"))
        .unwrap()
        .query_map([1], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(cleaned.is_empty());

    let answer = "Jawaban utuh tetap disimpan. ".repeat(6_000);
    assert_eq!(
        old.execute(
            legacy("save_answer"),
            params![id, answer, 0, 17, q, "Ringkasan", "[1]", "[2]"]
        )
        .unwrap(),
        1
    );
    let answer_id = old.last_insert_rowid();
    assert_eq!(
        old.execute(legacy("bind_answer_file"), params![answer_id, file, id])
            .unwrap(),
        1
    );
    let stored: (String, String, String, String) = old
        .query_row(
            "SELECT content,reasoning,steps,sources FROM messages WHERE id=?1 AND reply_to=?2",
            params![answer_id, q],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        stored,
        (answer, "Ringkasan".into(), "[1]".into(), "[2]".into())
    );
    let owner: (i64, String, String) = old.query_row(
        "SELECT original_user_id,original_email,original_name FROM archived_conversations WHERE conversation_id=?1",
        [&id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).unwrap();
    assert_eq!(owner, (user, ADMIN_EMAIL.into(), "Admin".into()));
    assert_eq!(
        std::fs::read(files::path_of(&state.files_dir, owner.0, &file, "txt")).unwrap(),
        FILE
    );
    assert_eq!(
        get_raw(&app, &format!("/api/files/{file}"), &cookie)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(old.execute(legacy("delete"), params![id, user]).unwrap(), 0);

    let pending = "e".repeat(32);
    old.execute(
        legacy("create_file"),
        params![
            pending,
            user,
            Option::<String>::None,
            "Uji.txt",
            "txt",
            4,
            "{}",
            0
        ],
    )
    .unwrap();
    old.execute(
        "UPDATE files SET conversation_id=?1 WHERE id=?2",
        params![id, pending],
    )
    .unwrap();
    assert_eq!(
        old.query_row("SELECT user_id FROM files WHERE id=?1", [&pending], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        archive::SYSTEM_ID
    );

    let other = conversations::begin_turn(&state, user, None, "Percakapan biasa".into(), vec![])
        .await
        .unwrap();
    assert_eq!(
        old.execute(
            legacy("save_answer"),
            params![
                other.id,
                "Jawaban biasa",
                0,
                1,
                other.question_id,
                "",
                "[]",
                "[]"
            ]
        )
        .unwrap(),
        1
    );
    assert!(
        old.prepare(legacy("delete_files"))
            .unwrap()
            .query(params![other.id, user])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
    assert_eq!(
        old.execute(legacy("delete"), params![other.id, user])
            .unwrap(),
        1
    );
    drop((old, app, state));
    std::fs::remove_dir_all(dir).unwrap();
    let _ = std::fs::remove_file(physical);
}

fn old_feedback(
    c: &Connection,
    admin: i64,
) -> rusqlite::Result<Vec<(Option<String>, Option<String>)>> {
    c.prepare(legacy("feedback"))?
        .query_map([admin], |r| {
            assert!(r.get::<_, Option<String>>(11)?.is_none());
            assert!(r.get::<_, Option<String>>(12)?.is_none());
            assert!(r.get::<_, Option<i64>>(13)?.is_none());
            Ok((r.get(4)?, r.get(5)?))
        })?
        .collect()
}

#[tokio::test]
async fn archive_is_hidden_from_all_roles_and_old_feedback_shows_only_shared_copy() {
    let state = app_state();
    let app = router(state.clone());
    let admin = admin_cookie(&app).await;
    let aid = user_id(&app, &admin, ADMIN_EMAIL).await;
    let (user, owner) = active_user(&app, &admin, "pemilik@example.com").await;
    let (other_id, other) = active_user(&app, &admin, "lain@example.com").await;
    let attachment = uploaded(&app, &owner, "uji.txt", FILE.to_vec()).await;
    let turn = conversations::begin_turn(
        &state,
        user,
        None,
        QUESTION.into(),
        vec![attachment.clone()],
    )
    .await
    .unwrap();
    let id = turn.id;
    let q = turn.question_id;
    let shared = conversations::save_answer(
        &state,
        id.clone(),
        q,
        "Jawaban dibagikan".into(),
        22,
        Default::default(),
    )
    .await
    .unwrap();
    let private = conversations::save_answer(
        &state,
        id.clone(),
        q,
        "JAWABAN TIDAK DIBAGIKAN".into(),
        22,
        Default::default(),
    )
    .await
    .unwrap();
    for (answer, share) in [(shared, true), (private, false)] {
        let response = send(
            &app,
            "POST",
            &format!("/api/messages/{answer}/feedback"),
            Some(json!({"rating":"down","share":share})),
            Some(&owner),
        )
        .await;
        assert_eq!(response.status, StatusCode::NO_CONTENT);
    }
    let path = format!("/api/conversations/{id}");
    let for_summary = id.clone();
    state
        .db
        .call(move |c| {
            c.execute(
            "INSERT INTO conversation_compactions (conversation_id, through_message_id, content)
             VALUES (?1, ?2, 'RINGKASAN PRIVAT')", params![for_summary, shared],
        )?;
            Ok(())
        })
        .await
        .unwrap();
    let before = get_raw(&app, &format!("/api/files/{attachment}"), &owner).await;
    assert_eq!(before.0, StatusCode::OK);
    assert_eq!(
        send(
            &app,
            "DELETE",
            &format!("{path}/archive-v1"),
            None,
            Some(&owner)
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    for cookie in [&owner, &other, &admin] {
        assert_eq!(
            send(&app, "GET", "/api/conversations", None, Some(cookie))
                .await
                .body,
            json!([])
        );
        assert_eq!(
            send(&app, "GET", &path, None, Some(cookie)).await.status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(
                &app,
                "POST",
                "/api/conversations/search",
                Some(json!({"q":QUESTION})),
                Some(cookie)
            )
            .await
            .body,
            json!([])
        );
        assert_eq!(
            get_raw(&app, &format!("/api/files/{attachment}"), cookie)
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get_raw(&app, &format!("/api/files/{attachment}/preview"), cookie)
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(
                &app,
                "POST",
                &format!("/api/messages/{shared}/feedback"),
                Some(json!({"rating":"up"})),
                Some(cookie)
            )
            .await
            .status,
            StatusCode::NOT_FOUND
        );
        let (status, sse) = chat(
            &app,
            cookie,
            json!({"message":"Dari tab lain","conversation_id":id}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(sse.contains("sudah diarsipkan"));
    }
    let feedback = send(&app, "GET", "/api/admin/feedback", None, Some(&admin))
        .await
        .body;
    assert!(feedback.to_string().contains("Jawaban dibagikan"));
    assert!(!feedback.to_string().contains("JAWABAN TIDAK DIBAGIKAN"));
    for item in feedback["items"].as_array().unwrap() {
        assert!(
            item["conversation_id"].is_null()
                && item["message_id"].is_null()
                && item["conversation_title"].is_null()
        );
    }
    let users = send(&app, "GET", "/api/admin/users", None, Some(&admin))
        .await
        .body;
    assert_eq!(users.as_array().unwrap().len(), 3);
    assert!(!users.to_string().contains(archive::SYSTEM_EMAIL));
    for (path, body) in [
        (
            "/api/admin/users/-1",
            json!({"status":"active","department":"Review"}),
        ),
        ("/api/admin/users/-1/reset-password", json!({})),
    ] {
        assert_eq!(
            send(&app, "POST", path, Some(body), Some(&admin))
                .await
                .status,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        login(&app, archive::SYSTEM_EMAIL, "password-uji-123")
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );

    state
        .db
        .call(move |c| {
            let search = legacy("search_query")
                .replace(
                    "{}",
                    &legacy("search_mask")
                        .replacen("{}", "c.title", 1)
                        .replacen("{}", "4", 1)
                        .replace("{i}", "0"),
                )
                .replace(
                    "{word_clauses}",
                    &legacy("search_clause").replace("{p}", "4"),
                );
            for uid in [user, aid, other_id] {
                assert!(
                    c.prepare(legacy("list_chats"))?
                        .query(params![uid, 300])?
                        .next()?
                        .is_none()
                );
                assert!(
                    c.prepare(legacy("owned"))?
                        .query(params![id, uid])?
                        .next()?
                        .is_none()
                );
                assert!(
                    c.prepare(&search)?
                        .query(params![uid, 50, 0, "%catatan-uji%"])?
                        .next()?
                        .is_none()
                );
                assert!(
                    c.prepare(legacy("download"))?
                        .query(params![attachment, uid])?
                        .next()?
                        .is_none()
                );
            }
            // Salinan feedback lama sengaja dipertahankan. Komentar privat dan tabel pesan tidak dipakai untuk membuat salinan baru.
            let rows = old_feedback(c, aid)?;
            assert!(rows.contains(&(Some(QUESTION.into()), Some("Jawaban dibagikan".into()))));
            assert!(rows.contains(&(None, None)));
            assert_eq!(rows.len(), 2);
            assert_eq!(
                c.execute(
                    legacy("update_user"),
                    params!["active", "user", "Review", true, 0, aid, other_id],
                )?,
                1
            );
            let other_hash = auth::hash_password("password-uji-123")?;
            assert_eq!(
                c.execute(legacy("reset"), params![other_hash, other_id])?,
                1
            );
            let (uid, status, hash): (i64, String, String) =
                c.query_row(legacy("login"), ["lain@example.com"], |r| {
                    Ok((r.get(0)?, r.get(5)?, r.get(6)?))
                })?;
            assert_eq!(uid, other_id);
            assert_eq!(status, "active");
            assert!(auth::verify_password("password-uji-123", &hash));
            assert_eq!(
                c.execute(legacy("session"), params!["sesi-lama", uid, 0, 1, hash])?,
                1
            );
            assert_eq!(
                c.prepare(legacy("list_users"))?
                    .query_map([], |r| r.get::<_, i64>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                    .len(),
                4
            );
            let (sys, status, hash): (i64, String, String) =
                c.query_row(legacy("login"), [archive::SYSTEM_EMAIL], |r| {
                    Ok((r.get(0)?, r.get(5)?, r.get(6)?))
                })?;
            assert_eq!(sys, archive::SYSTEM_ID);
            assert_eq!(status, "disabled");
            assert!(!auth::verify_password("sintetis", &hash));
            for error in [
                c.execute(
                    legacy("update_user"),
                    params!["active", "user", "Review", true, 0, aid, sys],
                )
                .unwrap_err(),
                c.execute(legacy("reset"), params!["hash-uji", sys])
                    .unwrap_err(),
                c.execute("DELETE FROM users WHERE id=?1", [sys])
                    .unwrap_err(),
                c.execute("INSERT INTO sessions VALUES ('uji',?1,0,1)", [sys])
                    .unwrap_err(),
            ] {
                assert_eq!(
                    error.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::ConstraintViolation)
                );
            }
            assert_eq!(
                c.execute(legacy("session"), params!["uji", sys, 0, 1, hash])?,
                0
            );
            assert_eq!(c.execute("DELETE FROM users WHERE id=?1", [user])?, 1);
            assert_eq!(old_feedback(c, aid)?, rows);
            assert_eq!(
                c.query_row(
                    "SELECT count(*) FROM feedback WHERE user_id=?1",
                    [sys],
                    |r| r.get::<_, i64>(0),
                )?,
                2
            );
            assert_eq!(
                c.query_row(
                    "SELECT c.user_id,a.original_user_id,m.content,cc.content,f.user_id
                 FROM archived_conversations a
                 JOIN conversations c ON c.id=a.conversation_id
                 JOIN messages m ON m.id=?2 AND m.conversation_id=c.id
                 JOIN conversation_compactions cc ON cc.conversation_id=c.id
                 JOIN files f ON f.conversation_id=c.id
                 WHERE c.id=?1",
                    params![id, private],
                    |r| Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?
                    )),
                )?,
                (
                    sys,
                    user,
                    "JAWABAN TIDAK DIBAGIKAN".into(),
                    "RINGKASAN PRIVAT".into(),
                    sys
                )
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn a_failed_archive_leaves_the_conversation_visible_and_usable() {
    let state = app_state();
    let app = router(state.clone());
    let cookie = admin_cookie(&app).await;
    let owner = user_id(&app, &cookie, ADMIN_EMAIL).await;
    let turn = conversations::begin_turn(&state, owner, None, QUESTION.into(), vec![])
        .await
        .unwrap();
    let path = format!("/api/conversations/{}", turn.id);
    state
        .db
        .call(|c| {
            c.execute_batch(
                "CREATE TRIGGER fail_transfer BEFORE UPDATE OF user_id ON conversations
                         BEGIN SELECT RAISE(ABORT, 'uji'); END;",
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let failed = send(
        &app,
        "DELETE",
        &format!("{path}/archive-v1"),
        None,
        Some(&cookie),
    )
    .await;
    assert_eq!(failed.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(failed.body.to_string().contains("Percakapan belum dihapus"));
    let detail = send(&app, "GET", &path, None, Some(&cookie)).await;
    assert_eq!(detail.body["messages"][0]["content"], QUESTION);
    state
        .db
        .call(|c| {
            assert_eq!(
                c.query_row("SELECT count(*) FROM archived_conversations", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
            assert_eq!(
                c.query_row("SELECT count(*) FROM users WHERE id=-1", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn restoring_a_database_from_before_the_file_keeps_its_bytes() {
    let dir = std::env::temp_dir().join(format!("app-archive-restore-{}", rand::random::<u64>()));
    let restored = dir.join("restored");
    std::fs::create_dir_all(&restored).unwrap();
    let files_dir = dir.join("files");
    let mut state = state_with(db::Db::open(&dir).unwrap(), None, vec![]);
    Arc::get_mut(&mut state).unwrap().files_dir = files_dir.clone();
    let app = router(state.clone());
    let cookie = admin_cookie(&app).await;
    let user = user_id(&app, &cookie, ADMIN_EMAIL).await;
    let snapshot = restored.join("rag-hub.sqlite");
    state
        .db
        .call(move |c| {
            c.execute("VACUUM INTO ?1", [snapshot.to_str().unwrap()])?;
            Ok(())
        })
        .await
        .unwrap();

    let attachment = uploaded(&app, &cookie, "uji.txt", FILE.to_vec()).await;
    let turn = conversations::begin_turn(
        &state,
        user,
        None,
        QUESTION.into(),
        vec![attachment.clone()],
    )
    .await
    .unwrap();
    assert_eq!(
        send(
            &app,
            "DELETE",
            &format!("/api/conversations/{}/archive-v1", turn.id),
            None,
            Some(&cookie)
        )
        .await
        .status,
        StatusCode::NO_CONTENT,
    );
    let restored_db = db::Db::open(&restored).unwrap();
    files::cleanup_orphans(restored_db.clone(), files_dir.clone()).await;
    let file = files::path_of(&files_dir, user, &attachment, "txt");
    assert_eq!(std::fs::read(file).unwrap(), FILE);
    drop((restored_db, app, state));
    std::fs::remove_dir_all(dir).unwrap();
}
