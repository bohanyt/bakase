//! `rag-hub-server ingest [--ulang]`: membaca file di folder sumber (`[[sources]]` di
//! config.toml), memotong isinya, lalu menyimpannya di index pencarian `data/index-v<N>`.
//! Setelah index lengkap, ingest melewati file yang sudah dibaca dan masih sama. Nama dan isi
//! dokumen tidak pernah dicetak.

use crate::config::{Config, SourceConfig};
use crate::db::{self, Db};
use crate::docdate;
use crate::llm::Llm;
use crate::search::{self, DocIndex, INDEX_TABLE, INDEX_VERSION, NewChunk};
use crate::usage::{self, Usage};
use anyhow::{Context, Result};
use futures::StreamExt;
use rusqlite::{TransactionBehavior, params};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tantivy::IndexWriter;

/// Panjang satu potongan, dalam karakter.
const CHUNK_CHARS: usize = 1_500;
/// Potongan sependek ini tidak ditutup walau baris berikutnya panjang, supaya judul
/// pendek tidak terpisah sendirian dari isinya.
const MIN_CHUNK_CHARS: usize = 200;
/// Batas potongan per file, supaya satu spreadsheet raksasa tidak memenuhi index.
const MAX_CHUNKS_PER_DOC: usize = 3_000;
pub(crate) const MAX_FILE_BYTES: u64 = 150 * 1024 * 1024;
/// Batas sel per spreadsheet saat dibaca.
const MAX_TABLE_CELLS: usize = 2_000_000;
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(180);
/// File yang dibaca bersamaan.
const PARALLEL: usize = 4;
/// Index di-commit tiap sekian file, supaya progres tidak hilang kalau proses berhenti.
const COMMIT_EVERY: usize = 50;
/// PDF dengan rata-rata huruf per halaman di bawah ini kemungkinan hasil scan.
pub(crate) const SCAN_CHARS_PER_PAGE: usize = 30;

/// File yang dikirim ke model paling besar sekian.
const MAX_VISION_BYTES: u64 = 20 * 1024 * 1024;
const VISION_TIMEOUT: Duration = Duration::from_secs(300);

/// Instruksi untuk model saat membaca gambar dan PDF hasil scan. Hasilnya hanya masuk
/// index, tidak pernah dicetak.
const VISION_INSTRUCTIONS: &str = "Kamu membaca dokumen kantor PT Example Corp untuk index pencarian. \
Salin semua teks yang terlihat apa adanya dalam bahasa aslinya, termasuk angka, kode, satuan, \
tanda tangan tertulis, dan isi tabel (tulis tabel sebagai tabel markdown). Untuk PDF, awali \
setiap halaman dengan satu baris `=== Halaman N ===`. Kalau ada foto, gambar teknik, atau \
diagram, tambahkan satu sampai tiga kalimat yang menjelaskan isinya secara faktual. Jangan \
menambah informasi yang tidak terlihat, jangan menerjemahkan, dan jangan memberi komentar.";

/// Status file di tabel `documents`.
pub mod status {
    pub const OK: &str = "ok";
    pub const SCAN: &str = "scan";
    pub const EMPTY: &str = "kosong";
    pub const FAILED: &str = "gagal";
    pub const IMAGE: &str = "gambar";
    pub const UNSUPPORTED: &str = "tidak_didukung";
    pub const TOO_BIG: &str = "terlalu_besar";
    /// Gambar atau PDF hasil scan yang teksnya dibaca model (`ingest --vision`).
    pub const VISION: &str = "vision";
    /// File yang tidak terbaca sendiri (scan, foto, kosong, gagal), dengan teks hasil baca
    /// ARCHIVE dari `.extracted-text/<id8>.md` di sebelahnya.
    pub const ARCHIVE: &str = "archive";
    /// Sedang dibaca; kalau proses berhenti, file ini dibaca ulang di ingest berikutnya.
    pub const PENDING: &str = "pending";
}

// Menandai baris v4 yang sudah punya entri nama dan lokasi. Baris v4 lama
// dengan status yang sama diproses lagi oleh ingest biasa.
pub const NAME_ONLY_VERSION: i64 = INDEX_VERSION + 2;

fn indexed_version(ext: &str, state: &str, has_chunks: bool) -> i64 {
    match state {
        status::OK | status::VISION | status::ARCHIVE => content_version(ext),
        status::SCAN if has_chunks => content_version(ext),
        _ => NAME_ONLY_VERSION,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Pdf,
    Office,
    Text,
    Image,
    Other,
}

fn classify(ext: &str) -> Kind {
    match ext {
        "pdf" => Kind::Pdf,
        "docx" | "doc" | "xlsx" | "xlsm" | "xls" | "pptx" | "ppt" | "odt" | "ods" | "odp"
        | "rtf" => Kind::Office,
        "txt" | "md" | "csv" | "tsv" => Kind::Text,
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tif" | "tiff" | "webp" | "heic" => Kind::Image,
        _ => Kind::Other,
    }
}

fn mime(ext: &str) -> &'static str {
    match ext {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "doc" => "application/msword",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "xlsm" => "application/vnd.ms-excel.sheet.macroEnabled.12",
        "xls" => "application/vnd.ms-excel",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "ppt" => "application/vnd.ms-powerpoint",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        "rtf" => "application/rtf",
        _ => "application/octet-stream",
    }
}

/// Spreadsheet dipotong per tabel dari grid selnya (lihat `sheet.rs`), bukan dari teks polos.
pub(crate) fn is_sheet(ext: &str) -> bool {
    matches!(ext, "xlsx" | "xlsm" | "xls" | "ods")
}

pub(crate) fn file_type(ext: &str) -> &'static str {
    match classify(ext) {
        Kind::Office if is_sheet(ext) => "Excel",
        Kind::Office if matches!(ext, "ppt" | "pptx" | "odp") => "PowerPoint",
        Kind::Office => "Word",
        Kind::Pdf => "PDF",
        Kind::Text => "teks",
        Kind::Image => "gambar/scan",
        Kind::Other => "",
    }
}

/// Tahun di path, ditambah tahun tanggal dokumen (`doc_year`) atau, kalau tidak ada, tahun
/// file diubah (WIB). Untuk file dari ARCHIVE, tanggal diubah adalah tanggal file ditarik.
pub(crate) fn document_years(
    path: &Path,
    doc_year: Option<i32>,
    modified: Option<SystemTime>,
) -> Vec<i64> {
    let path = path.to_string_lossy();
    let mut years: Vec<i64> = path
        .split(|c: char| !c.is_ascii_digit())
        .filter(|digits| digits.len() == 4 && digits.starts_with("20"))
        .filter_map(|digits| digits.parse().ok())
        .collect();
    if let Some(year) = doc_year {
        years.push(i64::from(year));
    } else if let Some(modified) = modified {
        let nanos = match modified.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_nanos() as i128,
            Err(e) => -(e.duration().as_nanos() as i128),
        };
        if let Ok(date) = time::OffsetDateTime::from_unix_timestamp_nanos(nanos) {
            years.push(i64::from(
                date.to_offset(time::UtcOffset::from_hms(7, 0, 0).expect("offset WIB valid"))
                    .year(),
            ));
        }
    }
    years.sort_unstable();
    years.dedup();
    years
}

/// Versi isi potongan yang tersimpan di `documents.version`. Spreadsheet satu versi di atas
/// yang lain sejak dipotong per tabel. Pembangunan index v4 memproses semua file;
/// potongan model yang filenya masih sama disalin dari index sebelumnya.
fn content_version(ext: &str) -> i64 {
    INDEX_VERSION + i64::from(is_sheet(ext))
}

/// Jenis file yang nomor halamannya berarti (halaman PDF, slide presentasi).
fn paged(ext: &str) -> bool {
    matches!(ext, "pdf" | "pptx" | "ppt" | "odp")
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn title_of(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().trim().to_string())
        .unwrap_or_default();
    if stem.is_empty() {
        path.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        stem
    }
}

struct Extracted {
    status: &'static str,
    pages: u32,
    /// Potongan beserta nomor halamannya.
    chunks: Vec<(Option<u32>, String)>,
    truncated: bool,
    /// Tanggal diubah dan dibuat dari metadata file.
    modified: Option<time::Date>,
    created: Option<time::Date>,
}

impl Extracted {
    fn skipped(status: &'static str) -> Self {
        Self {
            status,
            pages: 0,
            chunks: Vec::new(),
            truncated: false,
            modified: None,
            created: None,
        }
    }

    /// Teks dari ARCHIVE atau model menggantikan isi, tapi tanggal dari metadata file tetap.
    fn dated_like(self, file: &Extracted) -> Self {
        Self {
            modified: file.modified,
            created: file.created,
            ..self
        }
    }
}

async fn extract_file(path: PathBuf, ext: String, size: u64) -> Extracted {
    let kind = classify(&ext);
    match kind {
        Kind::Image => return Extracted::skipped(status::IMAGE),
        Kind::Other => return Extracted::skipped(status::UNSUPPORTED),
        _ if size > MAX_FILE_BYTES => return Extracted::skipped(status::TOO_BIG),
        _ => {}
    }
    let Ok(bytes) = tokio::fs::read(&path).await else {
        return Extracted::skipped(status::FAILED);
    };
    let today = docdate::today();
    let (pages, sections, cut, dates) = if kind == Kind::Text {
        (
            0,
            vec![(None, String::from_utf8_lossy(&bytes).into_owned())],
            false,
            (None, None),
        )
    } else {
        // Xberg tidak membaca metadata file Office lama.
        let legacy = matches!(ext.as_str(), "xls" | "doc" | "ppt")
            .then(|| docdate::legacy_office(&bytes, today));
        let Some(read) = extract_with_xberg(bytes, &ext, true).await else {
            return Extracted::skipped(status::FAILED);
        };
        let meta = |v: &Option<String>| v.as_deref().and_then(|v| docdate::from_metadata(v, today));
        let dates = legacy.unwrap_or_else(|| (meta(&read.modified), meta(&read.created)));
        (read.pages, read.sections, read.cut, dates)
    };
    let mut extracted = build(kind, pages, sections);
    extracted.truncated |= cut;
    (extracted.modified, extracted.created) = dates;
    extracted
}

/// Konfigurasi Xberg. `embedded`: objek Office yang disisipkan di dalam file (Excel di Word,
/// dan seterusnya) ikut dibaca, seperti untuk dokumen kerja di ingest. Lampiran user memakai
/// `false`: objek sisipan dilewati tanpa dibaca, karena isinya tidak ikut diperiksa sebelumnya.
pub(crate) fn xberg_config(embedded: bool) -> xberg::ExtractionConfig {
    xberg::ExtractionConfig {
        // Cache bawaan Xberg menyimpan hasil ekstraksi di %LOCALAPPDATA%\xberg, di luar
        // folder data. Isi dokumen kerja hanya boleh ada di folder index di bawah data.
        use_cache: false,
        max_archive_depth: if embedded {
            xberg::ExtractionConfig::default().max_archive_depth
        } else {
            0
        },
        pages: Some(xberg::PageConfig {
            extract_pages: true,
            ..Default::default()
        }),
        // Batas bawaan 100 ribu sel menolak spreadsheet operasional yang wajar. Isinya
        // tetap dibatasi MAX_CHUNKS_PER_DOC saat dipotong.
        security_limits: Some(xberg::SecurityLimits {
            max_table_cells: MAX_TABLE_CELLS,
            // Lampiran user: PDF atau presentasi dengan ribuan halaman ditolak sebelum
            // halamannya dibaca.
            max_pages: (!embedded).then_some(MAX_ATTACHMENT_PAGES),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Halaman (atau slide) paling banyak untuk satu lampiran user.
const MAX_ATTACHMENT_PAGES: usize = 500;

/// Hasil baca Xberg: teks per halaman (PDF dan presentasi) atau utuh, apakah teksnya sudah
/// dipotong di batas potongan per dokumen, dan tanggal dari metadata file (ISO 8601).
pub(crate) struct XbergText {
    pub pages: u32,
    pub sections: Vec<(Option<u32>, String)>,
    pub cut: bool,
    pub modified: Option<String>,
    pub created: Option<String>,
}

/// `None` kalau gagal, panik di pembaca, atau terlalu lama. `embedded`: lihat `xberg_config`.
pub(crate) async fn extract_with_xberg(
    bytes: Vec<u8>,
    ext: &str,
    embedded: bool,
) -> Option<XbergText> {
    let input = xberg::ExtractInput::from_bytes(bytes, mime(ext), None);
    let mut task = tokio::spawn(async move {
        let config = xberg_config(embedded);
        xberg::extract(input, &config).await
    });
    let doc = match tokio::time::timeout(EXTRACT_TIMEOUT, &mut task).await {
        Ok(Ok(Ok(result))) => result.results.into_iter().next()?,
        Ok(_) => return None,
        Err(_) => {
            task.abort();
            return None;
        }
    };
    let (modified, created) = (
        doc.metadata.modified_at.clone(),
        doc.metadata.created_at.clone(),
    );
    let read = |pages, sections, cut| XbergText {
        pages,
        sections,
        cut,
        modified,
        created,
    };
    if is_sheet(ext) && !doc.tables.is_empty() {
        let names = match &doc.metadata.format {
            Some(xberg::FormatMetadata::Excel(m)) => m.sheet_names.clone().unwrap_or_default(),
            _ => Vec::new(),
        };
        // Xberg mencatat sheet tersembunyi sebagai "A, B" di metadata.
        let hidden: HashSet<&str> = doc
            .metadata
            .additional
            .get("hidden_sheets")
            .and_then(|v| v.as_str())
            .map(|s| s.split(", ").collect())
            .unwrap_or_default();
        // `page_number` tabel spreadsheet adalah urutan sheet (mulai 1), termasuk sheet kosong.
        let sheets: Vec<crate::sheet::Sheet> = doc
            .tables
            .iter()
            .map(|t| {
                let i = t.page_number as usize;
                let name = names
                    .get(i.wrapping_sub(1))
                    .cloned()
                    .unwrap_or_else(|| format!("Sheet {i}"));
                crate::sheet::Sheet {
                    hidden: hidden.contains(name.as_str()),
                    name,
                    cells: &t.cells,
                }
            })
            .collect();
        let (text, cut) = crate::sheet::to_text(&sheets, MAX_CHUNKS_PER_DOC * CHUNK_CHARS);
        return Some(read(0, vec![(None, text)], cut));
    }
    let pages = doc.pages.unwrap_or_default();
    if paged(ext) && !pages.is_empty() {
        let count = u32::try_from(pages.len()).unwrap_or(u32::MAX);
        let sections = pages
            .into_iter()
            .map(|p| (Some(p.page_number), p.content))
            .collect();
        Some(read(count, sections, false))
    } else {
        Some(read(0, vec![(None, doc.content)], false))
    }
}

fn build(kind: Kind, pages: u32, sections: Vec<(Option<u32>, String)>) -> Extracted {
    let mut chunks = Vec::new();
    let mut letters = 0usize;
    let mut truncated = false;
    'sections: for (page, text) in sections {
        letters += text.chars().filter(|c| c.is_alphanumeric()).count();
        for chunk in chunk_text(&text) {
            if chunks.len() == MAX_CHUNKS_PER_DOC {
                truncated = true;
                break 'sections;
            }
            chunks.push((page, chunk));
        }
    }
    let status = if kind == Kind::Pdf && letters < SCAN_CHARS_PER_PAGE * pages.max(1) as usize {
        status::SCAN
    } else if chunks.is_empty() {
        status::EMPTY
    } else {
        status::OK
    };
    Extracted {
        status,
        pages,
        chunks,
        truncated,
        modified: None,
        created: None,
    }
}

/// Spasi berturut-turut jadi satu, karakter kontrol dibuang.
fn clean_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut space = false;
    for c in line.chars() {
        if c.is_whitespace() || c.is_control() || c == '\u{feff}' {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(c);
    }
    out
}

/// Garis pemisah tabel markdown: setiap sel hanya `-` dan `:`, dan minimal satu sel berisi
/// tiga `-`. Baris data berisi nilai kosong seperti `| | - | - |` atau `| - | - |` bukan pemisah.
fn is_table_separator(line: &str) -> bool {
    let cells: Vec<&str> = line
        .trim()
        .trim_start_matches('|')
        .trim_end_matches('|')
        .split('|')
        .map(str::trim)
        .collect();
    let dashes = |c: &str| c.chars().filter(|&ch| ch == '-').count();
    line.starts_with('|')
        && cells
            .iter()
            .all(|c| dashes(c) >= 1 && c.chars().all(|ch| matches!(ch, '-' | ':')))
        && cells.iter().any(|c| dashes(c) >= 3)
}

/// Baris tabel markdown yang berisi nilai apa pun (`| S1 | + |`, `| - | ✓ |`), bukan garis
/// pemisahnya dan bukan baris kosong (`| | |`).
fn is_table_row(line: &str) -> bool {
    let line = line.trim();
    line.len() > 1
        && line.starts_with('|')
        && line.ends_with('|')
        && !is_table_separator(line)
        && line[1..line.len() - 1]
            .split('|')
            .any(|c| !c.trim().is_empty())
}

fn has_letters(text: &str, min: usize) -> bool {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .take(min)
        .count()
        >= min
}

/// Memecah baris yang lebih panjang dari `max` karakter, sebisa mungkin di spasi.
fn split_long(line: &str, max: usize) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut rest = line;
    while let Some((cut, _)) = rest.char_indices().nth(max) {
        let at = match rest[..cut].rfind(' ') {
            Some(i) if i >= cut / 2 => i,
            _ => cut,
        };
        parts.push(rest[..at].trim_end());
        rest = rest[at..].trim_start();
    }
    if !rest.is_empty() {
        parts.push(rest);
    }
    parts
}

/// Memotong teks jadi potongan sekitar `CHUNK_CHARS` karakter di batas baris. Judul
/// bagian terakhir (baris `#`, misalnya nama sheet) dan kepala tabel markdown diulang
/// di awal potongan berikutnya, supaya setiap potongan bisa dipahami sendiri.
fn chunk_text(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_chars = 0usize;
    let mut has_body = false;
    let mut blank = false;
    let mut heading: Option<String> = None;
    let mut table_head: Option<String> = None;
    let mut previous: Option<String> = None;

    for raw in text.lines() {
        let line = clean_line(raw);
        let separator = is_table_separator(&line);
        // Baris tabel markdown selalu disimpan, juga yang hanya berisi tanda (`| \--- |`,
        // `| + | - |`), supaya tidak ada baris data yang hilang.
        let table_row = line.len() > 2 && line.starts_with('|') && line.ends_with('|');
        if !separator && !table_row && !has_letters(&line, 1) {
            blank = has_body;
            previous = None;
            continue;
        }
        if line.starts_with('#') {
            heading = Some(line.chars().take(200).collect());
            table_head = None;
        } else if separator {
            table_head = previous
                .as_ref()
                .filter(|p| p.starts_with('|') && p.chars().count() <= 600)
                .map(|p| format!("{p}\n{line}"));
        } else if !line.starts_with('|') {
            table_head = None;
        }
        for piece in split_long(&line, CHUNK_CHARS) {
            let len = piece.chars().count();
            // Garis pemisah tabel tidak pernah membuka potongan baru, supaya nama kolom di
            // baris sebelumnya tidak terpisah dari tabelnya.
            if has_body
                && !separator
                && current_chars >= MIN_CHUNK_CHARS
                && current_chars + len + 1 > CHUNK_CHARS
            {
                chunks.push(std::mem::take(&mut current));
                current_chars = 0;
                blank = false;
                let in_table = piece.starts_with('|') && !separator;
                let context = [
                    heading.as_deref().filter(|h| *h != piece),
                    table_head.as_deref().filter(|_| in_table),
                ];
                for c in context.into_iter().flatten() {
                    current.push_str(c);
                    current.push('\n');
                    current_chars += c.chars().count() + 1;
                }
            }
            if blank {
                current.push('\n');
                current_chars += 1;
                blank = false;
            }
            current.push_str(piece);
            current.push('\n');
            current_chars += len + 1;
            has_body = true;
        }
        previous = Some(line);
    }
    if has_body {
        chunks.push(current);
    }
    chunks
        .into_iter()
        .map(|c| c.trim_end().to_string())
        // Potongan pendek tanpa isi berarti dibuang, kecuali yang membawa baris tabel
        // spreadsheet (`| S1 | + |`): setiap baris data ikut terindeks.
        .filter(|c| has_letters(c, 10) || c.lines().any(is_table_row))
        .collect()
}

/// Semua file di bawah `dir`. File tersembunyi, file kunci Office (`~$...`), dan
/// symlink dan berkas metadata tanggal dilewati. Folder `skip` (path kanonik folder data server) juga dilewati,
/// supaya lampiran chat, database, dan index tidak pernah ikut terindeks walau folder
/// sumber ditulis keliru sehingga memuat folder data.
pub(crate) fn walk(dir: &Path, skip: Option<&Path>, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name.starts_with("~$") {
            continue;
        }
        if kind.is_dir() {
            let path = entry.path();
            let skipped = skip.is_some_and(|s| std::fs::canonicalize(&path).is_ok_and(|p| p == s));
            if !skipped {
                walk(&path, skip, out)?;
            }
        } else if kind.is_file() && !name.eq_ignore_ascii_case(ARCHIVE_UPLOAD_FILE) {
            out.push(entry.path());
        }
    }
    Ok(())
}

/// Path relatif dengan `/`, atau `None` kalau nama filenya bukan UTF-8 yang valid.
pub(crate) fn rel_path(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
    Some(parts?.join("/"))
}

pub(crate) fn file_stamp(meta: &fs::Metadata) -> (i64, i64) {
    let size = i64::try_from(meta.len()).unwrap_or(i64::MAX);
    let mtime = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    (size, mtime)
}

struct Existing {
    id: i64,
    size: i64,
    mtime: i64,
    visibility: String,
    root: String,
    version: i64,
    status: String,
    chunks: i64,
    pages: u32,
    /// Sidik teks ARCHIVE saat terakhir diindeks; 0 kalau tidak ada.
    sidecar: i64,
    /// Versi aturan tanggal dokumen saat terakhir dibaca; 0 kalau belum pernah.
    date_version: i64,
    /// `indexed_at` saat tanggal dokumen ditulis. Berbeda dari `indexed_at` kalau binary lama
    /// (tanpa tanggal dokumen) sempat menulis ulang baris ini, apa pun sebabnya.
    date_indexed_at: Option<i64>,
    indexed_at: i64,
    /// Baris sudah punya tanggal dokumen.
    dated: bool,
}

async fn load_existing(
    db: &Db,
    table: &'static str,
) -> Result<HashMap<(String, String), Existing>> {
    db.call(move |c| {
        // Tabel v3 (`documents`) tidak punya kolom sidecar dan date_version.
        let (sidecar, dated) = if table == INDEX_TABLE {
            (
                "sidecar",
                "date_version, date_indexed_at, indexed_at, doc_date IS NOT NULL",
            )
        } else {
            ("0", "0, NULL, 0, 0")
        };
        let mut stmt = c.prepare(&format!(
            "SELECT id, source, rel_path, size, mtime, visibility, root, version, status, chunks, pages, {sidecar}, {dated} FROM {table}",
        ))?;
        let rows = stmt.query_map([], |r| {
            Ok((
                (r.get::<_, String>(1)?, r.get::<_, String>(2)?),
                Existing {
                    id: r.get(0)?,
                    size: r.get(3)?,
                    mtime: r.get(4)?,
                    visibility: r.get(5)?,
                    root: r.get(6)?,
                    version: r.get(7)?,
                    status: r.get(8)?,
                    chunks: r.get(9)?,
                    pages: r.get(10)?,
                    sidecar: r.get(11)?,
                    date_version: r.get(12)?,
                    date_indexed_at: r.get(13)?,
                    indexed_at: r.get(14)?,
                    dated: r.get(15)?,
                },
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<HashMap<_, _>>>()?)
    })
    .await
}

struct Job {
    path: PathBuf,
    rel: String,
    size: i64,
    mtime: i64,
    modified: Option<SystemTime>,
    id: Option<i64>,
    carry: Option<(i64, i64, u32)>,
    /// Teks hasil baca ARCHIVE untuk file ini, kalau ada.
    sidecar: Option<PathBuf>,
    sidecar_print: i64,
    /// Tanggal upload ke ARCHIVE, cadangan terakhir untuk tanggal dokumen.
    uploaded: Option<time::Date>,
}

struct RowUpdate {
    id: i64,
    title: String,
    kind: String,
    pages: u32,
    chunks: usize,
    status: &'static str,
    size: i64,
    mtime: i64,
    version: i64,
    sidecar: i64,
    /// Tanggal dokumen `YYYY-MM-DD` dan asalnya, kalau diketahui.
    doc_date: Option<(String, &'static str)>,
}

/// Hak akses dan folder asal satu sumber, seperti yang dipakai ingest ini. Server hanya
/// menyajikan dokumen kalau config-nya sendiri juga mengizinkan, dan hanya mengunduh dari
/// folder yang sama dengan `root`.
struct Label {
    visibility: String,
    /// Path kanonik folder sumber.
    root: String,
}

async fn insert_pending(
    db: &Db,
    source: &str,
    job: &Job,
    previous_id: Option<i64>,
    title: &str,
    kind: &str,
    label: &Label,
) -> Result<i64> {
    let values = (
        source.to_string(),
        job.rel.clone(),
        title.to_string(),
        kind.to_string(),
        label.visibility.clone(),
        label.root.clone(),
    );
    db.call(move |c| {
        let (source, rel, title, kind, visibility, root) = values;
        // ID baru dipesan dari urutan yang juga dipakai ingest v3 saat rollback.
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = match previous_id {
            Some(id) => id,
            None => {
                tx.execute("UPDATE sqlite_sequence SET seq = seq + 1 WHERE name = 'documents'", [])?;
                tx.execute("INSERT INTO sqlite_sequence (name, seq) SELECT 'documents', 1
                            WHERE NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'documents')", [])?;
                tx.query_row("SELECT seq FROM sqlite_sequence WHERE name = 'documents'", [], |r| r.get(0))?
            }
        };
        tx.execute(
            &format!("INSERT INTO {INDEX_TABLE} (id, source, rel_path, title, kind, status, size, mtime, visibility, root, version, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, -1, ?7, ?8, 0, ?9)"),
            params![id, source, rel, title, kind, status::PENDING, visibility, root, db::now()],
        )?;
        tx.commit()?;
        Ok(id)
    })
    .await
}

/// Ditulis setelah index di-commit, supaya baris tidak pernah mengaku terindeks
/// sebelum potongannya benar-benar tersimpan.
async fn save_rows(db: &Db, rows: Vec<RowUpdate>, label: &Label) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let (visibility, root) = (label.visibility.clone(), label.root.clone());
    db.call(move |c| {
        let tx = c.transaction()?;
        {
            let mut stmt = tx.prepare(
                &format!("UPDATE {INDEX_TABLE} SET title = ?2, kind = ?3, pages = ?4, chunks = ?5, status = ?6,
                 size = ?7, mtime = ?8, visibility = ?9, version = ?10, indexed_at = ?11, root = ?12,
                 sidecar = ?13, doc_date = ?14, doc_date_from = ?15, date_version = ?16,
                 date_indexed_at = ?11
                 WHERE id = ?1"),
            )?;
            // Milidetik, sedangkan binary tanpa tanggal dokumen menulis detik: penulisan ulang
            // olehnya selalu membuat `indexed_at` berbeda dari `date_indexed_at`, juga di detik
            // yang sama. Kolom ini tidak dibaca di tempat lain.
            let now = db::now_millis();
            for r in &rows {
                stmt.execute(params![
                    r.id,
                    r.title,
                    r.kind,
                    r.pages,
                    r.chunks as i64,
                    r.status,
                    r.size,
                    r.mtime,
                    visibility,
                    r.version,
                    now,
                    root,
                    r.sidecar,
                    r.doc_date.as_ref().map(|(d, _)| d),
                    r.doc_date.as_ref().map(|(_, from)| *from),
                    DATE_VERSION
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    })
    .await
}

#[derive(Default)]
struct Counts {
    files: usize,
    read: usize,
    unchanged: usize,
    chunks: usize,
    truncated: usize,
    by_status: BTreeMap<String, usize>,
}

impl Counts {
    fn status(&mut self, status: &str) {
        *self.by_status.entry(status.to_string()).or_default() += 1;
    }

    fn print(&self, name: &str) {
        println!(
            "{name}: {} file, {} dibaca, {} tidak berubah",
            thousands(self.files),
            thousands(self.read),
            thousands(self.unchanged)
        );
        let labels = [
            (status::OK, "terindeks"),
            (status::SCAN, "kemungkinan scan"),
            (status::EMPTY, "tanpa teks"),
            (status::FAILED, "gagal dibaca"),
            (status::IMAGE, "gambar (belum dibaca)"),
            (status::UNSUPPORTED, "jenis tidak didukung"),
            (status::TOO_BIG, "terlalu besar"),
            (status::VISION, "dibaca model"),
            (status::ARCHIVE, "teks dari ARCHIVE"),
        ];
        let parts: Vec<String> = labels
            .iter()
            .filter_map(|(key, label)| {
                let n = self.by_status.get(*key).copied().unwrap_or(0);
                (n > 0).then(|| format!("{label} {}", thousands(n)))
            })
            .collect();
        println!(
            "  {} · {} potongan{}",
            if parts.is_empty() {
                "kosong".to_string()
            } else {
                parts.join(" · ")
            },
            thousands(self.chunks),
            if self.truncated > 0 {
                format!(" · {} file terpotong (terlalu panjang)", self.truncated)
            } else {
                String::new()
            }
        );
    }
}

fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// Ringkasan satu kali ingest.
#[derive(Debug, Default, PartialEq)]
pub struct Summary {
    /// File yang diproses, termasuk backfill, pembacaan ulang, dan percobaan yang gagal.
    /// Ingest biasa tanpa perubahan menghasilkan nol setelah backfill selesai.
    pub read: usize,
    /// Dokumen yang dihapus dari index karena filenya sudah tidak ada.
    pub removed: usize,
}

/// `vision`: kalau ada, gambar dan PDF hasil scan dikirim ke model untuk dibaca teksnya.
pub async fn run(cfg: &Config, rebuild: bool, vision: Option<&Llm>) -> Result<Summary> {
    run_with_embed(cfg, rebuild, vision, crate::embed::after_ingest).await
}

async fn run_with_embed(
    cfg: &Config,
    rebuild: bool,
    vision: Option<&Llm>,
    embed: impl FnOnce(&Path, &crate::config::MaknaConfig, bool) -> Result<()> + Send + 'static,
) -> Result<Summary> {
    let summary = ingest_documents(cfg, rebuild, vision).await?;
    if let Some(makna) = cfg.makna.clone() {
        println!(
            "Ingest pertama setelah mengaktifkan [makna] dan setiap ingest --ulang membuat vektor semua potongan di CPU. Untuk pembuatan awal, jalankan embed --gpu terlebih dahulu agar lebih cepat."
        );
        // Kunci tulis ingest sudah lepas. Embed mengambil kunci versi bersama sendiri.
        let data = cfg.data_dir.clone();
        match tokio::task::spawn_blocking(move || embed(&data, &makna, rebuild)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => println!(
                "Embedding gagal: {}",
                crate::embed::failure_category(&error)
            ),
            Err(_) => println!("Embedding gagal: lainnya"),
        }
    }
    Ok(summary)
}

async fn ingest_documents(cfg: &Config, rebuild: bool, vision: Option<&Llm>) -> Result<Summary> {
    anyhow::ensure!(
        !cfg.sources.is_empty(),
        "Belum ada [[sources]] di config.toml; contohnya ada di config.example.toml."
    );

    let _lock = ingest_lock(&cfg.data_dir)?;
    let index_dir = search::version_dir(&cfg.data_dir, INDEX_VERSION);
    let marker = index_dir.join(search::COMPLETE);
    // Pembaca folder versi lain tetap berjalan. Pembaca versi ini harus pindah slot dulu.
    let _rebuild_lock = if rebuild {
        let lock = search::version_lock(&cfg.data_dir)?;
        take_exclusive(
            &lock,
            "Index versi ini sedang dipakai oleh layanan, embed, atau eval. Tunggu embed dan eval selesai atau pindahkan layanan ke slot dengan versi index lain sebelum ingest --ulang.",
        )?;
        match fs::remove_file(&marker) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("gagal menghapus penanda index lengkap"),
        }
        Some(lock)
    } else {
        None
    };
    let building = !marker.is_file();
    let db = Db::open(&cfg.data_dir)?;
    if rebuild || !index_dir.join("meta.json").try_exists()? {
        db.call(|c| {
            Ok(c.execute(
                &format!("UPDATE {INDEX_TABLE} SET status = ?1"),
                [status::PENDING],
            )?)
        })
        .await?;
    }
    if rebuild && index_dir.try_exists()? {
        fs::remove_dir_all(&index_dir).context(
            "folder index versi ini gagal dihapus; periksa izin dan jalankan ingest --ulang lagi",
        )?;
    }
    let previous = if building {
        DocIndex::open_previous(&cfg.data_dir)?
    } else {
        None
    };
    // Baris lama juga menyediakan ID untuk file yang belum pernah diproses v4.
    let legacy = load_existing(&db, "documents").await?;
    let index = DocIndex::open_current(&index_dir, building)?;
    let mut writer = index.writer()?;

    let started = Instant::now();
    let mut summary = Summary::default();
    // Folder data (database, index, lampiran chat) tidak pernah diindeks.
    let data_dir = std::fs::canonicalize(&cfg.data_dir).ok();
    let existing = load_existing(&db, index.table()).await?;
    let mut seen = HashSet::new();
    let mut scanned: HashSet<&str> = HashSet::new();
    for source in &cfg.sources {
        if !source.path.is_dir() {
            let detail = if building {
                "Index baru belum bisa ditandai lengkap."
            } else {
                "Dokumen lamanya tetap di index."
            };
            println!(
                "{}: folder tidak ditemukan, dilewati. {detail}",
                source.name
            );
            continue;
        }
        let root = fs::canonicalize(&source.path)
            .with_context(|| format!("folder sumber {} tidak bisa dibuka", source.name))?;
        if data_dir.as_ref().is_some_and(|data| root.starts_with(data)) {
            println!(
                "{}: folder sumber ada di dalam folder data server, dilewati.",
                source.name
            );
            continue;
        }
        let counts = ingest_source(
            &db,
            &index,
            &mut writer,
            source,
            &existing,
            &legacy,
            root,
            building,
            previous.as_ref(),
            &mut seen,
            vision,
            data_dir.as_deref(),
        )
        .await?;
        scanned.insert(source.name.as_str());
        counts.print(&source.name);
        summary.read += counts.read;
    }

    let configured: HashSet<&str> = cfg.sources.iter().map(|s| s.name.as_str()).collect();
    let was_removed = |key: &(String, String)| {
        !seen.contains(key)
            && (scanned.contains(key.0.as_str()) || !configured.contains(key.0.as_str()))
    };
    hide_legacy_rows(
        &db,
        legacy
            .keys()
            .filter(|key| was_removed(key))
            .cloned()
            .collect(),
    )
    .await?;
    let removed: Vec<i64> = existing
        .iter()
        .filter(|(key, _)| was_removed(key))
        .map(|(_, row)| row.id)
        .collect();
    for id in &removed {
        index.delete_document(&writer, *id);
    }
    writer.commit()?;
    if !removed.is_empty() {
        let ids = removed.clone();
        db.call(move |c| {
            let tx = c.transaction()?;
            for id in ids {
                tx.execute(&format!("DELETE FROM {INDEX_TABLE} WHERE id = ?1"), [id])?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
    }
    writer.wait_merging_threads()?;
    if building {
        anyhow::ensure!(
            configured.is_subset(&scanned),
            "Index belum lengkap. Periksa folder sumber yang dilewati, lalu jalankan ingest lagi."
        );
        File::create_new(marker).context("gagal menandai index lengkap")?;
    }
    println!(
        "Dihapus dari index (file sudah tidak ada): {}",
        thousands(removed.len())
    );
    println!(
        "Selesai dalam {:.0} detik. Index: {}",
        started.elapsed().as_secs_f64(),
        index_dir.display()
    );
    summary.removed = removed.len();
    Ok(summary)
}

fn ingest_lock(data: &Path) -> Result<File> {
    fs::create_dir_all(data)?;
    let file = open_ingest_lock(data)?;
    take_exclusive(
        &file,
        "Ingest lain atau eval berbasis vektor sedang berjalan. Tunggu sampai selesai lalu coba lagi.",
    )?;
    Ok(file)
}

pub(crate) fn open_ingest_lock(data: &Path) -> Result<File> {
    Ok(File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(data.join("ingest.lock"))?)
}

fn take_exclusive(file: &File, busy: &str) -> Result<()> {
    match file.try_lock() {
        Ok(()) => Ok(()),
        Err(std::fs::TryLockError::WouldBlock) => anyhow::bail!("{busy}"),
        Err(std::fs::TryLockError::Error(e)) => Err(e).context("gagal mengambil kunci eksklusif"),
    }
}

async fn hide_legacy_rows(db: &Db, keys: Vec<(String, String)>) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }
    db.call(move |c| {
        let tx = c.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "UPDATE documents SET status = 'pending' WHERE source = ?1 AND rel_path = ?2 AND status != 'pending'",
            )?;
            for (source, rel) in keys {
                stmt.execute(params![source, rel])?;
            }
        }
        tx.commit()?;
        Ok(())
    })
    .await
}

#[allow(clippy::too_many_arguments)]
async fn ingest_source(
    db: &Db,
    index: &DocIndex,
    writer: &mut IndexWriter,
    source: &SourceConfig,
    existing: &HashMap<(String, String), Existing>,
    legacy: &HashMap<(String, String), Existing>,
    root: PathBuf,
    building: bool,
    previous: Option<&DocIndex>,
    seen: &mut HashSet<(String, String)>,
    vision: Option<&Llm>,
    data_dir: Option<&Path>,
) -> Result<Counts> {
    let label = Label {
        visibility: serde_json::to_string(&source.visibility)?,
        root: root.to_string_lossy().into_owned(),
    };
    let acl = search::acl_labels(&source.visibility);
    let mut counts = Counts::default();
    let root = source.path.clone();
    let skip = data_dir.map(Path::to_path_buf);
    let files = tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        walk(&root, skip.as_deref(), &mut out)?;
        out.sort();
        Ok::<_, std::io::Error>(out)
    })
    .await?
    .with_context(|| format!("folder sumber {} gagal dipindai", source.name))?;
    let uploads = archive_upload_dates(&source.path);

    let mut jobs = Vec::new();
    let mut stale = Vec::new();
    for path in files {
        counts.files += 1;
        let Some(rel) = rel_path(&source.path, &path) else {
            counts.status(status::UNSUPPORTED);
            continue;
        };
        let meta = std::fs::metadata(&path).ok();
        let (size, mtime) = meta.as_ref().map_or((-1, 0), file_stamp);
        let modified = meta.and_then(|m| m.modified().ok());
        let key = (source.name.clone(), rel.clone());
        let old = existing.get(&key);
        seen.insert(key.clone());
        let (sidecar, sidecar_print) = archive_sidecar(&path);
        let uploaded = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(archive_id8)
            .and_then(|id8| uploads.get(id8).copied());
        if let Some(row) = old
            && row.size == size
            && row.mtime == mtime
            // Teks ARCHIVE hanya berpengaruh untuk file yang tidak terbaca sendiri.
            && (!uses_sidecar(&row.status) || row.sidecar == stored_print(&row.status, sidecar_print))
            && row.visibility == label.visibility
            && row.root == label.root
            && row.version == indexed_version(&extension(&path), &row.status, row.chunks > 0)
            && row.status != status::PENDING
            // Tanggal dokumen dari aturan lama, atau baris yang sempat ditulis ulang tanpa tanggal:
            // baca ulang sekali. Hasil baca model tidak dibaca ulang hanya untuk tanggal, karena ingest tanpa
            // --vision akan menggantinya dengan entri nama saja.
            && (row.status == status::VISION
                || (row.date_version == DATE_VERSION && row.date_indexed_at == Some(row.indexed_at)))
            // Tanggal upload ARCHIVE baru tersedia untuk file yang belum punya tanggal dokumen: baca
            // ulang sekali, supaya tahun di index ikut benar.
            && (row.status == status::VISION || row.dated || uploaded.is_none())
            // Ingest rutin dengan --vision mencoba lagi gambar dan scan.
            && !(!building && vision.is_some() && matches!(row.status.as_str(), status::SCAN | status::IMAGE))
        {
            counts.unchanged += 1;
            counts.chunks += usize::try_from(row.chunks).unwrap_or(0);
            counts.status(&row.status);
            continue;
        }
        if legacy
            .get(&key)
            .is_some_and(|row| row.size != size || row.mtime != mtime || row.root != label.root)
        {
            stale.push(key.clone());
        }
        jobs.push(Job {
            path,
            rel,
            size,
            mtime,
            modified,
            id: old.map(|r| r.id),
            carry: legacy
                .get(&key)
                .filter(|row| {
                    row.status == status::VISION
                        && modified.is_some()
                        && row.size == size
                        && row.mtime == mtime
                        && row.root == label.root
                })
                .map(|row| (row.id, row.chunks, row.pages)),
            sidecar,
            sidecar_print,
            uploaded,
        });
    }

    hide_legacy_rows(db, stale).await?;
    let total = jobs.len();
    let mut results = futures::stream::iter(jobs.into_iter().map(|job| async move {
        let mut job = job;
        let ext = extension(&job.path);
        let size = u64::try_from(job.size).unwrap_or(u64::MAX);
        let carried = match (previous, job.carry) {
            (Some(previous), Some((id, chunks, pages))) => {
                carry_vision(previous, id, chunks, pages)?
            }
            _ => None,
        };
        let mut extracted = match carried {
            Some(extracted) => extracted,
            None if job.size < 0 => Extracted::skipped(status::FAILED),
            None => extract_file(job.path.clone(), ext.clone(), size).await,
        };
        // Teks ARCHIVE dipakai sebelum model, supaya file yang sudah dibaca ARCHIVE tidak dikirim ke model.
        if uses_sidecar(extracted.status)
            && let Some(sidecar) = job.sidecar.clone()
        {
            match read_sidecar(&sidecar, extracted.pages).await {
                SidecarRead::Text(read) => extracted = read.dated_like(&extracted),
                SidecarRead::Unusable => {}
                // Sidik tidak disimpan, supaya ingest berikutnya mencoba lagi.
                SidecarRead::Failed => job.sidecar_print = 0,
            }
        }
        if let Some(llm) = vision
            && (extracted.status == status::SCAN || extracted.status == status::IMAGE)
            && let Some(read) = read_with_vision(db, &job.path, &ext, size, async |input, extra| {
                llm.respond(input, extra).await
            })
            .await
        {
            extracted = read.dated_like(&extracted);
        }
        Ok::<_, anyhow::Error>((job, ext, extracted))
    }))
    .buffer_unordered(PARALLEL);

    let mut pending: Vec<RowUpdate> = Vec::new();
    while let Some(result) = results.next().await {
        let (job, ext, extracted) = result?;
        let version = indexed_version(&ext, extracted.status, !extracted.chunks.is_empty());
        let title = title_of(&job.path);
        let date = document_date(&title, &ext, &extracted).or_else(|| {
            job.uploaded.map(|date| docdate::DocDate {
                date,
                from: docdate::FROM_ARCHIVE,
            })
        });
        let id = match job.id {
            Some(id) => id,
            None => {
                let previous_id = legacy
                    .get(&(source.name.clone(), job.rel.clone()))
                    .map(|r| r.id);
                insert_pending(db, &source.name, &job, previous_id, &title, &ext, &label).await?
            }
        };
        index.delete_document(writer, id);
        let years = document_years(
            Path::new(&job.rel),
            date.map(|d| d.date.year()),
            job.modified,
        );
        let file_type = file_type(&ext);
        // Teks kosong menandai entri metadata di index. Jumlah potongan isi tetap nol.
        let metadata = (None, String::new());
        let entries = if extracted.chunks.is_empty() {
            std::slice::from_ref(&metadata)
        } else {
            &extracted.chunks
        };
        for (n, (page, text)) in entries.iter().enumerate() {
            index.add_chunk(
                writer,
                NewChunk {
                    doc_id: id,
                    n,
                    title: &title,
                    rel_path: &job.rel,
                    source: &source.name,
                    page: *page,
                    text,
                    acl: &acl,
                    file_type,
                    years: &years,
                },
            )?;
        }
        counts.read += 1;
        counts.chunks += extracted.chunks.len();
        counts.truncated += usize::from(extracted.truncated);
        counts.status(extracted.status);
        pending.push(RowUpdate {
            id,
            title,
            version,
            kind: ext,
            pages: extracted.pages,
            chunks: extracted.chunks.len(),
            status: extracted.status,
            size: job.size,
            mtime: job.mtime,
            sidecar: stored_print(extracted.status, job.sidecar_print),
            doc_date: date.map(|d| (d.date.to_string(), d.from)),
        });
        if pending.len() >= COMMIT_EVERY {
            writer.commit()?;
            save_rows(db, std::mem::take(&mut pending), &label).await?;
            eprint!("\r{}: {}/{} file dibaca", source.name, counts.read, total);
        }
    }
    writer.commit()?;
    save_rows(db, pending, &label).await?;
    if total >= COMMIT_EVERY {
        eprintln!();
    }
    Ok(counts)
}

/// Versi aturan tanggal dokumen (`docdate`). Dinaikkan kalau aturannya berubah, supaya
/// ingest berikutnya membaca ulang semua file sekali.
pub const DATE_VERSION: i64 = 1;

/// Tanggal dokumen dari judul, isi (bukan spreadsheet atau CSV/TSV, karena tanggal di sana
/// adalah data), lalu metadata file.
fn document_date(title: &str, ext: &str, extracted: &Extracted) -> Option<docdate::DocDate> {
    // Awalan ARCHIVE `<id8>_` bukan bagian judul; tanpa dibuang, tanggal atau kata SOP di awal
    // judul tidak terbaca.
    let title = archive_id8(title).map_or(title, |id8| &title[id8.len() + 1..]);
    let text = (!is_sheet(ext) && !matches!(ext, "csv" | "tsv")).then(|| {
        let mut text = String::new();
        for (_, chunk) in &extracted.chunks {
            if text.len() >= docdate::TEXT_CHARS {
                break;
            }
            text.push_str(chunk);
            text.push('\n');
        }
        text
    });
    docdate::pick(
        title,
        text.as_deref(),
        extracted.modified,
        extracted.created,
        docdate::today(),
    )
}

/// Folder tersembunyi di sebelah file berisi teks hasil baca ARCHIVE, satu file `<id8>.md` per
/// dokumen. `id8` adalah 8 huruf hex kecil di awal nama file (`<id8>_<judul>.<ext>`).
const ARCHIVE_TEXT_DIR: &str = ".extracted-text";
const MAX_SIDECAR_BYTES: u64 = 20 * 1024 * 1024;

/// Status hasil baca yang boleh diganti teks ARCHIVE: file yang isinya tidak terbaca sendiri.
fn uses_sidecar(state: &str) -> bool {
    !matches!(state, status::OK | status::VISION | status::PENDING)
}

/// File di akar folder sumber berisi tanggal upload ke ARCHIVE: satu baris `<id8>\t<YYYY-MM-DD>`
/// per dokumen; baris kosong dan baris `#` dilewati. Nama berawalan titik, jadi tidak ikut
/// terindeks.
const ARCHIVE_UPLOAD_FILE: &str = "document-dates.tsv";
const MAX_UPLOAD_FILE_BYTES: u64 = 5 * 1024 * 1024;

/// Tanggal upload ARCHIVE per `id8` dari `ARCHIVE_UPLOAD_FILE`; kosong kalau filenya tidak ada, berupa
/// symlink (seperti `walk`), atau terlalu besar. Baris yang tidak sesuai format dilewati.
fn archive_upload_dates(root: &Path) -> HashMap<String, time::Date> {
    let path = root.join(ARCHIVE_UPLOAD_FILE);
    let usable = std::fs::symlink_metadata(&path)
        .is_ok_and(|m| m.is_file() && m.len() <= MAX_UPLOAD_FILE_BYTES);
    let Some(text) = usable
        .then(|| std::fs::read_to_string(&path).ok())
        .flatten()
    else {
        return HashMap::new();
    };
    text.lines()
        .filter_map(|line| {
            // Tanpa trim di depan: kolom id8 yang kosong atau berspasi tidak boleh lolos. Kolom id8
            // harus tepat 8 huruf hex, bukan nama file berawalan id8.
            let (id8, date) = line.split_once('\t')?;
            if id8.len() != 8 {
                return None;
            }
            let id8 = archive_id8(&format!("{id8}_"))?.to_owned();
            let mut parts = date.trim().splitn(3, '-').map(str::parse::<i32>);
            let (y, m, d) = (
                parts.next()?.ok()?,
                parts.next()?.ok()?,
                parts.next()?.ok()?,
            );
            let date = docdate::date(y, u8::try_from(m).ok()?, u8::try_from(d).ok()?)?;
            Some((id8, date))
        })
        .collect()
}

/// Delapan huruf hex kecil di awal nama file yang diikuti `_`, atau `None`.
fn archive_id8(name: &str) -> Option<&str> {
    let head = name.as_bytes().get(..9)?;
    let hex = head[..8]
        .iter()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b));
    (hex && head[8] == b'_').then(|| &name[..8])
}

/// Lokasi teks ARCHIVE untuk `path` dan sidiknya (ukuran dan waktu ubah), atau `(None, 0)`.
fn archive_sidecar(path: &Path) -> (Option<PathBuf>, i64) {
    let Some(id8) = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(archive_id8)
    else {
        return (None, 0);
    };
    let Some(dir) = path.parent() else {
        return (None, 0);
    };
    // Symlink dan junction tidak diikuti, sama seperti `walk`: teks dari folder lain bisa
    // membawa isi dengan hak akses sumber yang berbeda.
    let teks_dir = dir.join(ARCHIVE_TEXT_DIR);
    if !std::fs::symlink_metadata(&teks_dir).is_ok_and(|m| m.is_dir()) {
        return (None, 0);
    }
    let sidecar = teks_dir.join(format!("{id8}.md"));
    let Ok(meta) = std::fs::symlink_metadata(&sidecar) else {
        return (None, 0);
    };
    if !meta.is_file() || meta.len() == 0 {
        return (None, 0);
    }
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    let size = i64::try_from(meta.len()).unwrap_or(i64::MAX);
    // Selalu positif dan tidak pernah 0 (0 berarti tidak ada teks ARCHIVE), supaya sidik `archive`
    // (positif) dan sidik teks tak terpakai (negatif) tidak tertukar; lihat stored_print.
    let print = (mtime.wrapping_mul(1_000_003).wrapping_add(size) & i64::MAX).max(1);
    (Some(sidecar), print)
}

/// Sidik yang disimpan untuk baris berstatus `state`: positif kalau isinya dari teks ARCHIVE,
/// negatif kalau teks ARCHIVE ada tapi tidak terpakai (kosong atau terlalu besar). Baris scan
/// yang membawa sidik positif berarti ditulis binary lain (misalnya ingest lama setelah
/// rollback), jadi file itu dibaca ulang.
fn stored_print(state: &str, print: i64) -> i64 {
    if state == status::ARCHIVE {
        print
    } else {
        print.wrapping_neg()
    }
}

enum SidecarRead {
    Text(Extracted),
    /// Kosong atau terlalu besar: hasil baca sendiri tetap dipakai sampai teksnya berubah.
    Unusable,
    /// Gagal dibuka atau dibaca (misalnya sedang dikunci): dicoba lagi di ingest berikutnya.
    Failed,
}

/// Teks ARCHIVE sebagai isi dokumen, dipotong seperti file teks, tanpa nomor halaman.
async fn read_sidecar(path: &Path, pages: u32) -> SidecarRead {
    use tokio::io::AsyncReadExt;
    let Ok(file) = tokio::fs::File::open(path).await else {
        return SidecarRead::Failed;
    };
    let mut bytes = Vec::new();
    if file
        .take(MAX_SIDECAR_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .is_err()
    {
        return SidecarRead::Failed;
    }
    if bytes.len() as u64 > MAX_SIDECAR_BYTES {
        return SidecarRead::Unusable;
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if text.trim().is_empty() {
        return SidecarRead::Unusable;
    }
    let mut read = build(Kind::Text, 0, vec![(None, text)]);
    if read.chunks.is_empty() {
        return SidecarRead::Unusable;
    }
    read.status = status::ARCHIVE;
    read.pages = pages;
    SidecarRead::Text(read)
}

fn carry_vision(index: &DocIndex, id: i64, chunks: i64, pages: u32) -> Result<Option<Extracted>> {
    let admin = crate::acl::AuthUser {
        id: 0,
        email: String::new(),
        name: String::new(),
        department: None,
        role: crate::acl::Role::Admin,
    };
    let Some((total, hits)) = index.document_chunks(id, &admin, 0, MAX_CHUNKS_PER_DOC)? else {
        return Ok(None);
    };
    if total == 0 || Some(total) != usize::try_from(chunks).ok() || hits.len() != total {
        return Ok(None);
    }
    Ok(Some(Extracted {
        pages,
        chunks: hits
            .into_iter()
            .map(|(_, hit)| (hit.page, hit.text))
            .collect(),
        status: status::VISION,
        truncated: false,
        modified: None,
        created: None,
    }))
}

/// Jenis gambar yang bisa dibaca model, dengan MIME-nya.
pub(crate) fn vision_image_mime(ext: &str) -> Option<&'static str> {
    match ext {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Membaca gambar atau PDF hasil scan lewat model. `None` kalau jenisnya tidak didukung,
/// filenya terlalu besar, atau model gagal; hasil dari Xberg tetap dipakai.
async fn read_with_vision(
    db: &Db,
    path: &Path,
    ext: &str,
    size: u64,
    respond: impl AsyncFnOnce(serde_json::Value, serde_json::Value) -> Result<serde_json::Value>,
) -> Option<Extracted> {
    use base64::Engine;
    if size > MAX_VISION_BYTES || (ext != "pdf" && vision_image_mime(ext).is_none()) {
        return None;
    }
    let bytes = read_limited(path, MAX_VISION_BYTES).await?;
    let part = if ext == "pdf" {
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        serde_json::json!({ "type": "input_file", "filename": format!("dokumen.{ext}"),
            "file_data": format!("data:application/pdf;base64,{data}") })
    } else {
        let mime = vision_image_mime(ext)?;
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        serde_json::json!({ "type": "input_image", "image_url": format!("data:{mime};base64,{data}") })
    };
    let input = serde_json::json!([{ "role": "user", "content": [
        { "type": "input_text", "text": "Baca dokumen ini." },
        part,
    ]}]);
    let extra = serde_json::json!({
        "instructions": VISION_INSTRUCTIONS,
        "reasoning": { "effort": "low" },
        "max_output_tokens": 16_000,
    });
    let mut usage = Usage::default();
    usage.start_request();
    let response = tokio::time::timeout(VISION_TIMEOUT, respond(input, extra))
        .await
        .ok()
        .and_then(Result::ok);
    if let Some(response) = &response {
        usage.add_response(response);
    }
    usage::record(db, usage::INGEST_VISION, None, None, usage).await;
    let response = response?;
    if response["status"]
        .as_str()
        .is_some_and(|s| s != "completed")
    {
        return None;
    }
    let text = crate::agent::output_text(&response);
    let sections = vision_sections(&text, ext == "pdf");
    let pages = sections.iter().filter_map(|(p, _)| *p).max().unwrap_or(0);
    let mut read = build(Kind::Office, pages, sections);
    if read.chunks.is_empty() {
        return None;
    }
    read.status = status::VISION;
    Some(read)
}

/// Isi file, atau `None` kalau lebih dari `max` byte. Ukuran dicek saat membaca, karena
/// file bisa membesar sejak dipindai.
async fn read_limited(path: &Path, max: u64) -> Option<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let file = tokio::fs::File::open(path).await.ok()?;
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes).await.ok()?;
    (bytes.len() as u64 <= max).then_some(bytes)
}

/// Teks dari model, dipisah per penanda `=== Halaman N ===` untuk PDF. Penanda dengan
/// nomor 0 atau tidak naik dianggap teks biasa, supaya sitasi tidak menyebut halaman
/// yang keliru.
fn vision_sections(text: &str, paged: bool) -> Vec<(Option<u32>, String)> {
    if !paged {
        return vec![(None, text.to_string())];
    }
    let mut sections: Vec<(Option<u32>, String)> = Vec::new();
    let mut current: (Option<u32>, String) = (None, String::new());
    for line in text.lines() {
        let last = sections
            .iter()
            .chain([&current])
            .filter_map(|(p, _)| *p)
            .max()
            .unwrap_or(0);
        let marker = line
            .trim()
            .strip_prefix("=== Halaman ")
            .and_then(|rest| rest.strip_suffix(" ==="))
            .and_then(|n| n.trim().parse::<u32>().ok())
            .filter(|&n| n > last && n <= 10_000);
        if let Some(page) = marker {
            if !current.1.trim().is_empty() {
                sections.push(std::mem::take(&mut current));
            }
            current = (Some(page), String::new());
        } else {
            current.1.push_str(line);
            current.1.push('\n');
        }
    }
    if !current.1.trim().is_empty() {
        sections.push(current);
    }
    sections
}

/// PDF sederhana dengan satu baris teks per halaman, untuk tes dan `cek-vision`. Teks hanya
/// boleh berisi huruf ASCII tanpa tanda kurung.
pub(crate) fn minimal_pdf(pages: &[&str]) -> Vec<u8> {
    let n = pages.len();
    let font = 3 + 2 * n;
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + 2 * i)).collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")),
    ];
    for (i, text) in pages.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R /Resources << /Font << /F1 {font} 0 R >> >> >>",
            4 + 2 * i
        ));
        let stream = format!("BT /F1 14 Tf 72 720 Td ({text}) Tj ET");
        objects.push(format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        ));
    }
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());
    let mut pdf = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{body}\nendobj\n", i + 1));
    }
    let xref = pdf.len();
    pdf.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    ));
    for offset in offsets {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    ));
    pdf.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vision_pages_follow_markers() {
        let text = "=== Halaman 1 ===\nJudul SOP\n\n=== Halaman 2 ===\nLangkah kerja\n=== Halaman 3 ===\n\n";
        assert_eq!(
            vision_sections(text, true),
            [
                (Some(1), "Judul SOP\n\n".to_string()),
                (Some(2), "Langkah kerja\n".to_string())
            ]
        );
        // Teks sebelum penanda pertama tetap dipakai, tanpa nomor halaman.
        assert_eq!(
            vision_sections("Kop surat\n=== Halaman 1 ===\nIsi", true).len(),
            2
        );
        assert_eq!(
            vision_sections("Foto mesin roll", false),
            [(None, "Foto mesin roll".to_string())]
        );
        assert_eq!(vision_image_mime("jpeg"), Some("image/jpeg"));
        assert_eq!(vision_image_mime("heic"), None);
        // Nomor 0, berulang, atau mundur bukan penanda halaman.
        let odd = "=== Halaman 0 ===\nA\n=== Halaman 2 ===\nB\n=== Halaman 2 ===\nC\n=== Halaman 1 ===\nD";
        let pages: Vec<Option<u32>> = vision_sections(odd, true).iter().map(|(p, _)| *p).collect();
        assert_eq!(pages, [None, Some(2)]);
        assert!(
            vision_sections(odd, true)[1]
                .1
                .contains("=== Halaman 1 ===")
        );
    }

    #[tokio::test]
    async fn vision_reads_are_size_limited() {
        let path = std::env::temp_dir().join(format!("rag-hub-limit-{}.bin", std::process::id()));
        std::fs::write(&path, vec![7u8; 11]).unwrap();
        assert_eq!(read_limited(&path, 11).await.map(|b| b.len()), Some(11));
        assert!(read_limited(&path, 10).await.is_none());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn lines_are_cleaned_and_split() {
        assert_eq!(clean_line("  a\t\tb \u{7}c  "), "a b c");
        assert_eq!(split_long("abc def ghi", 5), ["abc", "def", "ghi"]);
        assert_eq!(split_long("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(split_long("ééééé", 2), ["éé", "éé", "é"]);
        assert!(split_long("", 4).is_empty());
        assert!(is_table_separator("| --- | :-: |"));
        assert!(!is_table_separator("| a | b |"));
        assert_eq!(thousands(1234567), "1.234.567");
        assert_eq!(thousands(12), "12");
    }

    #[test]
    fn chunks_stay_under_the_limit_and_keep_context() {
        let mut text = String::from("# Sheet Stok\n\n| Kode | Nama | Qty |\n| --- | --- | --- |\n");
        for i in 0..200 {
            text.push_str(&format!("| P-{i:03} | produk contoh {i} | {} |\n", i * 3));
        }
        let chunks = chunk_text(&text);
        assert!(chunks.len() > 2);
        for chunk in &chunks {
            assert!(
                chunk.chars().count() <= CHUNK_CHARS + 900,
                "{}",
                chunk.len()
            );
        }
        // Potongan lanjutan tetap membawa nama sheet dan kepala tabel.
        for chunk in &chunks[1..] {
            assert!(
                chunk.starts_with("# Sheet Stok\n| Kode | Nama | Qty |\n| --- |"),
                "{chunk}"
            );
        }
        // Tidak ada baris yang hilang.
        let all = chunks.join("\n");
        for i in 0..200 {
            assert!(all.contains(&format!("| P-{i:03} |")));
        }
    }

    #[test]
    fn symbol_only_table_rows_survive_chunking() {
        let text = "## Review\n| Sample | OD | UTS |\n| --- | --- | --- |\n| S-001 | 60.5 | 410 |\n\
                    | \\--- | \\--- | \\--- |\n| + | - | - |\n| S-002 | 60.4 | 420 |\n";
        let all = chunk_text(text).join("\n");
        assert!(all.contains("| \\--- | \\--- | \\--- |"), "{all}");
        assert!(all.contains("| + | - | - |"), "{all}");
        assert!(all.contains("| S-002 | 60.4 | 420 |"), "{all}");
    }

    #[test]
    fn separators_need_three_dashes_per_cell() {
        assert!(is_table_separator("| --- | --- |"));
        assert!(is_table_separator("| :---: | ---: |"));
        assert!(!is_table_separator("|  | - | - |"));
        // Baris `---` dari isi sel di-escape oleh sheet.rs, jadi bukan pemisah.
        assert!(!is_table_separator("| \\--- | \\--- | \\--- |"));
        assert!(is_table_separator("| -- | --- |"));
        assert!(!is_table_separator("| - | - |"));
        // Baris nilai kosong tidak menjadi kepala yang diulang di potongan berikutnya.
        let mut text =
            String::from("# Review\n| Sample | OD |\n| --- | --- |\n| S-001 | 60.5 |\n|  | - |\n");
        for i in 0..200 {
            text.push_str(&format!("| S-{i:03} | 60.{i} |\n"));
        }
        for chunk in &chunk_text(&text)[1..] {
            assert!(
                chunk.starts_with("# Review\n| Sample | OD |\n| --- | --- |"),
                "{chunk}"
            );
        }
    }

    #[test]
    fn table_names_stay_with_their_separator() {
        for notes in 1_380..1_480 {
            let mut text = String::from(
                "# Sheet
",
            );
            text.push_str(&"x".repeat(notes));
            text.push_str(
                "
| Kolom A | Kolom B |
| --- | --- |
| 1 | 2 |
",
            );
            for chunk in chunk_text(&text) {
                assert!(
                    !chunk.contains(
                        "# Sheet
| --- |"
                    ),
                    "{chunk}"
                );
                if chunk.contains("| --- | --- |") {
                    assert!(
                        chunk.contains(
                            "| Kolom A | Kolom B |
| --- | --- |"
                        ),
                        "{chunk}"
                    );
                }
            }
        }
    }

    #[test]
    fn paragraphs_and_long_lines() {
        let text = format!(
            "Judul\n\n\n{}\n\nPenutup singkat di sini.",
            "kata ".repeat(700)
        );
        let chunks = chunk_text(&text);
        assert!(chunks.len() >= 3);
        assert!(chunks[0].starts_with("Judul\n\nkata"));
        assert!(
            chunks
                .iter()
                .all(|c| c.chars().count() <= CHUNK_CHARS + MIN_CHUNK_CHARS)
        );
        assert!(chunks.last().unwrap().ends_with("Penutup singkat di sini."));
        // Teks tanpa isi berarti tidak jadi potongan.
        assert!(chunk_text("  \n | | | \n -- \n").is_empty());
        // Sheet pendek tetap terindeks: setiap baris data dibawa.
        let short = crate::sheet::to_text(
            &[crate::sheet::Sheet {
                name: "Review".into(),
                hidden: false,
                cells: &[
                    vec!["S1".to_string(), "+".to_string()],
                    vec!["S2".to_string(), "-".to_string()],
                ],
            }],
            CHUNK_CHARS * MAX_CHUNKS_PER_DOC,
        )
        .0;
        let chunks = chunk_text(&short);
        assert_eq!(chunks.len(), 1, "{short}");
        // Potongan lanjutan yang barisnya hanya simbol tetap terindeks.
        let symbols = format!(
            "## Review\n{}",
            "| - | - | ✓ |  | - | - | ✗ |\n".repeat(400)
        );
        let kept: usize = chunk_text(&symbols)
            .iter()
            .map(|c| c.matches("| - | - | ✓ |").count())
            .sum();
        assert_eq!(kept, 400);
        assert!(
            chunks[0].contains("| S1 | + |") && chunks[0].contains("| S2 | - |"),
            "{chunks:?}"
        );
    }

    fn side_by_side_workbook() -> Vec<u8> {
        let mut wb = rust_xlsxwriter::Workbook::new();
        let ws = wb.add_worksheet().set_name("Operasional").unwrap();
        ws.write_string(0, 0, "PCS PER JAM").unwrap();
        let months = [
            "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec", "Jan",
        ];
        let mut row = 2;
        for fy in (2010..=2026).rev() {
            for (col, name) in [(0u16, "Intake"), (6, "Dispatch")] {
                ws.write_string(row, col, name).unwrap();
                ws.write_string(row + 1, col, format!("FY {fy}")).unwrap();
                for (j, h) in ["Month", "Prod (pcs)", "Working hour"].iter().enumerate() {
                    ws.write_string(row + 2, col + j as u16, *h).unwrap();
                }
                for (i, m) in months.iter().enumerate() {
                    let r = row + 3 + i as u32;
                    ws.write_string(r, col, *m).unwrap();
                    ws.write_number(r, col + 1, f64::from(fy) * 10.0 + i as f64)
                        .unwrap();
                    ws.write_number(r, col + 2, 500.0 + i as f64).unwrap();
                }
            }
            row += 3 + months.len() as u32 + 1;
        }
        wb.add_worksheet().set_name("Kosong").unwrap();
        let notes = wb.add_worksheet().set_name("Catatan").unwrap();
        notes.write_string(0, 0, "Data tiruan untuk tes.").unwrap();
        let archive = wb.add_worksheet().set_name("Arsip").unwrap();
        archive.write_string(0, 0, "Kode").unwrap();
        archive.write_string(0, 1, "Status").unwrap();
        archive.write_string(1, 0, "P-001").unwrap();
        archive.write_string(1, 1, "lama").unwrap();
        archive.set_hidden(true);
        wb.save_to_buffer().unwrap()
    }

    #[tokio::test]
    async fn spreadsheet_chunks_carry_sheet_section_and_header() {
        let read = extract_with_xberg(side_by_side_workbook(), "xlsx", true)
            .await
            .unwrap();
        let extracted = build(Kind::Office, read.pages, read.sections);
        assert_eq!(extracted.status, status::OK);
        let chunks: Vec<&str> = extracted.chunks.iter().map(|(_, c)| c.as_str()).collect();
        assert!(chunks.len() > 3, "{}", chunks.len());
        let mut rows = 0;
        for chunk in &chunks {
            let mut heading = None;
            for line in chunk.lines() {
                if let Some(h) = line.strip_prefix("## ") {
                    heading = Some(h);
                } else if line.starts_with("| ")
                    && heading.is_some_and(|h| h.starts_with("Operasional"))
                    && !line.starts_with("| Month")
                    && !line.starts_with("| ---")
                {
                    // Tabel berdampingan tidak digabung: satu baris berisi tiga kolom.
                    assert_eq!(line.matches(" |").count(), 3, "{line}");
                    let year: u32 = line.split(" | ").nth(1).unwrap().parse::<u32>().unwrap() / 10;
                    let h = heading.unwrap_or_else(|| panic!("baris tanpa judul: {line}\n{chunk}"));
                    assert!(
                        h.starts_with("Operasional › ") && h.ends_with(&format!("FY {year}")),
                        "{h} / {line}"
                    );
                    assert!(h.contains("Intake") || h.contains("Dispatch"), "{h}");
                    assert!(
                        chunk.contains("| Month | Prod (pcs) | Working hour |"),
                        "{chunk}"
                    );
                    rows += 1;
                }
            }
        }
        assert_eq!(rows, 17 * 2 * 10, "semua baris data masuk");
        assert!(chunks.iter().any(|c| c.contains("Data tiruan untuk tes.")));
        assert!(!chunks.iter().any(|c| c.contains("Kosong")));
        assert!(
            chunks.iter().any(|c| c.contains(
                "## Arsip (tersembunyi)
| Kode | Status |"
            )),
            "sheet tersembunyi ditandai"
        );
    }

    #[test]
    fn years_come_from_the_path_and_modification_time() {
        let modified = UNIX_EPOCH + Duration::from_secs(1_767_225_600);
        assert_eq!(
            document_years(
                Path::new("Review/2000/2025/Rekap2025-12024-2099-2100.xlsx"),
                None,
                Some(modified)
            ),
            [2000, 2025, 2026, 2099]
        );
        assert!(document_years(Path::new("Review/Rekap.xlsx"), None, None).is_empty());
        // Tanggal dokumen menggantikan tahun file diubah.
        assert_eq!(
            document_years(Path::new("Review/Rekap.xlsx"), Some(2024), Some(modified)),
            [2024]
        );
    }

    #[test]
    fn only_spreadsheets_get_the_new_content_version() {
        assert_eq!(content_version("xlsx"), INDEX_VERSION + 1);
        assert_eq!(content_version("ods"), INDEX_VERSION + 1);
        assert_eq!(content_version("pdf"), INDEX_VERSION);
        assert_eq!(content_version("docx"), INDEX_VERSION);
    }

    #[test]
    fn scanned_pdfs_and_limits() {
        let pages = vec![(Some(1), "  ".to_string()), (Some(2), "Hal 2".to_string())];
        assert_eq!(build(Kind::Pdf, 2, pages).status, status::SCAN);
        let text = "Ketebalan dinding produk minimal 2,3 mm untuk DEMO. ".repeat(10);
        let ok = build(Kind::Pdf, 1, vec![(Some(1), text)]);
        assert_eq!((ok.status, ok.chunks[0].0), (status::OK, Some(1)));
        assert_eq!(
            build(Kind::Office, 0, vec![(None, " ".into())]).status,
            status::EMPTY
        );
        let huge = "Baris data stok produk nomor sekian.\n".repeat(150_000);
        let big = build(Kind::Office, 0, vec![(None, huge)]);
        assert!(big.truncated && big.chunks.len() == MAX_CHUNKS_PER_DOC);
    }

    #[test]
    fn walk_skips_hidden_and_lock_files() {
        let dir = std::env::temp_dir().join(format!("rag-hub-walk-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Sub/Dalam")).unwrap();
        for name in ["a.pdf", "~$a.docx", ".rahasia", "Sub/Dalam/b.xlsx"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let mut files = Vec::new();
        walk(&dir, None, &mut files).unwrap();
        let mut rel: Vec<String> = files.iter().filter_map(|p| rel_path(&dir, p)).collect();
        rel.sort();
        assert_eq!(rel, ["Sub/Dalam/b.xlsx", "a.pdf"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn ingest_backfills_old_unreadable_rows_and_replaces_the_entry_after_reading() {
        let base =
            std::env::temp_dir().join(format!("rag-hub-unreadable-{}", rand::random::<u64>()));
        let cfg = test_config(&base, r#"{ type = "umum" }"#);
        let path = base.join("docs/catatan.txt");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "").unwrap();
        run(&cfg, false, None).await.unwrap();
        let id = rows(&base).await["catatan.txt"].2;
        let index = DocIndex::open(&cfg.data_dir).unwrap();
        {
            let mut writer = index.writer().unwrap();
            index.delete_document(&writer, id);
            writer.commit().unwrap();
        }
        let db = Db::open(&cfg.data_dir).unwrap();
        db.call(move |c| {
            c.execute(
                "UPDATE documents_v4 SET version = ?1 WHERE id = ?2",
                params![INDEX_VERSION, id],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(run(&cfg, false, None).await.unwrap().read, 1);
        let search = || {
            index.reload().unwrap();
            index
                .search("catatan", &user("Review"), &["Uji".into()], 0, 5, false)
                .unwrap()
        };
        let hits = search();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].text.is_empty());
        assert_eq!(run(&cfg, false, None).await.unwrap().read, 0);

        fs::write(&path, "Teks baru yang bisa dibaca.").unwrap();
        assert_eq!(run(&cfg, false, None).await.unwrap().read, 1);
        let hits = search();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].text, "Teks baru yang bisa dibaca.");
        assert_eq!(hits[0].doc_id, id);
        assert_ne!(hits[0].chunk_id, search::chunk_id(id, 0, ""));
        drop((db, index));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn archive_id8_needs_eight_lowercase_hex_and_an_underscore() {
        assert_eq!(archive_id8("0123abcd_Ringkasan.pdf"), Some("0123abcd"));
        assert_eq!(archive_id8("0123ABCD_Ringkasan.pdf"), None);
        assert_eq!(archive_id8("0123abc-_Ringkasan.pdf"), None);
        assert_eq!(archive_id8("0123abcdRingkasan.pdf"), None);
        assert_eq!(archive_id8("0123abcd"), None);
        assert!(
            uses_sidecar(status::SCAN)
                && uses_sidecar(status::IMAGE)
                && uses_sidecar(status::ARCHIVE)
        );
        assert!(!uses_sidecar(status::OK) && !uses_sidecar(status::VISION));
        assert_eq!(stored_print(status::ARCHIVE, 7), 7);
        assert_eq!(stored_print(status::SCAN, 7), -7);
        assert_eq!(stored_print(status::SCAN, 0), 0);
    }

    #[tokio::test]
    async fn unreadable_archive_text_is_retried_but_blank_text_is_final() {
        let dir =
            std::env::temp_dir().join(format!("rag-hub-archive-read-{}", rand::random::<u64>()));
        fs::create_dir_all(&dir).unwrap();
        let blank = dir.join("kosong.md");
        fs::write(&blank, "  \n\t").unwrap();
        assert!(matches!(
            read_sidecar(&blank, 0).await,
            SidecarRead::Unusable
        ));
        assert!(matches!(
            read_sidecar(&dir.join("tidak-ada.md"), 0).await,
            SidecarRead::Failed
        ));
        let text = dir.join("isi.md");
        fs::write(&text, "Teks hasil baca ARCHIVE.").unwrap();
        match read_sidecar(&text, 3).await {
            SidecarRead::Text(read) => {
                assert_eq!(read.status, status::ARCHIVE);
                assert_eq!(read.pages, 3);
                assert_eq!(
                    read.chunks,
                    vec![(None, "Teks hasil baca ARCHIVE.".to_string())]
                );
            }
            _ => panic!("teks ARCHIVE seharusnya terbaca"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn archive_text_behind_a_symlink_is_ignored() {
        let base =
            std::env::temp_dir().join(format!("rag-hub-archive-link-{}", rand::random::<u64>()));
        let docs = base.join("docs");
        let other = base.join("lain");
        fs::create_dir_all(&docs).unwrap();
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("0a1b2c3d.md"), "Teks rahasia dari sumber lain.").unwrap();
        std::os::unix::fs::symlink(&other, docs.join(ARCHIVE_TEXT_DIR)).unwrap();
        assert_eq!(archive_sidecar(&docs.join("0a1b2c3d_scan.pdf")), (None, 0));
        fs::remove_file(docs.join(ARCHIVE_TEXT_DIR)).unwrap();
        fs::create_dir_all(docs.join(ARCHIVE_TEXT_DIR)).unwrap();
        std::os::unix::fs::symlink(
            other.join("0a1b2c3d.md"),
            docs.join(ARCHIVE_TEXT_DIR).join("0a1b2c3d.md"),
        )
        .unwrap();
        assert_eq!(archive_sidecar(&docs.join("0a1b2c3d_scan.pdf")), (None, 0));
        let _ = fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn archive_text_fills_files_that_cannot_be_read_and_follows_changes() {
        let base = std::env::temp_dir().join(format!("rag-hub-archive-{}", rand::random::<u64>()));
        let cfg = test_config(&base, r#"{ type = "umum" }"#);
        let docs = base.join("docs");
        let teks = docs.join(ARCHIVE_TEXT_DIR);
        fs::create_dir_all(&teks).unwrap();
        fs::write(docs.join("0a1b2c3d_scan.pdf"), minimal_pdf(&[""])).unwrap();
        fs::write(docs.join("4e5f6a7b_foto.jpg"), b"bukan gambar sungguhan").unwrap();
        fs::write(docs.join("8c9d0e1f_catatan.txt"), "Isi asli catatan.").unwrap();
        fs::write(docs.join("tanpa_awalan.jpg"), b"x").unwrap();
        fs::write(
            teks.join("0a1b2c3d.md"),
            "Hasil baca ARCHIVE: jumlah barang belum cocok.",
        )
        .unwrap();
        fs::write(teks.join("8c9d0e1f.md"), "Teks ARCHIVE yang tidak dipakai.").unwrap();

        // Folder .extracted-text tidak ikut terindeks sebagai dokumen.
        assert_eq!(run(&cfg, false, None).await.unwrap().read, 4);
        let found = rows(&base).await;
        assert_eq!(found.len(), 4);
        assert_eq!(found["0a1b2c3d_scan.pdf"].0, status::ARCHIVE);
        assert_eq!(
            found["0a1b2c3d_scan.pdf"].1, 1,
            "jumlah halaman PDF tetap tercatat"
        );
        assert_eq!(found["4e5f6a7b_foto.jpg"].0, status::IMAGE);
        assert_eq!(found["8c9d0e1f_catatan.txt"].0, status::OK);
        assert_eq!(found["tanpa_awalan.jpg"].0, status::IMAGE);
        let search = |q: &str| {
            DocIndex::open(&cfg.data_dir)
                .unwrap()
                .search(q, &user("Review"), &["Uji".into()], 0, 5, false)
                .unwrap()
        };
        let hits = search("cocok");
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].text,
            "Hasil baca ARCHIVE: jumlah barang belum cocok."
        );
        assert_eq!(hits[0].page, None);
        assert!(
            search("dipakai").is_empty(),
            "file yang terbaca sendiri tidak memakai teks ARCHIVE"
        );
        assert_eq!(run(&cfg, false, None).await.unwrap().read, 0);

        // Teks ARCHIVE yang baru muncul membuat foto dibaca ulang tanpa --ulang.
        fs::write(
            teks.join("4e5f6a7b.md"),
            "Foto lokasi: oli tercecer di lantai.",
        )
        .unwrap();
        assert_eq!(run(&cfg, false, None).await.unwrap().read, 1);
        assert_eq!(rows(&base).await["4e5f6a7b_foto.jpg"].0, status::ARCHIVE);
        assert_eq!(search("tercecer").len(), 1);

        // Teks ARCHIVE yang berubah ikut dibaca ulang.
        fs::write(
            teks.join("0a1b2c3d.md"),
            "Revisi baca ARCHIVE: catatan pemeriksaan kemasan.",
        )
        .unwrap();
        assert_eq!(run(&cfg, false, None).await.unwrap().read, 1);
        assert_eq!(search("kemasan").len(), 1);
        assert!(search("cocok").is_empty());

        // Setelah rollback, ingest lama menulis status scan tetapi tidak menyentuh kolom sidecar.
        // Binary ini harus membaca ulang file itu, bukan menganggapnya tidak berubah.
        let db = Db::open(&base.join("data")).unwrap();
        db.call(|c| {
            Ok(c.execute(
                "UPDATE documents_v4 SET status = 'scan', chunks = 0 WHERE rel_path = '0a1b2c3d_scan.pdf'",
                [],
            )?)
        })
        .await
        .unwrap();
        drop(db);
        assert_eq!(run(&cfg, false, None).await.unwrap().read, 1);
        assert_eq!(rows(&base).await["0a1b2c3d_scan.pdf"].0, status::ARCHIVE);
        assert_eq!(run(&cfg, false, None).await.unwrap().read, 0);

        // Teks ARCHIVE yang dihapus atau kosong membuat file kembali menjadi entri nama saja.
        fs::remove_file(teks.join("4e5f6a7b.md")).unwrap();
        fs::write(teks.join("0a1b2c3d.md"), "   \n").unwrap();
        assert_eq!(run(&cfg, false, None).await.unwrap().read, 2);
        let found = rows(&base).await;
        assert_eq!(found["4e5f6a7b_foto.jpg"].0, status::IMAGE);
        assert_eq!(found["0a1b2c3d_scan.pdf"].0, status::SCAN);
        assert!(search("tercecer").is_empty());
        // Teks kosong bersifat final: tidak dibaca ulang terus-menerus.
        assert_eq!(run(&cfg, false, None).await.unwrap().read, 0);
        let _ = fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn ingest_retains_scan_text_and_backfills_blank_scans_and_failures_once() {
        let base = std::env::temp_dir().join(format!("rag-hub-scan-{}", rand::random::<u64>()));
        let cfg = test_config(&base, r#"{ type = "umum" }"#);
        let docs = base.join("docs");
        fs::create_dir_all(&docs).unwrap();
        let cover = "Nomor ABC123";
        fs::write(docs.join("sampul.pdf"), minimal_pdf(&[cover, ""])).unwrap();
        fs::write(docs.join("polos.pdf"), minimal_pdf(&[""])).unwrap();
        let broken = docs.join("rusak.pdf");
        fs::write(&broken, b"%PDF-1.4 bukan pdf").unwrap();

        assert_eq!(run(&cfg, false, None).await.unwrap().read, 3);
        let found = rows(&base).await;
        let scan = found["sampul.pdf"].2;
        let blank = found["polos.pdf"].2;
        let failed = found["rusak.pdf"].2;
        assert_eq!(found["sampul.pdf"].0, status::SCAN);
        assert_eq!(found["polos.pdf"].0, status::SCAN);
        assert_eq!(found["rusak.pdf"].0, status::FAILED);
        let index = DocIndex::open(&cfg.data_dir).unwrap();
        let expected_id = search::chunk_id(scan, 0, cover);
        let check_cover = || {
            index.reload().unwrap();
            let review = user("Review");
            let hits = index
                .search("ABC123", &review, &["Uji".into()], 0, 5, false)
                .unwrap();
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].chunk_id, expected_id);
            let hit = index.chunk(&expected_id, &review).unwrap().unwrap();
            assert_eq!((hit.page, hit.text.as_str()), (Some(1), cover));
        };
        check_cover();

        // Sebelum backfill, scan dengan teks sudah terindeks. Dua baris lain belum punya entri.
        let mut writer = index.writer().unwrap();
        index.delete_document(&writer, blank);
        index.delete_document(&writer, failed);
        writer.commit().unwrap();
        writer.wait_merging_threads().unwrap();
        let db = Db::open(&cfg.data_dir).unwrap();
        db.call(move |c| {
            c.execute(
                "UPDATE documents_v4 SET version = ?1 WHERE id IN (?2, ?3, ?4)",
                params![INDEX_VERSION, scan, blank, failed],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        assert_eq!(run(&cfg, false, None).await.unwrap().read, 2);
        check_cover();
        for id in [blank, failed] {
            let hit = index
                .chunk(&search::chunk_id(id, 0, ""), &user("Review"))
                .unwrap()
                .unwrap();
            assert!(hit.text.is_empty());
        }
        assert_eq!(run(&cfg, false, None).await.unwrap(), Summary::default());

        fs::write(
            broken,
            minimal_pdf(&["Dokumen sintetis sudah dapat dibaca kembali."]),
        )
        .unwrap();
        assert_eq!(run(&cfg, false, None).await.unwrap().read, 1);
        assert_eq!(rows(&base).await["rusak.pdf"].0, status::OK);
        drop((db, index));
        let _ = fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn ingest_vision_records_tokens_for_every_call_without_storing_content() {
        use serde_json::json;

        let base = std::env::temp_dir().join(format!("rag-hub-vision-{}", rand::random::<u64>()));
        fs::create_dir_all(&base).unwrap();
        let path = base.join("foto.png");
        fs::write(&path, b"gambar sintetis").unwrap();
        let db = Db::open(&base).unwrap();
        let server_db = Db::open(&base).unwrap();

        let response = |state| {
            json!({
                "status": state,
                "usage": {
                    "input_tokens": 14, "input_tokens_details": {"cached_tokens": 5},
                    "output_tokens": 10, "output_tokens_details": {"reasoning_tokens": 4}
                },
                "output": [{"type": "message", "content": [
                    {"type": "output_text", "text": "Isi gambar sintetis"}
                ]}]
            })
        };
        let result = read_with_vision(&db, &path, "png", 15, async |input, _| {
            assert_eq!(input[0]["content"][1]["type"], "input_image");
            Ok(response("completed"))
        })
        .await
        .unwrap();
        assert_eq!(result.status, status::VISION);
        assert_eq!(result.chunks[0].1, "Isi gambar sintetis");

        assert!(
            read_with_vision(&db, &path, "png", 15, async |_, _| Ok(response(
                "incomplete"
            )))
            .await
            .is_none()
        );
        assert!(
            read_with_vision(&db, &path, "png", 15, async |_, _| anyhow::bail!(
                "error uji"
            ))
            .await
            .is_none()
        );
        assert!(
            read_with_vision(
                &db,
                &path,
                "png",
                MAX_VISION_BYTES + 1,
                async |_, _| panic!("file terlalu besar tidak dikirim")
            )
            .await
            .is_none()
        );

        server_db
            .call(|c| {
                let mut statement = c.prepare("SELECT * FROM usage_log ORDER BY id")?;
                assert_eq!(
                    statement.column_names().collect::<Vec<_>>(),
                    [
                        "id",
                        "created_at",
                        "kind",
                        "user_id",
                        "message_id",
                        "input_tokens",
                        "cached_tokens",
                        "output_tokens",
                        "reasoning_tokens",
                        "requests",
                        "reported_requests"
                    ]
                );
                let mut rows = statement.query([])?;
                for expected in [
                    [14, 5, 10, 4, 1, 1],
                    [14, 5, 10, 4, 1, 1],
                    [0, 0, 0, 0, 1, 0],
                ] {
                    let row = rows.next()?.unwrap();
                    assert_eq!(row.get::<_, String>(2)?, "ingest_vision");
                    assert_eq!(row.get::<_, Option<i64>>(3)?, None);
                    assert_eq!(row.get::<_, Option<i64>>(4)?, None);
                    for (i, value) in expected.into_iter().enumerate() {
                        assert_eq!(row.get::<_, i64>(i + 5)?, value);
                    }
                }
                assert!(rows.next()?.is_none());
                Ok(())
            })
            .await
            .unwrap();
        drop((db, server_db));
        let _ = fs::remove_dir_all(base);
    }

    fn test_config(base: &Path, visibility: &str) -> Config {
        let text = format!(
            r#"data_dir = {data:?}
[azure]
endpoint = "http://127.0.0.1:9"
deployment = "x"
[[sources]]
name = "Uji"
path = {docs:?}
visibility = {visibility}
"#,
            data = base.join("data").to_string_lossy(),
            docs = base.join("docs").to_string_lossy(),
        );
        toml::from_str(&text).unwrap()
    }

    #[tokio::test]
    async fn configured_ingest_embeds_after_releasing_its_write_locks() {
        let base =
            std::env::temp_dir().join(format!("rag-hub-auto-embed-{}", rand::random::<u64>()));
        fs::create_dir_all(base.join("docs")).unwrap();
        fs::write(base.join("docs/uji.txt"), "Torsi sintetis.").unwrap();
        let mut cfg = test_config(&base, r#"{ type = "umum" }"#);
        let (called, mut received) = tokio::sync::oneshot::channel();
        run_with_embed(&cfg, false, None, |_, _, _| {
            let _ = called.send(());
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(
            received.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed)
        );
        let folder = base.join("model");
        fs::create_dir_all(&folder).unwrap();
        cfg.makna = Some(crate::config::MaknaConfig {
            model: folder.clone(),
            ort_dylib: None,
            threads: 2,
            gpu: true,
        });
        // Model yang belum ada tidak menggagalkan ingest.
        run(&cfg, false, None).await.unwrap();
        crate::embed::write_test_artifacts(&folder);
        fs::write(
            folder.join("app-model.toml"),
            "name = 'uji'\npooling = 'cls'\ndim = 2\nnormalize = true\n",
        )
        .unwrap();
        let mut previous = None;
        for (i, rebuild) in [false, false, true].into_iter().enumerate() {
            if i == 1 {
                fs::write(base.join("docs/baru.txt"), "Potongan tambahan.").unwrap();
            }
            run_with_embed(&cfg, rebuild, None, move |data, cfg, full| {
                assert_eq!(full, rebuild);
                let (_version, index) = crate::eval::open_index(data)?;
                let lock = ingest_lock(data)?;
                drop(lock);
                let model = crate::embed::Model::load(&cfg.model)?;
                crate::embed::write_test_store_mode(data, &model, &index, full)
            })
            .await
            .unwrap();
            let model = crate::embed::Model::load(&folder).unwrap();
            let current = crate::embed::Store::current(&model.store_dir(&cfg.data_dir)).unwrap();
            if i > 0 {
                assert_ne!(previous.as_ref(), Some(&current));
            }
            previous = Some(current);
        }
        let _ = fs::remove_dir_all(&base);
    }

    fn user(department: &str) -> crate::acl::AuthUser {
        crate::acl::AuthUser {
            id: 1,
            email: "a@example.com".into(),
            name: "A".into(),
            department: Some(department.into()),
            role: crate::acl::Role::User,
        }
    }

    /// rel_path → (status, halaman, id)
    async fn rows(base: &Path) -> BTreeMap<String, (String, i64, i64)> {
        let db = Db::open(&base.join("data")).unwrap();
        db.call(|c| {
            let mut stmt = c.prepare("SELECT rel_path, status, pages, id FROM documents_v4")?;
            let rows =
                stmt.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?))))?;
            Ok(rows.collect::<rusqlite::Result<BTreeMap<_, _>>>()?)
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn ingest_lock_rejects_a_second_run_before_opening_the_database() {
        let base = std::env::temp_dir().join(format!("rag-hub-lock-{}", rand::random::<u64>()));
        let cfg = test_config(&base, r#"{ type = "umum" }"#);
        let lock = ingest_lock(&cfg.data_dir).unwrap();
        for rebuild in [false, true] {
            let error = run(&cfg, rebuild, None).await.unwrap_err();
            assert_eq!(
                error.to_string(),
                "Ingest lain atau eval berbasis vektor sedang berjalan. Tunggu sampai selesai lalu coba lagi."
            );
        }
        assert!(!cfg.data_dir.join("rag-hub.sqlite").exists());
        drop(lock);
        drop(ingest_lock(&cfg.data_dir).unwrap());
        fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn ingest_hides_stale_legacy_rows_and_reserves_new_ids() {
        let base = std::env::temp_dir().join(format!("rag-hub-legacy-{}", rand::random::<u64>()));
        let cfg = test_config(&base, r#"{ type = "umum" }"#);
        let docs = &cfg.sources[0].path;
        fs::create_dir_all(docs).unwrap();
        for file in ["ubah.txt", "hapus.txt", "tetap.txt", "baru.txt"] {
            fs::write(docs.join(file), "dokumen uji").unwrap();
            File::options()
                .write(true)
                .open(docs.join(file))
                .unwrap()
                .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000))
                .unwrap();
        }
        let root = fs::canonicalize(docs)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let db = Db::open(&cfg.data_dir).unwrap();
        db.call(move |c| {
            for file in ["ubah.txt", "hapus.txt", "tetap.txt", "hanya-v3.txt"] {
                c.execute(
                    "INSERT INTO documents (source, rel_path, title, kind, status, size, mtime, visibility, root, version, indexed_at)
                     VALUES ('Uji', ?1, ?1, 'txt', 'ok', 11, 1700000000000, '{\"type\":\"umum\"}', ?2, 3, 1)",
                    params![file, root],
                )?;
            }
            Ok(())
        }).await.unwrap();

        assert_eq!(run(&cfg, false, None).await.unwrap().read, 4);
        let indexed = rows(&base).await;
        assert_eq!(indexed["ubah.txt"].2, 1);
        assert_eq!(indexed["hapus.txt"].2, 2);
        assert_eq!(indexed["tetap.txt"].2, 3);
        let new_id = indexed["baru.txt"].2;
        assert!(new_id > 4);
        let next_legacy_id = db.call(|c| {
            c.execute(
                "INSERT INTO documents (source, rel_path, title, kind, status, size, mtime, visibility, version, indexed_at)
                 VALUES ('Lain', 'hapus.txt', 'lain', 'txt', 'ok', 0, 0, '{}', 3, 1)",
                [],
            )?;
            Ok(c.last_insert_rowid())
        }).await.unwrap();
        assert!(next_legacy_id > new_id);

        fs::write(docs.join("ubah.txt"), "dokumen uji sudah diubah").unwrap();
        fs::remove_file(docs.join("hapus.txt")).unwrap();
        assert_eq!(
            run(&cfg, false, None).await.unwrap(),
            Summary {
                read: 1,
                removed: 1
            }
        );
        let legacy = load_existing(&db, "documents").await.unwrap();
        for file in ["ubah.txt", "hapus.txt", "hanya-v3.txt"] {
            assert_eq!(legacy[&("Uji".into(), file.into())].status, status::PENDING);
        }
        assert_eq!(
            legacy[&("Uji".into(), "tetap.txt".into())].status,
            status::OK
        );
        drop(db);
        let _ = fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn first_build_waits_for_all_sources_and_resumes_committed_rows() {
        let base = std::env::temp_dir().join(format!("rag-hub-resume-{}", rand::random::<u64>()));
        let mut cfg = test_config(&base, r#"{ type = "umum" }"#);
        cfg.sources.push(SourceConfig {
            name: "Kedua".into(),
            path: base.join("kedua"),
            visibility: crate::acl::Visibility::Umum,
        });
        let dir = search::version_dir(&cfg.data_dir, INDEX_VERSION);
        let marker = dir.join(search::COMPLETE);
        fs::create_dir_all(&cfg.sources[0].path).unwrap();
        fs::write(cfg.sources[0].path.join("awal.jpg"), "gambar uji").unwrap();
        assert!(
            run(&cfg, false, None)
                .await
                .unwrap_err()
                .to_string()
                .contains("Index belum lengkap")
        );
        assert!(!marker.exists());
        assert_eq!(rows(&base).await["awal.jpg"].0, status::IMAGE);

        fs::create_dir_all(&cfg.sources[1].path).unwrap();
        fs::write(
            cfg.sources[1].path.join("akhir.txt"),
            "dokumen dari sumber kedua",
        )
        .unwrap();
        let vision = Llm::new(&cfg.azure);
        assert_eq!(run(&cfg, false, Some(&vision)).await.unwrap().read, 1);
        assert!(marker.is_file());

        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(run(&cfg, false, None).await.unwrap().read, 2);
        assert!(marker.is_file());
        let _ = fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn ingest_reads_only_what_changed() {
        let base = std::env::temp_dir().join(format!("rag-hub-ingest-{}", rand::random::<u32>()));
        let docs = base.join("docs");
        std::fs::create_dir_all(docs.join("Laporan")).unwrap();
        std::fs::write(
            docs.join("sop.txt"),
            "Prosedur kalibrasi mikrometer setiap enam bulan.",
        )
        .unwrap();
        let pdf = minimal_pdf(&[
            "Halaman satu: standar produk contoh DEMO.",
            "Halaman dua: hasil uji tekanan hidrostatik.",
        ]);
        std::fs::write(docs.join("Laporan/uji.pdf"), pdf).unwrap();
        let xlsx: crate::files::XlsxSpec = serde_json::from_value(serde_json::json!({
            "filename": "rekap", "sheets": [{ "name": "Stok",
            "columns": [{ "header": "Kode" }, { "header": "Nama" }],
            "rows": [["P-001", "produk contoh tiga perempat inci"]] }] }))
        .unwrap();
        std::fs::write(
            docs.join("rekap.xlsx"),
            crate::files::build_xlsx(&xlsx).unwrap(),
        )
        .unwrap();
        let docx: crate::docx::DocxSpec = serde_json::from_value(serde_json::json!({
            "filename": "surat",
            "blocks": [{ "type": "paragraph", "text": "Undangan rapat evaluasi pemasok generik." }] }))
        .unwrap();
        std::fs::write(
            docs.join("surat.docx"),
            crate::docx::build_docx(&docx).unwrap(),
        )
        .unwrap();
        std::fs::write(docs.join("foto.jpg"), b"bukan gambar").unwrap();

        let review = r#"{ type = "departemen", departments = ["Review"] }"#;
        let summary = run(&test_config(&base, review), false, None).await.unwrap();
        assert_eq!(
            summary,
            Summary {
                read: 5,
                removed: 0
            }
        );
        let found = rows(&base).await;
        let statuses: Vec<&str> = [
            "sop.txt",
            "Laporan/uji.pdf",
            "rekap.xlsx",
            "surat.docx",
            "foto.jpg",
        ]
        .iter()
        .map(|rel| found[*rel].0.as_str())
        .collect();
        assert_eq!(
            statuses,
            [
                status::OK,
                status::OK,
                status::OK,
                status::OK,
                status::IMAGE
            ]
        );
        assert_eq!(found["Laporan/uji.pdf"].1, 2);

        let index = DocIndex::open(&base.join("data")).unwrap();
        let find = |q: &str, dept: &str| {
            index.reload().unwrap();
            index
                .search(q, &user(dept), &["Uji".to_string()], 0, 5, false)
                .unwrap()
        };
        let hits = find("hidrostatik", "Review");
        assert_eq!(hits.len(), 1);
        assert_eq!(
            (
                hits[0].page,
                hits[0].source.as_str(),
                hits[0].title.as_str()
            ),
            (Some(2), "Uji", "uji")
        );
        assert_eq!(find("generik", "Review").len(), 1);
        assert_eq!(find("pemasok", "Review").len(), 1);
        assert!(find("hidrostatik", "Operasional").is_empty());

        // Tanpa perubahan, tidak ada yang dibaca ulang.
        assert_eq!(
            run(&test_config(&base, review), false, None).await.unwrap(),
            Summary::default()
        );
        // File berubah dibaca ulang; file yang hilang dihapus dari index.
        std::fs::write(
            docs.join("sop.txt"),
            "Prosedur kalibrasi jangka sorong setiap tiga bulan sekali.",
        )
        .unwrap();
        std::fs::remove_file(docs.join("foto.jpg")).unwrap();
        assert_eq!(
            run(&test_config(&base, review), false, None).await.unwrap(),
            Summary {
                read: 1,
                removed: 1
            }
        );
        assert!(find("mikrometer", "Review").is_empty());
        assert_eq!(find("sorong", "Review").len(), 1);
        // Hak akses berubah: semua file dibaca ulang dengan label baru.
        let umum = r#"{ type = "umum" }"#;
        assert_eq!(
            run(&test_config(&base, umum), false, None)
                .await
                .unwrap()
                .read,
            4
        );
        assert_eq!(find("hidrostatik", "Operasional").len(), 1);
        let error = run(&test_config(&base, umum), true, None)
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Index versi ini sedang dipakai oleh layanan, embed, atau eval. Tunggu embed dan eval selesai atau pindahkan layanan ke slot dengan versi index lain sebelum ingest --ulang."
        );
        assert_eq!(rows(&base).await["sop.txt"].0, status::OK);
        // Folder sumber sedang tidak bisa dibuka: tidak ada yang dibaca atau dihapus.
        // (Server tetap menerapkan hak akses dari config-nya sendiri; dites di tests.rs.)
        std::fs::rename(&docs, base.join("docs-off")).unwrap();
        let admin_only = r#"{ type = "admin" }"#;
        assert_eq!(
            run(&test_config(&base, admin_only), false, None)
                .await
                .unwrap(),
            Summary::default()
        );
        assert_eq!(rows(&base).await.len(), 4);
        std::fs::rename(base.join("docs-off"), &docs).unwrap();
        assert_eq!(
            run(&test_config(&base, admin_only), false, None)
                .await
                .unwrap()
                .read,
            4
        );
        assert!(find("hidrostatik", "Operasional").is_empty());
        assert_eq!(rows(&base).await["sop.txt"].0, status::OK);
        drop(index);
        // --ulang membaca ulang semua file tanpa mengganti ID dokumen.
        let ids =
            |m: &BTreeMap<String, (String, i64, i64)>| m.values().map(|v| v.2).collect::<Vec<_>>();
        let before = ids(&rows(&base).await);
        assert_eq!(
            run(&test_config(&base, umum), true, None)
                .await
                .unwrap()
                .read,
            4
        );
        assert_eq!(ids(&rows(&base).await), before);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn document_dates_skip_the_archive_prefix_and_csv_rows() {
        let text = |t: &str| Extracted {
            chunks: vec![(None, t.to_string())],
            ..Extracted::skipped(status::OK)
        };
        let sop = text("Rev 01 berlaku 2 Jan 2023\nRev 02 berlaku 5 Mei 2025");
        let date = document_date("deadbeef_SOP Pemeriksaan Barang", "pdf", &sop).unwrap();
        assert_eq!(date.date.to_string(), "2025-05-05");
        let titled = document_date("deadbeef_13 Feb 2025 Laporan", "pdf", &text("")).unwrap();
        assert_eq!(
            (titled.date.to_string(), titled.from),
            ("2025-02-13".to_string(), docdate::FROM_TITLE)
        );
        assert!(document_date("Rekap", "csv", &text("Tanggal,Nilai\n12/03/2025,42")).is_none());
    }

    #[tokio::test]
    async fn ingest_stores_document_dates_and_their_source() {
        let base = std::env::temp_dir().join(format!("rag-hub-ingest-{}", rand::random::<u32>()));
        let docs = base.join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(
            docs.join("Laporan Cacat (13 Feb 2025).txt"),
            "Hasil inspeksi produk.",
        )
        .unwrap();
        std::fs::write(
            docs.join("form.txt"),
            "FORM-DEMO-001 Rev. 03 Berlaku 02-02-2024\nPemeriksaan Kualitas produk contoh\nTanggal: 17/06/2025\nHasil inspeksi produk.",
        )
        .unwrap();
        std::fs::write(docs.join("catatan.txt"), "Catatan inspeksi tanpa tanggal.").unwrap();
        let review = r#"{ type = "departemen", departments = ["Review"] }"#;
        run(&test_config(&base, review), false, None).await.unwrap();

        let dates = || async {
            let db = Db::open(&base.join("data")).unwrap();
            db.call(|c| {
                let mut stmt = c.prepare(
                    "SELECT rel_path, doc_date, doc_date_from, date_version FROM documents_v4",
                )?;
                let rows = stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        (
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, Option<String>>(2)?,
                            r.get::<_, i64>(3)?,
                        ),
                    ))
                })?;
                Ok(rows.collect::<rusqlite::Result<BTreeMap<_, _>>>()?)
            })
            .await
            .unwrap()
        };
        let found = dates().await;
        let dated =
            |date: &str, from: &str| (Some(date.to_string()), Some(from.to_string()), DATE_VERSION);
        assert_eq!(
            found["Laporan Cacat (13 Feb 2025).txt"],
            dated("2025-02-13", "judul")
        );
        assert_eq!(found["form.txt"], dated("2025-06-17", "isi"));
        assert_eq!(found["catatan.txt"], (None, None, DATE_VERSION));

        // Dibaca ulang sekali: aturan tanggal lama (catatan) dan file yang sempat dibaca binary
        // tanpa tanggal dokumen (form). Hasil baca model (laporan) tidak dibaca ulang.
        let db = Db::open(&base.join("data")).unwrap();
        db.call(|c| {
            c.execute_batch(
                "UPDATE documents_v4 SET date_version = 0 WHERE rel_path = 'catatan.txt';
                 UPDATE documents_v4 SET indexed_at = indexed_at / 1000 WHERE rel_path = 'form.txt';
                 UPDATE documents_v4 SET status = 'vision', date_version = 0
                     WHERE rel_path = 'Laporan Cacat (13 Feb 2025).txt';",
            )?;
            Ok(())
        })
        .await
        .unwrap();
        drop(db);
        let summary = run(&test_config(&base, review), false, None).await.unwrap();
        assert_eq!(summary.read, 2);
        assert_eq!(dates().await["catatan.txt"].2, DATE_VERSION);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn archive_upload_date_fills_files_without_a_document_date() {
        let base = std::env::temp_dir().join(format!("rag-hub-upload-{}", rand::random::<u32>()));
        let docs = base.join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(
            docs.join("deadbeef_Catatan.txt"),
            "Catatan inspeksi tanpa tanggal.",
        )
        .unwrap();
        std::fs::write(
            docs.join("cafe0123_Form (13 Feb 2025).txt"),
            "Hasil inspeksi.",
        )
        .unwrap();
        std::fs::write(docs.join("Lain.txt"), "Catatan lain tanpa tanggal.").unwrap();
        let review = r#"{ type = "departemen", departments = ["Review"] }"#;
        run(&test_config(&base, review), false, None).await.unwrap();

        let dates = || async {
            let db = Db::open(&base.join("data")).unwrap();
            db.call(|c| {
                let mut stmt =
                    c.prepare("SELECT rel_path, doc_date, doc_date_from FROM documents_v4")?;
                let rows = stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        (
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, Option<String>>(2)?,
                        ),
                    ))
                })?;
                Ok(rows.collect::<rusqlite::Result<BTreeMap<_, _>>>()?)
            })
            .await
            .unwrap()
        };
        assert_eq!(dates().await["deadbeef_Catatan.txt"], (None, None));

        // Daftar tanggal upload muncul belakangan: hanya file tanpa tanggal dokumen yang punya
        // tanggal upload yang dibaca ulang. Tanggal dari judul tetap menang.
        std::fs::write(
            docs.join(ARCHIVE_UPLOAD_FILE),
            "# id8\ttanggal\ndeadbeef\t2025-06-01\ndeadbeef_Catatan.txt\t2020-01-01\n\tdeadbeef\t2020-01-01\ncafe0123\t2026-01-02\nrusak\t2025-01-01\nabcdef12\t2025-13-40\n",
        )
        .unwrap();
        let summary = run(&test_config(&base, review), false, None).await.unwrap();
        assert_eq!(summary.read, 1);
        let found = dates().await;
        let dated = |date: &str, from: &str| (Some(date.to_string()), Some(from.to_string()));
        assert_eq!(
            found["deadbeef_Catatan.txt"],
            dated("2025-06-01", "archive")
        );
        assert_eq!(
            found["cafe0123_Form (13 Feb 2025).txt"],
            dated("2025-02-13", "judul")
        );
        assert_eq!(found["Lain.txt"], (None, None));
        assert!(!found.contains_key(ARCHIVE_UPLOAD_FILE));
        // Sudah bertanggal: tidak dibaca ulang lagi.
        assert_eq!(
            run(&test_config(&base, review), false, None)
                .await
                .unwrap()
                .read,
            0
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn data_folder_is_never_indexed() {
        // Folder sumber yang keliru memuat folder data server: lampiran chat, database, dan
        // index di dalamnya tidak ikut terbaca, dan sumber di dalam folder data dilewati.
        let base = std::env::temp_dir().join(format!("rag-hub-datadir-{}", std::process::id()));
        let docs = base.join("docs");
        std::fs::create_dir_all(docs.join("data/files/7")).unwrap();
        std::fs::write(
            docs.join("sop.txt"),
            "Prosedur kalibrasi mesin roll setiap minggu.",
        )
        .unwrap();
        std::fs::write(
            docs.join("data/files/7/lampiran.txt"),
            "Lampiran chat milik user tujuh, bukan dokumen kantor.",
        )
        .unwrap();
        let text = format!(
            r#"data_dir = {data:?}
[azure]
endpoint = "http://127.0.0.1:9"
deployment = "x"
[[sources]]
name = "Uji"
path = {docs:?}
visibility = {{ type = "umum" }}
[[sources]]
name = "Lampiran"
path = {files:?}
visibility = {{ type = "umum" }}
"#,
            data = docs.join("data").to_string_lossy(),
            docs = docs.to_string_lossy(),
            files = docs.join("data/files").to_string_lossy(),
        );
        let cfg: Config = toml::from_str(&text).unwrap();
        assert!(
            run(&cfg, false, None)
                .await
                .unwrap_err()
                .to_string()
                .contains("Index belum lengkap")
        );
        assert!(
            !search::version_dir(&cfg.data_dir, INDEX_VERSION)
                .join(search::COMPLETE)
                .exists()
        );
        let db = Db::open(&docs.join("data")).unwrap();
        let indexed: Vec<(String, String)> = db
            .call(|c| {
                let mut stmt =
                    c.prepare("SELECT source, rel_path FROM documents_v4 ORDER BY rel_path")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .await
            .unwrap();
        assert_eq!(indexed, [("Uji".to_string(), "sop.txt".to_string())]);
        drop(db);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[tokio::test]
    async fn text_and_skipped_files() {
        let dir = std::env::temp_dir().join(format!("rag-hub-extract-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let txt = dir.join("catatan.txt");
        std::fs::write(&txt, "Jadwal kalibrasi alat ukur setiap enam bulan.").unwrap();
        let got = extract_file(txt, "txt".into(), 10).await;
        assert_eq!((got.status, got.chunks.len()), (status::OK, 1));
        assert_eq!(
            extract_file(dir.join("x.jpg"), "jpg".into(), 1)
                .await
                .status,
            status::IMAGE
        );
        assert_eq!(
            extract_file(dir.join("x.exe"), "exe".into(), 1)
                .await
                .status,
            status::UNSUPPORTED
        );
        assert_eq!(
            extract_file(dir.join("x.pdf"), "pdf".into(), MAX_FILE_BYTES + 1)
                .await
                .status,
            status::TOO_BIG
        );
        assert_eq!(
            extract_file(dir.join("hilang.pdf"), "pdf".into(), 1)
                .await
                .status,
            status::FAILED
        );
        // File rusak tidak menghentikan ingest dan tidak masuk index.
        let broken = dir.join("rusak.pdf");
        std::fs::write(&broken, b"%PDF-1.4 bukan pdf").unwrap();
        let got = extract_file(broken, "pdf".into(), 18).await;
        assert!(
            got.status != status::OK && got.chunks.is_empty(),
            "{}",
            got.status
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
