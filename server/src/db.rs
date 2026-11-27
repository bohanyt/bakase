//! Database SQLite: satu file di `data_dir`. Semua akses lewat `Db::call`
//! supaya query berjalan di thread blocking, bukan di executor async.

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

macro_rules! users_table {
    ($name:literal, $id:literal) => {
        concat!(
            "CREATE TABLE ",
            $name,
            " (id INTEGER PRIMARY KEY",
            $id,
            r#",
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
);"#
        )
    };
}

const MIGRATIONS: &[&str] = &[
    concat!(
        users_table!("users", ""),
        r#"
CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX sessions_user ON sessions(user_id);
"#
    ),
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
        // `ingest` menulis dari proses lain saat server jalan.
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        enable_wal(&conn)?;
        // Migrasi yang membangun ulang tabel butuh foreign key mati: kalau aktif,
        // DROP TABLE ikut menjalankan ON DELETE di tabel lain. Pragma ini tidak
        // berlaku di dalam transaksi, jadi diatur sebelumnya, dan keutuhan relasi
        // dicek sebelum commit.
        conn.pragma_update(None, "foreign_keys", false)?;
        // Penulis jejak dan slot lain bisa commit sementara transaksi sedang membaca.
        conn.set_transaction_behavior(rusqlite::TransactionBehavior::Immediate);
        let tx = conn.transaction()?;
        let version = Self::schema_version(&tx)?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version.max(0) as usize) {
            tx.execute_batch(sql)
                .with_context(|| format!("migrasi {} gagal", i + 1))?;
        }
        tx.execute_batch(COMPACTIONS_SCHEMA)?;
        tx.execute_batch(crate::archive::SCHEMA)?;
        Self::preserve_user_ids(&tx)?;
        crate::access::init(&tx)?;
        tx.execute_batch(crate::microsoft::SCHEMA)?;
        tx.execute_batch(crate::microsoft::ADDRESS_SCHEMA)?;
        tx.execute_batch(crate::turn_trace::SCHEMA)?;
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
        tx.execute_batch(crate::scan::SCHEMA)?;
        // Jawaban yang berhenti sebelum selesai, dan kunci giliran supaya kiriman ulang tidak
        // dobel. Binary lama mengabaikan kedua kolom ini.
        for (column, declaration) in [
            ("partial", "INTEGER NOT NULL DEFAULT 0"),
            ("turn_key", "TEXT"),
        ] {
            let exists: i64 = tx.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = ?1",
                [column],
                |r| r.get(0),
            )?;
            if exists == 0 {
                tx.execute_batch(&format!(
                    "ALTER TABLE messages ADD COLUMN {column} {declaration};"
                ))?;
            }
        }
        tx.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS messages_turn_key ON messages(turn_key)
             WHERE turn_key IS NOT NULL;",
        )?;
        tx.commit()?;
        conn.pragma_update(None, "foreign_keys", true)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub(crate) fn table_exists(c: &Connection, name: &str) -> rusqlite::Result<bool> {
        c.query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_schema WHERE name = ?1 AND type = 'table')",
            [name],
            |r| r.get(0),
        )
    }

    pub(crate) fn preserves_user_ids(c: &Connection) -> rusqlite::Result<bool> {
        c.query_row(
            "SELECT instr(sql, 'AUTOINCREMENT') > 0 FROM sqlite_schema WHERE name = 'users'",
            [],
            |r| r.get(0),
        )
    }

    fn preserve_user_ids(tx: &rusqlite::Transaction<'_>) -> Result<()> {
        if Self::preserves_user_ids(tx)? {
            return Ok(());
        }
        // Akun yang dihapus tetap dirujuk oleh arsip dan giliran yang masih berjalan.
        let view: Option<String> = tx
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name = 'users_with_access'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let mut triggers = tx.prepare(
            "SELECT sql FROM sqlite_schema WHERE type = 'trigger' AND tbl_name = 'users'",
        )?;
        let triggers = triggers
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        tx.execute_batch(users_table!("users_new", " AUTOINCREMENT"))?;
        tx.execute_batch(
            "DROP VIEW IF EXISTS users_with_access;
             INSERT INTO users_new SELECT * FROM users;
             DROP TABLE users;
             ALTER TABLE users_new RENAME TO users;
             INSERT INTO sqlite_sequence (name, seq) SELECT 'users', 0
                 WHERE NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'users');
             UPDATE sqlite_sequence SET seq = MAX(seq,
                 COALESCE((SELECT MAX(original_user_id) FROM archived_conversations), 0),
                 COALESCE((SELECT MAX(user_id) FROM usage_log), 0))
             WHERE name = 'users';",
        )?;
        if let Some(view) = view {
            tx.execute_batch(&view)?;
        }
        for trigger in triggers {
            tx.execute_batch(&trigger)?;
        }
        Ok(())
    }

    pub(crate) fn open_existing(data_dir: &Path) -> Result<Self> {
        let mut conn = Connection::open_with_flags(
            data_dir.join("rag-hub.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        Self::schema_version(&conn)?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.set_transaction_behavior(rusqlite::TransactionBehavior::Immediate);
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn schema_version(c: &Connection) -> Result<i64> {
        let version = c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?;
        anyhow::ensure!(
            version <= MIGRATIONS.len() as i64,
            "database dibuat oleh versi server yang lebih baru (versi {version}); perbarui server"
        );
        Ok(version)
    }

    pub fn exists(data_dir: &Path) -> bool {
        data_dir.join("rag-hub.sqlite").is_file()
    }

    /// Untuk kode yang sudah berjalan di luar worker async.
    pub fn call_blocking<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut guard = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("koneksi database rusak"))?;
        f(&mut guard)
    }

    pub async fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let db = self.clone();
        tokio::task::spawn_blocking(move || db.call_blocking(f)).await?
    }

    pub(crate) fn open_trace(data_dir: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(
            data_dir.join("rag-hub.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?;
        conn.busy_timeout(std::time::Duration::from_millis(200))?;
        conn.pragma_update(None, "foreign_keys", true)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

fn enable_wal(conn: &Connection) -> rusqlite::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let result = conn
            .pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))
            .and_then(|mode| {
                if matches!(mode.as_str(), "wal" | "memory") {
                    Ok(())
                } else {
                    conn.pragma_update(None, "journal_mode", "WAL")
                }
            });
        if !matches!(&result, Err(e) if matches!(e.sqlite_error_code(), Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)))
            || std::time::Instant::now() >= deadline
        {
            return result;
        }
        // Pergantian journal pada DB baru bisa mengembalikan BUSY sebelum busy handler.
        std::thread::sleep(std::time::Duration::from_millis(25));
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

    #[test]
    fn two_slots_can_open_the_same_fresh_database() {
        let dir = std::env::temp_dir().join(format!("app-db-open-{}", rand::random::<u64>()));
        let start = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let open = || {
                start.wait();
                let db = Db::open(&dir).unwrap();
                let conn = db.conn.lock().unwrap();
                let version = conn
                    .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                    .unwrap();
                assert_eq!(version, MIGRATIONS.len() as i64);
            };
            let peer = scope.spawn(open);
            open();
            peer.join().unwrap();
        });
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn bd00a2c_accounts_and_legacy_sql_survive_the_user_id_migration() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("app-db-bd00a2c-{}", crate::files::new_id()));
        std::fs::create_dir(&dir)?;
        let path = dir.join("rag-hub.sqlite");
        let mut old = Connection::open(&path)?;
        enable_wal(&old)?;
        let tx = old.transaction()?;
        for sql in MIGRATIONS {
            tx.execute_batch(sql)?;
        }
        tx.pragma_update(None, "user_version", 10)?;
        tx.execute_batch(crate::archive::SCHEMA)?;
        crate::access::init(&tx)?;
        tx.execute_batch(crate::microsoft::SCHEMA)?;
        tx.execute_batch(
            "INSERT INTO users (id,email,name,password_hash,department,created_at) VALUES
                (1,'uji@example.test','Uji','','Review',0);
             INSERT INTO user_departments VALUES (1,'Operasional');
             INSERT INTO microsoft_identities VALUES ('t','o',1);
             INSERT INTO sessions VALUES ('uji',1,0,100);
             INSERT INTO usage_log (created_at,kind,user_id) VALUES (0,'chat',20);",
        )?;
        tx.commit()?;
        assert!(!Db::preserves_user_ids(&old)?);

        let new = Db::open(&dir)?;
        // Slot yang sudah terbuka tetap bisa membaca dan menjalankan SQL versi lama.
        assert_eq!(
            old.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?,
            10
        );
        let current = |c: &Connection| {
            c.query_row(
                "SELECT departments FROM users_with_access WHERE id=1",
                [],
                |r| crate::access::from_row(r, 0),
            )
        };
        assert_eq!(current(&old)?, ["Operasional", "Review"]);
        assert_eq!(
            old.query_row("SELECT count(*) FROM sessions WHERE user_id=1", [], |r| r
                .get::<_, i64>(
                0
            ))?,
            1
        );
        assert_eq!(
            old.query_row(
                "SELECT user_id FROM microsoft_identities WHERE tenant_id='t' AND object_id='o'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            1
        );
        old.execute("UPDATE users SET department='Umum' WHERE id=1", [])?;
        assert_eq!(current(&old)?, ["Umum"]);
        old.execute(
            "INSERT INTO users (email,name,password_hash,status,created_at)
                     VALUES ('baru@example.test','Uji','','pending',0)",
            [],
        )?;
        assert_eq!(old.last_insert_rowid(), 21);
        assert_eq!(
            old.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
                .get::<_, i64>(
                0
            ))?,
            0
        );
        drop((old, new));
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn access_backfill_and_6f7e8db_sql_share_one_database() {
        fn legacy(name: &str) -> &'static str {
            include_str!("tests/6f7e8db-access.sql")
                .split("\n-- ")
                .find_map(|part| {
                    let (tag, sql) = part.split_once('\n')?;
                    (tag == name).then(|| sql.trim_end())
                })
                .unwrap()
        }
        let dir = std::env::temp_dir().join(format!("app-db-access-{}", rand::random::<u64>()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("rag-hub.sqlite");
        let mut old = Connection::open(&path).unwrap();
        for sql in MIGRATIONS {
            old.execute_batch(sql).unwrap();
        }
        old.pragma_update(None, "user_version", 10).unwrap();
        old.execute_batch(
            "INSERT INTO users (id, email, name, password_hash, department, status, created_at)
             VALUES (1, 'legacy@example.test', 'Uji Lama', 'x', 'Review', 'active', 0),
                    (2, 'empty@example.test', 'Uji Kosong', 'x', NULL, 'active', 0),
                    (-1, 'archive@app.invalid', 'Arsip', '', 'Review', 'disabled', 0);",
        )
        .unwrap();

        let db = Db::open(&dir).unwrap();
        let mut new = db.conn.lock().unwrap();
        let read = |conn: &Connection, id| {
            conn.query_row(
                "SELECT departments FROM users_with_access WHERE id = ?1",
                [id],
                |r| crate::access::from_row(r, 0),
            )
            .unwrap()
        };
        assert_eq!(read(&new, 1), ["Review"]);
        assert!(read(&new, 2).is_empty());
        assert!(read(&new, -1).is_empty());

        // Startup rollback: 6f7e8db memakai batas user_version=10.
        drop(old);
        old = Connection::open(&path).unwrap();
        assert_eq!(
            old.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            10
        );
        let legacy_login = |email| {
            old.query_row(legacy("login"), [email], |r| {
                Ok((r.get::<_, Option<String>>(3)?, r.get::<_, String>(4)?))
            })
            .unwrap()
        };
        let replace = |conn: &mut Connection, departments: &[&str]| {
            let tx = conn.transaction().unwrap();
            let departments = departments
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>();
            crate::access::replace(&tx, 1, &departments).unwrap();
            tx.commit().unwrap();
        };
        replace(&mut new, &["Operasional", "Review"]);
        assert_eq!(read(&new, 1), ["Operasional", "Review"]);
        replace(&mut new, &["Operasional"]);
        assert_eq!(read(&new, 1), ["Operasional"]);
        assert_eq!(
            legacy_login("legacy@example.test"),
            (Some("Operasional".into()), "user".into())
        );
        assert_eq!(legacy_login("empty@example.test"), (None, "user".into()));

        old.execute(
            legacy("register"),
            rusqlite::params!["baru@example.test", "Uji Daftar", "x", "Contoh", 0],
        )
        .unwrap();
        let registered = old.last_insert_rowid();
        assert!(
            read(&new, registered).is_empty(),
            "permintaan signup tidak memberikan akses"
        );
        let list = |connection: &Connection| {
            connection
                .prepare(legacy("list_users"))
                .unwrap()
                .query_map([-1], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(4)?))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert!(list(&old).contains(&(1, Some("Operasional".into()))));

        replace(&mut new, &["Operasional", "Review"]);
        old.execute("INSERT INTO sessions VALUES ('uji', 1, 0, 100)", [])
            .unwrap();
        let session = || {
            use rusqlite::OptionalExtension;
            old.query_row(legacy("session"), rusqlite::params!["uji", 1], |r| {
                r.get::<_, Option<String>>(3)
            })
            .optional()
            .unwrap()
        };
        let old_update = |id, status, department: Option<&str>| {
            old.execute(
                legacy("update_user"),
                rusqlite::params![status, "user", department, false, 0, 1, id],
            )
            .unwrap();
        };
        assert_eq!(session(), Some(Some("Operasional".into())));
        old_update(1, "active", Some("Operasional"));
        assert_eq!(read(&new, 1), ["Operasional", "Review"]);
        assert!(session().is_some());
        old_update(1, "active", Some("Contoh"));
        assert_eq!(read(&new, 1), ["Contoh"]);
        assert_eq!(
            legacy_login("legacy@example.test").0.as_deref(),
            Some("Contoh")
        );
        assert!(session().is_none());
        old_update(1, "disabled", None);
        assert!(read(&new, 1).is_empty());
        old_update(registered, "active", Some("Contoh"));
        assert_eq!(read(&new, registered), ["Contoh"]);

        drop(new);
        drop((db, old));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn deferred_migration_cannot_upgrade_a_stale_wal_snapshot() {
        let dir = std::env::temp_dir().join(format!("app-db-deferred-{}", rand::random::<u64>()));
        let db = Db::open(&dir).unwrap();
        let mut stale = Connection::open(dir.join("rag-hub.sqlite")).unwrap();
        let tx = stale.transaction().unwrap();
        tx.query_row("SELECT count(*) FROM users", [], |r| r.get::<_, i64>(0))
            .unwrap();
        db.conn.lock().unwrap().execute(
            "INSERT INTO users (email,name,password_hash,created_at) VALUES ('db@example.test','Uji','',0)",
            [],
        ).unwrap();
        let err = tx.pragma_update(None, "user_version", 10).unwrap_err();
        assert_eq!(
            err.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseBusy)
        );
        drop(tx);
        drop((stale, db));
        std::fs::remove_dir_all(dir).unwrap();
    }

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
    fn trace_migration_preserves_old_messages_and_schema_version() {
        let conn = v3_database();
        conn.pragma_update(None, "foreign_keys", false).unwrap();
        for sql in &MIGRATIONS[3..] {
            conn.execute_batch(sql).unwrap();
        }
        conn.execute_batch(COMPACTIONS_SCHEMA).unwrap();
        conn.pragma_update(None, "user_version", 10).unwrap();
        let db = Db::init(conn).unwrap();
        let conn = db.conn.lock().unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        let old: (String, String, i64) = conn
            .query_row(
                "SELECT q.content, a.content, (SELECT count(*) FROM turn_traces)
             FROM messages q, messages a WHERE q.id = 1 AND a.id = 2",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(version, 10);
        assert_eq!(old, ("tanya".into(), "jawab".into(), 0));
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
