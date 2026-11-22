//! Dokumen sumber yang sudah diindeks: pencarian dengan hak akses, isi potongan untuk
//! panel sitasi, dan unduhan file aslinya.
//!
//! Dokumen hanya disajikan kalau semua sumber hak akses setuju: config yang dipakai
//! server ini, baris di tabel `documents` (ditulis ingest), dan label di index. Config
//! yang basi, di server maupun di ingest, hanya bisa memperketat, tidak membuka akses.

use crate::AppState;
use crate::acl::{AuthUser, Visibility, can_read};
use crate::config::SourceConfig;
use crate::db::Db;
use crate::error::ApiError;
use crate::ingest::{NAME_ONLY_VERSION, status};
use crate::search::{DocIndex, Hit, Refused, SearchFilters, doc_of_chunk};
use anyhow::Result;
use axum::{
    Json,
    body::Body,
    extract::{Path, Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use rusqlite::params_from_iter;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tower_http::services::ServeFile;

#[derive(Debug, Clone, Serialize)]
pub struct DocumentRow {
    pub id: i64,
    pub source: String,
    pub folder: String,
    #[serde(skip)]
    pub rel_path: String,
    pub title: String,
    /// Judul tanpa awalan ARCHIVE dan akhiran teknis, seperti di Sumber Data.
    pub display_title: String,
    pub kind: String,
    pub pages: i64,
    pub size: i64,
    /// Waktu file terakhir diubah (milidetik sejak epoch).
    pub modified: i64,
    /// Tanggal dokumen `YYYY-MM-DD` dari judul, isi, metadata file, atau upload ARCHIVE, kalau
    /// diketahui.
    pub doc_date: Option<String>,
    /// Asal tanggal dokumen: `judul`, `isi`, `metadata`, atau `archive`.
    pub doc_date_from: Option<String>,
    #[serde(skip)]
    pub visibility: Option<Visibility>,
    pub status: String,
    #[serde(skip)]
    version: i64,
    /// Path kanonik folder sumber saat diindeks.
    #[serde(skip)]
    pub root: String,
}

/// Boleh dibaca `user` menurut config server ini dan menurut baris `documents`. Sumber
/// yang tidak ada di config server tidak disajikan.
fn label_allows(
    user: &AuthUser,
    sources: &[SourceConfig],
    source: &str,
    visibility: Option<&Visibility>,
) -> bool {
    let Some(config) = sources.iter().find(|s| s.name == source) else {
        return false;
    };
    can_read(user, Some(&config.visibility)) && can_read(user, visibility)
}

/// Status yang bisa punya potongan isi, termasuk PDF scan dari index lama.
pub(crate) fn has_content(state: &str) -> bool {
    state == status::OK
        || state == status::SCAN
        || state == status::VISION
        || state == status::ARCHIVE
}

/// Seperti `label_allows`, dan isinya ada di index.
fn allowed(
    user: &AuthUser,
    sources: &[SourceConfig],
    source: &str,
    visibility: Option<&Visibility>,
    state: &str,
) -> bool {
    label_allows(user, sources, source, visibility) && has_content(state)
}

fn name_only(state: &str, version: i64) -> bool {
    state != status::PENDING
        && !matches!(state, status::OK | status::VISION | status::ARCHIVE)
        && version == NAME_ONLY_VERSION
}

impl DocumentRow {
    pub(crate) fn allows_entry(&self, user: &AuthUser, sources: &[SourceConfig]) -> bool {
        label_allows(user, sources, &self.source, self.visibility.as_ref())
            && (self.name_only() || has_content(&self.status))
    }

    pub(crate) fn name_only(&self) -> bool {
        name_only(&self.status, self.version)
    }

    pub(crate) fn matches_entry(&self, name_only: bool) -> bool {
        self.name_only() == name_only
    }

    fn readable_by(&self, user: &AuthUser, sources: &[SourceConfig]) -> bool {
        allowed(
            user,
            sources,
            &self.source,
            self.visibility.as_ref(),
            &self.status,
        )
    }
}

fn subfolder(rel_path: &str) -> String {
    rel_path
        .rsplit_once('/')
        .map_or("", |(dir, _)| dir)
        .to_string()
}

pub(crate) async fn rows(
    db: &Db,
    table: &'static str,
    ids: Vec<i64>,
) -> Result<HashMap<i64, DocumentRow>> {
    rows_with_guard(db, table, ids, ()).await
}

pub(crate) async fn rows_with_guard(
    db: &Db,
    table: &'static str,
    ids: Vec<i64>,
    guard: impl Send + 'static,
) -> Result<HashMap<i64, DocumentRow>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    db.call(move |c| {
        let _guard = guard;
        let tx = c.transaction()?;
        let mut out = HashMap::new();
        for ids in ids.chunks(500) {
            let marks = vec!["?"; ids.len()].join(",");
            let mut stmt = tx.prepare_cached(&format!(
                "SELECT id, source, rel_path, title, kind, pages, size, visibility, status, root, mtime, version, {}, {}
                 FROM {table} WHERE id IN ({marks})",
                v4_column(table, "doc_date"),
                v4_column(table, "doc_date_from")
            ))?;
            for row in stmt.query_map(params_from_iter(ids.iter()), document_row)? {
                let row = row?;
                out.insert(row.id, row);
            }
        }
        tx.commit()?;
        Ok(out)
    })
    .await
}

fn document_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<DocumentRow> {
    let visibility: String = r.get(7)?;
    let rel_path: String = r.get(2)?;
    let title: String = r.get(3)?;
    Ok(DocumentRow {
        id: r.get(0)?,
        source: r.get(1)?,
        folder: subfolder(&rel_path),
        rel_path,
        display_title: display_title(&title),
        title,
        kind: r.get(4)?,
        pages: r.get(5)?,
        size: r.get(6)?,
        // Label yang tidak terbaca berarti tertutup (hanya admin).
        visibility: serde_json::from_str(&visibility).ok(),
        status: r.get(8)?,
        root: r.get(9)?,
        modified: r.get(10)?,
        version: r.get(11)?,
        doc_date: r.get(12)?,
        doc_date_from: r.get(13)?,
    })
}

/// Tabel v3 (`documents`) tidak punya kolom tanggal dokumen, jadi `column` dibaca sebagai NULL.
fn v4_column(table: &str, column: &'static str) -> &'static str {
    if table == crate::search::INDEX_TABLE {
        column
    } else {
        "NULL"
    }
}

#[cfg(test)]
pub fn readable_ids(
    c: &rusqlite::Connection,
    user: &AuthUser,
    sources: &[SourceConfig],
    index: Option<&DocIndex>,
    ids: &[i64],
) -> Result<std::collections::HashSet<i64>> {
    let Some(index) = index else {
        return Ok(Default::default());
    };
    let mut allowed = history_row_ids(c, user, sources, index.table(), ids, false)?;
    let candidates: Vec<_> = allowed.iter().copied().collect();
    let refused = index.refusing_in(&index.snapshot(), &candidates, user)?;
    allowed.retain(|id| !refused.contains(id));
    Ok(allowed)
}

/// Pemeriksaan label SQLite. Pemanggil memeriksa label indeks setelah melepas mutex DB.
pub(crate) fn history_row_ids(
    c: &rusqlite::Connection,
    user: &AuthUser,
    sources: &[SourceConfig],
    table: &str,
    ids: &[i64],
    metadata: bool,
) -> Result<std::collections::HashSet<i64>> {
    let mut out = std::collections::HashSet::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let marks = vec!["?"; ids.len()].join(",");
    let mut stmt = c.prepare(&format!(
        "SELECT id, source, visibility, status, version
         FROM {table} WHERE id IN ({marks})"
    ))?;
    let rows = stmt.query_map(params_from_iter(ids.iter()), |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, i64>(4)?,
        ))
    })?;
    for row in rows {
        let (id, source, visibility, state, version) = row?;
        let visibility: Option<Visibility> = serde_json::from_str(&visibility).ok();
        let entry_only = name_only(&state, version);
        if if entry_only {
            metadata && label_allows(user, sources, &source, visibility.as_ref())
        } else {
            allowed(user, sources, &source, visibility.as_ref(), &state)
        } {
            out.insert(id);
        }
    }
    Ok(out)
}

async fn row(db: &Db, index: &DocIndex, id: i64) -> Result<Option<DocumentRow>> {
    Ok(rows(db, index.table(), vec![id]).await?.remove(&id))
}

pub(crate) fn keyword_query(text: &str) -> String {
    text.trim().chars().take(300).collect()
}

/// Potongan paling cocok yang boleh dibaca `user`. Label di index sudah menyaring;
/// hasilnya dicek lagi terhadap tabel `documents` dan config server.
#[allow(clippy::too_many_arguments)]
pub async fn search(
    index: Arc<DocIndex>,
    db: &Db,
    user: &AuthUser,
    sources: &[SourceConfig],
    query: String,
    limit: usize,
    name_only_limit: usize,
    snippets: bool,
    filters: SearchFilters,
) -> Result<Vec<(Hit, DocumentRow)>> {
    search_ranked(
        index,
        db,
        user,
        sources,
        query,
        limit,
        name_only_limit,
        snippets,
        filters,
        false,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn search_ranked(
    index: Arc<DocIndex>,
    db: &Db,
    user: &AuthUser,
    sources: &[SourceConfig],
    query: String,
    limit: usize,
    name_only_limit: usize,
    snippets: bool,
    filters: SearchFilters,
    distinct_documents: bool,
) -> Result<Vec<(Hit, DocumentRow)>> {
    let names: Arc<Vec<String>> = Arc::new(
        sources
            .iter()
            .filter(|s| can_read(user, Some(&s.visibility)))
            .map(|s| s.name.clone())
            .collect(),
    );
    let query = Arc::new(query);
    let filters = Arc::new(filters);
    // Semua halaman memakai snapshot index yang sama.
    let snapshot = index.snapshot();
    let table = index.table();
    let mut seen = std::collections::HashSet::new();
    let mut seen_documents = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut name_only_count = 0;
    // Kandidat diambil per halaman sampai hasil yang lolos cukup, supaya hasil yang
    // ditolak tabel `documents` tidak menutupi hasil yang boleh dibaca.
    let mut offset = 0;
    let candidate_limit = if distinct_documents {
        10 * MAX_CANDIDATES
    } else {
        MAX_CANDIDATES
    };
    while out.len() < limit && offset < candidate_limit {
        let (index, snapshot, who, names, query) = (
            index.clone(),
            snapshot.clone(),
            user.clone(),
            names.clone(),
            query.clone(),
        );
        let filters = filters.clone();
        // Dokumen dengan potongan yang labelnya di index menolak user (label bercampur)
        // dibuang seluruhnya, sama seperti di daftar, preview, sitasi, dan unduhan. Dicek di
        // snapshot yang sama.
        let (page, refused) = tokio::task::spawn_blocking(move || -> Result<_> {
            let page = index.search_in(
                &snapshot,
                &query,
                &who,
                &names,
                offset,
                CANDIDATE_PAGE,
                snippets,
                &filters,
            )?;
            let mut ids: Vec<i64> = page.iter().map(|h| h.doc_id).collect();
            ids.sort_unstable();
            ids.dedup();
            let refused = index.refusing_in(&snapshot, &ids, &who)?;
            Ok((page, refused))
        })
        .await??;
        let fetched = page.len();
        let found = rows(db, table, page.iter().map(|h| h.doc_id).collect()).await?;
        for hit in page {
            if out.len() == limit {
                break;
            }
            if !seen.insert(hit.chunk_id.clone()) {
                continue;
            }
            if let Some(row) = found.get(&hit.doc_id).filter(|r| {
                r.allows_entry(user, sources)
                    && r.matches_entry(hit.text.is_empty())
                    && !refused.contains(&r.id)
            }) {
                if distinct_documents && !seen_documents.insert(row.id) {
                    continue;
                }
                if row.name_only() {
                    if name_only_count == name_only_limit {
                        continue;
                    }
                    name_only_count += 1;
                }
                out.push((hit, row.clone()));
            }
        }
        if fetched < CANDIDATE_PAGE {
            break;
        }
        offset += CANDIDATE_PAGE;
    }
    Ok(out)
}

/// Kandidat yang diambil dari index sekali jalan, dan batas totalnya per pencarian.
const CANDIDATE_PAGE: usize = 50;
const MAX_CANDIDATES: usize = 1_000;

/// Satu file di halaman Sumber Data dan di tool `list_documents`.
#[derive(Debug, Clone, Serialize)]
pub struct ListedDocument {
    pub id: i64,
    pub title: String,
    pub display_title: String,
    pub source: String,
    /// Folder di dalam sumber, misalnya `Standar/2024`; kosong kalau di akar sumber.
    pub folder: String,
    pub kind: String,
    pub status: String,
    /// Waktu file terakhir diubah (milidetik sejak epoch).
    pub modified: i64,
    /// Tanggal dokumen `YYYY-MM-DD`, kalau diketahui.
    pub doc_date: Option<String>,
    /// Asal tanggal dokumen: `judul`, `isi`, `metadata`, atau `archive`.
    pub doc_date_from: Option<String>,
    pub size: i64,
    pub pages: i64,
    pub chunks: i64,
    /// File asli bisa diunduh. `chunks` hanya menghitung isi.
    pub available: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    #[default]
    Judul,
    Terbaru,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ListFilter {
    pub source: Option<String>,
    /// Kata dari judul atau path file; semua kata harus ada.
    pub query: String,
    /// Status di tabel `documents`; kosong berarti semua status.
    pub status: Vec<String>,
    pub sort: Sort,
    pub offset: usize,
    /// 0 berarti bawaan.
    pub limit: usize,
}

#[derive(Debug, Serialize)]
pub struct SourceCount {
    pub name: String,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct ListPage {
    pub documents: Vec<ListedDocument>,
    /// Dokumen yang cocok dengan filter. Hanya menghitung yang boleh dibaca user.
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    /// Sumber yang boleh dibaca user menurut config server, urut seperti di config.
    pub sources: Vec<SourceCount>,
    /// Jumlah per status untuk sumber dan kata yang sama, tanpa filter status.
    pub statuses: std::collections::BTreeMap<String, usize>,
}

const TECHNICAL_SUFFIXES: &[&str] = &["_extracted"];

pub(crate) fn display_title(original: &str) -> String {
    let Some((hash, name)) = original.split_once('_') else {
        return original.to_owned();
    };
    if hash.len() < 8
        || !hash.bytes().all(|b| b.is_ascii_hexdigit())
        || !hash.bytes().any(|b| b.is_ascii_digit())
        || !hash.bytes().any(|b| b.is_ascii_alphabetic())
    {
        return original.to_owned();
    }

    let name = TECHNICAL_SUFFIXES
        .iter()
        .find_map(|suffix| {
            let end = name.len().checked_sub(suffix.len())?;
            name.get(end..)?
                .eq_ignore_ascii_case(suffix)
                .then(|| &name[..end])
        })
        .unwrap_or(name);
    let title = name
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let title = title.trim();
    if title.chars().filter(|c| !c.is_whitespace()).take(2).count() < 2 {
        original.to_owned()
    } else {
        title.to_owned()
    }
}

#[cfg(test)]
mod title_tests {
    use super::display_title;

    #[test]
    fn display_title_keeps_original_document_names() {
        for name in [
            "Spesifikasi Widget v1.1",
            "SOP Rev.3",
            "20251118_Laporan",
            "12345678_Daftar",
            "deadbeef_Laporan",
            "cafef00_Laporan_Review",
            "Daftar_SOP_Review",
            "Form_IK_Review",
            "Prosedur_Kalibrasi_ISO_QMS",
            "Laporan_EXTRACTED",
        ] {
            assert_eq!(display_title(name), name);
        }
    }

    #[test]
    fn display_title_removes_hash_and_known_suffix() {
        for (name, expected) in [
            ("cafef00d_Catatan_Kegiatan_EXTRACTED", "Catatan Kegiatan"),
            (
                "BEEF0123ABCD_Ringkasan.Review_extracted",
                "Ringkasan.Review",
            ),
            ("BEEF0123ABCD_SOP_Rev.3", "SOP Rev.3"),
            ("cafef00d_Prosedur_ISO_QMS", "Prosedur ISO QMS"),
            ("cafef00d_Catatan_EXTRACTED_Lama", "Catatan EXTRACTED Lama"),
            ("cafef00d_検査_EXTRACTED", "検査"),
            ("cafef00d_Review_EXTRACTED", "Review"),
        ] {
            assert_eq!(display_title(name), expected);
        }
    }

    #[test]
    fn display_title_falls_back_for_empty_or_single_character() {
        for name in ["", "A", "cafef00d_", "cafef00d_検_EXTRACTED"] {
            assert_eq!(display_title(name), name);
        }
    }
}

const LIST_LIMIT: usize = 50;
const MAX_LIST_LIMIT: usize = 100;
const MAX_LIST_WORDS: usize = 8;

/// Daftar dokumen yang boleh dibaca `user`: config server dan label di baris `documents`
/// harus mengizinkan, dan dokumen dengan potongan yang labelnya di index menolak user tidak
/// ikut (`DocIndex::refused`). Filter hak akses dijalankan sebelum menghitung dan memotong
/// halaman, jadi jumlah dan halaman tidak pernah menghitung dokumen lain.
pub async fn list(
    db: &Db,
    index: &Arc<DocIndex>,
    user: &AuthUser,
    sources: &[SourceConfig],
    filter: ListFilter,
) -> Result<ListPage> {
    let readable: Vec<String> = sources
        .iter()
        .filter(|s| can_read(user, Some(&s.visibility)))
        .map(|s| s.name.clone())
        .collect();
    let table = index.table();
    let (index, who) = (index.clone(), user.clone());
    let refused = tokio::task::spawn_blocking(move || index.refused(&who, &readable)).await??;
    let (user, sources) = (user.clone(), sources.to_vec());
    db.call(move |c| list_in(c, table, &user, &sources, &filter, &refused))
        .await
}

fn list_in(
    c: &rusqlite::Connection,
    table: &str,
    user: &AuthUser,
    sources: &[SourceConfig],
    filter: &ListFilter,
    refused: &Refused,
) -> Result<ListPage> {
    use rusqlite::types::Value as Sql;
    let limit = match filter.limit {
        0 => LIST_LIMIT,
        n => n.min(MAX_LIST_LIMIT),
    };
    let offset = filter.offset.min(i64::MAX as usize);
    let mut page = ListPage {
        documents: Vec::new(),
        total: 0,
        offset,
        limit,
        sources: Vec::new(),
        statuses: Default::default(),
    };
    // Semua file satu folder sumber memakai label yang sama, jadi pasangan (sumber, label)
    // sedikit. Setiap pasangan dicek dengan `can_read`, lalu hanya yang lolos dipakai di SQL.
    let mut stmt = c.prepare(&format!("SELECT DISTINCT source, visibility FROM {table}"))?;
    let pairs: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|(source, visibility)| {
            let visibility: Option<Visibility> = serde_json::from_str(visibility).ok();
            label_allows(user, sources, source, visibility.as_ref())
                && !refused.sources.contains(source)
        })
        .collect();

    // Baris pending sedang dibaca ulang; labelnya bisa basi saat `ingest --ulang`.
    let mut readable = String::from("status != 'pending' AND (");
    let mut readable_params: Vec<Sql> = Vec::new();
    for (i, (source, visibility)) in pairs.iter().enumerate() {
        readable.push_str(if i == 0 { "" } else { " OR " });
        readable.push_str("(source = ? AND visibility = ?)");
        readable_params.push(Sql::Text(source.clone()));
        readable_params.push(Sql::Text(visibility.clone()));
    }
    readable.push(')');
    if !refused.docs.is_empty() {
        readable.push_str(" AND id NOT IN (SELECT value FROM json_each(?))");
        let ids: Vec<i64> = refused.docs.iter().copied().collect();
        readable_params.push(Sql::Text(serde_json::to_string(&ids)?));
    }

    let mut counts: HashMap<String, usize> = HashMap::new();
    if !pairs.is_empty() {
        let mut stmt = c.prepare(&format!(
            "SELECT source, count(*) FROM {table} WHERE {readable} GROUP BY source"
        ))?;
        let rows = stmt.query_map(params_from_iter(readable_params.iter()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (source, n) = row?;
            counts.insert(source, usize::try_from(n).unwrap_or(0));
        }
    }
    page.sources = sources
        .iter()
        .filter(|s| can_read(user, Some(&s.visibility)))
        .map(|s| SourceCount {
            name: s.name.clone(),
            total: counts.get(&s.name).copied().unwrap_or(0),
        })
        .collect();
    if pairs.is_empty() {
        return Ok(page);
    }

    let mut matching = readable;
    let mut params = readable_params;
    if let Some(source) = &filter.source {
        matching.push_str(" AND source = ?");
        params.push(Sql::Text(source.clone()));
    }
    c.create_scalar_function(
        "document_display_title",
        1,
        rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| Ok(display_title(&ctx.get::<String>(0)?)),
    )?;
    for word in filter.query.split_whitespace().take(MAX_LIST_WORDS) {
        matching.push_str(r" AND (title LIKE ? ESCAPE '\' OR rel_path LIKE ? ESCAPE '\')");
        // Setiap kata judul tampilan juga ada di nama asli, jadi cukup mencari nama asli.
        let pattern = crate::conversations::like_pattern(word);
        params.push(Sql::Text(pattern.clone()));
        params.push(Sql::Text(pattern));
    }

    let mut stmt = c.prepare(&format!(
        "SELECT status, count(*) FROM {table} WHERE {matching} GROUP BY status"
    ))?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (state, n) = row?;
        page.statuses.insert(state, usize::try_from(n).unwrap_or(0));
    }
    page.total = page
        .statuses
        .iter()
        .filter(|(state, _)| filter.status.is_empty() || filter.status.contains(state))
        .map(|(_, n)| n)
        .sum();

    if !filter.status.is_empty() {
        let marks = vec!["?"; filter.status.len()].join(",");
        matching.push_str(&format!(" AND status IN ({marks})"));
        params.extend(filter.status.iter().cloned().map(Sql::Text));
    }
    let doc_date = v4_column(table, "doc_date");
    let order = match filter.sort {
        Sort::Judul => "display_title COLLATE NOCASE, rel_path, id".to_owned(),
        // Tanggal dokumen; kalau tidak diketahui, tanggal file diubah (WIB).
        Sort::Terbaru => format!(
            "COALESCE({doc_date}, strftime('%Y-%m-%d', mtime / 1000 + 25200, 'unixepoch')) DESC, mtime DESC, id DESC"
        ),
    };
    params.push(Sql::Integer(limit as i64));
    params.push(Sql::Integer(offset as i64));
    let mut stmt = c.prepare(&format!(
        "SELECT id, source, rel_path, title, kind, status, mtime, size, pages, chunks, version,
                document_display_title(title) AS display_title, {doc_date}, {}
         FROM {table} WHERE {matching} ORDER BY {order} LIMIT ? OFFSET ?",
        v4_column(table, "doc_date_from")
    ))?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |r| {
        let rel_path: String = r.get(2)?;
        let state: String = r.get(5)?;
        Ok(ListedDocument {
            id: r.get(0)?,
            source: r.get(1)?,
            folder: subfolder(&rel_path),
            title: r.get(3)?,
            display_title: r.get(11)?,
            kind: r.get(4)?,
            available: has_content(&state) || name_only(&state, r.get(10)?),
            status: state,
            modified: r.get(6)?,
            doc_date: r.get(12)?,
            doc_date_from: r.get(13)?,
            size: r.get(7)?,
            pages: r.get(8)?,
            chunks: r.get(9)?,
        })
    })?;
    for row in rows {
        page.documents.push(row?);
    }
    Ok(page)
}

fn not_found() -> ApiError {
    ApiError::not_found("Dokumen tidak ditemukan atau sudah tidak ada.")
}

fn unavailable() -> ApiError {
    ApiError::not_found("Pencarian dokumen belum aktif di server ini.")
}

#[derive(Deserialize)]
pub struct SearchRequest {
    query: String,
}

#[derive(Serialize)]
struct SearchHit {
    chunk_id: String,
    page: Option<u32>,
    snippet: Vec<crate::search::Segment>,
    document: DocumentRow,
}

/// `POST /api/documents/search`: pencarian dokumen dari palet Ctrl+K.
pub async fn search_route(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Json(req): Json<SearchRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Some(index) = state.docs.clone() else {
        return Err(unavailable());
    };
    let query: String = req.query.trim().chars().take(200).collect();
    if query.is_empty() {
        return Ok(Json(serde_json::json!({ "hits": [] })));
    }
    let text_hits = search(
        index.clone(),
        &state.db,
        &user,
        &state.sources,
        query.clone(),
        20,
        20,
        true,
        SearchFilters::default(),
    );
    let deadline = tokio::time::Instant::now() + crate::image_index::SEARCH_DEADLINE;
    let image_hits = async {
        let Some(live) = &state.images else {
            return Vec::new();
        };
        if !live.available(deadline).await {
            return Vec::new();
        }
        let Ok(vector) = live.text(query, deadline).await else {
            return Vec::new();
        };
        let access = crate::image_index::Access {
            db: &state.db,
            index: &index,
            user: &user,
            sources: &state.sources,
        };
        live.search(access, vector, false, &SearchFilters::default(), deadline)
            .await
            .unwrap_or_default()
    };
    let (hits, images) = tokio::join!(text_hits, tokio::time::timeout_at(deadline, image_hits));
    let images = images.unwrap_or_default();
    let hits = hits.map_err(|e| {
        tracing::error!("pencarian dokumen gagal: {e:#}");
        ApiError::new(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "Pencarian dokumen sedang bermasalah.",
        )
    })?;
    let hits: Vec<SearchHit> = hits
        .into_iter()
        .map(|(hit, document)| SearchHit {
            chunk_id: hit.chunk_id,
            page: hit.page,
            snippet: hit.snippet,
            document,
        })
        .collect();
    Ok(Json(if images.is_empty() {
        serde_json::json!({ "hits": hits })
    } else {
        serde_json::json!({ "hits": hits, "images": images })
    }))
}

/// `GET /api/documents/chunks/{chunk_id}`: isi satu potongan untuk panel sitasi.
pub async fn chunk(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(chunk_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Some(index) = state.docs.clone() else {
        return Err(unavailable());
    };
    let doc_id = doc_of_chunk(&chunk_id).ok_or_else(not_found)?;
    // Hak akses dicek dari dokumennya sebelum isi potongan dibaca.
    let document = row(&state.db, &index, doc_id)
        .await
        .map_err(ApiError::from)?
        .filter(|d| d.allows_entry(&user, &state.sources))
        .ok_or_else(not_found)?;
    // Label index juga dicek sebelum pesan khusus di bawah, supaya dokumen yang ditolak
    // index dijawab sama seperti dokumen yang tidak ada.
    if refused(&index, doc_id, &user).await? {
        return Err(not_found());
    }
    let (id, who) = (chunk_id.clone(), user.clone());
    // Dokumennya boleh dibaca, jadi pesan ini tidak membocorkan apa pun. Biasanya
    // potongannya hilang karena dokumen sudah diperbarui sejak dikutip.
    let changed = || {
        ApiError::not_found(
            "Potongan ini sudah tidak ada; dokumennya mungkin sudah diperbarui. Unduh dokumen asli untuk melihat isi terbaru.",
        )
    };
    let hit = tokio::task::spawn_blocking(move || index.chunk(&id, &who))
        .await
        .map_err(|e| ApiError::from(anyhow::Error::from(e)))?
        .map_err(ApiError::from)?
        .filter(|h| h.doc_id == doc_id && document.matches_entry(h.text.is_empty()))
        .ok_or_else(changed)?;
    Ok(Json(serde_json::json!({
        "chunk_id": chunk_id,
        "page": hit.page,
        "text": hit.text,
        "document": document,
    })))
}

/// `POST /api/documents/list`: daftar dokumen untuk halaman Sumber Data. POST supaya kata
/// pencarian tidak tercatat di log akses server.
pub async fn list_route(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Json(mut filter): Json<ListFilter>,
) -> Result<Json<ListPage>, ApiError> {
    let Some(index) = state.docs.clone() else {
        return Err(unavailable());
    };
    filter.query = filter.query.trim().chars().take(200).collect();
    let page = list(&state.db, &index, &user, &state.sources, filter)
        .await
        .map_err(|e| {
            tracing::error!("daftar dokumen gagal: {e:#}");
            ApiError::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Daftar dokumen sedang bermasalah.",
            )
        })?;
    Ok(Json(page))
}

/// Potongan per halaman preview di Sumber Data.
const PREVIEW_CHUNKS: usize = 20;

#[derive(Deserialize)]
pub struct ChunkPage {
    #[serde(default)]
    from: usize,
    page: Option<u32>,
}

#[derive(Serialize)]
struct PreviewChunk {
    chunk_id: String,
    /// Urutan potongan di dokumen, mulai dari 0.
    n: usize,
    page: Option<u32>,
    text: String,
}

/// `GET /api/documents/{id}/chunks?from=0`: teks dokumen yang terindeks, berurutan, untuk
/// preview di Sumber Data. Aturannya sama dengan satu potongan: dokumen dicek dulu, lalu
/// label setiap potongan di index. `?page=N` mengambil potongan halaman PDF untuk fallback.
pub async fn document_chunks(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<i64>,
    axum::extract::Query(page): axum::extract::Query<ChunkPage>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Some(index) = state.docs.clone() else {
        return Err(unavailable());
    };
    let document = row(&state.db, &index, id)
        .await
        .map_err(ApiError::from)?
        .filter(|d| {
            !d.name_only()
                && d.readable_by(&user, &state.sources)
                && (page.page.is_none() || d.kind == "pdf")
        })
        .ok_or_else(not_found)?;
    let (from, who) = (page.from, user.clone());
    // Potongan yang labelnya di index menolak user: dijawab sama seperti dokumen yang
    // tidak ada, tanpa data dokumennya.
    let (total, hits) = tokio::task::spawn_blocking(move || match page.page {
        Some(number) => index.document_page_chunks(id, &who, number, PREVIEW_CHUNKS),
        None => index.document_chunks(id, &who, from, PREVIEW_CHUNKS),
    })
    .await
    .map_err(|e| ApiError::from(anyhow::Error::from(e)))?
    .map_err(ApiError::from)?
    .ok_or_else(not_found)?;
    let chunks: Vec<PreviewChunk> = hits
        .into_iter()
        .map(|(n, hit)| PreviewChunk {
            chunk_id: hit.chunk_id,
            n,
            page: hit.page,
            text: hit.text,
        })
        .collect();
    Ok(Json(serde_json::json!({
        "document": document,
        "total": total,
        "from": from,
        "limit": PREVIEW_CHUNKS,
        "chunks": chunks,
    })))
}

/// Dokumen `id` punya potongan yang labelnya di index menolak `user`.
async fn refused(index: &Arc<DocIndex>, id: i64, user: &AuthUser) -> Result<bool, ApiError> {
    let (index, who) = (index.clone(), user.clone());
    tokio::task::spawn_blocking(move || index.refuses(id, &who))
        .await
        .map_err(|e| ApiError::from(anyhow::Error::from(e)))?
        .map_err(ApiError::from)
}

/// `GET /api/documents/{id}/file`: file asli dari folder sumber. Aturannya sama dengan
/// preview: config server, baris `documents`, dan label di index.
pub async fn download(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<i64>,
    mut req: Request,
) -> Result<Response, ApiError> {
    let Some(index) = state.docs.clone() else {
        return Err(unavailable());
    };
    let document = row(&state.db, &index, id)
        .await
        .map_err(ApiError::from)?
        .filter(|d| d.allows_entry(&user, &state.sources))
        .ok_or_else(not_found)?;
    let who = user.clone();
    let indexed = tokio::task::spawn_blocking(move || index.has_readable_entry(id, &who))
        .await
        .map_err(|e| ApiError::from(anyhow::Error::from(e)))?
        .map_err(ApiError::from)?;
    if !indexed {
        return Err(not_found());
    }
    // File asli hanya diambil dari folder yang sama dengan saat dokumen diindeks, karena
    // hak aksesnya mengikuti folder itu. Kalau folder sumber di config berganti, tunggu
    // ingest berikutnya.
    let root =
        source_root(&state.sources, &document.source, &document.root).ok_or_else(not_found)?;
    let path = resolve(&root, &document.rel_path).ok_or_else(not_found)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| document.title.clone());
    if req.method() != Method::GET {
        req.headers_mut().remove(header::RANGE);
    }
    let conditional =
        req.headers().contains_key(header::RANGE) && req.headers().contains_key(header::IF_RANGE);
    let fallback = conditional.then(|| {
        let mut full = Request::new(Body::empty());
        *full.headers_mut() = req.headers().clone();
        full.headers_mut().remove(header::RANGE);
        full
    });
    let mut file = ServeFile::new(path);
    let mut response = file
        .try_call(req)
        .await
        .map_err(|_| not_found())?
        .into_response();
    // ServeFile belum mendukung If-Range. Pakai validator dari file yang benar-benar dibuka.
    if let Some(mut full) = fallback
        && matches!(
            response.status(),
            StatusCode::PARTIAL_CONTENT | StatusCode::RANGE_NOT_SATISFIABLE
        )
        && let Some(validator) = full.headers_mut().remove(header::IF_RANGE)
        && ![header::ETAG, header::LAST_MODIFIED]
            .iter()
            .any(|name| response.headers().get(name) == Some(&validator))
    {
        drop(response);
        response = file
            .try_call(full)
            .await
            .map_err(|_| not_found())?
            .into_response();
    }
    if response.status() == StatusCode::NOT_FOUND {
        return Err(not_found());
    }
    let success = response.status().is_success();
    let headers = response.headers_mut();
    if success {
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(mime_of(&document.kind)),
        );
        headers.insert(
            header::CONTENT_DISPOSITION,
            format!("inline; {}", crate::files::filename_parameters(&name))
                .parse()
                .expect("nama file sudah di-encode"),
        );
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    Ok(response)
}

pub(crate) fn source_root(
    sources: &[SourceConfig],
    source: &str,
    expected: &str,
) -> Option<std::path::PathBuf> {
    let root = sources
        .iter()
        .find(|item| item.name == source)?
        .path
        .canonicalize()
        .ok()?;
    (root.to_string_lossy() == expected).then_some(root)
}

/// Path file di dalam folder sumber. `None` kalau file hilang atau path-nya keluar dari
/// folder sumber (misalnya lewat `..` atau symlink).
pub(crate) fn resolve(root: &std::path::Path, rel: &str) -> Option<std::path::PathBuf> {
    let root = std::fs::canonicalize(root).ok()?;
    let path = std::fs::canonicalize(root.join(rel)).ok()?;
    (path.starts_with(&root) && path.is_file()).then_some(path)
}

fn mime_of(kind: &str) -> &'static str {
    match kind {
        "pdf" => "application/pdf",
        "xlsx" => crate::files::XLSX_MIME,
        "docx" => crate::docx::DOCX_MIME,
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "xls" => "application/vnd.ms-excel",
        "doc" => "application/msword",
        "ppt" => "application/vnd.ms-powerpoint",
        "txt" | "md" | "csv" | "tsv" => "text/plain; charset=utf-8",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_stays_inside_the_source_folder() {
        let base = std::env::temp_dir().join(format!("rag-hub-docs-{}", rand::random::<u32>()));
        let root = base.join("sumber");
        std::fs::create_dir_all(root.join("Sub")).unwrap();
        std::fs::write(root.join("Sub/a.pdf"), b"x").unwrap();
        std::fs::write(base.join("luar.pdf"), b"x").unwrap();
        assert!(resolve(&root, "Sub/a.pdf").is_some());
        assert!(resolve(&root, "../luar.pdf").is_none());
        assert!(resolve(&root, "Sub").is_none());
        assert!(resolve(&root, "hilang.pdf").is_none());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
