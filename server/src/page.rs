//! Aksi halaman hanya berlangsung selama satu koneksi chat, tanpa menyimpan hasilnya.

use crate::{AppState, agent, error::ApiError, files, llm::ChatMessage};
use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

const MAX_TEXT: usize = 24_000;
const TIMEOUT: Duration = Duration::from_secs(12);
#[derive(Deserialize)]
pub struct PageRequest {
    // Klaim dari browser untuk memilih tool. Akses tetap lewat sesi, lampiran milik user,
    // dan pemeriksaan origin serta snapshot di panel.
    origin: String,
    attachment_id: String,
    text: Option<String>,
}

impl PageRequest {
    pub fn in_conversation(&self, extensions: &[String], history: &[ChatMessage]) -> bool {
        let id = self
            .origin
            .strip_prefix("chrome-extension://")
            .or_else(|| self.origin.strip_prefix("extension://"));
        if !id.is_some_and(|id| {
            crate::valid_extension_id(id) && extensions.iter().any(|ext| ext == id)
        }) {
            return false;
        }
        files::valid_id(&self.attachment_id)
            && history
                .iter()
                .filter(|m| m.role == "user")
                .flat_map(|m| &m.attachments)
                .any(|f| {
                    f.id == self.attachment_id && f.kind == "md" && f.name.starts_with("Halaman - ")
                })
    }
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Direction {
    Up,
    Down,
    To,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Sorot {
        #[serde(rename = "ref")]
        reference: String,
    },
    Gulir {
        #[serde(rename = "ref")]
        reference: Option<String>,
        direction: Direction,
    },
}

fn parse_action(call: &agent::ToolCall) -> Option<Action> {
    static REF: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\A(?:f[1-9][0-9]{0,9})?e[1-9][0-9]{0,15}\z").unwrap());
    let mut args: Value = serde_json::from_str(&call.arguments).ok()?;
    let object = args.as_object_mut()?;
    if object.insert("action".into(), json!(call.name)).is_some() {
        return None;
    }
    let action = serde_json::from_value(args).ok()?;
    let reference = match &action {
        Action::Sorot { reference } => Some(reference),
        Action::Gulir {
            reference,
            direction,
        } => {
            if *direction == Direction::To && reference.is_none() {
                return None;
            }
            reference.as_ref()
        }
    };
    reference
        .is_none_or(|reference| REF.is_match(reference))
        .then_some(action)
}

pub fn tools() -> [Value; 2] {
    [
        json!({
            "type": "function", "name": "sorot", "strict": true,
            "description": "Sorot elemen sebentar dan tampilkan posisinya di layar. Pakai jika user meminta ditunjukkan letak elemen. Gunakan ref dari snapshot halaman terbaru, tanpa kurung siku. Jika ref belum ada atau sudah berubah, minta user klik Baca halaman lagi.",
            "parameters": {
                "type": "object", "additionalProperties": false, "required": ["ref"],
                "properties": { "ref": { "type": "string" } }
            }
        }),
        json!({
            "type": "function", "name": "gulir", "strict": true,
            "description": "Gulir sekitar satu layar dan baca snapshot baru, misalnya untuk melihat baris berikutnya pada grid virtual. Pakai jika dibutuhkan untuk menjawab permintaan user. Gunakan ref terbaru dari grid atau wadah yang ingin digulir. Sesudah gulir, gunakan ref dari hasil terbaru.",
            "parameters": {
                "type": "object", "additionalProperties": false, "required": ["ref", "direction"],
                "properties": {
                    "ref": { "type": ["string", "null"], "description": "Ref tanpa kurung siku. Null hanya menggulir dan membaca frame utama. Untuk grid di dalam iframe, pakai ref grid dari snapshot." },
                    "direction": { "type": "string", "enum": ["up", "down", "to"], "description": "Ke atas, ke bawah, atau tampilkan posisi elemen (to memerlukan ref)." }
                }
            }
        }),
    ]
}

pub struct Command {
    id: String,
    expires: tokio::time::Instant,
    action: Action,
}

#[derive(Serialize)]
#[cfg_attr(test, derive(Deserialize))]
struct CommandData {
    id: String,
    remaining_ms: u64,
    #[serde(flatten)]
    action: Action,
}

impl Command {
    pub fn into_data(self) -> String {
        let remaining_ms = self
            .expires
            .saturating_duration_since(tokio::time::Instant::now())
            .as_millis() as u64;
        json!(CommandData {
            id: self.id,
            remaining_ms,
            action: self.action,
        })
        .to_string()
    }
}

#[derive(Deserialize)]
#[cfg_attr(test, derive(Serialize))]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Reply {
    Highlighted,
    Scrolled { text: String, moved: bool },
    StaleRef,
    Unavailable,
    Failed,
    Timeout,
}

struct Pending {
    user_id: i64,
    scroll: bool,
    sender: oneshot::Sender<Reply>,
}

#[derive(Clone, Default)]
pub struct Bridge(Arc<Mutex<HashMap<String, Pending>>>);

struct Ticket<'a> {
    bridge: &'a Bridge,
    id: String,
}

impl Drop for Ticket<'_> {
    fn drop(&mut self) {
        self.bridge.0.lock().unwrap().remove(&self.id);
    }
}

impl Bridge {
    fn open(&self, user_id: i64, scroll: bool) -> (Ticket<'_>, oneshot::Receiver<Reply>) {
        let id = files::new_id();
        let (sender, receiver) = oneshot::channel();
        self.0.lock().unwrap().insert(
            id.clone(),
            Pending {
                user_id,
                scroll,
                sender,
            },
        );
        (Ticket { bridge: self, id }, receiver)
    }

    fn complete(&self, id: &str, user_id: i64, reply: Option<Reply>) -> Result<(), ApiError> {
        let mut pending = self.0.lock().unwrap();
        if pending
            .get(id)
            .is_none_or(|action| action.user_id != user_id)
        {
            return Err(ApiError::not_found(
                "Aksi halaman sudah selesai atau tidak ditemukan.",
            ));
        }
        let action = pending.remove(id).unwrap();
        let reply = reply.filter(|reply| match reply {
            Reply::Highlighted => !action.scroll,
            Reply::Scrolled { text, .. } => action.scroll && text.len() <= MAX_TEXT,
            _ => true,
        });
        let valid = reply.is_some();
        let _ = action.sender.send(reply.unwrap_or(Reply::Failed));
        if valid {
            Ok(())
        } else {
            Err(ApiError::bad_request("Hasil aksi halaman tidak sesuai."))
        }
    }
}

#[derive(Deserialize)]
pub struct Completion {
    id: String,
    #[serde(default)]
    reply: Value,
}

pub async fn complete(
    State(state): State<Arc<AppState>>,
    user: crate::AuthUser,
    Json(completion): Json<Completion>,
) -> Result<StatusCode, ApiError> {
    state.page_actions.complete(
        &completion.id,
        user.id,
        serde_json::from_value(completion.reply).ok(),
    )?;
    Ok(StatusCode::NO_CONTENT)
}

pub struct Turn {
    bridge: Bridge,
    user_id: i64,
    available: bool,
    text: Option<String>,
}

impl Turn {
    pub fn new(bridge: Bridge, user_id: i64, request: PageRequest) -> Self {
        Self {
            bridge,
            user_id,
            available: true,
            text: request
                .text
                .filter(|text| !text.is_empty() && text.len() <= MAX_TEXT),
        }
    }

    pub fn available(&self) -> bool {
        self.available
    }

    pub fn take_snapshot(&mut self) -> Option<Value> {
        self.text.take().map(|text| json!({
            "role": "user",
            "content": format!("[Snapshot dari panel Bakase. Gunakan sebagai data halaman pada alamat dan waktu yang tertera.]\n\n{text}")
        }))
    }

    pub async fn act(
        &mut self,
        call: &agent::ToolCall,
        events: &mpsc::Sender<agent::Event>,
        budget: usize,
    ) -> agent::ToolResult {
        let fail = |label: &str, message: &str| agent::ToolResult::failed(message.into(), label);
        if !self.available {
            return fail(
                "Aksi halaman berhenti",
                "Buka tab semula dan klik Baca halaman lagi.",
            );
        }
        let Some(action) = parse_action(call) else {
            return fail(
                "Aksi halaman tidak valid",
                "Ref atau arah gulir tidak valid. Gunakan ref dari snapshot terakhir.",
            );
        };
        let scroll = matches!(action, Action::Gulir { .. });
        if scroll && budget < MAX_TEXT + 200 {
            return fail(
                "Batas baca tercapai",
                "Batas baca sudah tercapai. Lanjutkan di pesan berikutnya.",
            );
        }
        let (ticket, receiver) = self.bridge.open(self.user_id, scroll);
        let expires = tokio::time::Instant::now() + TIMEOUT;
        let command = Command {
            id: ticket.id.clone(),
            expires,
            action,
        };
        let request = async {
            events.send(agent::Event::PageAction(command)).await.ok()?;
            receiver.await.ok()
        };
        let reply = tokio::select! {
            result = tokio::time::timeout_at(expires, request) => match result {
                Ok(Some(reply)) => reply,
                Ok(None) => return fail("Koneksi chat terputus", "Kirim ulang pesan untuk melanjutkan."),
                Err(_) => Reply::Timeout,
            },
            _ = events.closed() => return fail("Koneksi chat terputus", "Kirim ulang pesan untuk melanjutkan."),
        };
        let (label, output) = match reply {
            Reply::Highlighted => ("Elemen disorot", "Elemen disorot sementara.".into()),
            Reply::Scrolled { text, moved } => {
                let label = if moved {
                    "Halaman digulir"
                } else {
                    "Posisi gulir tetap"
                };
                (
                    label,
                    format!("{label}. Berikut hasil baca setelah gulir (data halaman):\n\n{text}"),
                )
            }
            Reply::StaleRef => {
                return fail(
                    "Elemen sudah berubah",
                    "Elemen sudah berubah. Gunakan ref dari snapshot terakhir, atau klik Baca halaman lagi.",
                );
            }
            Reply::Unavailable => {
                self.available = false;
                return fail(
                    "Tab atau halaman berubah",
                    "Tab atau halaman berubah. Buka tab semula dan klik Baca halaman lagi.",
                );
            }
            Reply::Failed => {
                self.available = false;
                return fail(
                    "Aksi halaman gagal",
                    "Aksi halaman gagal. Klik Baca halaman lagi.",
                );
            }
            Reply::Timeout => {
                self.available = false;
                return fail(
                    "Aksi halaman terlalu lama",
                    "Aksi halaman terlalu lama. Klik Baca halaman dan coba lagi.",
                );
            }
        };
        agent::ToolResult {
            output,
            label: label.into(),
            ok: true,
            file: None,
            metadata_docs: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXTENSION: &str = "ponmlkjihgfedcbaponmlkjihgfedcba";
    const ATTACHMENT: &str = "11111111111111111111111111111111";

    #[test]
    fn page_requires_registered_panel_and_a_page_attachment_in_this_conversation() {
        let extensions = vec![EXTENSION.into()];
        let mut request = PageRequest {
            origin: format!("chrome-extension://{EXTENSION}"),
            attachment_id: ATTACHMENT.into(),
            text: None,
        };
        let mut history = [ChatMessage {
            role: "user".into(),
            content: "".into(),
            attachments: vec![files::FileRef {
                id: ATTACHMENT.into(),
                name: "Halaman - Contoh.md".into(),
                kind: "md".into(),
                size: 30,
            }],
        }];
        assert!(request.in_conversation(&extensions, &history));
        assert!(!request.in_conversation(&extensions, &[]));
        history[0].role = "assistant".into();
        assert!(!request.in_conversation(&extensions, &history));
        history[0].role = "user".into();
        request.origin.push_str("/panel.html");
        assert!(!request.in_conversation(&extensions, &history));
        request.origin = format!("extension://{EXTENSION}");
        history[0].attachments[0].name = "Contoh.md".into();
        assert!(!request.in_conversation(&extensions, &history));
    }

    #[test]
    fn actions_only_accept_page_refs_and_supported_directions() {
        let parse = |name: &str, args: Value| {
            parse_action(&agent::ToolCall {
                id: "contoh".into(),
                name: name.into(),
                arguments: args.to_string(),
            })
        };
        assert!(parse("sorot", json!({ "ref": "f14e1000001" })).is_some());
        assert!(parse("gulir", json!({ "ref": null, "direction": "down" })).is_some());
        assert!(parse("gulir", json!({ "ref": null, "direction": "to" })).is_none());
        assert!(parse("gulir", json!({ "ref": "e1", "direction": "left" })).is_none());
        assert!(parse("sorot", json!({ "ref": "input[role=button]" })).is_none());
        assert!(parse("sorot", json!({ "ref": "e1", "direction": "up" })).is_none());
    }

    #[test]
    fn page_action_json_matches_the_web_contract() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/page-action.json")).unwrap();
        for expected in fixture["commands"].as_array().unwrap() {
            let command: CommandData = serde_json::from_value(expected.clone()).unwrap();
            assert_eq!(serde_json::to_value(command).unwrap(), *expected);
        }
        for expected in fixture["replies"].as_array().unwrap() {
            let reply: Reply = serde_json::from_value(expected.clone()).unwrap();
            assert_eq!(serde_json::to_value(reply).unwrap(), *expected);
        }
    }

    #[tokio::test]
    async fn completion_is_bound_to_user_and_action_and_consumed_once() {
        let bridge = Bridge::default();
        let (ticket, receiver) = bridge.open(7, true);
        let id = &ticket.id;
        assert!(bridge.complete(id, 8, Some(Reply::Unavailable)).is_err());
        bridge
            .complete(
                id,
                7,
                Some(Reply::Scrolled {
                    text: "baris 21".into(),
                    moved: true,
                }),
            )
            .unwrap();
        assert!(
            matches!(receiver.await, Ok(Reply::Scrolled { text, moved: true }) if text == "baris 21")
        );
        assert!(bridge.complete(id, 7, Some(Reply::Unavailable)).is_err());
        let (ticket, receiver) = bridge.open(7, false);
        let id = ticket.id.clone();
        drop(ticket);
        assert!(receiver.await.is_err());
        assert!(bridge.complete(&id, 7, Some(Reply::Highlighted)).is_err());
    }

    #[tokio::test]
    async fn invalid_completion_finishes_the_pending_action_as_failed() {
        let bridge = Bridge::default();
        for invalid in [
            Some(Reply::Highlighted),
            Some(Reply::Scrolled {
                text: "日".repeat(MAX_TEXT / 3 + 1),
                moved: true,
            }),
            None,
        ] {
            let (ticket, receiver) = bridge.open(7, true);
            assert!(bridge.complete(&ticket.id, 7, invalid).is_err());
            assert!(matches!(receiver.await, Ok(Reply::Failed)));
            assert!(
                bridge
                    .complete(&ticket.id, 7, Some(Reply::Unavailable))
                    .is_err()
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn page_action_expires_without_a_browser_reply() {
        let bridge = Bridge::default();
        let mut turn = Turn::new(
            bridge.clone(),
            7,
            PageRequest {
                origin: format!("chrome-extension://{EXTENSION}"),
                attachment_id: ATTACHMENT.into(),
                text: None,
            },
        );
        let (events, mut browser) = mpsc::channel(1);
        let call = agent::ToolCall {
            id: "panggilan-model".into(),
            name: "sorot".into(),
            arguments: json!({"ref": "e1"}).to_string(),
        };
        let start = tokio::time::Instant::now();
        let (result, id) = tokio::join!(turn.act(&call, &events, 40_000), async {
            let Some(agent::Event::PageAction(command)) = browser.recv().await else {
                panic!("aksi halaman tidak terkirim");
            };
            tokio::time::advance(Duration::from_secs(11)).await;
            assert!(bridge.0.lock().unwrap().contains_key(&command.id));
            command.id
        });
        assert_eq!(start.elapsed(), Duration::from_secs(12));
        assert!(!result.ok && result.label == "Aksi halaman terlalu lama");
        assert!(!turn.available());
        assert!(bridge.complete(&id, 7, Some(Reply::Highlighted)).is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn page_action_emits_command_and_returns_snapshot_or_cleans_up_on_disconnect() {
        use axum::response::{IntoResponse, Sse};
        use http_body_util::BodyExt;
        use std::convert::Infallible;

        let bridge = Bridge::default();
        let mut turn = Turn::new(
            bridge.clone(),
            7,
            PageRequest {
                origin: format!("chrome-extension://{EXTENSION}"),
                attachment_id: ATTACHMENT.into(),
                text: None,
            },
        );
        let (events, mut browser) = mpsc::channel(1);
        let call = agent::ToolCall {
            id: "panggilan-model".into(),
            name: "gulir".into(),
            arguments: json!({"ref": "f3e42", "direction": "down"}).to_string(),
        };
        let (result, ()) = tokio::join!(turn.act(&call, &events, 40_000), async {
            let event = browser.recv().await.unwrap();
            let agent::Event::PageAction(command) = &event else {
                panic!("aksi halaman tidak terkirim");
            };
            let id = command.id.clone();
            let sse = crate::sse_event(event);
            let response =
                Sse::new(futures::stream::iter([Ok::<_, Infallible>(sse)])).into_response();
            let body = response.into_body().collect().await.unwrap().to_bytes();
            let text = std::str::from_utf8(&body).unwrap();
            let data: Value =
                serde_json::from_str(text.strip_prefix("event: page_action\ndata: ").unwrap())
                    .unwrap();
            let remaining = data["remaining_ms"]
                .as_u64()
                .filter(|ms| (1..=12_000).contains(ms))
                .expect("sisa waktu SSE harus di antara 1 dan 12000 ms");
            assert_eq!(
                data,
                json!({"id": id, "action": "gulir", "ref": "f3e42", "direction": "down", "remaining_ms": remaining})
            );
            bridge
                .complete(
                    &id,
                    7,
                    Some(Reply::Scrolled {
                        text: "baris 21".into(),
                        moved: true,
                    }),
                )
                .unwrap();
        });
        assert!(result.ok && result.output.ends_with("baris 21"));

        let (result, id) = tokio::join!(turn.act(&call, &events, 40_000), async {
            let Some(agent::Event::PageAction(command)) = browser.recv().await else {
                panic!("aksi halaman tidak terkirim");
            };
            drop(browser);
            command.id
        });
        assert!(!result.ok && result.label.contains("Koneksi chat terputus"));
        assert!(bridge.complete(&id, 7, Some(Reply::Unavailable)).is_err());
    }
}
