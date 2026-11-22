//! Feedback 👍/👎 untuk jawaban Bakase.
//!
//! Isi percakapan tetap privat: admin hanya melihat pertanyaan dan jawaban
//! kalau 👎 dikirim dengan "kirim ke admin" tercentang (tercentang dari awal di
//! form, user bisa menghapusnya). Salinannya disimpan di baris
//! feedback, jadi tetap ada walau user kemudian menghapus percakapannya.

use crate::{AppState, acl::AuthUser, auth::AdminUser, db::now, error::ApiError};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const REASONS: &[&str] = &["salah", "kurang_lengkap", "tidak_relevan", "lainnya"];
const MAX_COMMENT: usize = 1000;

#[derive(Deserialize)]
pub struct FeedbackRequest {
    /// "up", "down", atau null untuk membatalkan.
    rating: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    share: bool,
}

pub async fn submit(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(message_id): Path<i64>,
    Json(req): Json<FeedbackRequest>,
) -> Result<StatusCode, ApiError> {
    let rating = match req.rating.as_deref() {
        None => None,
        Some(r @ ("up" | "down")) => Some(r.to_string()),
        Some(_) => return Err(ApiError::bad_request("Penilaian tidak dikenal.")),
    };
    // Alasan, komentar, dan berbagi hanya berlaku untuk 👎.
    let down = rating.as_deref() == Some("down");
    let reason = match req.reason.as_deref().map(str::trim) {
        Some(r) if down && !r.is_empty() => {
            if !REASONS.contains(&r) {
                return Err(ApiError::bad_request("Alasan tidak dikenal."));
            }
            Some(r.to_string())
        }
        _ => None,
    };
    let comment = req
        .comment
        .as_deref()
        .map(str::trim)
        .filter(|c| down && !c.is_empty())
        .map(str::to_string);
    if comment
        .as_ref()
        .is_some_and(|c| c.chars().count() > MAX_COMMENT)
    {
        return Err(ApiError::bad_request(format!(
            "Komentar maksimal {MAX_COMMENT} karakter."
        )));
    }
    let share = down && req.share;

    let found = state
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            // Jawaban asisten di percakapan milik user ini saja.
            let owned = tx
                .query_row(
                    "SELECT m.conversation_id FROM messages m
                     JOIN conversations c ON c.id = m.conversation_id
                     WHERE m.id = ?1 AND m.role = 'assistant' AND c.user_id = ?2",
                    params![message_id, user.id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?;
            let Some(conversation_id) = owned else {
                return Ok(false);
            };
            let Some(rating) = rating else {
                tx.execute(
                    "DELETE FROM feedback WHERE message_id = ?1 AND user_id = ?2",
                    params![message_id, user.id],
                )?;
                tx.commit()?;
                return Ok(true);
            };
            let (question, answer) = if share {
                let answer: String = tx.query_row(
                    "SELECT content FROM messages WHERE id = ?1",
                    params![message_id],
                    |r| r.get(0),
                )?;
                // Pakai pertanyaan asal yang tercatat; pesan user sebelumnya hanya
                // cadangan untuk jawaban lama yang belum punya `reply_to`.
                let question: Option<String> = tx
                    .query_row(
                        "SELECT q.content FROM messages a JOIN messages q ON q.id = a.reply_to
                         WHERE a.id = ?1",
                        params![message_id],
                        |r| r.get(0),
                    )
                    .optional()?;
                let question = match question {
                    Some(q) => Some(q),
                    None => tx
                        .query_row(
                            "SELECT content FROM messages
                             WHERE conversation_id = ?1 AND id < ?2 AND role = 'user'
                             ORDER BY id DESC LIMIT 1",
                            params![conversation_id, message_id],
                            |r| r.get(0),
                        )
                        .optional()?,
                };
                (question, Some(answer))
            } else {
                (None, None)
            };
            tx.execute(
                "INSERT INTO feedback
                    (message_id, user_id, rating, reason, comment, shared_question, shared_answer, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(message_id, user_id) DO UPDATE SET
                    rating = excluded.rating, reason = excluded.reason, comment = excluded.comment,
                    shared_question = excluded.shared_question, shared_answer = excluded.shared_answer,
                    created_at = excluded.created_at",
                params![message_id, user.id, rating, reason, comment, question, answer, now()],
            )?;
            tx.commit()?;
            Ok(true)
        })
        .await?;
    if !found {
        return Err(ApiError::not_found("Jawaban tidak ditemukan."));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct FeedbackRow {
    id: i64,
    rating: String,
    reason: Option<String>,
    comment: Option<String>,
    question: Option<String>,
    answer: Option<String>,
    created_at: i64,
    user_name: String,
    user_email: String,
    user_department: Option<String>,
    user_role: String,
    /// Hanya terisi untuk feedback milik admin yang melihat: percakapan user lain
    /// tetap privat, admin hanya melihat yang sengaja mereka bagikan.
    conversation_id: Option<String>,
    conversation_title: Option<String>,
    message_id: Option<i64>,
}

#[derive(Serialize)]
pub struct FeedbackList {
    up: i64,
    down: i64,
    items: Vec<FeedbackRow>,
}

pub async fn admin_list(
    State(state): State<Arc<AppState>>,
    AdminUser(admin): AdminUser,
) -> Result<Json<FeedbackList>, ApiError> {
    let list = state
        .db
        .call(move |c| {
            let (up, down) = c.query_row(
                "SELECT COALESCE(SUM(rating = 'up'), 0), COALESCE(SUM(rating = 'down'), 0) FROM feedback",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let mut stmt = c.prepare(
                "SELECT f.id, f.rating, f.reason, f.comment, f.shared_question, f.shared_answer,
                        f.created_at, u.name, u.email, u.department, u.role,
                        c.id, c.title, CASE WHEN c.id IS NOT NULL THEN f.message_id END
                 FROM feedback f
                 JOIN users u ON u.id = f.user_id
                 LEFT JOIN messages m ON m.id = f.message_id
                 LEFT JOIN conversations c
                        ON c.id = m.conversation_id AND c.user_id = f.user_id AND f.user_id = ?1
                 ORDER BY f.created_at DESC, f.id DESC LIMIT 200",
            )?;
            let items = stmt
                .query_map([admin.id], |r| {
                    Ok(FeedbackRow {
                        id: r.get(0)?,
                        rating: r.get(1)?,
                        reason: r.get(2)?,
                        comment: r.get(3)?,
                        question: r.get(4)?,
                        answer: r.get(5)?,
                        created_at: r.get(6)?,
                        user_name: r.get(7)?,
                        user_email: r.get(8)?,
                        user_department: r.get(9)?,
                        user_role: r.get(10)?,
                        conversation_id: r.get(11)?,
                        conversation_title: r.get(12)?,
                        message_id: r.get(13)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(FeedbackList { up, down, items })
        })
        .await?;
    Ok(Json(list))
}
