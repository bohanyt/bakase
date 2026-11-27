//! Index pencarian dokumen (tantivy) di `data/index-v<N>`. `ingest` menulis, server membaca.
//!
//! Setiap potongan membawa label hak akses dokumennya, jadi pencarian langsung menyaring
//! dokumen yang tidak boleh dibaca user. Pemanggil tetap mengecek ulang setiap hasil
//! dengan `acl::can_read` terhadap tabel `documents`.

use crate::acl::{AuthUser, Role, Visibility};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};
use tantivy::collector::{Count, DocSetCollector, TopDocs};
#[cfg(not(windows))]
use tantivy::directory::MmapDirectory;
use tantivy::query::{
    AllQuery, Bm25StatisticsProvider, BooleanQuery, EnableScoring, Occur, Query, QueryParser,
    RangeQuery, TermQuery,
};
use tantivy::schema::{
    Field, INDEXED, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions,
    Value,
};
use tantivy::snippet::SnippetGenerator;
use tantivy::tokenizer::{
    LowerCaser, RemoveLongFilter, TextAnalyzer, Token, TokenStream, Tokenizer,
};
use tantivy::{
    DocAddress, DocSet, Index, IndexReader, IndexWriter, ReloadPolicy, Searcher, TERMINATED,
    TantivyDocument, Term,
};

#[cfg(windows)]
mod windows_directory;
#[cfg(windows)]
use windows_directory::MmapDirectory;

/// Saat skema Tantivy berubah, perbarui juga tabel status, pembaca skema, dan fallback.
pub const INDEX_VERSION: i64 = 4;
pub const INDEX_TABLE: &str = "documents_v4";
const TOKENIZER: &str = "hub";
/// Panjang cuplikan di hasil pencarian UI.
const SNIPPET_CHARS: usize = 220;
/// Batas potongan dengan label tertolak per sumber yang dibaca satu per satu di
/// `DocIndex::refused`. Lebih dari ini, sumbernya disembunyikan seluruhnya.
const MAX_REFUSED_CHUNKS: usize = 10_000;

/// Jumlah semua potongan satu dokumen, dan potongan satu halaman (urutan, isi).
pub type ChunkPage = (usize, Vec<(usize, Hit)>);

/// Hasil `DocIndex::refused`.
#[derive(Debug, Default)]
pub struct Refused {
    /// Dokumen yang punya potongan dengan label index yang tidak boleh dibaca user.
    pub docs: HashSet<i64>,
    /// Sumber dengan terlalu banyak potongan seperti itu; disembunyikan seluruhnya.
    pub sources: HashSet<String>,
}

#[derive(Clone, Copy)]
pub struct Fields {
    chunk_id: Field,
    doc_id: Field,
    acl: Field,
    title: Field,
    path: Field,
    text: Field,
    page: Field,
    source: Field,
    file_type: Option<Field>,
    year: Option<Field>,
}

fn schema_v3() -> (tantivy::schema::SchemaBuilder, Fields) {
    let mut b = Schema::builder();
    let indexing = TextFieldIndexing::default()
        .set_tokenizer(TOKENIZER)
        .set_index_option(IndexRecordOption::WithFreqsAndPositions);
    let searchable = TextOptions::default().set_indexing_options(indexing);
    let fields = Fields {
        chunk_id: b.add_text_field("chunk_id", STRING | STORED),
        doc_id: b.add_u64_field("doc_id", INDEXED | STORED),
        acl: b.add_text_field("acl", STRING),
        title: b.add_text_field("title", searchable.clone().set_stored()),
        path: b.add_text_field("path", searchable.clone()),
        text: b.add_text_field("text", searchable.set_stored()),
        page: b.add_u64_field("page", STORED),
        source: b.add_text_field("source", STRING | STORED),
        file_type: None,
        year: None,
    };
    (b, fields)
}

fn schema_v4() -> (Schema, Fields) {
    let (mut b, mut fields) = schema_v3();
    fields.file_type = Some(b.add_text_field("file_type", STRING));
    fields.year = Some(b.add_i64_field("year", INDEXED));
    (b.build(), fields)
}

/// Kata dari huruf atau angka jadi satu token. Huruf Jepang, Cina, dan Korea ditulis
/// tanpa spasi, jadi dipotong per dua karakter (bigram) supaya kata di tengah kalimat
/// tetap ketemu.
#[derive(Clone, Default)]
struct HubTokenizer;

struct VecTokenStream {
    tokens: Vec<Token>,
    next: usize,
}

impl TokenStream for VecTokenStream {
    fn advance(&mut self) -> bool {
        if self.next < self.tokens.len() {
            self.next += 1;
            true
        } else {
            false
        }
    }

    fn token(&self) -> &Token {
        &self.tokens[self.next.saturating_sub(1)]
    }

    fn token_mut(&mut self) -> &mut Token {
        &mut self.tokens[self.next.saturating_sub(1)]
    }
}

impl Tokenizer for HubTokenizer {
    type TokenStream<'a> = VecTokenStream;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> VecTokenStream {
        VecTokenStream {
            tokens: tokenize(text),
            next: 0,
        }
    }
}

fn is_cjk(c: char) -> bool {
    is_japanese_script_block(c) || matches!(c as u32, 0xAC00..=0xD7AF)
}

pub(crate) fn is_japanese_script_block(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF     // hiragana, katakana
        | 0x3400..=0x4DBF   // kanji tambahan
        | 0x4E00..=0x9FFF   // kanji
        | 0xF900..=0xFAFF   // kanji kompatibel
        | 0xFF66..=0xFF9F) // katakana setengah lebar
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some(&(start, c)) = chars.peek() {
        if is_cjk(c) {
            let mut run: Vec<(usize, usize)> = Vec::new();
            while let Some(&(i, c)) = chars.peek() {
                if !is_cjk(c) {
                    break;
                }
                run.push((i, i + c.len_utf8()));
                chars.next();
            }
            if run.len() == 1 {
                spans.push(run[0]);
            }
            spans.extend(run.windows(2).map(|w| (w[0].0, w[1].1)));
        } else if c.is_alphanumeric() {
            let mut end = start;
            while let Some(&(i, c)) = chars.peek() {
                if !c.is_alphanumeric() || is_cjk(c) {
                    break;
                }
                end = i + c.len_utf8();
                chars.next();
            }
            spans.push((start, end));
        } else {
            chars.next();
        }
    }
    spans
        .into_iter()
        .enumerate()
        .map(|(position, (from, to))| Token {
            offset_from: from,
            offset_to: to,
            position,
            text: text[from..to].to_string(),
            position_length: 1,
        })
        .collect()
}

fn analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(HubTokenizer)
        .filter(RemoveLongFilter::limit(60))
        .filter(LowerCaser)
        .build()
}

/// Label hak akses di index: `umum`, `admin`, atau `dept:<nama>` per departemen.
pub fn acl_labels(visibility: &Visibility) -> Vec<String> {
    match visibility {
        Visibility::Umum => vec!["umum".into()],
        Visibility::Admin => vec!["admin".into()],
        Visibility::Departemen(list) => list
            .iter()
            .map(|d| d.trim())
            .filter(|d| !d.is_empty())
            .map(|d| format!("dept:{}", d.to_ascii_lowercase()))
            .collect(),
    }
}

/// Label yang boleh dibaca user, sama dengan aturan `acl::can_read`. `None` untuk admin.
fn readable_labels(user: &AuthUser) -> Option<Vec<String>> {
    if user.role == Role::Admin {
        return None;
    }
    let mut labels = vec!["umum".to_string()];
    labels.extend(
        user.departments
            .iter()
            .map(|d| d.trim())
            .filter(|d| !d.is_empty())
            .map(|d| format!("dept:{}", d.to_ascii_lowercase())),
    );
    Some(labels)
}

/// ID potongan: `<dokumen>-<urutan>-<sidik isi>`. Sidik isi membuat sitasi lama tidak
/// diam-diam menunjuk ke potongan lain setelah dokumennya berubah.
pub fn chunk_id(doc_id: i64, n: usize, text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    format!("{doc_id}-{n}-{}", hex::encode(&digest[..4]))
}

/// ID dokumen dari ID potongan, atau `None` kalau formatnya salah.
pub fn doc_of_chunk(chunk_id: &str) -> Option<i64> {
    let mut parts = chunk_id.split('-');
    let (doc, n, digest) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some()
        || n.parse::<u32>().is_err()
        || digest.len() != 8
        || !digest.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    doc.parse().ok().filter(|id: &i64| *id > 0)
}

/// Urutan potongan di dokumennya, dari ID potongan.
fn chunk_number(chunk_id: &str) -> Option<usize> {
    chunk_id.split('-').nth(1)?.parse().ok()
}

/// Satu potongan untuk ditulis ke index.
pub struct NewChunk<'a> {
    pub doc_id: i64,
    pub n: usize,
    pub title: &'a str,
    pub rel_path: &'a str,
    pub source: &'a str,
    pub page: Option<u32>,
    pub text: &'a str,
    pub acl: &'a [String],
    pub file_type: &'a str,
    pub years: &'a [i64],
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct SearchFilters {
    pub source: Option<String>,
    pub file_type: Option<String>,
    pub year: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Segment {
    pub text: String,
    pub hit: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub chunk_id: String,
    pub doc_id: i64,
    pub title: String,
    pub source: String,
    pub page: Option<u32>,
    pub text: String,
    /// Cuplikan dengan kata yang cocok ditandai; hanya untuk pencarian di UI.
    pub snippet: Vec<Segment>,
}

pub const COMPLETE: &str = "complete";

pub fn version_dir(data: &Path, version: i64) -> PathBuf {
    data.join(format!("index-v{version}"))
}

// Di luar folder index agar file lock tetap sama selama --ulang.
pub fn version_lock(data: &Path) -> Result<File> {
    Ok(File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(data.join(format!("index-v{INDEX_VERSION}.lock")))?)
}

// Skor BM25 untuk gabungan hanya memakai statistik dari potongan yang lolos filter.
struct VisibleStats<'a> {
    snapshot: &'a Searcher,
    eligible: &'a HashSet<DocAddress>,
    tokens: RefCell<HashMap<Field, u64>>,
}

impl Bm25StatisticsProvider for VisibleStats<'_> {
    fn total_num_tokens(&self, field: Field) -> tantivy::Result<u64> {
        if let Some(&total) = self.tokens.borrow().get(&field) {
            return Ok(total);
        }
        let readers = self
            .snapshot
            .segment_readers()
            .iter()
            .map(|segment| segment.fieldnorms_readers().get_field(field))
            .collect::<tantivy::Result<Vec<_>>>()?;
        let total = self
            .eligible
            .iter()
            .map(|address| {
                readers[address.segment_ord as usize]
                    .as_ref()
                    .map_or(0, |reader| u64::from(reader.fieldnorm(address.doc_id)))
            })
            .sum::<u64>()
            .max(1);
        self.tokens.borrow_mut().insert(field, total);
        Ok(total)
    }

    fn total_num_docs(&self) -> tantivy::Result<u64> {
        Ok(self.eligible.len() as u64)
    }

    fn doc_freq(&self, term: &Term) -> tantivy::Result<u64> {
        let mut count = 0;
        for (ord, segment) in self.snapshot.segment_readers().iter().enumerate() {
            let inverted = segment.inverted_index(term.field())?;
            if let Some(mut postings) = inverted.read_postings(term, IndexRecordOption::Basic)? {
                let mut id = postings.doc();
                while id != TERMINATED {
                    count += u64::from(self.eligible.contains(&DocAddress::new(ord as u32, id)));
                    id = postings.advance();
                }
            }
        }
        Ok(count)
    }
}

/// Sisa file sementara index dari proses yang mati. Hanya dipanggil pemegang lock penulis.
pub(crate) fn remove_stale_temps(dir: &Path) {
    #[cfg(windows)]
    windows_directory::remove_stale_temps(dir);
    #[cfg(not(windows))]
    let _ = dir;
}

pub struct DocIndex {
    index: Index,
    reader: IndexReader,
    fields: Fields,
    table: &'static str,
    _version_lock: Option<File>,
}

impl DocIndex {
    pub fn open(data: &Path) -> Result<Self> {
        let current = version_dir(data, INDEX_VERSION);
        if current.join(COMPLETE).is_file() {
            match Self::open_marked(data, &current) {
                Ok(Some(index)) => return Ok(index),
                Ok(None) => {}
                Err(error) => tracing::warn!(
                    %error,
                    "Index v{INDEX_VERSION} gagal dibuka. Mencoba index sebelumnya."
                ),
            }
        }
        if let Some(index) = Self::open_previous(data)? {
            tracing::warn!(
                "Index v{INDEX_VERSION} belum siap. Memakai index lama; filter untuk kolom yang belum ada tidak menemukan hasil. Jalankan ingest lalu pindah slot."
            );
            return Ok(index);
        }
        tracing::info!("Belum ada index lengkap. Jalankan ingest lalu pindah slot.");
        let (schema, fields) = schema_v4();
        Self::from_index(Index::create_in_ram(schema), fields)
    }

    fn open_marked(data: &Path, current: &Path) -> Result<Option<Self>> {
        let lock = version_lock(data)?;
        match lock.try_lock_shared() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => return Ok(None),
            Err(fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        if !current.join(COMPLETE).is_file() {
            return Ok(None);
        }
        let mut index = Self::open_current(current, false)?;
        index._version_lock = Some(lock);
        Ok(Some(index))
    }

    pub fn open_previous(data: &Path) -> Result<Option<Self>> {
        let previous = version_dir(data, 3);
        let legacy = data.join("index");
        let dir = if previous.join(COMPLETE).is_file() {
            previous
        } else {
            legacy
        };
        if !dir.join("meta.json").is_file() {
            return Ok(None);
        }
        let index = Index::open_in_dir(dir)?;
        let (builder, fields) = schema_v3();
        anyhow::ensure!(
            index.schema() == builder.build(),
            "skema index dokumen lama tidak dikenal"
        );
        Self::from_index(index, fields).map(Some)
    }

    pub fn open_current(dir: &Path, create: bool) -> Result<Self> {
        Self::open_current_with_policy(dir, create, ReloadPolicy::OnCommitWithDelay)
    }

    pub(crate) fn open_for_ingest(dir: &Path, create: bool) -> Result<Self> {
        // Ingest only reads OCR from the snapshot taken before it writes.
        Self::open_current_with_policy(dir, create, ReloadPolicy::Manual)
    }

    fn open_current_with_policy(dir: &Path, create: bool, policy: ReloadPolicy) -> Result<Self> {
        let (schema, fields) = schema_v4();
        let index = if create {
            fs::create_dir_all(dir)?;
            Index::open_or_create(MmapDirectory::open(dir)?, schema.clone())?
        } else {
            Index::open(MmapDirectory::open(dir)?)?
        };
        anyhow::ensure!(
            index.schema() == schema,
            "skema index v{INDEX_VERSION} tidak cocok"
        );
        let mut index = Self::from_index_with_policy(index, fields, policy)?;
        index.table = INDEX_TABLE;
        Ok(index)
    }

    pub fn table(&self) -> &'static str {
        self.table
    }

    #[cfg(test)]
    pub fn create_legacy_fixture(data: &Path, chunk: NewChunk<'_>) -> Result<()> {
        let dir = data.join("index");
        fs::create_dir_all(&dir)?;
        let (builder, fields) = schema_v3();
        let index = Self::from_index(Index::create_in_dir(&dir, builder.build())?, fields)?;
        let mut writer = index.index.writer_with_num_threads(1, 20_000_000)?;
        index.add_chunk(&writer, chunk)?;
        writer.commit()?;
        writer.wait_merging_threads()?;
        Ok(())
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        let (schema, fields) = schema_v4();
        Self::from_index(Index::create_in_ram(schema), fields)
    }

    #[cfg(test)]
    pub(crate) fn in_memory_v4() -> Result<Self> {
        let mut index = Self::in_memory()?;
        index.table = INDEX_TABLE;
        Ok(index)
    }

    fn from_index(index: Index, fields: Fields) -> Result<Self> {
        Self::from_index_with_policy(index, fields, ReloadPolicy::OnCommitWithDelay)
    }

    fn from_index_with_policy(index: Index, fields: Fields, policy: ReloadPolicy) -> Result<Self> {
        index.tokenizers().register(TOKENIZER, analyzer());
        let reader = index.reader_builder().reload_policy(policy).try_into()?;
        Ok(Self {
            index,
            reader,
            fields,
            table: "documents",
            _version_lock: None,
        })
    }

    /// Penulis index. Hanya satu yang boleh terbuka; `ingest` kedua akan gagal di sini.
    pub fn writer(&self) -> Result<IndexWriter> {
        anyhow::ensure!(
            self.fields.file_type.is_some(),
            "index lama hanya dibaca; jalankan ingest untuk membangun index versi ini"
        );
        self.index
            .writer_with_num_threads(2, 100_000_000)
            .context("index sedang ditulis proses lain (ingest lain masih jalan?)")
    }

    /// Membaca ulang index sekarang, tanpa menunggu pemantau file.
    pub fn reload(&self) -> Result<()> {
        Ok(self.reader.reload()?)
    }

    pub(crate) fn commit_opstamp(&self) -> Result<tantivy::Opstamp> {
        Ok(self.index.load_metas()?.opstamp)
    }

    pub fn add_chunk(&self, writer: &IndexWriter, c: NewChunk) -> Result<()> {
        let f = self.fields;
        let mut doc = TantivyDocument::default();
        doc.add_text(f.chunk_id, chunk_id(c.doc_id, c.n, c.text));
        doc.add_u64(f.doc_id, c.doc_id as u64);
        for label in c.acl {
            doc.add_text(f.acl, label);
        }
        doc.add_text(f.title, c.title);
        doc.add_text(f.path, c.rel_path);
        doc.add_text(f.text, c.text);
        if let Some(page) = c.page {
            doc.add_u64(f.page, page.into());
        }
        doc.add_text(f.source, c.source);
        if let Some(field) = f.file_type {
            doc.add_text(field, c.file_type);
        }
        if let Some(field) = f.year {
            for &year in c.years {
                doc.add_i64(field, year);
            }
        }
        writer.add_document(doc)?;
        Ok(())
    }

    pub(crate) fn visit_chunks(&self, mut visit: impl FnMut(Hit) -> Result<()>) -> Result<()> {
        self.visit_snapshot(&self.snapshot(), |_, hit| visit(hit))
    }

    pub(crate) fn visit_snapshot(
        &self,
        snapshot: &Searcher,
        mut visit: impl FnMut(DocAddress, Hit) -> Result<()>,
    ) -> Result<()> {
        let unreadable = || anyhow::anyhow!("Potongan index gagal dibaca atau tidak valid.");
        for (ord, segment) in snapshot.segment_readers().iter().enumerate() {
            let store = segment.get_store_reader(1).map_err(|_| unreadable())?;
            let ids = (0..segment.max_doc()).filter(|&id| !segment.is_deleted(id));
            for (id, doc) in ids.zip(store.iter::<TantivyDocument>(segment.alive_bitset())) {
                let hit = self
                    .hit(&doc.map_err(|_| unreadable())?)
                    .ok_or_else(unreadable)?;
                visit(DocAddress::new(ord as u32, id), hit)?;
            }
        }
        Ok(())
    }

    /// Menghapus semua potongan satu dokumen (berlaku setelah commit).
    pub fn delete_document(&self, writer: &IndexWriter, doc_id: i64) {
        writer.delete_term(Term::from_field_u64(self.fields.doc_id, doc_id as u64));
    }

    /// Snapshot index saat ini. Pencarian beberapa halaman memakai snapshot yang sama,
    /// supaya commit ingest di tengah jalan tidak membuat hasil ganda atau terlewat.
    pub fn snapshot(&self) -> Searcher {
        self.reader.searcher()
    }

    /// Seperti `search_in`, dengan snapshot terbaru.
    #[cfg(test)]
    pub fn search(
        &self,
        query: &str,
        user: &AuthUser,
        sources: &[String],
        offset: usize,
        limit: usize,
        snippets: bool,
    ) -> Result<Vec<Hit>> {
        self.search_in(
            &self.snapshot(),
            query,
            user,
            sources,
            offset,
            limit,
            snippets,
            &SearchFilters::default(),
        )
    }

    /// Potongan paling cocok yang boleh dibaca `user`, urut dari skor tertinggi. Hanya dari
    /// `sources`, yaitu sumber yang boleh dibaca user menurut config server, supaya hasil
    /// yang nanti ditolak tidak menutupi hasil yang boleh dibaca.
    #[allow(clippy::too_many_arguments)]
    pub fn search_in(
        &self,
        searcher: &Searcher,
        query: &str,
        user: &AuthUser,
        sources: &[String],
        offset: usize,
        limit: usize,
        snippets: bool,
        filters: &SearchFilters,
    ) -> Result<Vec<Hit>> {
        let Some(mut clauses) = self.filter_clauses(sources, filters) else {
            return Ok(Vec::new());
        };
        let f = self.fields;
        let mut parser = QueryParser::for_index(&self.index, vec![f.title, f.path, f.text]);
        parser.set_field_boost(f.title, 2.0);
        let (text_query, _) = parser.parse_query_lenient(query);
        clauses.insert(0, (Occur::Must, text_query.box_clone()));
        let query = self.readable(Box::new(BooleanQuery::new(clauses)), user);
        let top = searcher.search(
            &*query,
            &TopDocs::with_limit(limit.max(1))
                .and_offset(offset)
                .order_by_score(),
        )?;
        let generator = if snippets {
            let mut g = SnippetGenerator::create(searcher, &*text_query, f.text)?;
            g.set_max_num_chars(SNIPPET_CHARS);
            Some(g)
        } else {
            None
        };
        let mut hits = Vec::with_capacity(top.len());
        for (_, address) in top {
            let doc: TantivyDocument = searcher.doc(address)?;
            if let Some(mut hit) = self.hit(&doc) {
                if let Some(g) = &generator {
                    hit.snippet = snippet(g, &hit.text);
                }
                hits.push(hit);
            }
        }
        Ok(hits)
    }

    fn filter_clauses(
        &self,
        sources: &[String],
        filters: &SearchFilters,
    ) -> Option<Vec<(Occur, Box<dyn Query>)>> {
        let sources = sources
            .iter()
            .filter(|name| filters.source.as_ref().is_none_or(|source| *name == source));
        let in_sources: Vec<(Occur, Box<dyn Query>)> = sources
            .map(|name| {
                let term = Term::from_field_text(self.fields.source, name);
                let q: Box<dyn Query> = Box::new(TermQuery::new(term, IndexRecordOption::Basic));
                (Occur::Should, q)
            })
            .collect();
        if in_sources.is_empty() {
            return None;
        }
        let f = self.fields;
        let mut clauses: Vec<(Occur, Box<dyn Query>)> =
            vec![(Occur::Must, Box::new(BooleanQuery::new(in_sources)))];
        if let Some(file_type) = &filters.file_type {
            let field = f.file_type?;
            let term = Term::from_field_text(field, file_type);
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(term, IndexRecordOption::Basic)),
            ));
        }
        if let Some(year) = filters.year {
            let field = f.year?;
            let term = Term::from_field_i64(field, year);
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(term, IndexRecordOption::Basic)),
            ));
        }
        Some(clauses)
    }

    pub(crate) fn semantic_addresses(
        &self,
        snapshot: &Searcher,
        user: &AuthUser,
        sources: &[String],
        filters: &SearchFilters,
    ) -> Result<(HashSet<DocAddress>, HashSet<DocAddress>)> {
        let Some(clauses) = self.filter_clauses(sources, filters) else {
            return Ok(Default::default());
        };
        let query = self.readable(Box::new(BooleanQuery::new(clauses)), user);
        let allowed = snapshot.search(&*query, &DocSetCollector)?;
        let refused = match self.refused_by(Box::new(AllQuery), user) {
            Some(query) => snapshot.search(&*query, &DocSetCollector)?,
            None => HashSet::new(),
        };
        Ok((allowed, refused))
    }

    pub(crate) fn hit_at(&self, snapshot: &Searcher, address: DocAddress) -> Result<Hit> {
        let doc: TantivyDocument = snapshot.doc(address)?;
        self.hit(&doc).context("Potongan index tidak valid.")
    }

    pub(crate) fn keyword_scores(
        &self,
        snapshot: &Searcher,
        query: &str,
        eligible: &HashSet<DocAddress>,
    ) -> Result<Vec<(f32, DocAddress)>> {
        if eligible.is_empty() {
            return Ok(Vec::new());
        }
        let f = self.fields;
        let mut parser = QueryParser::for_index(&self.index, vec![f.title, f.path, f.text]);
        parser.set_field_boost(f.title, 2.0);
        let (query, _) = parser.parse_query_lenient(query);
        let stats = VisibleStats {
            snapshot,
            eligible,
            tokens: RefCell::default(),
        };
        let weight = query.weight(EnableScoring::enabled_from_statistics_provider(
            &stats, snapshot,
        ))?;
        let mut out = Vec::new();
        for (ord, segment) in snapshot.segment_readers().iter().enumerate() {
            let mut scorer = weight.scorer(segment, 1.0)?;
            let mut id = scorer.doc();
            while id != TERMINATED {
                let address = DocAddress::new(ord as u32, id);
                if eligible.contains(&address) {
                    out.push((scorer.score(), address));
                }
                id = scorer.advance();
            }
        }
        Ok(out)
    }

    /// Potongan dengan salah satu label `labels`.
    fn with_labels(&self, labels: &[String]) -> Box<dyn Query> {
        let allowed: Vec<(Occur, Box<dyn Query>)> = labels
            .iter()
            .map(|label| {
                let term = Term::from_field_text(self.fields.acl, label);
                let q: Box<dyn Query> = Box::new(TermQuery::new(term, IndexRecordOption::Basic));
                (Occur::Should, q)
            })
            .collect();
        Box::new(BooleanQuery::new(allowed))
    }

    /// `query` dibatasi ke potongan yang labelnya boleh dibaca `user`.
    fn readable(&self, query: Box<dyn Query>, user: &AuthUser) -> Box<dyn Query> {
        let Some(labels) = readable_labels(user) else {
            return query;
        };
        Box::new(BooleanQuery::new(vec![
            (Occur::Must, query),
            (Occur::Must, self.with_labels(&labels)),
        ]))
    }

    /// `query` dibatasi ke potongan yang labelnya TIDAK boleh dibaca `user`. `None` untuk
    /// admin (semua boleh).
    fn refused_by(&self, query: Box<dyn Query>, user: &AuthUser) -> Option<Box<dyn Query>> {
        let labels = readable_labels(user)?;
        Some(Box::new(BooleanQuery::new(vec![
            (Occur::Must, query),
            (Occur::MustNot, self.with_labels(&labels)),
        ])))
    }

    /// Dokumen di `sources` yang punya potongan dengan label index yang tidak boleh dibaca
    /// `user`. Biasanya kosong, karena label index dan baris `documents` ditulis dari config
    /// yang sama; bisa terisi kalau ingest berhenti di antara commit index dan penulisan
    /// baris, atau config ingest berbeda dari config server. Daftar dokumen menyembunyikan
    /// dokumen ini, supaya judul dan jumlahnya tidak bocor. Sumber dengan terlalu banyak
    /// potongan seperti itu disembunyikan seluruhnya (tanpa membaca setiap potongan).
    pub fn refused(&self, user: &AuthUser, sources: &[String]) -> Result<Refused> {
        let mut out = Refused::default();
        let f = self.fields;
        let searcher = self.reader.searcher();
        for name in sources {
            let term = Term::from_field_text(f.source, name);
            let Some(query) = self.refused_by(
                Box::new(TermQuery::new(term, IndexRecordOption::Basic)),
                user,
            ) else {
                return Ok(out);
            };
            let n = searcher.search(&*query, &Count)?;
            if n == 0 {
                continue;
            }
            if n > MAX_REFUSED_CHUNKS {
                out.sources.insert(name.clone());
                continue;
            }
            for address in searcher.search(&*query, &DocSetCollector)? {
                let doc: TantivyDocument = searcher.doc(address)?;
                if let Some(id) = doc
                    .get_first(f.doc_id)
                    .and_then(|v| v.as_u64())
                    .and_then(|v| i64::try_from(v).ok())
                {
                    out.docs.insert(id);
                }
            }
        }
        Ok(out)
    }

    /// Satu potongan berdasarkan ID-nya, hanya kalau labelnya di index boleh dibaca
    /// `user`. Pemanggil tetap mengecek dokumennya di tabel `documents`, jadi isi lama
    /// yang belum terganti di index tidak lolos dengan hak akses baru, atau sebaliknya.
    pub fn chunk(&self, chunk_id: &str, user: &AuthUser) -> Result<Option<Hit>> {
        let term = Term::from_field_text(self.fields.chunk_id, chunk_id);
        let query = self.readable(
            Box::new(TermQuery::new(term, IndexRecordOption::Basic)),
            user,
        );
        let searcher = self.reader.searcher();
        let top = searcher.search(&*query, &TopDocs::with_limit(1).order_by_score())?;
        let Some((_, address)) = top.into_iter().next() else {
            return Ok(None);
        };
        let doc: TantivyDocument = searcher.doc(address)?;
        Ok(self.hit(&doc))
    }

    /// Dokumen `doc_id` punya potongan yang labelnya di index tidak boleh dibaca `user`. Route
    /// dokumen (preview, sitasi, unduhan) lalu menjawab seperti dokumen yang tidak ada.
    pub fn refuses(&self, doc_id: i64, user: &AuthUser) -> Result<bool> {
        self.refuses_in(&self.reader.searcher(), doc_id, user)
    }

    /// Untuk unduhan, index harus memuat entri dan semua labelnya harus mengizinkan.
    pub fn has_readable_entry(&self, doc_id: i64, user: &AuthUser) -> Result<bool> {
        let snapshot = self.snapshot();
        let term = Term::from_field_u64(self.fields.doc_id, doc_id as u64);
        let query = TermQuery::new(term, IndexRecordOption::Basic);
        Ok(!self.refuses_in(&snapshot, doc_id, user)? && snapshot.search(&query, &Count)? > 0)
    }

    /// Dari `ids`, dokumen yang punya potongan dengan label index yang menolak `user`, di
    /// snapshot `searcher`.
    pub fn refusing_in(
        &self,
        searcher: &Searcher,
        ids: &[i64],
        user: &AuthUser,
    ) -> Result<HashSet<i64>> {
        let mut out = HashSet::new();
        for &id in ids {
            if self.refuses_in(searcher, id, user)? {
                out.insert(id);
            }
        }
        Ok(out)
    }

    fn refuses_in(&self, searcher: &Searcher, doc_id: i64, user: &AuthUser) -> Result<bool> {
        let term = Term::from_field_u64(self.fields.doc_id, doc_id as u64);
        let Some(query) = self.refused_by(
            Box::new(TermQuery::new(term, IndexRecordOption::Basic)),
            user,
        ) else {
            return Ok(false);
        };
        Ok(searcher.search(&*query, &Count)? > 0)
    }

    /// Potongan urutan `from..from + limit` satu dokumen, urut sesuai isinya, beserta jumlah
    /// semua potongannya. `None` kalau ada potongan dokumen ini yang labelnya di index tidak
    /// boleh dibaca `user`: dokumennya dianggap tidak ada, sama seperti di daftar dokumen.
    /// Jadi semua potongan yang dihitung boleh dibaca, dan urutannya `0..total` tanpa celah.
    /// Pemanggil tetap mengecek dokumennya di tabel `documents`.
    pub fn document_chunks(
        &self,
        doc_id: i64,
        user: &AuthUser,
        from: usize,
        limit: usize,
    ) -> Result<Option<ChunkPage>> {
        let f = self.fields;
        let of_doc = || self.document_chunks_query(doc_id);
        let searcher = self.reader.searcher();
        if self.refuses_in(&searcher, doc_id, user)? {
            return Ok(None);
        }
        let total = searcher.search(&*of_doc(), &Count)?;
        // Urutan potongan hanya ada di ID-nya (`<dokumen>-<urutan>-<sidik>`), jadi setiap
        // urutan dicari lewat rentang term. Sidiknya huruf hex, semuanya di bawah "g".
        let ranges: Vec<(Occur, Box<dyn Query>)> = (from..from.saturating_add(limit).min(total))
            .map(|n| {
                let lower = Term::from_field_text(f.chunk_id, &format!("{doc_id}-{n}-"));
                let upper = Term::from_field_text(f.chunk_id, &format!("{doc_id}-{n}-g"));
                let q: Box<dyn Query> = Box::new(RangeQuery::new(
                    Bound::Included(lower),
                    Bound::Excluded(upper),
                ));
                (Occur::Should, q)
            })
            .collect();
        if ranges.is_empty() {
            return Ok(Some((total, Vec::new())));
        }
        let query = self.readable(
            Box::new(BooleanQuery::new(vec![
                (Occur::Must, of_doc()),
                (Occur::Must, Box::new(BooleanQuery::new(ranges))),
            ])),
            user,
        );
        let top = searcher.search(&*query, &TopDocs::with_limit(limit).order_by_score())?;
        let chunks =
            self.collect_document_chunks(&searcher, top.into_iter().map(|(_, addr)| addr), None)?;
        Ok(Some((total, chunks)))
    }

    pub fn document_page_chunks(
        &self,
        doc_id: i64,
        user: &AuthUser,
        page: u32,
        limit: usize,
    ) -> Result<Option<ChunkPage>> {
        let searcher = self.reader.searcher();
        if self.refuses_in(&searcher, doc_id, user)? {
            return Ok(None);
        }
        let query = self.readable(self.document_chunks_query(doc_id), user);
        // Nomor halaman hanya disimpan di index lama. Cari di dokumen ini saja.
        let addresses = searcher.search(&*query, &DocSetCollector)?;
        let mut chunks = self.collect_document_chunks(&searcher, addresses, Some(page))?;
        let total = chunks.len();
        chunks.truncate(limit);
        Ok(Some((total, chunks)))
    }

    fn document_chunks_query(&self, doc_id: i64) -> Box<dyn Query> {
        let f = self.fields;
        let term = Term::from_field_u64(f.doc_id, doc_id as u64);
        let metadata = Term::from_field_text(f.chunk_id, &chunk_id(doc_id, 0, ""));
        Box::new(BooleanQuery::new(vec![
            (
                Occur::Must,
                Box::new(TermQuery::new(term, IndexRecordOption::Basic)),
            ),
            (
                Occur::MustNot,
                Box::new(TermQuery::new(metadata, IndexRecordOption::Basic)),
            ),
        ]))
    }

    fn collect_document_chunks(
        &self,
        searcher: &Searcher,
        addresses: impl IntoIterator<Item = DocAddress>,
        page: Option<u32>,
    ) -> Result<Vec<(usize, Hit)>> {
        let mut out = Vec::new();
        for address in addresses {
            let doc: TantivyDocument = searcher.doc(address)?;
            if let Some(page) = page
                && doc.get_first(self.fields.page).and_then(|v| v.as_u64()) != Some(page.into())
            {
                continue;
            }
            if let Some(hit) = self.hit(&doc)
                && let Some(n) = chunk_number(&hit.chunk_id)
            {
                out.push((n, hit));
            }
        }
        out.sort_by_key(|(n, _)| *n);
        out.dedup_by_key(|(n, _)| *n);
        Ok(out)
    }

    fn hit(&self, doc: &TantivyDocument) -> Option<Hit> {
        let f = self.fields;
        let text = |field: Field| {
            doc.get_first(field)
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        Some(Hit {
            chunk_id: text(f.chunk_id)?,
            doc_id: i64::try_from(doc.get_first(f.doc_id)?.as_u64()?).ok()?,
            title: text(f.title).unwrap_or_default(),
            source: text(f.source).unwrap_or_default(),
            page: doc
                .get_first(f.page)
                .and_then(|v| v.as_u64())
                .and_then(|p| u32::try_from(p).ok()),
            text: text(f.text).unwrap_or_default(),
            snippet: Vec::new(),
        })
    }
}

/// Cuplikan dengan bagian yang cocok ditandai. Kalau yang cocok hanya judulnya,
/// cuplikan diambil dari awal potongan.
fn snippet(generator: &SnippetGenerator, text: &str) -> Vec<Segment> {
    let s = generator.snippet(text);
    let fragment = s.fragment();
    if fragment.trim().is_empty() {
        let start: String = text.chars().take(SNIPPET_CHARS).collect();
        let more = start.len() < text.len();
        return vec![Segment {
            text: if more { format!("{start}…") } else { start },
            hit: false,
        }];
    }
    let mut out = Vec::new();
    let mut at = 0;
    for range in s.highlighted() {
        if range.start < at || range.end > fragment.len() {
            continue;
        }
        if range.start > at {
            out.push(Segment {
                text: fragment[at..range.start].to_string(),
                hit: false,
            });
        }
        out.push(Segment {
            text: fragment[range.clone()].to_string(),
            hit: true,
        });
        at = range.end;
    }
    if at < fragment.len() {
        out.push(Segment {
            text: fragment[at..].to_string(),
            hit: false,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn words(text: &str) -> Vec<String> {
        tokenize(text).into_iter().map(|t| t.text).collect()
    }

    fn user(department: Option<&str>, role: Role) -> AuthUser {
        AuthUser {
            id: 1,
            email: "a@example.com".into(),
            name: "A".into(),
            departments: department.map(Into::into).into_iter().collect(),
            role,
        }
    }

    #[test]
    fn tokenizer_splits_words_and_cjk_bigrams() {
        assert_eq!(
            words("Widget DEMO-13A, ø25.4 mm"),
            ["Widget", "DEMO", "13A", "ø25", "4", "mm"]
        );
        assert_eq!(words("溶接検査"), ["溶接", "接検", "検査"]);
        assert_eq!(words("検 査"), ["検", "査"]);
        assert_eq!(
            words("NG品は再検査"),
            ["NG", "品は", "は再", "再検", "検査"]
        );
        assert!(words("  — ** ").is_empty());
    }

    #[test]
    fn chunk_ids_round_trip() {
        let id = chunk_id(42, 3, "isi");
        assert_eq!(doc_of_chunk(&id), Some(42));
        // Isi berubah, ID berubah.
        assert_ne!(id, chunk_id(42, 3, "isi baru"));
        assert_eq!(id, chunk_id(42, 3, "isi"));
        for bad in [
            "",
            "42",
            "42-3",
            "-3-abcdef12",
            "x-1-abcdef12",
            "42-x-abcdef12",
            "0-1-abcdef12",
            "1-2-abc",
            "1-2-zzzzzzzz",
            "1-2-abcdef12-4",
        ] {
            assert_eq!(doc_of_chunk(bad), None, "{bad}");
        }
    }

    #[test]
    fn acl_labels_follow_can_read() {
        let review = Visibility::Departemen(vec![" Review ".into(), "".into()]);
        assert_eq!(acl_labels(&review), ["dept:review"]);
        assert_eq!(
            readable_labels(&user(Some("review"), Role::User)).unwrap(),
            ["umum", "dept:review"]
        );
        assert_eq!(readable_labels(&user(None, Role::User)).unwrap(), ["umum"]);
        assert!(readable_labels(&user(None, Role::Admin)).is_none());
    }

    fn add(
        index: &DocIndex,
        writer: &IndexWriter,
        doc_id: i64,
        visibility: &Visibility,
        text: &str,
    ) {
        index
            .add_chunk(
                writer,
                NewChunk {
                    doc_id,
                    n: 0,
                    title: &format!("Dokumen {doc_id}"),
                    rel_path: "Standar/dokumen.pdf",
                    source: "Uji",
                    page: Some(2),
                    text,
                    acl: &acl_labels(visibility),
                    file_type: "PDF",
                    years: &[],
                },
            )
            .unwrap();
    }

    #[test]
    fn filters_narrow_search_without_widening_access() {
        let index = DocIndex::in_memory().unwrap();
        let mut writer = index.writer().unwrap();
        for (doc_id, source, file_type, years) in [
            (1, "Review", "Excel", &[2025, 2026][..]),
            (2, "Review", "PDF", &[2025][..]),
            (3, "Umum", "Excel", &[2025][..]),
            (4, "Review", "Excel", &[2024][..]),
            (5, "Review", "Excel", &[2025][..]),
            (6, "Khusus", "Excel", &[2025][..]),
        ] {
            let visibility = if doc_id == 5 {
                Visibility::Admin
            } else {
                Visibility::Umum
            };
            index
                .add_chunk(
                    &writer,
                    NewChunk {
                        doc_id,
                        n: 0,
                        title: "Laporan",
                        rel_path: "",
                        source,
                        page: None,
                        text: "hasil pengujian",
                        acl: &acl_labels(&visibility),
                        file_type,
                        years,
                    },
                )
                .unwrap();
        }
        writer.commit().unwrap();
        index.reload().unwrap();
        let ids = |source: Option<&str>, file_type: Option<&str>, year| {
            let filters = SearchFilters {
                source: source.map(str::to_owned),
                file_type: file_type.map(str::to_owned),
                year,
            };
            let mut ids: Vec<i64> = index
                .search_in(
                    &index.snapshot(),
                    "pengujian",
                    &user(Some("Review"), Role::User),
                    &["Review".into(), "Umum".into()],
                    0,
                    10,
                    false,
                    &filters,
                )
                .unwrap()
                .into_iter()
                .map(|h| h.doc_id)
                .collect();
            ids.sort_unstable();
            ids
        };
        assert_eq!(ids(Some("Review"), None, None), [1, 2, 4]);
        assert_eq!(ids(None, Some("Excel"), None), [1, 3, 4]);
        assert_eq!(ids(None, None, Some(2025)), [1, 2, 3]);
        assert_eq!(ids(None, None, Some(2026)), [1]);
        assert_eq!(ids(Some("Review"), Some("Excel"), Some(2025)), [1]);
        assert!(ids(Some("review"), None, None).is_empty());
        assert!(ids(Some("Khusus"), Some("Excel"), Some(2025)).is_empty());
    }

    #[tokio::test]
    async fn versioned_build_preserves_legacy_reader_and_carries_vision() {
        let base =
            std::env::temp_dir().join(format!("rag-hub-skema-lama-{}", rand::random::<u64>()));
        let mut cfg: crate::config::Config =
            toml::from_str("[azure]\nendpoint = 'http://127.0.0.1:9'\ndeployment = 'uji'").unwrap();
        cfg.data_dir = base.join("data");
        cfg.sources.push(crate::config::SourceConfig {
            name: "Uji".into(),
            path: base.join("dokumen-uji-2099"),
            visibility: Visibility::Departemen(vec!["Review".into()]),
        });
        let dir = cfg.data_dir.join("index");
        std::fs::create_dir_all(&dir).unwrap();
        let (builder, fields) = schema_v3();
        let schema = builder.build();
        {
            let index =
                DocIndex::from_index(Index::create_in_dir(&dir, schema).unwrap(), fields).unwrap();
            let mut writer = index.index.writer_with_num_threads(1, 20_000_000).unwrap();
            add(&index, &writer, 1, &Visibility::Umum, "hasil pengujian");
            add(
                &index,
                &writer,
                9,
                &Visibility::Umum,
                "pembacaan vision tersimpan",
            );
            writer.commit().unwrap();
            writer.wait_merging_threads().unwrap();
        }
        let current = version_dir(&cfg.data_dir, INDEX_VERSION);
        drop(DocIndex::open_current(&current, true).unwrap());
        let marker = current.join(COMPLETE);
        let lock = version_lock(&cfg.data_dir).unwrap();
        lock.try_lock().unwrap();
        std::fs::File::create(&marker).unwrap();
        assert_eq!(DocIndex::open(&cfg.data_dir).unwrap().table(), "documents");
        std::fs::remove_file(&marker).unwrap();
        assert_eq!(DocIndex::open(&cfg.data_dir).unwrap().table(), "documents");
        drop(lock);

        // Penanda tersisa setelah penghapusan sebagian: v3 masih bisa dibuka.
        std::fs::remove_file(current.join("meta.json")).unwrap();
        std::fs::File::create(&marker).unwrap();
        let index = DocIndex::open(&cfg.data_dir).unwrap();
        assert_eq!(index.table(), "documents");
        std::fs::remove_file(marker).unwrap();
        let Err(error) = index.writer() else {
            panic!("index lama bisa ditulis")
        };
        assert_eq!(
            error.to_string(),
            "index lama hanya dibaca; jalankan ingest untuk membangun index versi ini"
        );
        let search = |index: &DocIndex, filters| {
            index
                .search_in(
                    &index.snapshot(),
                    "pengujian",
                    &user(None, Role::User),
                    &["Uji".into()],
                    0,
                    10,
                    false,
                    &filters,
                )
                .unwrap()
        };
        assert_eq!(search(&index, SearchFilters::default()).len(), 1);
        assert_eq!(
            search(
                &index,
                SearchFilters {
                    source: Some("Uji".into()),
                    ..Default::default()
                }
            )
            .len(),
            1
        );
        assert!(
            search(
                &index,
                SearchFilters {
                    file_type: Some("PDF".into()),
                    ..Default::default()
                }
            )
            .is_empty()
        );
        assert!(
            search(
                &index,
                SearchFilters {
                    year: Some(2025),
                    ..Default::default()
                }
            )
            .is_empty()
        );
        let source = &cfg.sources[0].path;
        std::fs::create_dir_all(source.join("2001")).unwrap();
        let mut file = std::fs::File::create(source.join("2001/Rekap 2025.txt")).unwrap();
        file.write_all(b"hasil pengujian material").unwrap();
        // 31 Des 2025 18:00 UTC sudah 1 Jan 2026 di WIB.
        let timestamp = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_767_204_000);
        file.set_modified(timestamp).unwrap();
        drop(file);
        let scan = source.join("2001/scan.pdf");
        std::fs::write(&scan, b"scan uji").unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&scan)
            .unwrap()
            .set_modified(timestamp)
            .unwrap();
        let legacy_root = std::fs::canonicalize(source)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let db = crate::db::Db::open(&cfg.data_dir).unwrap();
        db.call(move |c| {
            c.execute(
                "INSERT INTO documents (id, source, rel_path, title, kind, status, size, mtime, visibility, root, version, indexed_at, chunks, pages)
                 VALUES (9, 'Uji', '2001/scan.pdf', 'scan', 'pdf', 'vision', 8, 1767204000000, '{\"type\":\"umum\"}', ?1, 3, 1, 1, 2),
                        (1, 'Uji', 'lama.txt', 'lama', 'txt', 'ok', 10, 1, '{\"type\":\"umum\"}', ?1, 3, 1, 1, 0)",
                [legacy_root],
            )?;
            Ok(())
        }).await.unwrap();
        assert_eq!(crate::ingest::run(&cfg, false, None).await.unwrap().read, 2);
        assert!(current.join(COMPLETE).is_file());
        assert_eq!(crate::ingest::run(&cfg, false, None).await.unwrap().read, 0);
        index.reload().unwrap();
        assert_eq!(search(&index, SearchFilters::default())[0].doc_id, 1);
        let index = std::sync::Arc::new(index);
        let filtered = SearchFilters {
            source: Some("Uji".into()),
            ..Default::default()
        };
        let prod = user(Some("Operasional"), Role::User);
        assert_eq!(search(&index, filtered.clone()).len(), 1);
        assert!(
            crate::documents::search(
                index.clone(),
                &db,
                &prod,
                &cfg.sources,
                "pengujian".into(),
                10,
                10,
                false,
                filtered
            )
            .await
            .unwrap()
            .is_empty()
        );
        let ctx = crate::agent::ToolContext {
            files: crate::files::FileStore::Local { dir: base.clone() },
            docs: Some(crate::agent::DocAccess {
                semantic: None,
                images: None,
                index: index.clone(),
                db: db.clone(),
                user: user(Some("Review"), Role::User),
                sources: std::sync::Arc::new(cfg.sources.clone()),
                session: None,
            }),
        };
        let result = crate::agent::run_tool(
            &crate::agent::ToolCall {
                id: "uji".into(),
                name: "search_documents".into(),
                arguments: r#"{ "query": "pengujian", "file_type": "PDF" }"#.into(),
            },
            &ctx,
            120_000,
            &mut Vec::new(),
        )
        .await;
        assert!(result.ok && result.output.contains("Coba cari tanpa filter."));
        drop(ctx);
        let old = crate::documents::search(
            index,
            &db,
            &user(Some("Review"), Role::User),
            &cfg.sources,
            "pengujian".into(),
            10,
            10,
            false,
            SearchFilters::default(),
        )
        .await
        .unwrap();
        assert!(old.is_empty());
        {
            let index = DocIndex::open(&cfg.data_dir).unwrap();
            assert_eq!(index.table(), "documents_v4");
            let with_year = |year| {
                index
                    .search_in(
                        &index.snapshot(),
                        "pengujian",
                        &user(Some("Review"), Role::User),
                        &["Uji".into()],
                        0,
                        10,
                        false,
                        &SearchFilters {
                            source: Some("Uji".into()),
                            file_type: Some("teks".into()),
                            year: Some(year),
                        },
                    )
                    .unwrap()
            };
            for year in [2001, 2025, 2026] {
                assert_eq!(with_year(year)[0].title, "Rekap 2025");
            }
            assert!(with_year(2099).is_empty());
            let review = user(Some("Review"), Role::User);
            let prod = user(Some("Operasional"), Role::User);
            let (total, chunks) = index.document_chunks(9, &review, 0, 10).unwrap().unwrap();
            assert_eq!(total, 1);
            assert_eq!(chunks[0].1.text, "pembacaan vision tersimpan");
            assert!(index.document_chunks(9, &prod, 0, 10).unwrap().is_none());
            let hits = crate::documents::search(
                std::sync::Arc::new(index),
                &db,
                &review,
                &cfg.sources,
                "tersimpan".into(),
                10,
                10,
                false,
                SearchFilters {
                    file_type: Some("PDF".into()),
                    year: Some(2026),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            assert_eq!(
                (hits[0].1.id, hits[0].1.pages, hits[0].1.status.as_str()),
                (9, 2, "vision")
            );
            assert_eq!(hits[0].1.folder, "2001");
        }
        drop(db);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn index_version_matches_schema_and_legacy_fallback() {
        assert_eq!(
            INDEX_VERSION, 4,
            "perbarui skema, tabel status, dan fallback index"
        );
        assert_eq!(INDEX_TABLE, format!("documents_v{INDEX_VERSION}"));
    }

    #[test]
    fn search_respects_access_labels() {
        let index = DocIndex::in_memory().unwrap();
        let mut writer = index.writer().unwrap();
        add(
            &index,
            &writer,
            1,
            &Visibility::Umum,
            "Torsi baut flange 45 Nm.",
        );
        add(
            &index,
            &writer,
            2,
            &Visibility::Departemen(vec!["Review".into()]),
            "Torsi baut uji tarik Review.",
        );
        add(
            &index,
            &writer,
            3,
            &Visibility::Admin,
            "Torsi baut catatan direksi.",
        );
        add(
            &index,
            &writer,
            4,
            &Visibility::Departemen(vec![]),
            "Torsi baut tanpa departemen.",
        );
        writer.commit().unwrap();
        index.reload().unwrap();

        let ids = |u: &AuthUser, q: &str| {
            let mut ids: Vec<i64> = index
                .search(q, u, &["Uji".to_string()], 0, 10, false)
                .unwrap()
                .iter()
                .map(|h| h.doc_id)
                .collect();
            ids.sort();
            ids
        };
        assert_eq!(ids(&user(Some("Review"), Role::User), "torsi"), [1, 2]);
        assert_eq!(ids(&user(Some("Operasional"), Role::User), "torsi"), [1]);
        assert_eq!(ids(&user(None, Role::User), "torsi"), [1]);
        assert_eq!(ids(&user(None, Role::Admin), "torsi"), [1, 2, 3, 4]);
        // Sintaks kueri tidak bisa membuka dokumen di luar hak akses.
        assert_eq!(
            ids(&user(Some("Operasional"), Role::User), "acl:admin OR torsi"),
            [1]
        );
        assert_eq!(
            ids(&user(Some("Review"), Role::User), "\"torsi baut\" -- ((("),
            [1, 2]
        );
        assert!(ids(&user(Some("Review"), Role::User), "").is_empty());

        let admin = user(None, Role::Admin);
        let id = chunk_id(2, 0, "Torsi baut uji tarik Review.");
        let hit = index
            .chunk(&id, &user(Some("Review"), Role::User))
            .unwrap()
            .unwrap();
        assert_eq!(
            (hit.doc_id, hit.page, hit.source.as_str()),
            (2, Some(2), "Uji")
        );
        // Label di index ikut dicek saat mengambil satu potongan.
        assert!(
            index
                .chunk(&id, &user(Some("Operasional"), Role::User))
                .unwrap()
                .is_none()
        );
        assert!(index.chunk(&id, &admin).unwrap().is_some());
        assert!(index.chunk("2-9-00000000", &admin).unwrap().is_none());

        // Dihapus berlaku setelah commit.
        index.delete_document(&writer, 1);
        writer.commit().unwrap();
        index.reload().unwrap();
        assert!(ids(&user(Some("Review"), Role::User), "torsi") == [2]);
    }

    #[test]
    fn document_chunks_come_in_order_and_respect_labels() {
        let index = DocIndex::in_memory().unwrap();
        let mut writer = index.writer().unwrap();
        let review = acl_labels(&Visibility::Departemen(vec!["Review".into()]));
        // Ditulis terbalik dan dengan dokumen lain berawalan sama (1 dan 11), supaya urutan
        // tidak kebetulan benar.
        for doc_id in [1, 11] {
            for n in (0..12).rev() {
                index
                    .add_chunk(
                        &writer,
                        NewChunk {
                            doc_id,
                            n,
                            title: "Laporan",
                            rel_path: "laporan.txt",
                            source: "Uji",
                            page: None,
                            text: &format!("dokumen {doc_id} bagian {n}"),
                            acl: &review,
                            file_type: "teks",
                            years: &[],
                        },
                    )
                    .unwrap();
            }
        }
        writer.commit().unwrap();
        index.reload().unwrap();

        let review_user = user(Some("Review"), Role::User);
        let numbers = |from, limit| {
            let (total, hits) = index
                .document_chunks(1, &review_user, from, limit)
                .unwrap()
                .unwrap();
            assert!(hits.iter().all(|(_, h)| h.doc_id == 1));
            (total, hits.into_iter().map(|(n, _)| n).collect::<Vec<_>>())
        };
        assert_eq!(numbers(0, 5), (12, vec![0, 1, 2, 3, 4]));
        assert_eq!(numbers(1, 2), (12, vec![1, 2]));
        assert_eq!(numbers(10, 5), (12, vec![10, 11]));
        assert_eq!(numbers(12, 5), (12, vec![]));
        let (_, hits) = index
            .document_chunks(1, &review_user, 3, 1)
            .unwrap()
            .unwrap();
        assert_eq!(hits[0].1.text, "dokumen 1 bagian 3");
        // Label di index ikut dicek: dokumennya dianggap tidak ada.
        let prod = user(Some("Operasional"), Role::User);
        assert!(index.document_chunks(1, &prod, 0, 5).unwrap().is_none());
        let admin = user(None, Role::Admin);
        assert_eq!(
            index
                .document_chunks(11, &admin, 0, 20)
                .unwrap()
                .unwrap()
                .1
                .len(),
            12
        );
        assert_eq!(
            index.document_chunks(5, &admin, 0, 20).unwrap().unwrap().0,
            0
        );
    }

    #[test]
    fn mixed_labels_hide_the_whole_document() {
        let index = DocIndex::in_memory().unwrap();
        let mut writer = index.writer().unwrap();
        let admin_only = acl_labels(&Visibility::Admin);
        let umum = acl_labels(&Visibility::Umum);
        // Dokumen 2: potongan 0 hanya admin, potongan 1 umum (label index bercampur).
        // Dokumen 3: umum semua.
        for (doc_id, n, acl) in [(2, 0, &admin_only), (2, 1, &umum), (3, 0, &umum)] {
            index
                .add_chunk(
                    &writer,
                    NewChunk {
                        doc_id,
                        n,
                        title: "Laporan",
                        rel_path: "laporan.txt",
                        source: "Uji",
                        page: None,
                        text: &format!("dokumen {doc_id} bagian {n}"),
                        acl,
                        file_type: "teks",
                        years: &[],
                    },
                )
                .unwrap();
        }
        writer.commit().unwrap();
        index.reload().unwrap();
        let someone = user(Some("Review"), Role::User);
        let admin = user(None, Role::Admin);
        assert!(index.document_chunks(2, &someone, 0, 20).unwrap().is_none());
        let (total, hits) = index.document_chunks(2, &admin, 0, 20).unwrap().unwrap();
        assert_eq!((total, hits.len()), (2, 2));
        assert_eq!(
            index
                .document_chunks(3, &someone, 0, 20)
                .unwrap()
                .unwrap()
                .0,
            1
        );
        let refused = index.refused(&someone, &["Uji".into()]).unwrap();
        assert_eq!(refused.docs, HashSet::from([2]));
        assert!(refused.sources.is_empty());
        let refused = index.refused(&admin, &["Uji".into()]).unwrap();
        assert!(refused.docs.is_empty() && refused.sources.is_empty());
        let refused = index.refused(&someone, &["Lain".into()]).unwrap();
        assert!(refused.docs.is_empty());
    }

    #[test]
    fn snippets_mark_matches_and_titles_match_too() {
        let index = DocIndex::in_memory().unwrap();
        let mut writer = index.writer().unwrap();
        add(
            &index,
            &writer,
            7,
            &Visibility::Umum,
            "Langkah 3: periksa ketebalan dinding produk dengan mikrometer.",
        );
        writer.commit().unwrap();
        index.reload().unwrap();
        let admin = user(None, Role::Admin);
        let uji = ["Uji".to_string()];
        let hits = index
            .search("mikrometer", &admin, &uji, 0, 5, true)
            .unwrap();
        let marked: Vec<&str> = hits[0]
            .snippet
            .iter()
            .filter(|s| s.hit)
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(marked, ["mikrometer"]);
        // Judul ikut dicari; cuplikannya dari awal potongan.
        let hits = index.search("Dokumen", &admin, &uji, 0, 5, true).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet[0].text.starts_with("Langkah 3"));
        // Path folder juga ikut dicari.
        assert_eq!(
            index
                .search("standar", &admin, &uji, 0, 5, false)
                .unwrap()
                .len(),
            1
        );
        // Hanya dari sumber yang diminta.
        let lain = ["Lain".to_string()];
        assert!(
            index
                .search("standar", &admin, &lain, 0, 5, false)
                .unwrap()
                .is_empty()
        );
        assert!(
            index
                .search("standar", &admin, &[], 0, 5, false)
                .unwrap()
                .is_empty()
        );
    }
}
