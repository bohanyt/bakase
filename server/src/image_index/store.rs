use super::eg2::DIM;
use anyhow::Result;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::{collections::HashMap, path::Path};

pub(super) type Hash = [u8; 32];
pub(super) type Vectors = HashMap<Hash, (Vec<String>, Vec<f32>)>;
const MAX_ATTEMPTS: u32 = 3;

#[derive(Debug, PartialEq)]
pub(super) enum FileState {
    Work,
    Done,
    Failed,
}

pub(super) struct Reference {
    pub(super) doc_id: i64,
    pub(super) stamp: Hash,
    pub(super) location: String,
}

pub(super) struct Indexed {
    pub(super) hash: Hash,
    pub(super) vector: Vec<f32>,
    pub(super) refs: Vec<Reference>,
}

fn floats(bytes: Vec<u8>) -> rusqlite::Result<Vec<f32>> {
    if bytes.len() != DIM * 4 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("empat byte")))
        .collect())
}

pub(super) fn open_reader(path: &Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?)
}

pub(super) fn snapshot(conn: &mut Connection) -> Result<Vec<Indexed>> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)?;
    let mut vectors = {
        let mut s = tx.prepare("SELECT hash, embedding FROM vectors")?;
        s.query_map([], |row| {
            Ok((
                row.get::<_, Hash>(0)?,
                Indexed {
                    hash: row.get(0)?,
                    vector: floats(row.get(1)?)?,
                    refs: Vec::new(),
                },
            ))
        })?
        .collect::<rusqlite::Result<HashMap<_, _>>>()?
    };
    {
        let mut s = tx.prepare(
            "SELECT r.hash, r.doc_id, f.stamp, r.location FROM refs r
            JOIN files f USING(doc_id) WHERE f.complete = 1 ORDER BY r.doc_id, r.location",
        )?;
        let mut rows = s.query([])?;
        while let Some(row) = rows.next()? {
            if let Some(vec) = vectors.get_mut(&row.get::<_, Hash>(0)?) {
                vec.refs.push(Reference {
                    doc_id: row.get(1)?,
                    stamp: row.get(2)?,
                    location: row.get(3)?,
                });
            }
        }
    }
    tx.commit()?;
    Ok(vectors
        .into_values()
        .filter(|entry| !entry.refs.is_empty())
        .collect())
}

pub(super) fn reference(
    path: &Path,
    doc_id: i64,
    stamp: &Hash,
    hash: &Hash,
) -> Result<Option<String>> {
    if !path.is_file() {
        return Ok(None);
    }
    let conn = open_reader(path)?;
    Ok(conn
        .query_row(
            "SELECT r.location FROM files f JOIN refs r USING(doc_id)
         WHERE f.doc_id = ? AND f.stamp = ? AND r.hash = ? AND f.complete = 1
         ORDER BY r.location LIMIT 1",
            params![doc_id, stamp, hash],
            |r| r.get(0),
        )
        .optional()?)
}
pub(super) struct Store {
    conn: Connection,
}

impl Store {
    pub(super) fn open(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        conn.set_transaction_behavior(rusqlite::TransactionBehavior::Immediate);
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS files (
                doc_id INTEGER PRIMARY KEY, stamp BLOB NOT NULL, complete INTEGER NOT NULL,
                attempts INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS vectors (
                hash BLOB PRIMARY KEY, embedding BLOB NOT NULL CHECK(length(embedding) = 3072)
             ) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS refs (
                doc_id INTEGER NOT NULL REFERENCES files(doc_id) ON DELETE CASCADE,
                location TEXT NOT NULL, hash BLOB NOT NULL REFERENCES vectors(hash),
                PRIMARY KEY(doc_id, location)
             ) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS image_refs_hash ON refs(hash);",
        )?;
        Ok(Self { conn })
    }

    pub(super) fn reconcile(&mut self, available: &HashMap<i64, Hash>) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut list = tx.prepare("SELECT doc_id, stamp FROM files")?;
            let stale = list
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, Hash>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut delete = tx.prepare_cached("DELETE FROM files WHERE doc_id = ?")?;
            for (id, stamp) in stale {
                if available.get(&id) != Some(&stamp) {
                    delete.execute([id])?;
                }
            }
        }
        tx.execute("DELETE FROM vectors WHERE NOT EXISTS (SELECT 1 FROM refs WHERE refs.hash = vectors.hash)", [])?;
        tx.commit()?;
        Ok(())
    }

    pub(super) fn begin(&self, id: i64, stamp: &Hash) -> Result<FileState> {
        if self.conn.execute(
            "INSERT INTO files VALUES (?, ?, 0, 1)
             ON CONFLICT(doc_id) DO UPDATE SET attempts = attempts + 1
             WHERE stamp = excluded.stamp AND complete = 0 AND attempts < ?",
            params![id, stamp, MAX_ATTEMPTS],
        )? == 1
        {
            return Ok(FileState::Work);
        }
        let complete = self.conn.query_row(
            "SELECT complete FROM files WHERE doc_id = ? AND stamp = ?",
            params![id, stamp],
            |row| row.get(0),
        )?;
        Ok(if complete {
            FileState::Done
        } else {
            FileState::Failed
        })
    }

    pub(super) fn fail(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE files SET attempts = ? WHERE doc_id = ? AND complete = 0",
            params![MAX_ATTEMPTS, id],
        )?;
        Ok(())
    }

    pub(super) fn has(&self, hash: &Hash) -> Result<bool> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM vectors WHERE hash = ?", [hash], |_| Ok(()))
            .optional()?
            .is_some())
    }

    pub(super) fn finish(&mut self, id: i64, rows: Vectors) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut insert_vector = tx.prepare_cached("INSERT INTO vectors VALUES (?, ?)")?;
            let mut insert_ref =
                tx.prepare_cached("INSERT OR REPLACE INTO refs VALUES (?, ?, ?)")?;
            for (hash, (locations, vector)) in rows {
                if !vector.is_empty() {
                    let bytes: Vec<_> = vector.into_iter().flat_map(f32::to_le_bytes).collect();
                    insert_vector.execute(params![hash, bytes])?;
                }
                for location in locations {
                    insert_ref.execute(params![id, location, hash])?;
                }
            }
        }
        tx.execute("UPDATE files SET complete = 1 WHERE doc_id = ?", [id])?;
        tx.commit()?;
        Ok(())
    }

    pub(super) fn counts(&self) -> Result<(u64, u64)> {
        Ok(self.conn.query_row(
            "SELECT count(DISTINCT r.hash), count(*) FROM refs r JOIN files f USING (doc_id)
             WHERE f.complete = 1",
            [],
            |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, i64>(1)? as u64)),
        )?)
    }
}

#[allow(dead_code)]
pub(crate) struct Entry {
    pub(crate) location: String,
    pub(crate) vector: Vec<f32>,
}

pub(super) fn document(path: &Path, id: i64, stamp: &Hash) -> Result<Vec<Entry>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut statement = conn.prepare(
        "SELECT r.location, v.embedding FROM files f JOIN refs r USING(doc_id)
         JOIN vectors v USING(hash)
         WHERE f.doc_id = ? AND f.stamp = ? AND f.complete = 1 ORDER BY r.location",
    )?;
    let rows = statement.query_map(params![id, stamp], |row| {
        Ok(Entry {
            location: row.get(0)?,
            vector: floats(row.get(1)?)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}
