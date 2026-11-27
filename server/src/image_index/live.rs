use super::{current, eg2, extract, image_hash, published, stamp, store, store_path, worker};
use crate::{
    AppState,
    acl::AuthUser,
    agent::SourceRef,
    config::{GambarConfig, SourceConfig},
    db::Db,
    documents::{self, DocumentRow},
    embed::{Embedder, OrtEmbedder},
    error::ApiError,
    search::{DocIndex, INDEX_TABLE, SearchFilters},
};
use anyhow::{Result, anyhow, ensure};
use axum::{
    extract::{Path as AxumPath, State},
    http::header,
    response::{IntoResponse, Response},
};
use futures::{StreamExt, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};
use tokio::sync::{Semaphore, SemaphorePermit};
use tokio::time::Instant;

const LIMIT: usize = 3;
pub(crate) const SEARCH_DEADLINE: Duration = Duration::from_millis(1800);
const WARM_DEADLINE: Duration = Duration::from_secs(120);
pub(super) const TEXT_MIN: f32 = 0.66;
pub(super) const IMAGE_MIN: f32 = 0.88;
const THUMB_SIDE: u32 = 512;
const CACHE_BYTES: usize = 16 * 1024 * 1024;
const CACHE_COUNT: usize = 128;
const QUERY_PREFIX: &str = "task: search result | query: ";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct Picture {
    pub key: String,
    pub place: String,
    pub similar: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub evidence_only: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct Access<'a> {
    pub db: &'a Db,
    pub index: &'a Arc<DocIndex>,
    pub user: &'a AuthUser,
    pub sources: &'a [SourceConfig],
}

pub(crate) struct Live {
    cfg: GambarConfig,
    data: PathBuf,
    tokens: Mutex<Option<eg2::Tokens>>,
    text: Mutex<Option<Box<dyn Embedder>>>,
    vision: Mutex<Option<eg2::Encoder>>,
    snapshot: Mutex<Option<Snapshot>>,
    cache: Mutex<Cache>,
    decode: Arc<Semaphore>,
    query_text: Arc<Semaphore>,
    query_vision: Arc<Semaphore>,
}

struct Snapshot {
    path: PathBuf,
    reader: rusqlite::Connection,
    version: i64,
    vectors: Arc<Vec<store::Indexed>>,
}

#[derive(PartialEq)]
struct CacheKey {
    doc_id: i64,
    stamp: store::Hash,
    hash: store::Hash,
}

#[derive(Default)]
struct Cache {
    bytes: usize,
    items: VecDeque<(CacheKey, AxumBytes)>,
}

type AxumBytes = axum::body::Bytes;

pub(super) struct Blocking(Semaphore);

pub(super) static BLOCKING: Blocking = Blocking(Semaphore::const_new(4));

impl Blocking {
    async fn permit(&'static self, deadline: Instant) -> Option<SemaphorePermit<'static>> {
        if Instant::now() >= deadline {
            return None;
        }
        let permit = tokio::time::timeout_at(deadline, self.0.acquire())
            .await
            .ok()?
            .ok()?;
        (Instant::now() < deadline).then_some(permit)
    }

    pub(super) async fn run<T: Send + 'static>(
        &'static self,
        deadline: Instant,
        work: impl FnOnce() -> Result<T> + Send + 'static,
    ) -> Result<Option<T>> {
        let Some(permit) = self.permit(deadline).await else {
            return Ok(None);
        };
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work().map(Some)
        })
        .await?
    }

    pub(super) async fn rows(
        &'static self,
        db: &Db,
        table: &'static str,
        ids: Vec<i64>,
        deadline: Instant,
    ) -> Result<Option<std::collections::HashMap<i64, DocumentRow>>> {
        let Some(permit) = self.permit(deadline).await else {
            return Ok(None);
        };
        documents::rows_with_guard(db, table, ids, permit)
            .await
            .map(Some)
    }
}

impl Cache {
    fn get(&mut self, key: &CacheKey) -> Option<AxumBytes> {
        let i = self.items.iter().position(|(other, _)| other == key)?;
        let entry = self.items.remove(i)?;
        let bytes = entry.1.clone();
        self.items.push_back(entry);
        Some(bytes)
    }

    fn insert(&mut self, key: CacheKey, bytes: AxumBytes) {
        if bytes.len() > CACHE_BYTES || self.get(&key).is_some() {
            return;
        }
        while self.items.len() >= CACHE_COUNT || self.bytes + bytes.len() > CACHE_BYTES {
            let Some((_, old)) = self.items.pop_front() else {
                break;
            };
            self.bytes -= old.len();
        }
        self.bytes += bytes.len();
        self.items.push_back((key, bytes));
    }
}

impl Live {
    pub(crate) fn start(cfg: GambarConfig, data: PathBuf) -> Arc<Self> {
        let live = Self::new(cfg, data);
        let warm = live.clone();
        tokio::spawn(async move {
            // Memuat model pertama kali butuh beberapa detik, jadi pemanasan tidak memakai
            // tenggat pencarian 1,8 detik.
            let deadline = Instant::now() + WARM_DEADLINE;
            if warm.available(deadline).await
                && let Err(err) = warm.text("gambar".into(), deadline).await
            {
                tracing::warn!("Encoder kueri gambar gagal dimuat: {err:#}");
            }
        });
        live
    }

    pub(crate) fn new(cfg: GambarConfig, data: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            cfg,
            data,
            tokens: Mutex::new(None),
            text: Mutex::new(None),
            vision: Mutex::new(None),
            snapshot: Mutex::new(None),
            cache: Mutex::new(Cache::default()),
            decode: Arc::new(Semaphore::new(2)),
            query_text: Arc::new(Semaphore::new(1)),
            query_vision: Arc::new(Semaphore::new(1)),
        })
    }

    fn tokens(&self) -> Result<eg2::Tokens> {
        let mut saved = self.tokens.lock().unwrap();
        if let Some(tokens) = *saved {
            return Ok(tokens);
        }
        let tokens = eg2::Tokens::load(&self.cfg.model)?;
        *saved = Some(tokens);
        Ok(tokens)
    }

    fn store_path(&self) -> Result<PathBuf> {
        store_path(&self.data, &self.cfg, self.tokens()?)
    }

    pub(crate) async fn available(self: &Arc<Self>, deadline: Instant) -> bool {
        let live = self.clone();
        BLOCKING
            .run(deadline, move || Ok(!live.vectors()?.is_empty()))
            .await
            .ok()
            .flatten()
            .unwrap_or(false)
    }

    pub(crate) async fn text(
        self: &Arc<Self>,
        query: String,
        deadline: Instant,
    ) -> Result<Vec<f32>> {
        let permit =
            tokio::time::timeout_at(deadline, self.query_text.clone().acquire_owned()).await??;
        let live = self.clone();
        let encode = move || {
            let _permit = permit;
            let mut encoder = live.text.lock().unwrap();
            if encoder.is_none() {
                *encoder = Some(Box::new(OrtEmbedder::image_query(
                    &live.cfg.model,
                    live.cfg.ort_dylib.as_deref(),
                )?));
            }
            let mut result = encoder
                .as_mut()
                .unwrap()
                .embed(&[format!("{QUERY_PREFIX}{query}")])?;
            result
                .pop()
                .ok_or_else(|| anyhow!("Vektor kueri gambar kosong."))
        };
        BLOCKING
            .run(deadline, encode)
            .await?
            .ok_or_else(|| anyhow!("Pencarian gambar sedang sibuk."))
    }

    #[cfg(test)]
    pub(super) fn test_text(&self, encoder: impl Embedder + 'static) {
        *self.text.lock().unwrap() = Some(Box::new(encoder));
    }

    #[cfg(test)]
    pub(super) fn clear_thumbnail_cache(&self) {
        *self.cache.lock().unwrap() = Cache::default();
    }

    pub(crate) async fn vision(
        self: &Arc<Self>,
        bytes: Vec<u8>,
        deadline: Instant,
    ) -> Result<Vec<f32>> {
        let permit =
            tokio::time::timeout_at(deadline, self.query_vision.clone().acquire_owned()).await??;
        let live = self.clone();
        let encode = move || {
            let _permit = permit;
            let image =
                extract::user_image(bytes).ok_or_else(|| anyhow!("Gambar kueri gagal dibaca."))?;
            let dll = live
                .cfg
                .ort_dylib
                .clone()
                .or_else(|| std::env::var_os("ORT_DYLIB_PATH").map(PathBuf::from))
                .ok_or_else(|| anyhow!("Isi gambar.ort_dylib atau ORT_DYLIB_PATH."))?;
            let mut encoder = live.vision.lock().unwrap();
            if encoder.is_none() {
                *encoder = Some(eg2::Encoder::open(
                    &live.cfg,
                    &dll,
                    live.cfg.gpu,
                    live.tokens()?,
                )?);
            }
            let mut result = encoder
                .as_mut()
                .unwrap()
                .embed(vec![eg2::prepare(&image)])?;
            result
                .pop()
                .ok_or_else(|| anyhow!("Vektor kueri gambar kosong."))
        };
        BLOCKING
            .run(deadline, encode)
            .await?
            .ok_or_else(|| anyhow!("Pencarian gambar sedang sibuk."))
    }

    fn vectors(&self) -> Result<Arc<Vec<store::Indexed>>> {
        let path = self.store_path()?;
        let mut cache = self.snapshot.lock().unwrap();
        if !published(&path) {
            *cache = None;
            return Ok(Arc::default());
        }
        if cache.as_ref().is_none_or(|cached| cached.path != path) {
            let reader = store::open_reader(&path)?;
            *cache = Some(Snapshot {
                path,
                reader,
                version: -1,
                vectors: Arc::default(),
            });
        }
        let cached = cache.as_mut().expect("pembaca sudah dibuka");
        let version = cached
            .reader
            .pragma_query_value(None, "data_version", |row| row.get(0))?;
        if version != cached.version {
            cached.vectors = Arc::new(store::snapshot(&mut cached.reader)?);
            cached.version = version;
        }
        Ok(cached.vectors.clone())
    }

    pub(crate) async fn search(
        self: &Arc<Self>,
        access: Access<'_>,
        vector: Vec<f32>,
        similar: bool,
        filters: &SearchFilters,
        deadline: Instant,
    ) -> Result<Vec<SourceRef>> {
        if access.index.table() != INDEX_TABLE {
            return Ok(Vec::new());
        }
        ensure!(
            vector.len() == eg2::DIM,
            "Dimensi pencarian gambar tidak valid."
        );
        let live = self.clone();
        let rank = move || {
            let index = live.vectors()?;
            let min = if similar { IMAGE_MIN } else { TEXT_MIN };
            let mut ranked: Vec<_> = index
                .iter()
                .enumerate()
                .filter_map(|(i, row)| {
                    let score = crate::embed::cosine(&row.vector, &vector);
                    (score.is_finite() && score >= min).then_some((i, score))
                })
                .collect();
            ranked.sort_unstable_by(|a, b| {
                b.1.total_cmp(&a.1)
                    .then_with(|| index[a.0].hash.cmp(&index[b.0].hash))
            });
            Ok::<_, anyhow::Error>((index, ranked))
        };
        let Some((index, ranked)) = BLOCKING.run(deadline, rank).await? else {
            return Ok(Vec::new());
        };
        let mut found = Vec::new();
        let mut unavailable = HashSet::new();
        for batch in ranked.chunks(24) {
            let ids: Vec<_> = batch
                .iter()
                .flat_map(|&(i, _)| index[i].refs.iter().map(|r| r.doc_id))
                .filter(|id| !unavailable.contains(id))
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            let Some(rows) = BLOCKING
                .rows(access.db, access.index.table(), ids, deadline)
                .await?
            else {
                break;
            };
            let (user, sources, doc_index) = (
                access.user.clone(),
                access.sources.to_vec(),
                access.index.clone(),
            );
            let index = index.clone();
            let batch = batch.iter().map(|&(i, _)| i).collect::<Vec<_>>();
            let filters = filters.clone();
            let check = move || {
                for i in batch {
                    let entry = &index[i];
                    for reference in &entry.refs {
                        if unavailable.contains(&reference.doc_id) {
                            continue;
                        }
                        let Some(doc) = rows
                            .get(&reference.doc_id)
                            .filter(|doc| filter(doc, reference.location.is_empty(), &filters))
                        else {
                            unavailable.insert(reference.doc_id);
                            continue;
                        };
                        let version = stamp(&doc.root, &doc.rel_path, doc.size, doc.modified);
                        if version != reference.stamp {
                            continue;
                        }
                        let Some(path) = original(&doc_index, &user, &sources, doc)? else {
                            unavailable.insert(reference.doc_id);
                            continue;
                        };
                        found.push((
                            source(doc, entry.hash, &reference.location, similar),
                            path,
                            reference.location.clone(),
                            doc.clone(),
                        ));
                        break;
                    }
                    if found.len() == LIMIT {
                        break;
                    }
                }
                Ok::<_, anyhow::Error>((found, unavailable))
            };
            let Some(checked) = BLOCKING.run(deadline, check).await? else {
                return Ok(Vec::new());
            };
            (found, unavailable) = checked;
            if found.len() == LIMIT {
                break;
            }
        }
        let labels = crate::scan::labels(
            access.db,
            access.index.table(),
            found.iter().map(|(_, _, _, doc)| doc),
        )
        .await
        .unwrap_or_default();
        let labels = &labels;
        Ok(futures::future::join_all(found.into_iter().map(
            |(mut hit, path, location, doc)| async move {
                hit.handwriting = crate::scan::handwriting(&doc, labels.get(&doc.id), hit.page);
                if matches!(hit.kind.as_str(), "pptx" | "xlsx") {
                    let placement = async {
                        let _permit = self.decode.acquire().await.ok()?;
                        worker::place(path, hit.kind.clone(), location).await
                    };
                    if let Ok(Some((place, page))) =
                        tokio::time::timeout(Duration::from_millis(450), placement).await
                    {
                        hit.page = page;
                        hit.image.as_mut().unwrap().place = place;
                    }
                }
                hit
            },
        ))
        .await)
    }

    pub(crate) async fn thumbnail(
        self: &Arc<Self>,
        access: Access<'_>,
        doc_id: i64,
        key: &str,
    ) -> Result<Option<AxumBytes>> {
        let deadline = Instant::now() + SEARCH_DEADLINE;
        tokio::time::timeout_at(deadline, self.read_thumbnail(access, doc_id, key, deadline))
            .await
            .unwrap_or(Ok(None))
    }

    pub(crate) async fn thumbnails(
        self: &Arc<Self>,
        access: Access<'_>,
        hits: Vec<SourceRef>,
        deadline: Instant,
    ) -> (Vec<(SourceRef, AxumBytes)>, bool) {
        let mut pending: FuturesUnordered<_> = hits
            .into_iter()
            .enumerate()
            .map(|(rank, hit)| async move {
                let image = hit.image.as_ref()?;
                let bytes = self
                    .read_thumbnail(access, hit.doc_id, &image.key, deadline)
                    .await
                    .ok()??;
                Some((rank, hit, bytes))
            })
            .collect();
        let mut ready = Vec::new();
        let mut expired = false;
        while !pending.is_empty() {
            if Instant::now() >= deadline {
                expired = true;
                break;
            }
            match tokio::time::timeout_at(deadline, pending.next()).await {
                Ok(Some(item)) => ready.extend(item),
                Ok(None) => break,
                Err(_) => {
                    expired = true;
                    break;
                }
            }
        }
        ready.sort_unstable_by_key(|(rank, _, _)| *rank);
        (
            ready
                .into_iter()
                .map(|(_, hit, bytes)| (hit, bytes))
                .collect(),
            expired,
        )
    }

    async fn read_thumbnail(
        self: &Arc<Self>,
        access: Access<'_>,
        doc_id: i64,
        key: &str,
        deadline: Instant,
    ) -> Result<Option<AxumBytes>> {
        let Ok(hash) = hex::decode(key).and_then(|bytes| {
            bytes
                .try_into()
                .map_err(|_| hex::FromHexError::InvalidStringLength)
        }) else {
            return Ok(None);
        };
        if access.index.table() != INDEX_TABLE {
            return Ok(None);
        }
        let Some(doc) = BLOCKING
            .rows(access.db, access.index.table(), vec![doc_id], deadline)
            .await?
            .and_then(|mut rows| rows.remove(&doc_id))
        else {
            return Ok(None);
        };
        let live = self.clone();
        let (index, user, sources) = (
            access.index.clone(),
            access.user.clone(),
            access.sources.to_vec(),
        );
        let check = move || {
            let Some(path) = original(&index, &user, &sources, &doc)? else {
                return Ok(None);
            };
            let version = stamp(&doc.root, &doc.rel_path, doc.size, doc.modified);
            let store = live.store_path()?;
            if !published(&store) {
                return Ok(None);
            }
            let location = store::reference(&store, doc.id, &version, &hash)?;
            Ok::<_, anyhow::Error>(location.map(|loc| (path, loc, version, doc.size, doc.modified)))
        };
        let info = BLOCKING.run(deadline, check).await?.flatten();
        let Some((path, location, version, size, modified)) = info else {
            return Ok(None);
        };
        let key = CacheKey {
            doc_id,
            stamp: version,
            hash,
        };
        if let Some(bytes) = self.cache.lock().unwrap().get(&key) {
            return Ok(Some(bytes));
        }
        let permit = self.decode.clone().acquire_owned().await?;
        if let Some(bytes) = self.cache.lock().unwrap().get(&key) {
            return Ok(Some(bytes));
        }
        let request = worker::Request {
            path: path.clone(),
            selection: worker::Selection::One(location),
        };
        let mut image = None;
        worker::visit(request, &mut |_, decoded| {
            image = Some(decoded);
            Ok(false)
        })
        .await?;
        let encode = move || {
            let _permit = permit;
            let Some(image) =
                image.filter(|image| image_hash(image) == hash && current(&path, size, modified))
            else {
                return Ok(None);
            };
            let image = image::DynamicImage::ImageRgb8(image).thumbnail(THUMB_SIDE, THUMB_SIDE);
            let mut bytes = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 80)
                .encode_image(&image)?;
            Ok(Some(AxumBytes::from(bytes)))
        };
        let jpg = BLOCKING.run(deadline, encode).await?.flatten();
        if let Some(bytes) = &jpg {
            self.cache.lock().unwrap().insert(key, bytes.clone());
        }
        Ok(jpg)
    }
}

#[cfg(test)]
#[tokio::test]
async fn slow_source_checks_keep_their_slots_after_cancellation() {
    static GATE: Blocking = Blocking(Semaphore::const_new(4));
    let (started, mut in_folder) = tokio::sync::mpsc::unbounded_channel();
    let mut requests = Vec::new();
    let mut releases = Vec::new();
    for _ in 0..4 {
        let (release, stuck) = std::sync::mpsc::channel::<()>();
        let started = started.clone();
        releases.push(release);
        requests.push(tokio::spawn(GATE.run(
            Instant::now() + SEARCH_DEADLINE,
            move || {
                started.send(()).unwrap();
                let _ = stuck.recv();
                Ok(())
            },
        )));
    }
    for _ in 0..4 {
        in_folder.recv().await.unwrap();
    }
    for request in requests {
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
    }
    let short = Instant::now() + Duration::from_millis(20);
    let rejected = futures::future::join_all((0..64).map(|_| GATE.run(short, || Ok(())))).await;
    assert!(rejected.into_iter().all(|result| result.unwrap().is_none()));
    let fifth = GATE.run(Instant::now() + SEARCH_DEADLINE, || Ok(5));
    tokio::pin!(fifth);
    assert!(futures::poll!(&mut fifth).is_pending());
    drop(releases);
    assert_eq!(fifth.await.unwrap(), Some(5));
}

pub(super) fn original(
    index: &DocIndex,
    user: &AuthUser,
    sources: &[SourceConfig],
    doc: &DocumentRow,
) -> Result<Option<PathBuf>> {
    if doc.visibility.is_none()
        || !doc.allows_entry(user, sources)
        || !index.has_readable_entry(doc.id, user)?
    {
        return Ok(None);
    }
    Ok(documents::source_root(sources, &doc.source, &doc.root)
        .and_then(|root| documents::resolve(&root, &doc.rel_path))
        .filter(|path| current(path, doc.size, doc.modified)))
}

fn filter(doc: &DocumentRow, direct: bool, filters: &SearchFilters) -> bool {
    filters
        .source
        .as_ref()
        .is_none_or(|source| &doc.source == source)
        && filters.file_type.as_ref().is_none_or(|kind| {
            kind == if direct {
                "gambar/scan"
            } else {
                crate::ingest::file_type(&doc.kind)
            }
        })
        && filters.year.is_none_or(|year| {
            let known = doc
                .doc_date
                .as_deref()
                .and_then(|date| date.get(..4))
                .and_then(|year| year.parse().ok());
            let modified = u64::try_from(doc.modified)
                .ok()
                .and_then(|ms| UNIX_EPOCH.checked_add(Duration::from_millis(ms)));
            crate::ingest::document_years(Path::new(&doc.rel_path), known, modified).contains(&year)
        })
}

fn source(doc: &DocumentRow, hash: store::Hash, location: &str, similar: bool) -> SourceRef {
    let key = hex::encode(hash);
    let page = location
        .strip_prefix("halaman:")
        .and_then(|loc| loc.split(':').next()?.parse().ok());
    let place = if let Some(page) = page {
        format!("Hal. {page}")
    } else if location.is_empty() {
        "Gambar asli".into()
    } else {
        Path::new(location)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Media dokumen")
            .into()
    };
    SourceRef {
        n: 0,
        chunk_id: format!("gambar-{}-{key}", doc.id),
        doc_id: doc.id,
        title: doc.title.clone(),
        display_title: doc.display_title.clone(),
        source: doc.source.clone(),
        kind: doc.kind.clone(),
        page,
        scan: crate::scan::is_scan(&doc.status, &doc.kind),
        handwriting: None,
        scan_rev: crate::scan::revision(doc),
        sheet: None,
        evidence_only: false,
        image: Some(Picture {
            key,
            place,
            similar,
            evidence_only: false,
        }),
    }
}

pub(crate) async fn thumbnail_route(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    AxumPath((doc_id, key)): AxumPath<(i64, String)>,
) -> Result<Response, ApiError> {
    let (Some(live), Some(index)) = (&state.images, &state.docs) else {
        return Err(ApiError::not_found("Gambar sumber tidak ditemukan."));
    };
    let access = Access {
        db: &state.db,
        index,
        user: &user,
        sources: &state.sources,
    };
    let bytes = live
        .thumbnail(access, doc_id, &key)
        .await
        .ok()
        .flatten()
        .ok_or_else(|| ApiError::not_found("Gambar sumber tidak ditemukan."))?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "private, no-store"),
        ],
        bytes,
    )
        .into_response())
}
