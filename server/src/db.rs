//! Database SQLite: satu file di `data_dir`. Semua akses lewat `Db::call`
//! supaya query berjalan di thread blocking, bukan di executor async.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    email TEXT NOT NULL UNIQUE COLLATE NOCASE,
    name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    requested_department TEXT,
    department TEXT,
    role TEXT NOT NULL DEFAULT 'user' CHECK (role IN ('user', 'admin')),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'active', 'disabled')),
    created_at INTEGER NOT NULL,
    approved_at INTEGER,
    approved_by INTEGER REFERENCES users(id)
);
CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX sessions_user ON sessions(user_id);
"#,
    r#"
CREATE TABLE conversations (
    id TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX conversations_user ON conversations(user_id, updated_at DESC);
CREATE TABLE messages (
    id INTEGER PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    content TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    duration_ms INTEGER
);
CREATE INDEX messages_conversation ON messages(conversation_id, id);
"#,
    r#"
CREATE TABLE feedback (
    id INTEGER PRIMARY KEY,
    -- NULL setelah percakapannya dihapus; penilaian dan salinan yang dibagikan tetap ada.
    message_id INTEGER REFERENCES messages(id) ON DELETE SET NULL,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    rating TEXT NOT NULL CHECK (rating IN ('up', 'down')),
    reason TEXT,
    comment TEXT,
    shared_question TEXT,
    shared_answer TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE (message_id, user_id)
);
CREATE INDEX feedback_created ON feedback(created_at DESC);
"#,
    r#"
-- Tabel pesan dibangun ulang:
-- * AUTOINCREMENT, supaya ID pesan yang dihapus tidak pernah dipakai lagi. Feedback
--   dan tab yang masih terbuka menunjuk ke ID ini.
-- * reply_to: pertanyaan asal setiap jawaban, supaya feedback membagikan pasangan
--   yang benar walau percakapan yang sama dilanjutkan dari dua tab sekaligus.
CREATE TABLE messages_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    content TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    duration_ms INTEGER,
    reply_to INTEGER REFERENCES messages(id) ON DELETE SET NULL
);
INSERT INTO messages_new (id, conversation_id, role, content, created_at, duration_ms)
    SELECT id, conversation_id, role, content, created_at, duration_ms FROM messages;
DROP TABLE messages;
ALTER TABLE messages_new RENAME TO messages;
CREATE INDEX messages_conversation ON messages(conversation_id, id);
"#,
    r#"
-- File buatan Bakase (Excel, nanti Word). Isinya di data/files/<user_id>/<id>.<kind>;
-- `spec` adalah isi terstruktur yang dipakai untuk membuat file dan untuk preview.
CREATE TABLE files (
    id TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    conversation_id TEXT REFERENCES conversations(id) ON DELETE CASCADE,
    message_id INTEGER REFERENCES messages(id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    size INTEGER NOT NULL,
    spec TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX files_conversation ON files(conversation_id);
CREATE INDEX files_message ON files(message_id);
"#,
    r#"
-- Ringkasan pemikiran dan langkah tool (JSON) ikut disimpan, supaya tetap tampil
-- saat percakapan dibuka lagi.
ALTER TABLE messages ADD COLUMN reasoning TEXT;
ALTER TABLE messages ADD COLUMN steps TEXT;
"#,
    r#"
-- Dokumen dari folder sumber untuk pencarian, diisi `ingest`. Isinya ada di index
-- data/index; tabel ini mencatat asal, hak akses, dan status setiap file.
-- AUTOINCREMENT: sitasi lama menunjuk ke ID ini, jadi ID tidak boleh dipakai ulang.
CREATE TABLE documents (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    rel_path TEXT NOT NULL,
    title TEXT NOT NULL,
    kind TEXT NOT NULL,
    pages INTEGER NOT NULL DEFAULT 0,
    chunks INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL,
    size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    visibility TEXT NOT NULL,
    version INTEGER NOT NULL,
    indexed_at INTEGER NOT NULL,
    UNIQUE (source, rel_path)
);
-- Potongan dokumen yang dikutip jawaban (JSON).
ALTER TABLE messages ADD COLUMN sources TEXT;
"#,
    r#"
-- Path kanonik folder sumber saat dokumen diindeks. Server hanya mengunduh file asli
-- kalau folder sumber di config-nya sama dengan folder ini. Baris lama (kosong) dibaca
-- ulang oleh ingest berikutnya.
ALTER TABLE documents ADD COLUMN root TEXT NOT NULL DEFAULT '';
"#,
    r#"
-- Pemakaian token per giliran ke model (chat atau tamu), tanpa isi. Tetap ada walau
-- percakapannya dihapus, supaya laporan biaya tidak berubah.
CREATE TABLE usage_log (
    id INTEGER PRIMARY KEY,
    created_at INTEGER NOT NULL,
    kind TEXT NOT NULL,
    user_id INTEGER,
    message_id INTEGER,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    cached_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    reasoning_tokens INTEGER NOT NULL DEFAULT 0,
    requests INTEGER NOT NULL DEFAULT 1,
    -- Permintaan yang tokennya dilaporkan Azure; sisanya terputus sebelum selesai.
    reported_requests INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX usage_log_created ON usage_log (created_at);
"#,
    r#"
-- Kurs USD ke IDR harian untuk laporan biaya. `date` adalah tanggal kurs dari sumbernya.
CREATE TABLE fx_rates (
    date TEXT PRIMARY KEY,
    usd_idr REAL NOT NULL,
    fetched_at INTEGER NOT NULL
);
"#,
];

// Slot sebelumnya mengabaikan tabel tambahan dan tetap bisa membuka versi skema ini.
const COMPACTIONS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS conversation_compactions (
    conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
    through_message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    content TEXT NOT NULL,
    references_json TEXT NOT NULL DEFAULT '{}'
);
"#;

impl Db {
    pub fn open_read_only(data_dir: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(
            data_dir.join("rag-hub.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)
            .with_context(|| format!("tidak bisa membuat {}", data_dir.display()))?;
        let conn = Connection::open(data_dir.join("rag-hub.sqlite"))?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // `ingest` menulis dari proses lain saat server jalan.
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        // Migrasi yang membangun ulang tabel butuh foreign key mati: kalau aktif,
        // DROP TABLE ikut menjalankan ON DELETE di tabel lain. Pragma ini tidak
        // berlaku di dalam transaksi, jadi diatur sebelumnya, dan keutuhan relasi
        // dicek sebelum commit.
        conn.pragma_update(None, "foreign_keys", false)?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        // Server lama tidak boleh menurunkan versi database yang sudah dimigrasi server
        // atau `ingest` yang lebih baru.
        anyhow::ensure!(
            version <= MIGRATIONS.len() as i64,
            "database dibuat oleh versi server yang lebih baru (versi {version}); perbarui server"
        );
        let tx = conn.transaction()?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version.max(0) as usize) {
            tx.execute_batch(sql)
                .with_context(|| format!("migrasi {} gagal", i + 1))?;
        }
        tx.execute_batch(COMPACTIONS_SCHEMA)?;
        let broken: i64 =
            tx.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })?;
        anyhow::ensure!(broken == 0, "migrasi merusak {broken} relasi antar tabel");
        tx.pragma_update(None, "user_version", MIGRATIONS.len() as i64)?;
        // Tabel index terpisah dari user_version agar binary v3 tetap bisa membuka DB.
        let table = crate::search::INDEX_TABLE;
        tx.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {table} (
                id INTEGER PRIMARY KEY,
                source TEXT NOT NULL,
                rel_path TEXT NOT NULL,
                title TEXT NOT NULL,
                kind TEXT NOT NULL,
                pages INTEGER NOT NULL DEFAULT 0,
                chunks INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL,
                size INTEGER NOT NULL,
                mtime INTEGER NOT NULL,
                visibility TEXT NOT NULL,
                version INTEGER NOT NULL,
                indexed_at INTEGER NOT NULL,
                root TEXT NOT NULL DEFAULT '',
                sidecar INTEGER NOT NULL DEFAULT 0,
                doc_date TEXT,
                doc_date_from TEXT,
                date_version INTEGER NOT NULL DEFAULT 0,
                date_indexed_at INTEGER,
                UNIQUE (source, rel_path)
            );",
        ))?;
        // Kolom baru untuk tabel yang sudah ada; binary lama tidak membacanya. `sidecar`: sidik
        // teks ARCHIVE di `.extracted-text`. `doc_date*`/`date_*`: tanggal dokumen, asalnya, versi aturannya,
        // dan `indexed_at` baris saat tanggal itu ditulis (lihat `docdate`).
        for (column, declaration) in [
            ("sidecar", "INTEGER NOT NULL DEFAULT 0"),
            ("doc_date", "TEXT"),
            ("doc_date_from", "TEXT"),
            ("date_version", "INTEGER NOT NULL DEFAULT 0"),
            ("date_indexed_at", "INTEGER"),
        ] {
            let exists: i64 = tx.query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1"),
                [column],
                |r| r.get(0),
            )?;
            if exists == 0 {
                tx.execute_batch(&format!(
                    "ALTER TABLE {table} ADD COLUMN {column} {declaration};"
                ))?;
            }
        }
        tx.commit()?;
        conn.pragma_update(None, "foreign_keys", true)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub async fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = conn
                .lock()
                .map_err(|_| anyhow::anyhow!("koneksi database rusak"))?;
            f(&mut guard)
        })
        .await?
    }
}

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Database versi 3 (sebelum migrasi 4) berisi satu jawaban yang sudah dinilai.
    fn v3_database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        for sql in &MIGRATIONS[..3] {
            conn.execute_batch(sql).unwrap();
        }
        conn.execute_batch(
            "INSERT INTO users (id, email, name, password_hash, created_at) VALUES (1, 'a@example.com', 'A', 'x', 0);
             INSERT INTO conversations VALUES ('c1', 1, 'judul', 0, 0);
             INSERT INTO messages (id, conversation_id, role, content, created_at) VALUES
                 (1, 'c1', 'user', 'tanya', 0), (2, 'c1', 'assistant', 'jawab', 0);
             INSERT INTO feedback (message_id, user_id, rating, created_at) VALUES (2, 1, 'up', 0);
             PRAGMA user_version = 3;",
        )
        .unwrap();
        conn
    }

    #[test]
    fn rebuilding_messages_keeps_data_and_feedback_links() {
        let db = Db::init(v3_database()).unwrap();
        let conn = db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
        let linked: Option<i64> = conn
            .query_row("SELECT message_id FROM feedback", [], |r| r.get(0))
            .unwrap();
        assert_eq!(linked, Some(2));
        let fk: bool = conn
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert!(fk, "foreign key harus aktif lagi setelah migrasi");
    }

    #[test]
    fn compaction_migration_preserves_history_and_cascades_on_delete() {
        let conn = v3_database();
        const PREVIOUS_VERSION: i64 = 10;
        conn.pragma_update(None, "foreign_keys", false).unwrap();
        for sql in &MIGRATIONS[3..PREVIOUS_VERSION as usize] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", PREVIOUS_VERSION)
            .unwrap();

        let db = Db::init(conn).unwrap();
        let conn = db.conn.lock().unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert!(
            version <= PREVIOUS_VERSION,
            "binary lama menolak versi {version}"
        );
        assert_eq!(version, PREVIOUS_VERSION);
        assert!(crate::compaction::load(&conn, "c1").unwrap().is_none());
        let answer: String = conn
            .query_row("SELECT content FROM messages WHERE id = 2", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(answer, "jawab");
        conn.execute(
            "INSERT INTO conversation_compactions (conversation_id, through_message_id, content)
             VALUES ('c1', 2, 'ringkasan uji')",
            [],
        )
        .unwrap();
        assert_eq!(
            crate::compaction::load(&conn, "c1")
                .unwrap()
                .unwrap()
                .through_message_id,
            2
        );
        conn.execute("DELETE FROM conversations WHERE id = 'c1'", [])
            .unwrap();
        assert!(crate::compaction::load(&conn, "c1").unwrap().is_none());
    }

    #[test]
    fn deleted_message_ids_are_never_reused() {
        let db = Db::init(v3_database()).unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute("DELETE FROM conversations WHERE id = 'c1'", [])
            .unwrap();
        let linked: Option<i64> = conn
            .query_row("SELECT message_id FROM feedback", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            linked, None,
            "hapus percakapan melepas feedback, bukan menghapusnya"
        );
        conn.execute_batch(
            "INSERT INTO conversations VALUES ('c2', 1, 'baru', 0, 0);
             INSERT INTO messages (conversation_id, role, content, created_at) VALUES ('c2', 'user', 'lagi', 0);",
        )
        .unwrap();
        assert_eq!(conn.last_insert_rowid(), 3);
    }
}
