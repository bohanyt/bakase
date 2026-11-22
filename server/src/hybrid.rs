use crate::{
    acl::{AuthUser, can_read},
    config::{MaknaConfig, SourceConfig},
    db::Db,
    documents::{self, DocumentRow},
    embed::{Embedder, Model, OrtEmbedder, Store, cosine},
    search::{DocIndex, Hit, INDEX_TABLE, SearchFilters},
};
use anyhow::{Result, ensure};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tantivy::{DocAddress, Searcher, SearcherGeneration};

pub(crate) const NAME_ONLY_LIMIT: usize = 2;
type Hits = Vec<(Hit, DocumentRow)>;
type Encoder = Arc<Mutex<Box<dyn Embedder>>>;
thread_local! {
    pub(crate) static LOADER_RUNNING: Cell<bool> = const { Cell::new(false) };
}

struct Chunk {
    id: String,
    doc_id: i64,
    source: String,
    name_only: bool,
    vector: Option<usize>,
}

pub(crate) struct Ready {
    snapshot: Searcher,
    chunks: HashMap<DocAddress, Chunk>,
    store: Arc<Store>,
    encoder: Encoder,
    prefix: String,
    dim: usize,
}

impl Ready {
    fn matches(&self, generation: &SearcherGeneration) -> bool {
        self.snapshot.generation().segments() == generation.segments()
    }

    fn build(
        index: &DocIndex,
        snapshot: Searcher,
        store: Arc<Store>,
        model: &Model,
        encoder: Encoder,
    ) -> Result<Self> {
        let mut chunks = HashMap::new();
        store.matching_vectors(index, &snapshot, |address, hit, vector| {
            chunks.insert(
                address,
                Chunk {
                    id: hit.chunk_id,
                    doc_id: hit.doc_id,
                    source: hit.source,
                    name_only: hit.text.is_empty(),
                    vector,
                },
            );
        })?;
        Ok(Self {
            snapshot,
            chunks,
            store,
            encoder,
            prefix: model.query_prefix.clone(),
            dim: model.dim,
        })
    }

    pub(crate) fn for_eval(
        data: &Path,
        model: &Model,
        index: &DocIndex,
        encoder: Box<dyn Embedder>,
    ) -> Result<Arc<Self>> {
        let dir = model.store_dir(data);
        let store = Arc::new(Store::open_generation(&dir, &Store::current(&dir)?, model)?);
        let ready = Self::build(
            index,
            index.snapshot(),
            store,
            model,
            Arc::new(Mutex::new(encoder)),
        )?;
        ensure!(
            ready.chunks.len() == ready.store.len()
                && ready
                    .chunks
                    .values()
                    .filter_map(|chunk| chunk.vector)
                    .collect::<HashSet<_>>()
                    .len()
                    == ready.store.len(),
            "Vektor belum sesuai dengan index saat ini. Jalankan embed lagi."
        );
        Ok(Arc::new(ready))
    }
}

#[derive(Default)]
pub(crate) struct Live {
    ready: RwLock<Option<Arc<Ready>>>,
    warned: AtomicBool,
}

impl Live {
    pub(crate) fn start(
        cfg: Option<MaknaConfig>,
        data: PathBuf,
        index: Arc<DocIndex>,
    ) -> Option<Arc<Self>> {
        let cfg = cfg?;
        let live = Arc::new(Self::default());
        if index.table() != INDEX_TABLE {
            live.warn();
            return None;
        }
        let handle = Arc::downgrade(&live);
        tokio::spawn(async move {
            let mut loader = Loader::default();
            let mut interval = tokio::time::interval(Duration::from_secs(2));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let Some(live) = handle.upgrade() else { break };
                let (cfg, data, index, target) =
                    (cfg.clone(), data.clone(), index.clone(), live.clone());
                let task = tokio::task::spawn_blocking(move || {
                    LOADER_RUNNING.set(true);
                    loader.run(&target, |loader| {
                        loader.publish(
                            &target,
                            &data,
                            &cfg.model,
                            &index,
                            Store::open_generation,
                            |model| {
                                Ok(Box::new(OrtEmbedder::open(
                                    &cfg.model,
                                    model,
                                    cfg.ort_dylib.as_deref(),
                                    cfg.threads,
                                    cfg.gpu,
                                )?))
                            },
                        );
                    });
                    LOADER_RUNNING.set(false);
                    loader
                });
                loader = match task.await {
                    Ok(updated) => updated,
                    Err(_) => {
                        tracing::warn!("Loader pencarian makna berhenti. Memakai pencarian kata.");
                        *live.ready.write().unwrap() = None;
                        break;
                    }
                };
            }
        });
        Some(live)
    }

    fn warn(&self) {
        if !self.warned.swap(true, Ordering::Relaxed) {
            tracing::warn!("Pencarian makna tidak tersedia. Memakai pencarian kata.");
        }
    }

    /// Pencarian makna jalan lagi setelah `warn` tercatat; peringatan berikutnya tercatat lagi.
    fn recovered(&self) -> bool {
        self.warned.swap(false, Ordering::Relaxed)
    }
}

#[derive(Default)]
struct Loader {
    model: Option<Result<(Model, Encoder), ()>>,
    observed: Option<SearcherGeneration>,
    store: Option<(PathBuf, Arc<Store>)>,
    ready: Option<Arc<Ready>>,
    /// Generasi vektor yang sudah dicatat aktif di log.
    announced: Option<PathBuf>,
}

impl Loader {
    fn run(&mut self, live: &Live, publish: impl FnOnce(&mut Self)) {
        if catch_unwind(AssertUnwindSafe(|| publish(self))).is_err() {
            tracing::warn!("Loader pencarian makna gagal. Memulai ulang loader.");
            *live.ready.write().unwrap() = None;
            self.ready = None;
            self.observed = None;
        }
    }

    fn publish(
        &mut self,
        live: &Live,
        data: &Path,
        folder: &Path,
        index: &DocIndex,
        open_store: impl FnOnce(&Path, &Path, &Model) -> Result<Store>,
        open_encoder: impl FnOnce(&Model) -> Result<Box<dyn Embedder>>,
    ) {
        if self.model.is_none() {
            // Jika inisialisasi panik, jangan coba membuka sesi lagi.
            self.model = Some(Err(()));
            let initialized = Model::load(folder)
                .and_then(|model| Ok((open_encoder(&model)?, model)))
                .map(|(encoder, model)| (model, Arc::new(Mutex::new(encoder))))
                .map_err(|_| {
                    tracing::warn!("Pencarian makna dikonfigurasi, tetapi model gagal dimuat. Memakai pencarian kata.");
                });
            self.model = Some(initialized);
        }
        if !matches!(self.model, Some(Ok(_))) {
            return;
        }

        let snapshot = index.snapshot();
        let generation = snapshot.generation();
        self.drop_stale(live, generation);
        // Reload tanpa perubahan segmen tetap menghasilkan ID generasi baru.
        if self.observed.as_ref().map(SearcherGeneration::segments) != Some(generation.segments()) {
            self.observed = Some(generation.clone());
            return;
        }
        match self.refresh(data, index, snapshot, open_store) {
            Ok(Some(ready)) => {
                // Satu baris per generasi vektor, sebagai bukti di log bahwa pencarian makna jalan,
                // dan lagi setelah sempat tercatat tidak tersedia (misalnya selama ingest).
                let generation = self.store.as_ref().map(|(path, _)| path.clone());
                if live.recovered() || self.announced != generation {
                    tracing::info!("Pencarian makna aktif: {} vektor.", ready.store.len());
                    self.announced = generation;
                }
                *live.ready.write().unwrap() = Some(ready.clone());
                self.ready = Some(ready);
            }
            Ok(None) => {}
            Err(_) => {
                self.drop_stale(live, index.snapshot().generation());
                if self.ready.is_none() {
                    live.warn();
                }
            }
        }
    }

    fn drop_stale(&mut self, live: &Live, generation: &SearcherGeneration) {
        if self
            .ready
            .take_if(|ready| !ready.matches(generation))
            .is_some()
        {
            *live.ready.write().unwrap() = None;
        }
    }

    fn refresh(
        &mut self,
        data: &Path,
        index: &DocIndex,
        snapshot: Searcher,
        open_store: impl FnOnce(&Path, &Path, &Model) -> Result<Store>,
    ) -> Result<Option<Arc<Ready>>> {
        let (model, encoder) = self.model.as_ref().unwrap().as_ref().unwrap();
        let dir = model.store_dir(data);
        let generation = Store::current(&dir)?;
        if !self
            .store
            .as_ref()
            .is_some_and(|(old, _)| *old == generation)
        {
            let store = Arc::new(open_store(&dir, &generation, model)?);
            self.store = Some((generation, store));
        }
        let (_, store) = self.store.as_ref().unwrap();
        if self
            .ready
            .as_ref()
            .is_some_and(|ready| Arc::ptr_eq(&ready.store, store))
        {
            return Ok(None);
        }
        let ready = Arc::new(Ready::build(
            index,
            snapshot,
            store.clone(),
            model,
            encoder.clone(),
        )?);
        ensure!(
            ready.matches(index.snapshot().generation()),
            "Index berubah saat pencarian makna dimuat."
        );
        Ok(Some(ready))
    }
}

#[cfg(test)]
mod tests;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn chat(
    live: Option<&Live>,
    index: Arc<DocIndex>,
    db: &Db,
    user: &AuthUser,
    sources: &[SourceConfig],
    query: String,
    limit: usize,
    filters: SearchFilters,
) -> Result<Hits> {
    if let Some(live) = live {
        let ready = live.ready.read().unwrap().clone();
        if let Some(ready) = ready.filter(|ready| ready.matches(index.snapshot().generation())) {
            match rank(
                ready.clone(),
                index.clone(),
                db,
                user,
                sources,
                &query,
                limit,
                false,
                &filters,
            )
            .await
            {
                Ok((hits, embedded)) if ready.matches(index.snapshot().generation()) => {
                    // Pulih setelah pencarian sebelumnya gagal, dengan vektor yang sama. Hanya kalau
                    // pertanyaannya benar-benar di-embed.
                    if embedded && live.recovered() {
                        tracing::info!("Pencarian makna aktif: {} vektor.", ready.store.len());
                    }
                    return Ok(hits);
                }
                Ok(_) => {}
                Err(_) => live.warn(),
            }
        }
    }
    documents::search(
        index,
        db,
        user,
        sources,
        query,
        limit,
        NAME_ONLY_LIMIT,
        false,
        filters,
    )
    .await
}

/// Hasil gabungan, dan apakah pertanyaannya di-embed (tidak, kalau tidak ada vektor yang boleh
/// dipakai).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn rank(
    ready: Arc<Ready>,
    index: Arc<DocIndex>,
    db: &Db,
    user: &AuthUser,
    sources: &[SourceConfig],
    query: &str,
    limit: usize,
    distinct_documents: bool,
    filters: &SearchFilters,
) -> Result<(Hits, bool)> {
    let depth = limit.saturating_mul(5).max(100);
    let names = sources
        .iter()
        .filter(|source| can_read(user, Some(&source.visibility)))
        .map(|source| source.name.clone())
        .collect::<Vec<_>>();
    let (view, search, who, filters) =
        (ready.clone(), index.clone(), user.clone(), filters.clone());
    let candidates = tokio::task::spawn_blocking(move || -> Result<_> {
        let (allowed, refused) =
            search.semantic_addresses(&view.snapshot, &who, &names, &filters)?;
        let refused: HashSet<_> = refused
            .into_iter()
            .map(|address| view.chunks[&address].doc_id)
            .collect();
        Ok(allowed
            .into_iter()
            .filter(|address| !refused.contains(&view.chunks[address].doc_id))
            .collect::<HashSet<_>>())
    })
    .await??;
    let mut ids = candidates
        .iter()
        .map(|address| ready.chunks[address].doc_id)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    let rows = documents::rows(db, index.table(), ids).await?;
    let eligible: HashSet<_> = candidates
        .into_iter()
        .filter(|address| {
            let chunk = &ready.chunks[address];
            rows.get(&chunk.doc_id).is_some_and(|row| {
                row.allows_entry(user, sources)
                    && row.source == chunk.source
                    && row.matches_entry(chunk.name_only)
            })
        })
        .collect();
    let query = query.to_owned();
    tokio::task::spawn_blocking(move || {
        let keyword = index.keyword_scores(&ready.snapshot, &query, &eligible)?;
        let mut vectors = eligible
            .into_iter()
            .filter_map(|address| {
                ready.chunks[&address]
                    .vector
                    .map(|vector| (vector, address))
            })
            .collect::<Vec<_>>();
        vectors.sort_unstable_by_key(|&(vector, _)| vector);
        let embedded = !vectors.is_empty();
        let meaning = if !embedded {
            Vec::new()
        } else {
            let query = embed_query(
                &ready.encoder,
                format!("{}{query}", ready.prefix),
                ready.dim,
            )?;
            score_vectors(&ready.store, &query, vectors)
        };
        let lists = [keyword, meaning].map(|scores| top_scores(scores, &ready.chunks, depth));
        let mut names = 0;
        let mut seen = HashSet::new();
        fusion(lists, &ready.chunks)
            .into_iter()
            .filter(|&(address, _)| {
                let chunk = &ready.chunks[&address];
                names += usize::from(chunk.name_only);
                if chunk.name_only && names > NAME_ONLY_LIMIT {
                    return false;
                }
                let row = &rows[&chunk.doc_id];
                !distinct_documents || seen.insert((&row.source, &row.rel_path))
            })
            .take(limit)
            .map(|(address, _)| {
                index
                    .hit_at(&ready.snapshot, address)
                    .map(|hit| (hit, rows[&ready.chunks[&address].doc_id].clone()))
            })
            .collect::<Result<Hits>>()
            .map(|hits| (hits, embedded))
    })
    .await?
}

fn embed_query(encoder: &Encoder, query: String, dim: usize) -> Result<Vec<f32>> {
    let mut queries = encoder
        .lock()
        .map_err(|_| anyhow::anyhow!("Model tidak tersedia."))?
        .embed(&[query])?;
    ensure!(
        queries.len() == 1
            && queries[0].len() == dim
            && (cosine(&queries[0], &queries[0]) - 1.0).abs() < 0.001,
        "Vektor pertanyaan tidak valid."
    );
    Ok(queries.pop().unwrap())
}

fn score_vectors(
    store: &Store,
    query: &[f32],
    vectors: Vec<(usize, DocAddress)>,
) -> Vec<(f32, DocAddress)> {
    vectors
        .into_iter()
        .map(|(vector, address)| (cosine(store.vector(vector), query), address))
        .collect()
}

fn top_scores(
    scores: Vec<(f32, DocAddress)>,
    chunks: &HashMap<DocAddress, Chunk>,
    depth: usize,
) -> Vec<DocAddress> {
    let (mut names, mut content): (Vec<_>, Vec<_>) = scores
        .into_iter()
        .partition(|(_, address)| chunks[address].name_only);
    let order = |a: &(f32, DocAddress), b: &(f32, DocAddress)| {
        b.0.total_cmp(&a.0)
            .then_with(|| chunks[&a.1].id.cmp(&chunks[&b.1].id))
    };
    for (list, limit) in [(&mut names, NAME_ONLY_LIMIT), (&mut content, depth)] {
        if list.len() > limit {
            list.select_nth_unstable_by(limit, order);
            list.truncate(limit);
        }
    }
    content.extend(names);
    content.sort_unstable_by(order);
    content
        .into_iter()
        .take(depth)
        .map(|(_, address)| address)
        .collect()
}

fn fusion(
    lists: [Vec<DocAddress>; 2],
    chunks: &HashMap<DocAddress, Chunk>,
) -> Vec<(DocAddress, f64)> {
    let mut totals = HashMap::new();
    for list in lists {
        let mut seen = HashSet::new();
        for (rank, address) in list
            .into_iter()
            .filter(|address| seen.insert(&chunks[address].id))
            .enumerate()
        {
            let (_, score) = totals.entry(&chunks[&address].id).or_insert((address, 0.0));
            *score += 1.0 / (60 + rank + 1) as f64;
        }
    }
    let mut scores: Vec<_> = totals.into_values().collect();
    scores.sort_unstable_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| chunks[&a.0].id.cmp(&chunks[&b.0].id))
    });
    scores
}
