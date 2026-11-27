use crate::{AppState, AuthUser, error::ApiError};
use axum::{
    extract::{Path, State},
    http::StatusCode,
};
use rusqlite::{Transaction, TransactionBehavior::Immediate, params};
use std::sync::Arc;

pub(crate) const SYSTEM_ID: i64 = -1;
pub(crate) const SYSTEM_EMAIL: &str = "archive@app.invalid";

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS archived_conversations (
    conversation_id TEXT PRIMARY KEY REFERENCES conversations(id),
    original_user_id INTEGER NOT NULL,
    original_email TEXT NOT NULL,
    original_name TEXT NOT NULL,
    archived_at INTEGER NOT NULL
);
CREATE TRIGGER IF NOT EXISTS archive_protect_account_update BEFORE UPDATE ON users
WHEN OLD.id = -1 BEGIN SELECT RAISE(ABORT, 'akun sistem terkunci'); END;
CREATE TRIGGER IF NOT EXISTS archive_protect_account_delete BEFORE DELETE ON users
WHEN OLD.id = -1 BEGIN SELECT RAISE(ABORT, 'akun sistem terkunci'); END;
CREATE TRIGGER IF NOT EXISTS archive_keep_feedback BEFORE DELETE ON users
WHEN OLD.id <> -1
BEGIN
    UPDATE feedback SET user_id=-1 WHERE user_id=OLD.id AND message_id IN (
        SELECT m.id FROM archived_conversations a JOIN messages m ON m.conversation_id=a.conversation_id
        WHERE a.original_user_id=OLD.id
    );
END;
CREATE TRIGGER IF NOT EXISTS archive_protect_session BEFORE INSERT ON sessions
WHEN NEW.user_id = -1 BEGIN SELECT RAISE(ABORT, 'akun sistem terkunci'); END;
CREATE TRIGGER IF NOT EXISTS archive_normalize_new_file AFTER INSERT ON files
WHEN NEW.user_id <> -1 AND EXISTS (
    SELECT 1 FROM conversations WHERE id=NEW.conversation_id AND user_id=-1
)
BEGIN
    UPDATE files SET user_id=-1 WHERE id=NEW.id;
END;
CREATE TRIGGER IF NOT EXISTS archive_normalize_updated_file AFTER UPDATE OF user_id, conversation_id ON files
WHEN NEW.user_id <> -1 AND EXISTS (
    SELECT 1 FROM conversations WHERE id=NEW.conversation_id AND user_id=-1
)
BEGIN
    UPDATE files SET user_id=-1 WHERE id=NEW.id;
END;
-- Pembersih file versi lama masih menghapus file tanpa message_id.
CREATE TRIGGER IF NOT EXISTS archive_keep_files BEFORE DELETE ON files
WHEN OLD.user_id = -1 BEGIN SELECT RAISE(IGNORE); END;
"#;

pub(crate) async fn conversation(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let changed = state
        .db
        .call(move |c| {
            let tx = c.transaction_with_behavior(Immediate)?;
            let (archived, _) = archive_conversations(&tx, user.id, Some(&id))?;
            tx.commit()?;
            Ok(archived != 0)
        })
        .await
        .map_err(|_| {
            tracing::warn!("pengarsipan percakapan gagal");
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "Arsip IT gagal disimpan. Percakapan belum dihapus. Coba lagi.",
            )
        })?;
    if !changed {
        return Err(ApiError::not_found(
            "Percakapan tidak ditemukan atau sudah diarsipkan.",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) fn archive_conversations(
    tx: &Transaction<'_>,
    owner: i64,
    id: Option<&str>,
) -> anyhow::Result<(usize, usize)> {
    let archived = tx.execute(
        "INSERT INTO archived_conversations
             SELECT c.id, u.id, u.email, u.name, ?3
             FROM conversations c JOIN users u ON u.id=c.user_id
             WHERE c.user_id=?2 AND (?1 IS NULL OR c.id=?1)",
        params![id, owner, crate::db::now()],
    )?;
    if archived == 0 {
        return Ok((0, 0));
    }
    ensure_system(tx)?;
    let files = tx.execute(
        "UPDATE files SET user_id=?3 WHERE conversation_id IN (
             SELECT id FROM conversations WHERE user_id=?2 AND (?1 IS NULL OR id=?1)
         )",
        params![id, owner, SYSTEM_ID],
    )?;
    let moved = tx.execute(
        "UPDATE conversations SET user_id=?3 WHERE user_id=?2 AND (?1 IS NULL OR id=?1)",
        params![id, owner, SYSTEM_ID],
    )?;
    anyhow::ensure!(archived == moved, "jumlah percakapan arsip tidak cocok");
    Ok((archived, files))
}

pub(crate) fn ensure_system(tx: &Transaction<'_>) -> anyhow::Result<()> {
    tx.execute(
        "INSERT INTO users (id, email, name, status, password_hash, created_at)
             VALUES (?1, ?2, 'Arsip IT', 'disabled', '', ?3) ON CONFLICT(id) DO NOTHING",
        params![SYSTEM_ID, SYSTEM_EMAIL, crate::db::now()],
    )?;
    let reserved: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND email=?2 AND status='disabled'
                AND password_hash='' AND role='user' AND NOT EXISTS(SELECT 1 FROM sessions WHERE user_id=?1))",
        params![SYSTEM_ID, SYSTEM_EMAIL], |r| r.get(0),
    )?;
    anyhow::ensure!(reserved, "identitas akun sistem tidak sah");
    Ok(())
}
