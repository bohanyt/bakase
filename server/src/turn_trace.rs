//! Jejak privat. Teks yang berasal dari model atau user disimpan apa adanya.

use crate::{
    agent::{SourceRef, ToolCall},
    db::{Db, now_millis},
    documents::DocumentRow,
    search::Hit,
};
use rusqlite::{Connection, TransactionBehavior::Immediate, params};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{File, TryLockError},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{OwnedRwLockReadGuard, OwnedSemaphorePermit, RwLock, Semaphore, mpsc, oneshot};

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS turn_traces (
    id TEXT PRIMARY KEY,
    question_id INTEGER UNIQUE REFERENCES messages(id) ON DELETE CASCADE,
    guest_question TEXT,
    owner TEXT NOT NULL,
    deployment TEXT NOT NULL,
    server_version TEXT NOT NULL,
    started_at_ms INTEGER NOT NULL,
    duration_ms INTEGER,
    status TEXT NOT NULL CHECK (status IN ('running', 'ok', 'failed', 'stopped')),
    reason TEXT,
    dropped_events INTEGER NOT NULL DEFAULT 0,
    CHECK ((question_id IS NULL) <> (guest_question IS NULL))
);
CREATE INDEX IF NOT EXISTS turn_traces_running ON turn_traces(owner) WHERE status = 'running';
CREATE TABLE IF NOT EXISTS trace_events (
    seq INTEGER PRIMARY KEY,
    turn_id TEXT NOT NULL REFERENCES turn_traces(id) ON DELETE CASCADE,
    span INTEGER NOT NULL,
    at_ms INTEGER NOT NULL,
    kind TEXT NOT NULL,
    item INTEGER,
    data TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS trace_events_turn ON trace_events(turn_id, seq);
CREATE TABLE IF NOT EXISTS trace_texts (
    sha256 TEXT PRIMARY KEY,
    text TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS trace_text_refs (
    turn_id TEXT NOT NULL REFERENCES turn_traces(id) ON DELETE CASCADE,
    sha256 TEXT NOT NULL REFERENCES trace_texts(sha256),
    PRIMARY KEY (turn_id, sha256)
);
CREATE INDEX IF NOT EXISTS trace_text_refs_hash ON trace_text_refs(sha256);
CREATE TRIGGER IF NOT EXISTS trace_prune_text AFTER DELETE ON trace_text_refs
WHEN NOT EXISTS (SELECT 1 FROM trace_text_refs WHERE sha256=OLD.sha256)
BEGIN
    DELETE FROM trace_texts WHERE sha256=OLD.sha256;
END;
CREATE TABLE IF NOT EXISTS trace_overflow (
    owner TEXT PRIMARY KEY,
    dropped_events INTEGER NOT NULL
);
"#;

const CHUNK: usize = 8 * 1024;
const LIMIT: usize = 64 * 1024;
const INPUT_LIMIT: usize = 1024 * 1024;
const CHECKPOINT: Duration = Duration::from_millis(500);

const QUEUE_BYTES: usize = 8 * 1024 * 1024;
const OVERHEAD: usize = 160;

#[derive(Clone, Default)]
pub(crate) struct Store {
    queue: Option<Arc<Queue>>,
    active: Arc<RwLock<()>>,
}

struct Queue {
    tx: mpsc::Sender<Packet>,
    bytes: Arc<Semaphore>,
    writer: Arc<Writer>,
    #[cfg(test)]
    task: tokio::task::JoinHandle<()>,
}

struct Writer {
    db: Db,
    owner: String,
    lease: Option<(PathBuf, File)>,
    discarded: AtomicU64,
}

impl Writer {
    fn discard(&self) {
        let dropped = self.discarded.fetch_add(1, Ordering::Relaxed) + 1;
        if dropped.is_power_of_two() {
            tracing::warn!(dropped, "kejadian jejak dilewati karena antrean penuh");
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        if let Some((path, handle)) = self.lease.take() {
            drop(handle);
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Store {
    pub(crate) fn open(data_dir: &Path) -> Self {
        let open = || -> anyhow::Result<Writer> {
            let db = Db::open_trace(data_dir)?;
            let dir = data_dir.join("trace-owners");
            std::fs::create_dir_all(&dir)?;
            let owner = crate::files::new_id();
            let path = dir.join(&owner);
            let handle = File::create_new(&path)?;
            handle.try_lock()?;
            Ok(Writer {
                db,
                owner,
                lease: Some((path, handle)),
                discarded: AtomicU64::new(0),
            })
        };
        match open() {
            Ok(writer) => Self::spawn(writer, QUEUE_BYTES),
            Err(_) => {
                tracing::warn!("penyimpanan jejak tidak tersedia");
                Self::default()
            }
        }
    }

    fn spawn(writer: Writer, bytes: usize) -> Self {
        let writer = Arc::new(writer);
        let (tx, receiver) = mpsc::channel(bytes / OVERHEAD + 1);
        let _task = tokio::spawn(persist(writer.clone(), receiver));
        Self {
            queue: Some(Arc::new(Queue {
                tx,
                bytes: Arc::new(Semaphore::new(bytes)),
                writer,
                #[cfg(test)]
                task: _task,
            })),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn in_memory(db: Db) -> Self {
        Self::spawn(
            Writer {
                db,
                owner: "uji".into(),
                lease: None,
                discarded: AtomicU64::new(0),
            },
            QUEUE_BYTES,
        )
    }

    #[cfg(test)]
    pub(crate) async fn close_for_test(self) {
        let queue = Arc::into_inner(self.queue.unwrap())
            .unwrap_or_else(|| panic!("test masih memegang Store"));
        drop(queue.tx);
        queue.task.await.unwrap();
    }

    pub(crate) async fn recover(&self) {
        let Some(queue) = &self.queue else { return };
        let Some((path, _)) = &queue.writer.lease else {
            return;
        };
        let dir = path.with_file_name("");
        let owner = queue.writer.owner.clone();
        if queue
            .writer
            .db
            .call(move |c| recover(c, &dir, &owner))
            .await
            .is_err()
        {
            tracing::warn!("pemulihan status jejak gagal");
        }
    }

    pub(crate) fn start(&self, question_id: i64, deployment: &str) -> Turn {
        self.track(Some(question_id), None, deployment)
    }

    pub(crate) fn guest(&self, question: &str, deployment: &str) -> Turn {
        self.track(None, Some(question), deployment)
    }

    fn track(&self, question: Option<i64>, guest: Option<&str>, deployment: &str) -> Turn {
        let Ok(guard) = self.active.clone().try_read_owned() else {
            return Turn::default();
        };
        let live = self.queue.as_ref().and_then(|queue| {
            // Dua slot paket dipesan di awal agar Begin dan End tetap bisa masuk saat penuh.
            let budget = 2 * OVERHEAD + 32 + deployment.len() + guest.map_or(0, str::len);
            let Some(permit) = queue.reserve(budget) else {
                queue.writer.discard();
                return None;
            };
            let key = Arc::new(Key {
                id: crate::files::new_id(),
                question,
                guest: guest.map(str::to_owned),
                deployment: deployment.to_owned(),
                started_at: now_millis(),
                discarded: AtomicU64::new(0),
                _budget: permit,
            });
            queue
                .tx
                .try_send(Packet::Entry(key.clone(), Command::Begin, None))
                .ok()?;
            Some(Live {
                queue: queue.clone(),
                key,
                started: Instant::now(),
                span: 0,
                span_started: None,
                delta: None,
                checkpoint: None,
                answer_started: false,
                tool_spans: Vec::new(),
                excerpts: HashMap::new(),
                finished: false,
            })
        });
        Turn {
            _guard: Some(guard),
            live,
        }
    }

    pub(crate) async fn flush(&self) {
        let _idle = self.active.write().await;
        if let Some(queue) = &self.queue {
            let (ack, ready) = oneshot::channel();
            if queue.tx.send(Packet::Flush(ack)).await.is_ok() {
                let _ = ready.await;
            }
        }
    }
}

impl Queue {
    fn reserve(&self, bytes: usize) -> Option<OwnedSemaphorePermit> {
        self.bytes
            .clone()
            .try_acquire_many_owned(u32::try_from(bytes).ok()?)
            .ok()
    }
}

struct Key {
    id: String,
    question: Option<i64>,
    guest: Option<String>,
    deployment: String,
    started_at: i64,
    discarded: AtomicU64,
    _budget: OwnedSemaphorePermit,
}

enum Packet {
    Entry(Arc<Key>, Command, Option<OwnedSemaphorePermit>),
    Flush(oneshot::Sender<()>),
}

enum Command {
    Begin,
    Event(Record),
    Prompt(String, String),
    End(Status, Option<&'static str>, i64),
}

struct Record {
    span: u32,
    at: i64,
    kind: &'static str,
    item: Option<u64>,
    data: String,
}

async fn persist(writer: Arc<Writer>, mut receiver: mpsc::Receiver<Packet>) {
    let mut batch = Vec::new();
    let mut logged = 0;
    while receiver.recv_many(&mut batch, 64).await != 0 {
        let packets = Arc::new(std::mem::take(&mut batch));
        loop {
            let copies = packets.clone();
            let owner = writer.owner.clone();
            let dropped = writer.discarded.load(Ordering::Relaxed);
            match writer
                .db
                .call(move |c| write(c, &owner, &copies, (dropped > logged).then_some(dropped)))
                .await
            {
                Ok(()) => {
                    logged = dropped;
                    break;
                }
                Err(err) if is_busy(&err) => tokio::time::sleep(Duration::from_millis(25)).await,
                Err(_) => {
                    tracing::warn!("penyimpanan jejak gagal");
                    break;
                }
            }
        }
        batch = Arc::into_inner(packets).expect("batch sudah selesai diproses");
        for packet in batch.drain(..) {
            if let Packet::Flush(ack) = packet {
                let _ = ack.send(());
            }
        }
    }
}

fn is_busy(err: &anyhow::Error) -> bool {
    err.downcast_ref::<rusqlite::Error>()
        .and_then(rusqlite::Error::sqlite_error_code)
        .is_some_and(|code| {
            matches!(
                code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
        })
}

fn write(
    c: &mut Connection,
    owner: &str,
    packets: &[Packet],
    dropped: Option<u64>,
) -> anyhow::Result<()> {
    let tx = c.transaction_with_behavior(Immediate)?;
    for packet in packets {
        let Packet::Entry(key, command, _budget) = packet else {
            continue;
        };
        match command {
            Command::Begin => {
                tx.prepare_cached(
                    "INSERT INTO turn_traces (id,question_id,guest_question,owner,deployment,server_version,started_at_ms,status)
                     SELECT ?1,?2,?3,?4,?5,?6,?7,'running'
                     WHERE ?2 IS NULL OR EXISTS(SELECT 1 FROM messages WHERE id=?2 AND role='user')",
                )?.execute(params![key.id,key.question,key.guest,owner,key.deployment,env!("CARGO_PKG_VERSION"),key.started_at])?;
            }
            Command::Event(e) => {
                tx.prepare_cached(
                    "INSERT INTO trace_events (turn_id,span,at_ms,kind,item,data)
                     SELECT id,?2,?3,?4,?5,?6 FROM turn_traces WHERE id=?1",
                )?
                .execute(params![
                    key.id,
                    e.span,
                    e.at,
                    e.kind,
                    e.item.and_then(|id| i64::try_from(id).ok()),
                    e.data
                ])?;
            }
            Command::Prompt(sha, text) => {
                tx.prepare_cached(
                    "INSERT OR IGNORE INTO trace_texts
                     SELECT ?1,?2 FROM turn_traces WHERE id=?3",
                )?
                .execute(params![sha, text, key.id])?;
                tx.prepare_cached(
                    "INSERT OR IGNORE INTO trace_text_refs
                     SELECT id,?2 FROM turn_traces WHERE id=?1",
                )?
                .execute(params![key.id, sha])?;
            }
            Command::End(status, reason, duration) => {
                tx.prepare_cached(
                    "UPDATE turn_traces SET status=?2,reason=?3,duration_ms=?4,dropped_events=?5 WHERE id=?1",
                )?.execute(params![key.id,status.as_str(),reason,duration,key.discarded.load(Ordering::Relaxed) as i64])?;
            }
        }
    }
    if let Some(dropped) = dropped {
        tx.execute(
            "INSERT INTO trace_overflow (owner,dropped_events) VALUES(?1,?2)
             ON CONFLICT(owner) DO UPDATE SET dropped_events=excluded.dropped_events",
            params![owner, dropped as i64],
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn recover(c: &mut Connection, dir: &Path, current: &str) -> anyhow::Result<()> {
    let owners: Vec<String> = c
        .prepare("SELECT DISTINCT owner FROM turn_traces WHERE status='running' AND owner<>?1")?
        .query_map([current], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for owner in owners {
        if !crate::files::valid_id(&owner) {
            continue;
        }
        let path = dir.join(&owner);
        let abandoned = match File::options().read(true).write(true).open(&path) {
            Ok(file) => match file.try_lock() {
                Ok(()) => true,
                Err(TryLockError::WouldBlock | TryLockError::Error(_)) => false,
            },
            Err(err) => err.kind() == io::ErrorKind::NotFound,
        };
        if !abandoned {
            continue;
        }
        let tx = c.transaction_with_behavior(Immediate)?;
        tx.execute(
            "INSERT INTO trace_events (turn_id,span,at_ms,kind,data)
             SELECT t.id, e.span, ?2, 'end', '{\"status\":\"stopped\"}'
             FROM turn_traces t JOIN trace_events e ON e.seq=(
                 SELECT seq FROM trace_events WHERE turn_id=t.id AND kind IN ('model','tool','end')
                 ORDER BY seq DESC LIMIT 1)
             WHERE t.owner=?1 AND t.status='running' AND e.kind<>'end'",
            params![owner, now_millis()],
        )?;
        tx.execute(
            "UPDATE turn_traces SET status='stopped', reason='process_interrupted'
             WHERE owner=?1 AND status='running'",
            [&owner],
        )?;
        tx.commit()?;
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    Ok,
    Failed,
    Stopped,
}

impl Status {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Default)]
pub(crate) struct Turn {
    _guard: Option<OwnedRwLockReadGuard<()>>,
    live: Option<Live>,
}

struct Live {
    queue: Arc<Queue>,
    key: Arc<Key>,
    started: Instant,
    span: u32,
    span_started: Option<Instant>,
    delta: Option<Record>,
    checkpoint: Option<Instant>,
    answer_started: bool,
    tool_spans: Vec<u32>,
    excerpts: HashMap<u32, String>,
    finished: bool,
}

impl Live {
    fn send(&self, command: Command) {
        let bytes = match &command {
            Command::Event(e) => OVERHEAD + e.data.capacity(),
            Command::Prompt(sha, text) => OVERHEAD + sha.capacity() + text.capacity(),
            _ => 0,
        };
        let permit = if bytes == 0 {
            Some(None)
        } else {
            self.queue.reserve(bytes).map(Some)
        };
        if !permit.is_some_and(|permit| {
            self.queue
                .tx
                .try_send(Packet::Entry(self.key.clone(), command, permit))
                .is_ok()
        }) {
            self.key.discarded.fetch_add(1, Ordering::Relaxed);
            self.queue.writer.discard();
        }
    }

    fn emit(&self, kind: &'static str, item: Option<u64>, data: String) {
        self.send(Command::Event(Record {
            span: self.span,
            at: now_millis(),
            kind,
            item,
            data,
        }));
    }

    fn json(&self, kind: &'static str, data: Value) {
        let data = data.to_string();
        if data.len() <= LIMIT {
            self.emit(kind, None, data);
        } else {
            self.emit(
                kind,
                None,
                json!({"original_bytes":data.len(),"truncated":true}).to_string(),
            );
        }
    }

    fn chunks(&self, kind: &'static str, item: Option<u64>, text: &str) {
        self.split(kind, item, text, CHUNK);
    }

    fn split(&self, kind: &'static str, item: Option<u64>, mut text: &str, limit: usize) {
        while !text.is_empty() {
            let end = text.floor_char_boundary(text.len().min(limit));
            self.emit(kind, item, text[..end].to_owned());
            text = &text[end..];
        }
    }

    fn begin(&mut self, kind: &'static str, data: Value) {
        self.span += 1;
        self.span_started = Some(Instant::now());
        self.checkpoint = None;
        self.json(kind, data);
    }

    fn drain_delta(&mut self) {
        if let Some(delta) = self.delta.take() {
            self.send(Command::Event(delta));
            self.checkpoint = Some(Instant::now());
        }
    }

    fn end(&mut self, status: Status) {
        self.drain_delta();
        if let Some(start) = self.span_started.take() {
            self.json("end", json!({"status":status,"duration_ms":elapsed(start)}));
        }
    }

    fn finish(&mut self, status: Status, reason: Option<&'static str>) {
        self.end(status);
        self.finished = true;
        self.send(Command::End(status, reason, elapsed(self.started)));
    }
}

fn elapsed(start: Instant) -> i64 {
    i64::try_from(start.elapsed().as_millis()).unwrap_or(i64::MAX)
}

fn text_input(input: &Value) -> Vec<Value> {
    input
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|message| {
            let role = message["role"].as_str()?;
            let content = &message["content"];
            if let Some(parts) = content.as_array() {
                let parts: Vec<_> = parts
                    .iter()
                    .filter_map(|part| {
                        let kind = part["type"].as_str()?;
                        match kind {
                            "input_text" => {
                                Some(json!({"type":kind,"text":part["text"].as_str()?}))
                            }
                            "input_image" => Some(json!({"type":kind})),
                            "input_file" => {
                                Some(json!({"type":kind,"filename":part["filename"].as_str()}))
                            }
                            _ => None,
                        }
                    })
                    .collect();
                Some(json!({"role": role, "content": parts}))
            } else {
                Some(json!({"role": role, "content": content.as_str()?}))
            }
        })
        .collect()
}

fn token_usage(response: &Value) -> Option<Value> {
    let usage = response["usage"].as_object()?;
    let mut tokens = serde_json::Map::new();
    for key in ["input_tokens", "output_tokens", "total_tokens"] {
        if let Some(count) = usage.get(key).and_then(Value::as_u64) {
            tokens.insert(key.into(), json!(count));
        }
    }
    for (group, key) in [
        ("input_tokens_details", "cached_tokens"),
        ("output_tokens_details", "reasoning_tokens"),
    ] {
        if let Some(count) = usage.get(group).and_then(|details| details[key].as_u64()) {
            tokens.insert(group.into(), json!({(key): count}));
        }
    }
    (!tokens.is_empty()).then_some(Value::Object(tokens))
}

impl Turn {
    pub(crate) fn begin_model(&mut self, phase: &'static str, request: &Value) {
        if let Some(live) = &mut self.live {
            let prompt = request["instructions"].as_str().unwrap_or_default();
            let prompt_sha256 = hex::encode(Sha256::digest(prompt));
            live.send(Command::Prompt(prompt_sha256.clone(), prompt.to_owned()));
            live.begin(
                "model",
                json!({
                    "phase":phase, "prompt_sha256":prompt_sha256, "tool_spans":live.tool_spans,
                    "effort":request["reasoning"]["effort"],
                }),
            );
            if phase != "answer" || !live.answer_started {
                live.split(
                    "input",
                    None,
                    &json!(text_input(&request["input"])).to_string(),
                    INPUT_LIMIT,
                );
                live.answer_started |= phase == "answer";
            }
        }
    }

    pub(crate) fn model_metadata(&mut self, response: &Value) {
        if let Some(live) = &mut self.live {
            let model = response["model"].as_str();
            let effort = response["reasoning"]["effort"].as_str();
            let response_id = response["id"].as_str();
            let usage = token_usage(response);
            if model.is_some() || effort.is_some() || response_id.is_some() || usage.is_some() {
                live.drain_delta();
                live.json("model_metadata", json!({
                    "model": model, "effort": effort, "response_id": response_id, "usage": usage,
                }));
            }
        }
    }

    pub(crate) fn delta(&mut self, kind: &'static str, item: Option<u64>, mut text: &str) {
        let Some(live) = &mut self.live else { return };
        while !text.is_empty() {
            if !live
                .delta
                .as_ref()
                .is_some_and(|part| part.kind == kind && part.item == item)
            {
                live.drain_delta();
                live.delta = Some(Record {
                    span: live.span,
                    at: now_millis(),
                    kind,
                    item,
                    data: String::new(),
                });
            }
            let part = live.delta.as_mut().unwrap();
            let room = CHUNK - part.data.len();
            let end = text.floor_char_boundary(room.min(text.len()));
            part.data.push_str(&text[..end]);
            text = &text[end..];
            if !text.is_empty() || live.checkpoint.is_none_or(|at| at.elapsed() >= CHECKPOINT) {
                live.drain_delta();
            }
        }
    }

    pub(crate) fn model_call(&mut self, index: Option<u64>, item: &Value) {
        if let Some(live) = &mut self.live {
            live.drain_delta();
            live.json("model_call", json!({
                "index": index, "item_id": item["id"].as_str(), "call_id": item["call_id"].as_str(),
                "name": item["name"].as_str(),
            }));
            if let Some(arguments) = item["arguments"].as_str() {
                live.chunks("model_arguments", index, arguments);
            }
        }
    }

    pub(crate) fn begin_tool(&mut self, call: &ToolCall) {
        if let Some(live) = &mut self.live {
            live.begin("tool", json!({"name":call.name,"call_id":call.id}));
            live.chunks("tool_arguments", None, &call.arguments);
        }
    }

    pub(crate) fn tool_result(&mut self, text: &str, sources: &[SourceRef], status: Status) {
        if let Some(live) = &mut self.live {
            live.chunks("tool_output", None, text);
            let images: Vec<_> = sources.iter().filter_map(image_ref).collect();
            if !images.is_empty() {
                live.json("tool_images", json!(images));
            }
            live.tool_spans.push(live.span);
            live.end(status);
        }
    }

    pub(crate) fn excerpt(&mut self, number: u32, hit: &Hit, doc: &DocumentRow) {
        if let Some(live) = &mut self.live {
            live.excerpts.insert(number, hit.chunk_id.clone());
            live.json("excerpt", json!({
                "model_number":number, "chunk_id":hit.chunk_id, "doc_id":doc.id, "title":doc.title,
                "source":doc.source, "page":hit.page, "document_date":doc.doc_date,
                "modified_ms":doc.modified, "text_sha256":hex::encode(Sha256::digest(&hit.text)),
            }));
            live.chunks("excerpt_text", Some(number.into()), &hit.text);
        }
    }

    pub(crate) fn image(&mut self, source: &SourceRef) {
        if let Some(live) = &mut self.live {
            live.excerpts.insert(source.n, source.chunk_id.clone());
            live.json("excerpt", json!({
                "model_number":source.n, "chunk_id":source.chunk_id, "doc_id":source.doc_id,
                "title":source.title, "source":source.source, "page":source.page, "image":image_ref(source),
            }));
        }
    }

    pub(crate) fn calculation(&mut self, source: &SourceRef) {
        if let Some(live) = &mut self.live {
            live.excerpts.insert(source.n, source.chunk_id.clone());
            live.json(
                "excerpt",
                json!({
                    "model_number":source.n, "chunk_id":source.chunk_id, "doc_id":source.doc_id,
                    "title":source.title, "source":source.source, "sheet":source.sheet,
                }),
            );
        }
    }

    pub(crate) fn citations(&mut self, cited: &HashSet<u32>, sources: &[SourceRef]) {
        if let Some(live) = &mut self.live {
            let mut mapping: Vec<_> = cited
                .iter()
                .filter_map(|n| {
                    let chunk_id = live.excerpts.get(n)?;
                    let source = sources
                        .iter()
                        .find(|source| source.chunk_id == *chunk_id && source.visible())?;
                    Some((*n, source.n))
                })
                .collect();
            if !mapping.is_empty() {
                mapping.sort_unstable();
                live.json("citations", json!(mapping));
            }
        }
    }

    pub(crate) fn end(&mut self, status: Status) {
        if let Some(live) = &mut self.live {
            live.end(status);
        }
    }

    pub(crate) fn finish(
        &mut self,
        status: Status,
        reason: Option<&'static str>,
        answer: &str,
        reasoning: &str,
    ) {
        if let Some(live) = &mut self.live {
            live.end(status);
            live.span = 0;
            live.chunks("answer", None, answer);
            live.chunks("reasoning", None, reasoning);
            live.finish(status, reason);
        }
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        if let Some(live) = &mut self.live
            && !live.finished
        {
            let reason = if live.span == 0 {
                "preparation_interrupted"
            } else {
                "process_interrupted"
            };
            live.finish(Status::Stopped, Some(reason));
        }
    }
}

fn image_ref(source: &SourceRef) -> Option<Value> {
    Some(json!({
        "label":format!("Gambar sumber [{}]", source.n), "detail":"low", "doc_id":source.doc_id,
        "chunk_id":source.chunk_id, "image_key":source.image.as_ref()?.key,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        dir: PathBuf,
        db: Db,
    }

    impl Fixture {
        async fn create() -> Self {
            let dir =
                std::env::temp_dir().join(format!("app-trace-test-{}", rand::random::<u64>()));
            let db = Db::open(&dir).unwrap();
            db.call(|c| Ok(c.execute_batch(
                "INSERT INTO users (id,email,name,password_hash,created_at) VALUES (1,'uji@example.test','Uji','x',0);
                 INSERT INTO conversations VALUES ('uji',1,'uji',0,0);
                 INSERT INTO messages (id,conversation_id,role,content,created_at) VALUES (1,'uji','user','Tanya',0);"
            )?)).await.unwrap();
            Self { dir, db }
        }

        fn reader(&self) -> Connection {
            Connection::open(self.dir.join("rag-hub.sqlite")).unwrap()
        }

        fn erase(self) {
            drop(self.db);
            std::fs::remove_dir_all(self.dir).unwrap();
        }
    }

    async fn committed(store: &Store) {
        let (ack, ready) = oneshot::channel();
        store
            .queue
            .as_ref()
            .unwrap()
            .tx
            .send(Packet::Flush(ack))
            .await
            .unwrap();
        ready.await.unwrap();
    }

    async fn notify_on_busy(store: &Store) -> mpsc::UnboundedReceiver<()> {
        let (busy, entered) = mpsc::unbounded_channel();
        store
            .queue
            .as_ref()
            .unwrap()
            .writer
            .db
            .call(move |c| {
                c.busy_handler(Some(move |_: i32| {
                    let _ = busy.send(());
                    false
                }))?;
                Ok(())
            })
            .await
            .unwrap();
        entered
    }

    #[tokio::test]
    async fn a_real_sqlite_writer_lock_does_not_lose_final_turns() {
        let fixture = Fixture::create().await;
        let store = Store::open(&fixture.dir);
        let mut chat = store.start(1, "uji");
        let mut guest = store.guest("Tamu", "uji");
        committed(&store).await;
        let mut entered = notify_on_busy(&store).await;
        let blocker = fixture.reader();
        blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
        for turn in [&mut chat, &mut guest] {
            turn.finish(Status::Ok, None, "Tersimpan setelah lock", "");
        }
        drop((chat, guest));
        entered.recv().await.unwrap();
        entered.recv().await.unwrap();
        let running: i64 = blocker
            .query_row(
                "SELECT count(*) FROM turn_traces WHERE status='running'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(running, 2);
        blocker.execute_batch("COMMIT").unwrap();
        store.flush().await;
        let counts = fixture
            .db
            .call(|c| {
                Ok(c.query_row(
                    "SELECT count(t.id), sum(t.guest_question IS NOT NULL)
             FROM turn_traces t JOIN trace_events e ON t.id=e.turn_id
             WHERE t.status='ok' AND e.kind='answer' AND e.data='Tersimpan setelah lock'",
                    [],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(counts, (2, 1));
        store.close_for_test().await;
        drop(blocker);
        fixture.erase();
    }

    #[tokio::test]
    async fn model_input_excludes_media_and_reassembles_large_utf8_input() {
        let db = Db::open_in_memory().unwrap();
        let store = Store::in_memory(db.clone());
        let mut normal = store.guest("Uji media", "uji");
        normal.begin_model(
            "guest",
            &json!({"instructions":"Uji", "input":[
                {"role":"user","content":[{"type":"input_text","text":"teks asli"},
                   {"type":"input_image","image_url":"base64-sintetis"},
                   {"type":"input_file","filename":"Uji.pdf","file_data":"biner-sintetis"}]},
                {"type":"reasoning","encrypted_content":"opaque-sintetis"}
            ]}),
        );
        normal.finish(Status::Ok, None, "", "");
        let large_input = json!([{"role":"user","content":"a".repeat(INPUT_LIMIT - 29) + &"\u{1f4c4}".repeat(2048)}]);
        let mut large = store.guest("Uji batas", "uji");
        large.begin_model("guest", &json!({"input":large_input}));
        large.finish(Status::Ok, None, "", "");
        drop((normal, large));
        store.flush().await;

        db.call(move |c| {
            let read = |question| -> anyhow::Result<Vec<String>> {
                Ok(c.prepare(
                    "SELECT e.data FROM trace_events e JOIN turn_traces t ON t.id=e.turn_id
                     WHERE t.guest_question=?1 AND e.kind='input' ORDER BY e.seq",
                )?
                .query_map([question], |r| r.get(0))?
                .collect::<Result<_, _>>()?)
            };
            assert_eq!(
                serde_json::from_str::<Value>(&read("Uji media")?.concat())?,
                json!([{"role":"user","content":[
                    {"type":"input_text","text":"teks asli"}, {"type":"input_image"},
                    {"type":"input_file","filename":"Uji.pdf"}
                ]}])
            );
            let chunks = read("Uji batas")?;
            assert_eq!(chunks.len(), 2);
            assert!(chunks.iter().all(|chunk| chunk.len() <= INPUT_LIMIT));
            assert_eq!(
                serde_json::from_str::<Value>(&chunks.concat())?,
                large_input
            );
            Ok(())
        })
        .await
        .unwrap();
        store.close_for_test().await;
    }

    #[tokio::test]
    async fn full_queue_never_waits_for_sqlite_and_records_dropped_events() {
        let fixture = Fixture::create().await;
        let db = Db::open_trace(&fixture.dir).unwrap();
        let store = Store::spawn(
            Writer {
                db,
                owner: "saturasi".into(),
                lease: None,
                discarded: AtomicU64::new(0),
            },
            64 * 1024,
        );
        let mut turn = store.start(1, "uji");
        committed(&store).await;
        let mut entered = notify_on_busy(&store).await;
        let blocker = fixture.reader();
        blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
        turn.begin_tool(&ToolCall {
            id: "call-uji".into(),
            name: "gulir".into(),
            arguments: "{}".into(),
        });
        entered.recv().await.unwrap();
        turn.tool_result(&"a".repeat(128 * 1024), &[], Status::Ok);
        turn.finish(Status::Ok, None, "Terus menjawab saat SQLite terkunci", "");
        drop(turn);
        let discarded = store
            .queue
            .as_ref()
            .unwrap()
            .writer
            .discarded
            .load(Ordering::Relaxed);
        assert!(discarded > 0);
        blocker.execute_batch("COMMIT").unwrap();
        store.flush().await;
        let result = blocker
            .query_row(
                "SELECT t.status, t.dropped_events, o.dropped_events
             FROM turn_traces t JOIN trace_overflow o ON o.owner=t.owner",
                [],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(result, ("ok".into(), discarded as i64, discarded as i64));
        store.close_for_test().await;
        drop(blocker);
        fixture.erase();
    }

    #[tokio::test]
    async fn recovery_marks_tool_of_dead_process_interrupted() {
        let fixture = Fixture::create().await;
        let store = Store::open(&fixture.dir);
        let (started, running) = oneshot::channel();
        let (resume, blocked) = oneshot::channel();
        let tool_store = store.clone();
        let running_tool = tokio::spawn(async move {
            let mut turn = tool_store.start(1, "uji");
            turn.begin_tool(&ToolCall {
                id: "call-uji".into(),
                name: "gulir".into(),
                arguments: "{}".into(),
            });
            started.send(()).unwrap();
            blocked.await.unwrap();
            turn.finish(Status::Ok, None, "", "");
        });
        running.await.unwrap();
        committed(&store).await;
        let reader = fixture.reader();
        let before: (String, String) = reader
            .query_row(
                "SELECT t.status, e.data FROM turn_traces t JOIN trace_events e ON t.id=e.turn_id
             WHERE t.question_id=1 AND e.kind='tool'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(before.0, "running");
        assert_eq!(
            serde_json::from_str::<Value>(&before.1).unwrap()["call_id"],
            "call-uji"
        );

        let peer = Store::open(&fixture.dir);
        peer.recover().await;
        assert_eq!(
            reader
                .query_row("SELECT status FROM turn_traces", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "running"
        );

        let restored = fixture.dir.join("restored");
        std::fs::create_dir_all(&restored).unwrap();
        reader
            .execute(
                "VACUUM INTO ?1",
                [restored.join("rag-hub.sqlite").to_str().unwrap()],
            )
            .unwrap();
        let recovered = Store::open(&restored);
        recovered.recover().await;
        let restored = Connection::open(restored.join("rag-hub.sqlite")).unwrap();
        let after: (String, String, String) = restored
            .query_row(
                "SELECT t.status, t.reason, json_extract(e.data,'$.status')
             FROM turn_traces t JOIN trace_events e ON e.turn_id=t.id WHERE e.kind='end'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            after,
            (
                "stopped".into(),
                "process_interrupted".into(),
                "stopped".into()
            )
        );
        resume.send(()).unwrap();
        running_tool.await.unwrap();
        store.flush().await;
        store.close_for_test().await;
        peer.close_for_test().await;
        recovered.close_for_test().await;
        drop((reader, restored));
        fixture.erase();
    }

    #[tokio::test]
    #[ignore = "ukur overhead lokal, tanpa Azure"]
    async fn long_answer_trace_stay_fast() {
        let fixture = Fixture::create().await;
        let enabled = Store::open(&fixture.dir);
        let disabled = Store::default();
        let text = "u".repeat(137_513);
        let body =
            json!({"instructions":"Prompt uji", "input":[{"role":"user","content":"Tanya"}]});
        for (name, store) in [("tanpa jejak", &disabled), ("dengan jejak", &enabled)] {
            let mut samples = Vec::new();
            for _ in 0..40 {
                let start = Instant::now();
                let mut turn = store.guest("uji", "sintetis");
                turn.begin_model("guest", &body);
                let mut output = String::new();
                for i in (0..text.len()).step_by(256) {
                    let delta = &text[i..(i + 256).min(text.len())];
                    output.push_str(delta);
                    turn.delta("model_text", None, delta);
                }
                turn.finish(Status::Ok, None, &output, "");
                samples.push(start.elapsed().as_micros());
                drop(turn);
                store.flush().await;
            }
            samples.sort_unstable();
            println!("{name}: median {} us, p95 {} us", samples[20], samples[37]);
        }
        enabled.close_for_test().await;
        fixture.erase();
    }
}
