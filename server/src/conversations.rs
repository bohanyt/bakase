//! Riwayat percakapan per user. Server memegang riwayat: konteks yang dikirim
//! ke model selalu dibaca dari database, bukan dari browser, jadi browser tidak
//! bisa menyisipkan jawaban asisten atau pesan sistem palsu.

use crate::files::FileRef;
use crate::{
    AppState,
    acl::{AuthUser, Role},
    agent::SourceRef,
    db::now,
    error::ApiError,
    llm::ChatMessage,
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

const TITLE_CHARS: usize = 60;
const LIST_LIMIT: i64 = 300;

#[derive(Serialize)]
pub struct ConversationSummary {
    id: String,
    title: String,
    updated_at: i64,
}

#[derive(Serialize)]
pub struct StoredMessage {
    id: i64,
    role: String,
    content: String,
    created_at: i64,
    duration_ms: Option<i64>,
    /// Feedback user sendiri untuk jawaban ini: "up", "down", atau kosong.
    feedback: Option<String>,
    /// File yang dibuat Bakase di jawaban ini, atau lampiran di pesan user.
    files: Vec<FileRef>,
    /// Ringkasan pemikiran model untuk jawaban ini, kalau ada.
    reasoning: Option<String>,
    /// Langkah tool, sebagai daftar `{ id, label, done }`.
    steps: serde_json::Value,
    /// Potongan dokumen yang dikutip jawaban ini, hanya yang masih boleh dibaca user.
    sources: Vec<SourceRef>,
    /// Jawaban berhenti sebelum selesai.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    partial: bool,
}

/// Bagian jawaban selain teks: ringkasan pemikiran dan langkah tool.
#[derive(Default)]
pub struct AnswerExtras {
    pub reasoning: String,
    pub steps: Vec<crate::agent::StepRecord>,
    /// File buatan giliran ini; ditautkan ke jawaban dalam transaksi yang sama.
    pub file_ids: Vec<String>,
    /// Potongan dokumen yang dikutip jawaban.
    pub sources: Vec<SourceRef>,
    pub partial: bool,
}

#[derive(Serialize)]
pub struct ConversationDetail {
    id: String,
    title: String,
    messages: Vec<StoredMessage>,
    summary: Option<crate::compaction::Summary>,
}

fn new_id() -> String {
    let mut bytes = [0u8; 16];
    rand::fill(&mut bytes);
    hex::encode(bytes)
}

/// Kontrak bingkai sama dengan web/src/quote.ts: jangan melewati teks setelah penutup,
/// dan jangan menghilangkan apa pun jika struktur bagian kutipannya tidak lengkap.
fn quoted_title(message: &str) -> Option<String> {
    let section = message.strip_prefix(":::kutipan\n")?;
    let mut lines = section.split('\n').peekable();
    let mut first_quote = None;
    loop {
        let mut text_lines = 0;
        while let Some(line) = lines.peek().copied() {
            let Some(text) = line
                .strip_prefix("> ")
                .or_else(|| (line == ">").then_some(""))
            else {
                break;
            };
            first_quote.get_or_insert(text);
            text_lines += 1;
            lines.next();
        }
        if text_lines == 0 {
            return None;
        }
        if lines.peek().is_some_and(|line| line.starts_with("— ")) {
            lines.next();
        }
        if lines
            .peek()
            .is_some_and(|line| line.starts_with("Komentar: "))
        {
            lines.next();
        }
        match lines.next()? {
            ":::akhir-kutipan" => {
                let question = match lines.next() {
                    None => String::new(),
                    Some("") => lines.collect::<Vec<_>>().join("\n"),
                    _ => return None,
                };
                return Some(if question.trim().is_empty() {
                    format!("Kutipan: {}", first_quote?)
                } else {
                    question
                });
            }
            "" => (),
            _ => return None,
        }
    }
}

/// Judul dari pertanyaan, atau kutipan pertama kalau pesan hanya berisi kutipan.
pub fn title_from(message: &str) -> String {
    let text = quoted_title(message).unwrap_or_else(|| {
        let start = message.trim_start_matches('\\');
        let escaped = start == ":::kutipan"
            || start.starts_with(":::kutipan\n")
            || start.starts_with(":::kutipan\r\n");
        if escaped {
            message.strip_prefix('\\').unwrap_or(message).to_owned()
        } else {
            message.to_owned()
        }
    });
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= TITLE_CHARS {
        return if line.is_empty() {
            "Percakapan baru".into()
        } else {
            line
        };
    }
    let cut: String = line.chars().take(TITLE_CHARS).collect();
    let cut = match cut.rfind(' ') {
        Some(i) if i > TITLE_CHARS / 2 => &cut[..i],
        _ => cut.as_str(),
    };
    format!("{}…", cut.trim_end())
}

fn owned(c: &Connection, id: &str, user_id: i64) -> rusqlite::Result<Option<String>> {
    c.query_row(
        "SELECT title FROM conversations WHERE id = ?1 AND user_id = ?2",
        params![id, user_id],
        |r| r.get(0),
    )
    .optional()
}

fn not_found() -> ApiError {
    ApiError::not_found("Percakapan tidak ditemukan.")
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Json<Vec<ConversationSummary>>, ApiError> {
    let rows = state
        .db
        .call(move |c| {
            let mut stmt = c.prepare(
                "SELECT id, title, updated_at FROM conversations
                 WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(params![user.id, LIST_LIMIT], |r| {
                    Ok(ConversationSummary {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        updated_at: r.get(2)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;
    Ok(Json(rows))
}

pub async fn get(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ConversationDetail>, ApiError> {
    let user_id = user.id;
    let mut detail = state
        .db
        .call(move |c| {
            let Some(title) = owned(c, &id, user_id)? else {
                return Ok(None);
            };
            let mut stmt = c.prepare(
                "SELECT m.id, m.role, m.content, m.created_at, m.duration_ms, f.rating,
                        m.reasoning, m.steps, m.sources, m.partial
                 FROM messages m
                 LEFT JOIN feedback f ON f.message_id = m.id AND f.user_id = ?2
                 WHERE m.conversation_id = ?1 ORDER BY m.id",
            )?;
            let mut messages = stmt
                .query_map(params![id, user_id], |r| {
                    let (content, sources) = stored_answer(r.get(2)?, r.get(8)?);
                    Ok(StoredMessage {
                        id: r.get(0)?,
                        role: r.get(1)?,
                        content,
                        created_at: r.get(3)?,
                        duration_ms: r.get(4)?,
                        feedback: r.get(5)?,
                        files: Vec::new(),
                        reasoning: r.get(6)?,
                        steps: r
                            .get::<_, Option<String>>(7)?
                            .and_then(|s| serde_json::from_str(&s).ok())
                            .unwrap_or_else(|| serde_json::json!([])),
                        sources,
                        partial: r.get(9)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let mut by_message = files_by_message(c, &id, None)?;
            for m in &mut messages {
                m.files = by_message.remove(&m.id).unwrap_or_default();
            }
            let summary = crate::compaction::load(c, &id)?;
            Ok(Some(ConversationDetail {
                id,
                title,
                messages,
                summary,
            }))
        })
        .await?
        .ok_or_else(not_found)?;
    let mut lists: Vec<_> = detail.messages.iter_mut().map(|m| &mut m.sources).collect();
    let revoked = keep_readable(state.as_ref(), Some(user), &mut lists, history_deadline()).await?;
    if let Some(index) = &state.docs {
        crate::scan::refresh_citations(
            &state.db,
            index.table(),
            lists.iter_mut().flat_map(|sources| sources.iter_mut()),
        )
        .await?;
    }
    let mut first_revoked = None;
    for (message, access) in detail.messages.iter_mut().zip(revoked) {
        if access.revoked {
            first_revoked.get_or_insert(message.id);
        }
        if access.revoked_image {
            message.content = IMAGE_ACCESS_REVOKED.into();
            message.reasoning = None;
            message.steps = serde_json::json!([]);
            message.files.clear();
        }
    }
    detail.summary = detail
        .summary
        .filter(|summary| summary.readable_before(first_revoked));
    Ok(Json(detail))
}

#[derive(Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    q: String,
}

#[derive(Serialize)]
pub struct SearchHit {
    id: String,
    title: String,
    snippet: Option<String>,
    updated_at: i64,
}

const SEARCH_LIMIT: i64 = 50;
const SNIPPET_CONTEXT: usize = 70;

/// Pola LIKE `%kata%`; `%`, `_`, dan `\` dari user dicari apa adanya.
pub(crate) fn like_pattern(q: &str) -> String {
    let escaped = q
        .replace('\\', r"\\")
        .replace('%', r"\%")
        .replace('_', r"\_");
    format!("%{escaped}%")
}

const MAX_SEARCH_WORDS: usize = 8;

/// Kata kunci unik dari query, huruf kecil, maksimal 8 kata.
pub fn search_words(q: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for w in q.split_whitespace().map(str::to_lowercase) {
        if !words.contains(&w) {
            words.push(w);
        }
        if words.len() == MAX_SEARCH_WORDS {
            break;
        }
    }
    words
}

/// Teks tanpa blok grafik dan diagram (```chart, ```mermaid). Batas bloknya dibaca parser
/// Markdown, jadi contoh di dalam blok kode lain atau kode berindentasi tidak ikut terbuang.
fn without_visual_blocks(text: &str) -> std::borrow::Cow<'_, str> {
    use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag};
    let mut kept = String::new();
    let mut from = 0;
    for (event, range) in Parser::new(text).into_offset_iter() {
        if let Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) = event
            && matches!(info.split_whitespace().next(), Some("chart" | "mermaid"))
            && range.start >= from
        {
            kept.push_str(&text[from..range.start]);
            from = range.end;
        }
    }
    if from == 0 {
        return text.into();
    }
    kept.push_str(&text[from..]);
    kept.into()
}

/// Membuang simbol markdown supaya potongan teks enak dibaca: garis tabel,
/// penanda judul, kutipan, daftar, tebal, miring, dan kode. Isi blok grafik dan diagram
/// dibuang karena berupa data, bukan kalimat.
pub fn strip_markdown(text: &str) -> String {
    without_visual_blocks(text)
        .lines()
        .filter(|line| {
            let t = line.trim();
            // Penanda bingkai kutipan bukan isi.
            if t == ":::kutipan" || t == ":::akhir-kutipan" {
                return false;
            }
            // Baris pemisah tabel seperti |---|:---:| atau garis horizontal.
            !(t.len() >= 3
                && t.chars()
                    .all(|c| matches!(c, '|' | '-' | ':' | ' ' | '*' | '_')))
        })
        .map(|line| {
            let t = line
                .trim_start()
                .trim_start_matches(['#', '>'])
                .trim_start();
            let t = t
                .strip_prefix("- ")
                .or_else(|| t.strip_prefix("* "))
                .unwrap_or(t);
            t.replace('|', " ")
                .replace("**", "")
                .replace("__", "")
                .replace("~~", "")
                .replace('`', "")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Potongan teks satu baris di sekitar kata kunci pertama yang ditemukan.
pub fn snippet(text: &str, words: &[String]) -> Option<String> {
    let flat: Vec<char> = strip_markdown(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .collect();
    let lower: Vec<char> = flat
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let (pos, len) = words.iter().find_map(|w| {
        let needle: Vec<char> = w.chars().collect();
        lower
            .windows(needle.len().max(1))
            .position(|win| win == needle.as_slice())
            .map(|p| (p, needle.len()))
    })?;
    let start = pos.saturating_sub(SNIPPET_CONTEXT);
    let end = (pos + len + SNIPPET_CONTEXT).min(flat.len());
    let mut out: String = flat[start..end].iter().collect();
    if start > 0 {
        out.insert(0, '…');
    }
    if end < flat.len() {
        out.push('…');
    }
    Some(out)
}

/// Mencari percakapan milik user sendiri yang memuat semua kata kunci, di
/// judul atau di pesan mana pun, tanpa peduli urutan dan jarak antarkata.
pub async fn search(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    // Kata kunci lewat body, bukan URL, supaya tidak ikut tercatat di log akses.
    Json(query): Json<SearchQuery>,
) -> Result<Json<Vec<SearchHit>>, ApiError> {
    if query.q.chars().count() > 200 {
        return Err(ApiError::bad_request("Kata kunci terlalu panjang."));
    }
    let words = search_words(&query.q);
    if words.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let patterns: Vec<String> = words.iter().map(|w| like_pattern(w)).collect();
    let user_id = user.id;
    let mut hits = Vec::new();
    let mut offset = 0;
    let deadline = history_deadline();
    loop {
        let patterns = patterns.clone();
        let (candidates, mut matches) = state.db.call(move |c| {
            let mask = |column: &str, start: usize| {
                (0..patterns.len())
                    .map(|i| format!(r"(({} LIKE ?{} ESCAPE '\') << {i})", column, i + start))
                    .collect::<Vec<_>>().join(" | ")
            };
            let word_clauses = (0..patterns.len()).map(|i| {
                let p = i + 4;
                format!(r"(c.title LIKE ?{p} ESCAPE '\' OR EXISTS (
                    SELECT 1 FROM messages m WHERE m.conversation_id=c.id AND m.content LIKE ?{p} ESCAPE '\'))")
            }).collect::<Vec<_>>().join(" AND ");
            let sql = format!("SELECT c.id,c.title,c.updated_at,{} FROM conversations c
                WHERE c.user_id=?1 AND {word_clauses}
                ORDER BY c.updated_at DESC,c.id LIMIT ?2 OFFSET ?3", mask("c.title", 4));
            let mut values: Vec<rusqlite::types::Value> = vec![user_id.into(), SEARCH_LIMIT.into(), offset.into()];
            values.extend(patterns.iter().cloned().map(Into::into));
            let mut stmt = c.prepare(&sql)?;
            let candidates = stmt.query_map(rusqlite::params_from_iter(values), |r| {
                Ok((r.get::<_, String>(0)?,r.get::<_, String>(1)?,r.get::<_, i64>(2)?,r.get::<_, u16>(3)?))
            })?.collect::<rusqlite::Result<Vec<_>>>()?;
            if candidates.is_empty() {
                return Ok((candidates, Vec::new()));
            }
            let marks = (1..=candidates.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(",");
            let sql = format!("SELECT conversation_id,content,sources,{} AS words FROM messages
                WHERE conversation_id IN ({marks}) AND words != 0 ORDER BY id", mask("content", candidates.len()+1));
            let values = candidates.iter().map(|row| &row.0).chain(patterns.iter());
            let mut stmt = c.prepare(&sql)?;
            let matches = stmt.query_map(rusqlite::params_from_iter(values), |r| {
                let (content, sources) = stored_answer(r.get(1)?, r.get(2)?);
                Ok((r.get::<_, String>(0)?,content,sources,r.get::<_, u16>(3)?))
            })?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok((candidates, matches))
        }).await?;
        let checked = candidates.len();
        let mut lists: Vec<_> = matches.iter_mut().map(|row| &mut row.2).collect();
        let revoked = keep_readable(&state, Some(user.clone()), &mut lists, deadline).await?;
        let mut readable = HashMap::new();
        for ((id, content, _, mask), access) in matches.into_iter().zip(revoked) {
            if !access.revoked_image {
                let entry = readable.entry(id).or_insert((0, content));
                entry.0 |= mask;
            }
        }
        let all = (1 << words.len()) - 1;
        for (id, title, updated_at, title_mask) in candidates {
            let (message_mask, content) = readable.remove(&id).unwrap_or_default();
            if title_mask | message_mask == all {
                hits.push(SearchHit {
                    id,
                    title,
                    updated_at,
                    snippet: snippet(&content, &words),
                });
                if hits.len() == SEARCH_LIMIT as usize {
                    return Ok(Json(hits));
                }
            }
        }
        if checked < SEARCH_LIMIT as usize {
            return Ok(Json(hits));
        }
        offset += SEARCH_LIMIT;
    }
}

#[derive(Deserialize)]
pub struct Rename {
    title: String,
}

pub async fn rename(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<Rename>,
) -> Result<StatusCode, ApiError> {
    let title = req.title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() || title.chars().count() > 120 {
        return Err(ApiError::bad_request("Judul harus 1 sampai 120 karakter."));
    }
    let n = state
        .db
        .call(move |c| {
            Ok(c.execute(
                "UPDATE conversations SET title = ?1 WHERE id = ?2 AND user_id = ?3",
                params![title, id, user.id],
            )?)
        })
        .await?;
    if n == 0 {
        return Err(not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let (n, files) = state
        .db
        .call(move |c| {
            let mut stmt = c.prepare(
                "SELECT id, kind FROM files WHERE conversation_id = ?1 AND user_id = ?2",
            )?;
            let files = stmt
                .query_map(params![id, user.id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let n = c.execute(
                "DELETE FROM conversations WHERE id = ?1 AND user_id = ?2",
                params![id, user.id],
            )?;
            Ok((n, files))
        })
        .await?;
    if n == 0 {
        return Err(not_found());
    }
    // Baris file ikut terhapus lewat cascade; isinya di disk dihapus di sini.
    for (file_id, kind) in files {
        let path = crate::files::path_of(&state.files_dir, user.id, &file_id, &kind);
        if let Err(e) = tokio::fs::remove_file(&path).await {
            tracing::warn!("file {} tidak terhapus: {e}", path.display());
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Untuk model, supaya "lanjutkan" tidak dianggap meneruskan jawaban yang sudah utuh.
const PARTIAL_NOTE: &str = "\n\n[Jawaban ini berhenti sebelum selesai.]";

/// Hasil menyiapkan satu giliran chat.
pub struct Turn {
    pub id: String,
    /// ID pesan user yang baru disimpan; jawaban nanti menunjuk ke sini.
    pub question_id: i64,
    pub title: String,
    pub history: Vec<ChatMessage>,
    pub message_ids: Vec<i64>,
    pub summary: Option<crate::compaction::Summary>,
    pub first_revoked: Option<i64>,
    pub trace: crate::turn_trace::Turn,
}

/// Pertanyaan yang sudah tersimpan dengan kunci giliran yang sama.
pub struct KnownTurn {
    pub id: String,
    pub title: String,
    pub question_id: i64,
}

fn turn_key(user_id: i64, turn_id: &str) -> String {
    format!("{user_id}:{turn_id}")
}

/// Kiriman ulang dari browser yang tidak sempat menerima balasan pertama.
pub async fn known_turn(
    state: &AppState,
    user_id: i64,
    turn_id: &str,
) -> Result<Option<KnownTurn>, ApiError> {
    let key = turn_key(user_id, turn_id);
    Ok(state
        .db
        .call(move |c| {
            Ok(c.query_row(
                "SELECT c.id, c.title, m.id FROM messages m
                 JOIN conversations c ON c.id = m.conversation_id
                 WHERE m.turn_key = ?1 AND c.user_id = ?2",
                params![key, user_id],
                |r| {
                    Ok(KnownTurn {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        question_id: r.get(2)?,
                    })
                },
            )
            .optional()?)
        })
        .await?)
}

#[cfg(test)]
pub async fn begin_turn(
    state: &AppState,
    user_id: i64,
    conversation_id: Option<String>,
    message: String,
    attachment_ids: Vec<String>,
) -> Result<Turn, ApiError> {
    begin_keyed_turn(
        state,
        user_id,
        conversation_id,
        message,
        attachment_ids,
        None,
    )
    .await
}

/// Membuat percakapan baru atau memastikan percakapan milik user, menyimpan pesan user
/// beserta lampirannya, lalu mengembalikan seluruh riwayat untuk dikirim ke model.
/// Kalau lampiran tidak valid, tidak ada yang tersimpan.
pub async fn begin_keyed_turn(
    state: &AppState,
    user_id: i64,
    conversation_id: Option<String>,
    message: String,
    attachment_ids: Vec<String>,
    turn_id: Option<&str>,
) -> Result<Turn, ApiError> {
    let turn_key = turn_id.map(|turn_id| turn_key(user_id, turn_id));
    let traces = state.traces.clone();
    let deployment = state.llm.deployment().to_owned();
    let prepared = state
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            let now = now();
            let attached = match crate::attachments::pending(&tx, user_id, &attachment_ids)? {
                Ok(files) => files,
                Err(e) => return Ok(Err(e)),
            };
            let (id, title) = match conversation_id {
                Some(id) => {
                    let Some(title) = owned(&tx, &id, user_id)? else {
                        return Ok(Err(ApiError::not_found("Percakapan tidak ditemukan atau sudah diarsipkan.")));
                    };
                    (id, title)
                }
                None => {
                    let id = new_id();
                    let title = match attached.first() {
                        Some(f) if message.trim().is_empty() => {
                            title_from(&format!("Lampiran: {}", f.name))
                        }
                        _ => title_from(&message),
                    };
                    tx.execute(
                        "INSERT INTO conversations (id, user_id, title, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?4)",
                        params![id, user_id, title, now],
                    )?;
                    (id, title)
                }
            };
            tx.execute(
                "INSERT INTO messages (conversation_id, role, content, created_at, turn_key) VALUES (?1, 'user', ?2, ?3, ?4)",
                params![id, message, now, turn_key],
            )?;
            let question_id = tx.last_insert_rowid();
            crate::attachments::bind(&tx, user_id, &attachment_ids, &id, question_id, now)?;
            tx.execute(
                "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
                params![now, id],
            )?;
            let mut sheet_notes = HashMap::new();
            let rows = {
                // Jawaban yang membuat file diberi catatan nama dan id file, supaya model
                // bisa membacanya lagi dengan read_xlsx saat diminta mengubah. Pesan user
                // yang membawa lampiran diberi catatan yang sama. Jawaban yang mengutip
                // dokumen diberi daftar sumbernya, supaya nomor [n] tetap jelas.
                let mut stmt = tx.prepare(
                    "SELECT m.id, m.role, m.content, m.sources, m.partial
                     FROM messages m WHERE m.conversation_id = ?1 ORDER BY m.id",
                )?;
                let mut by_message = files_by_message(&tx, &id, Some(&mut sheet_notes))?;
                stmt
                    .query_map(params![id], |r| {
                        let (mut content, sources) = stored_answer(r.get(2)?, r.get(3)?);
                        if r.get(4)? {
                            content.push_str(PARTIAL_NOTE);
                        }
                        let message_id = r.get(0)?;
                        Ok((
                            message_id,
                            r.get::<_, String>(1)?,
                            content,
                            by_message.remove(&message_id).unwrap_or_default(),
                            sources,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?
            };
            let reader = acl_user(&tx, user_id)?;
            let summary = crate::compaction::load(&tx, &id)?;
            tx.commit()?;
            let trace = traces.start(question_id, &deployment);
            Ok(Ok((id, question_id, title, reader, rows, sheet_notes, summary, trace)))
        })
        .await??;
    let (id, question_id, title, reader, mut rows, sheet_notes, summary, trace) = prepared;
    let mut lists: Vec<_> = rows.iter_mut().map(|row| &mut row.4).collect();
    let revoked = keep_readable(state, reader, &mut lists, history_deadline()).await?;
    let mut first_revoked = None;
    for ((message_id, _, content, files, _), access) in rows.iter_mut().zip(revoked) {
        if access.revoked {
            first_revoked.get_or_insert(*message_id);
        }
        if access.revoked_image {
            *content = IMAGE_ACCESS_REVOKED.into();
            files.clear();
        }
    }
    if let Some(index) = &state.docs {
        crate::scan::refresh_citations(
            &state.db,
            index.table(),
            rows.iter_mut().flat_map(|row| row.4.iter_mut()),
        )
        .await?;
    }
    let message_ids = rows.iter().map(|row| row.0).collect();
    let history = rows
        .into_iter()
        .map(|(_, role, mut content, files, sources)| {
            let mut attachments = Vec::new();
            if role == "user" && !files.is_empty() {
                if !content.is_empty() {
                    content.push_str("\n\n");
                }
                content.push_str(&crate::attachments::history_note(&files, &sheet_notes));
                attachments = files;
            } else if !files.is_empty() {
                let list: Vec<String> = files
                    .iter()
                    .map(|f| format!("{} (id {})", f.name, f.id))
                    .collect();
                content.push_str(&format!(
                    "\n\n[File yang dibuat di jawaban ini: {}]",
                    list.join(", ")
                ));
            }
            content.push_str(&sources_note(&sources));
            ChatMessage {
                role,
                content,
                attachments,
            }
        })
        .collect();
    Ok(Turn {
        id,
        question_id,
        title,
        history,
        message_ids,
        summary,
        first_revoked,
        trace,
    })
}

/// File per pesan di satu percakapan: file buatan Bakase di jawaban, lampiran di pesan user.
fn files_by_message(
    c: &Connection,
    conversation_id: &str,
    mut sheet_notes: Option<&mut HashMap<String, crate::attachments::FileNote>>,
) -> rusqlite::Result<HashMap<i64, Vec<FileRef>>> {
    let mut stmt = c.prepare(
        "SELECT id, name, kind, size, message_id,
            CASE WHEN ?2 AND kind IN ('xlsx', 'xls') AND json_valid(spec)
                 THEN json_extract(spec, '$.sheet_note') END,
            CASE WHEN ?2 AND instr(spec, '\"fokus\":') > 0 AND json_valid(spec)
                 THEN json_extract(spec, '$.fokus') END
         FROM files
         WHERE conversation_id = ?1 AND message_id IS NOT NULL ORDER BY created_at, rowid",
    )?;
    let mut by_message: HashMap<i64, Vec<FileRef>> = HashMap::new();
    for row in stmt.query_map(params![conversation_id, sheet_notes.is_some()], |r| {
        Ok((
            r.get::<_, i64>(4)?,
            FileRef {
                id: r.get(0)?,
                name: r.get(1)?,
                kind: r.get(2)?,
                size: r.get(3)?,
            },
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<String>>(6)?,
        ))
    })? {
        let (message_id, file, sheets, focus) = row?;
        if let Some(notes) = sheet_notes.as_deref_mut()
            && (sheets.is_some() || focus.is_some())
        {
            notes.insert(
                file.id.clone(),
                crate::attachments::FileNote {
                    sheets: sheets.unwrap_or_default(),
                    focus: focus.unwrap_or_default(),
                },
            );
        }
        by_message.entry(message_id).or_default().push(file);
    }
    Ok(by_message)
}

/// Jawaban tersimpan dengan sumbernya. Jawaban lama dengan nomor sitasi yang loncat
/// diurutkan seperti jawaban baru, sebelum sumber yang aksesnya dicabut dibuang, supaya
/// nomornya tetap sama setelah itu.
fn stored_answer(content: String, sources: Option<String>) -> (String, Vec<SourceRef>) {
    let sources = parse_sources(sources);
    if sources.is_empty() {
        return (content, sources);
    }
    crate::agent::renumber_citations(&content, sources)
}

fn parse_sources(json: Option<String>) -> Vec<SourceRef> {
    let mut sources: Vec<SourceRef> = json
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    for source in &mut sources {
        if source.display_title.is_empty() {
            source.display_title = crate::documents::display_title(&source.title);
        }
    }
    sources
}

/// Role dan departemen user saat ini, untuk mengecek hak akses dokumen.
fn acl_user(c: &Connection, user_id: i64) -> rusqlite::Result<Option<AuthUser>> {
    c.query_row(
        "SELECT role, departments FROM users_with_access WHERE id = ?1 AND status = 'active'",
        [user_id],
        |r| {
            let role: String = r.get(0)?;
            Ok(AuthUser {
                id: user_id,
                email: String::new(),
                name: String::new(),
                departments: crate::access::from_row(r, 1)?,
                role: Role::parse(&role).unwrap_or(Role::User),
            })
        },
    )
    .optional()
}

const IMAGE_ACCESS_REVOKED: &str =
    "Jawaban ini memuat gambar dari dokumen yang sudah tidak bisa kamu buka.";

fn history_deadline() -> tokio::time::Instant {
    tokio::time::Instant::now() + crate::image_index::SEARCH_DEADLINE
}

struct HistoryAccess {
    revoked: bool,
    revoked_image: bool,
}

#[derive(Clone, Copy, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) enum EvidenceKind {
    Text,
    Metadata,
    Image,
}

#[derive(Clone, Copy, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) struct DocumentEvidence {
    pub doc_id: i64,
    kind: EvidenceKind,
}

impl From<&SourceRef> for DocumentEvidence {
    fn from(source: &SourceRef) -> Self {
        let kind = if source.image.is_some() {
            EvidenceKind::Image
        } else if source.chunk_id.is_empty() {
            EvidenceKind::Metadata
        } else {
            EvidenceKind::Text
        };
        Self {
            doc_id: source.doc_id,
            kind,
        }
    }
}

pub(crate) async fn summary_references(
    state: &AppState,
    user_id: i64,
    conversation_id: String,
    range: (i64, i64),
    mut references: crate::compaction::References,
) -> anyhow::Result<Option<crate::compaction::References>> {
    let checked = state
        .db
        .call(move |c| {
            let Some(reader) = acl_user(c, user_id)? else {
                return Ok(None);
            };
            if owned(c, &conversation_id, user_id)?.is_none() {
                return Ok(None);
            }
            let args = params![conversation_id, range.0, range.1];
            let mut stmt = c.prepare(
                "SELECT sources FROM messages
                 WHERE conversation_id = ?1 AND id > ?2 AND id <= ?3 AND sources IS NOT NULL",
            )?;
            for row in stmt.query_map(args, |r| r.get(0))? {
                for source in parse_sources(row?) {
                    references.documents.insert(DocumentEvidence::from(&source));
                    if source.visible() && !source.chunk_id.is_empty() {
                        references.citations.insert((source.doc_id, source.chunk_id));
                    }
                }
            }
            let mut stmt = c.prepare(
                "SELECT f.id FROM files f JOIN messages m ON m.id = f.message_id
                 WHERE f.conversation_id = ?1 AND m.id > ?2 AND m.id <= ?3 AND m.role = 'assistant'",
            )?;
            for row in stmt.query_map(args, |r| r.get(0))? {
                references.files.insert(row?);
            }
            Ok(Some((reader, references)))
        })
        .await?;
    let Some((reader, references)) = checked else {
        return Ok(None);
    };
    let allowed = readable_evidence(
        state,
        Some(reader),
        &references.documents,
        history_deadline(),
    )
    .await?;
    Ok((allowed.len() == references.documents.len()).then_some(references))
}

pub(crate) async fn can_reuse_summary(
    state: &AppState,
    user_id: i64,
    documents: &BTreeSet<DocumentEvidence>,
) -> anyhow::Result<bool> {
    let reader = state.db.call(move |c| Ok(acl_user(c, user_id)?)).await?;
    let Some(reader) = reader else {
        return Ok(false);
    };
    let allowed = readable_evidence(state, Some(reader), documents, history_deadline()).await?;
    Ok(allowed.len() == documents.len())
}

async fn readable_evidence(
    state: &AppState,
    user: Option<AuthUser>,
    documents: &BTreeSet<DocumentEvidence>,
    deadline: tokio::time::Instant,
) -> anyhow::Result<BTreeSet<DocumentEvidence>> {
    let (Some(user), Some(index)) = (user, state.docs.clone()) else {
        return Ok(BTreeSet::new());
    };
    let ids_for = |kind| {
        documents
            .iter()
            .filter(|doc| doc.kind == kind)
            .map(|doc| doc.doc_id)
            .collect::<Vec<_>>()
    };
    let ids = ids_for(EvidenceKind::Text);
    let metadata_ids = ids_for(EvidenceKind::Metadata);
    let image_ids = ids_for(EvidenceKind::Image);
    let (text_allowed, image_allowed) = {
        let text = async {
            if ids.is_empty() && metadata_ids.is_empty() {
                return Ok(Default::default());
            }
            let user = user.clone();
            let sources = state.sources.clone();
            let table = index.table();
            let row_user = user.clone();
            let (mut allowed, mut metadata) = state
                .db
                .call(move |c| {
                    let mut allowed = std::collections::HashSet::new();
                    for chunk in ids.chunks(500) {
                        allowed.extend(crate::documents::history_row_ids(
                            c, &row_user, &sources, table, chunk, false,
                        )?);
                    }
                    let mut metadata = std::collections::HashSet::new();
                    for chunk in metadata_ids.chunks(500) {
                        metadata.extend(crate::documents::history_row_ids(
                            c, &row_user, &sources, table, chunk, true,
                        )?);
                    }
                    Ok((allowed, metadata))
                })
                .await?;
            let candidates: Vec<_> = allowed.union(&metadata).copied().collect();
            let index = index.clone();
            let refused = tokio::task::spawn_blocking(move || {
                index.refusing_in(&index.snapshot(), &candidates, &user)
            })
            .await??;
            allowed.retain(|id| !refused.contains(id));
            metadata.retain(|id| !refused.contains(id));
            Ok::<_, anyhow::Error>((allowed, metadata))
        };
        let images = async {
            if image_ids.is_empty() || tokio::time::Instant::now() >= deadline {
                return Ok(Default::default());
            }
            tokio::time::timeout_at(
                deadline,
                crate::image_index::readable_images(
                    &state.db,
                    index.clone(),
                    user.clone(),
                    state.sources.clone(),
                    image_ids,
                    deadline,
                ),
            )
            .await
            .unwrap_or(Ok(Default::default()))
        };
        tokio::try_join!(text, images)?
    };
    let (readable, readable_metadata) = text_allowed;
    Ok(documents
        .iter()
        .copied()
        .filter(|doc| match doc.kind {
            EvidenceKind::Text => readable.contains(&doc.doc_id),
            EvidenceKind::Metadata => readable_metadata.contains(&doc.doc_id),
            EvidenceKind::Image => image_allowed.contains(&doc.doc_id),
        })
        .collect())
}

/// Jawaban yang pernah melihat gambar sumber disembunyikan jika akses ke gambar dicabut.
async fn keep_readable(
    state: &AppState,
    user: Option<AuthUser>,
    lists: &mut [&mut Vec<SourceRef>],
    deadline: tokio::time::Instant,
) -> anyhow::Result<Vec<HistoryAccess>> {
    let documents = lists
        .iter()
        .flat_map(|list| list.iter())
        .map(DocumentEvidence::from)
        .collect();
    let allowed = readable_evidence(state, user, &documents, deadline).await?;
    Ok(lists
        .iter_mut()
        .map(|list| {
            let readable = |source: &SourceRef| allowed.contains(&DocumentEvidence::from(source));
            let revoked_image = list
                .iter()
                .any(|source| source.image.is_some() && !readable(source));
            let revoked = list.iter().any(|source| !readable(source));
            if revoked_image {
                list.clear();
            } else {
                list.retain(|source| source.visible() && readable(source));
            }
            HistoryAccess {
                revoked,
                revoked_image,
            }
        })
        .collect())
}

/// Catatan sumber untuk riwayat yang dikirim ke model.
fn sources_note(sources: &[SourceRef]) -> String {
    if sources.is_empty() {
        return String::new();
    }
    let list: Vec<String> = sources
        .iter()
        .map(|s| {
            let place = if let Some(sheet) = &s.sheet {
                format!(", sheet {sheet}")
            } else {
                s.page.map(|p| format!(", hal. {p}")).unwrap_or_default()
            };
            format!(
                "[{}] {} ({}{place}{}, dokumen {}, sitasi {})",
                s.n,
                s.title,
                s.source,
                crate::scan::note(s.scan, s.handwriting.as_ref()),
                s.doc_id,
                s.chunk_id
            )
        })
        .collect();
    format!(
        "\n\n[Sumber yang dikutip di jawaban ini: {}]",
        list.join("; ")
    )
}

/// Menyimpan jawaban asisten, termasuk jawaban yang terpotong. Mengembalikan
/// ID pesan supaya browser bisa mengirim feedback untuk jawaban itu.
pub async fn save_answer(
    state: &AppState,
    conversation_id: String,
    question_id: i64,
    content: String,
    duration_ms: i64,
    extras: AnswerExtras,
) -> Option<i64> {
    if content.trim().is_empty() {
        return None;
    }
    let reasoning = Some(extras.reasoning).filter(|r| !r.trim().is_empty());
    let steps = (!extras.steps.is_empty())
        .then(|| serde_json::to_string(&extras.steps).ok())
        .flatten();
    let extras_file_ids = extras.file_ids;
    let sources = (!extras.sources.is_empty())
        .then(|| serde_json::to_string(&extras.sources).ok())
        .flatten();
    let result = state
        .db
        .call(move |c| {
            let now = now();
            let tx = c.transaction()?;
            let n = tx.execute(
                "INSERT INTO messages (conversation_id, role, content, created_at, duration_ms, reply_to, reasoning, steps, sources, partial)
                 SELECT ?1, 'assistant', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9 WHERE EXISTS (SELECT 1 FROM conversations WHERE id = ?1)",
                params![conversation_id, content, now, duration_ms, question_id, reasoning, steps, sources, extras.partial],
            )?;
            let id = (n == 1).then(|| tx.last_insert_rowid());
            if let Some(message_id) = id {
                for file_id in &extras_file_ids {
                    tx.execute(
                        "UPDATE files SET message_id = ?1 WHERE id = ?2 AND conversation_id = ?3",
                        params![message_id, file_id, conversation_id],
                    )?;
                }
            }
            tx.execute(
                "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
                params![now, conversation_id],
            )?;
            tx.commit()?;
            Ok(id)
        })
        .await;
    result.unwrap_or_else(|e| {
        tracing::error!("gagal menyimpan jawaban: {e:#}");
        None
    })
}

#[cfg(test)]
mod tests {
    use super::{like_pattern, search_words, snippet, strip_markdown, title_from};

    #[test]
    fn snippets_drop_quote_markers() {
        let text = ":::kutipan\n> 45 Nm\n— Standar torsi\n:::akhir-kutipan\n\nBenar?";
        let plain = strip_markdown(text);
        assert!(!plain.contains(":::"), "{plain}");
        assert!(
            plain.contains("45 Nm") && plain.contains("Benar?"),
            "{plain}"
        );
    }

    #[test]
    fn snippets_and_words() {
        assert_eq!(
            search_words("  Team   2 team	TARGET "),
            vec!["team", "2", "target"]
        );
        let text = format!(
            "{} Target operasional
Team 2 naik {}",
            "awal ".repeat(40),
            "akhir ".repeat(40)
        );
        let s = snippet(&text, &search_words("team target")).unwrap();
        assert!(
            s.contains("Target operasional Team 2"),
            "baris baru jadi spasi: {s}"
        );
        assert!(s.starts_with('…') && s.ends_with('…'));
        assert_eq!(
            snippet("pendek saja", &search_words("pendek")).unwrap(),
            "pendek saja"
        );
        assert!(snippet("tidak ada", &search_words("zzz")).is_none());
        assert_eq!(like_pattern("50%_a"), r"%50\%\_a%");
        let table = "| Jenis | Penyebab |\n|---|:---:|\n| **Kurang** menyatu | `panas` |\n## Judul";
        assert_eq!(
            strip_markdown(table)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            "Jenis Penyebab Kurang menyatu panas Judul"
        );
        let visual = "Tim B tertinggi.\n```chart\n{\"type\": \"bar\"}\n```\nAlur:\n```mermaid\nflowchart LR\n  A --> B\n```\n```rust\nlet x = 1;\n```";
        assert_eq!(
            strip_markdown(visual)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            "Tim B tertinggi. Alur: rust let x = 1;"
        );
        let words = |t: &str| {
            strip_markdown(t)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };
        // Contoh blok grafik di dalam blok kode empat backtick tetap teks biasa.
        assert_eq!(
            words("````md\n```chart\n{\"a\":1}\n```\n````\nSesudah"),
            "md chart {\"a\":1} Sesudah"
        );
        // Baris backtick dengan teks tambahan bukan penutup; blok tanpa penutup dibuang.
        assert_eq!(words("```chart\n{}\n```js\n```\nAkhir"), "Akhir");
        assert_eq!(words("Awal\n~~~mermaid\nflowchart LR"), "Awal");
        // Kode berindentasi empat spasi bukan pagar; paragraf sesudahnya tetap ada.
        assert_eq!(
            words("Contoh:\n\n    ```chart\n    {}\n\nSesudah"),
            "Contoh: chart {} Sesudah"
        );
    }

    #[test]
    fn titles() {
        assert_eq!(title_from("  halo \n dunia  "), "halo dunia");
        assert_eq!(title_from("   "), "Percakapan baru");
        let long = "kata ".repeat(30);
        let t = title_from(&long);
        assert!(t.ends_with('…'));
        assert!(t.chars().count() <= 61);
        // Kutipan dilewati; judul dari pertanyaannya.
        assert_eq!(
            title_from(
                ":::kutipan\n> Torsi baut M16 210 Nm.\n— Jawaban Bakase\nKomentar: kelas 8.8?\n:::akhir-kutipan\n\nTolong cek ulang"
            ),
            "Tolong cek ulang"
        );
        assert_eq!(
            title_from(
                ":::kutipan\n> | Qty | Harga |\n> | --- | --- |\n— Rekap.xlsx · Data!B2:C3\n:::akhir-kutipan"
            ),
            "Kutipan: | Qty | Harga |"
        );
    }

    #[test]
    fn titles_keep_plain_comparisons_and_metadata() {
        for text in [
            ">20 mm boleh?",
            "> 10 mm",
            "— Review",
            "Komentar: batas bawah",
        ] {
            assert_eq!(title_from(text), text);
            assert_eq!(
                title_from(&format!(
                    ":::kutipan\n> asli\n— Jawaban Bakase\n:::akhir-kutipan\n\n{text}"
                )),
                text
            );
        }
        let question = "> 10 mm\n— Review\nKomentar: batas bawah";
        assert_eq!(
            title_from(question),
            "> 10 mm — Review Komentar: batas bawah"
        );
        assert_eq!(
            title_from(&format!(
                ":::kutipan\n> asli\n:::akhir-kutipan\n\n{question}"
            )),
            "> 10 mm — Review Komentar: batas bawah"
        );
        assert_eq!(title_from("Tanya\n>20 mm boleh?"), "Tanya >20 mm boleh?");
    }

    #[test]
    fn titles_only_skip_complete_leading_quote_sections() {
        assert_eq!(
            title_from(
                ":::kutipan\n> pertama\n\n> kedua\n— Review\nKomentar: cek\n:::akhir-kutipan"
            ),
            "Kutipan: pertama"
        );
        assert_eq!(
            title_from(":::kutipan\n> :::akhir-kutipan\n:::akhir-kutipan"),
            "Kutipan: :::akhir-kutipan"
        );
        for text in [
            ":::kutipan\n> tanpa akhir",
            ":::kutipan\n:::akhir-kutipan",
            ":::kutipan\n> isi\nbaris liar\n:::akhir-kutipan",
            ":::kutipan\n> isi\n:::akhir-kutipan\nlangsung teks",
            "Tanya\n:::kutipan\n> isi\n:::akhir-kutipan",
        ] {
            assert_eq!(title_from(text), text.replace('\n', " "));
        }
        for slashes in ["\\", "\\\\", "\\\\\\"] {
            assert_eq!(
                title_from(&format!("{slashes}:::kutipan\n> diketik\n:::akhir-kutipan")),
                format!("{}:::kutipan > diketik :::akhir-kutipan", &slashes[1..])
            );
        }
    }
}
