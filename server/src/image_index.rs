#[cfg(test)]
mod bench;
mod eg2;
mod extract;
mod live;
mod store;
#[cfg(test)]
mod tests;
pub(crate) mod worker;

use crate::{
    acl::{AuthUser, Visibility},
    config::{Config, GambarConfig},
    db::Db,
    documents, ingest,
    search::{DocIndex, INDEX_TABLE, INDEX_VERSION},
};
use anyhow::{Result, anyhow, bail, ensure};
use eg2::{Encoder, Prepared, Tokens};
#[cfg(test)]
pub(crate) use extract::tests::pdf_with_image;
use extract::{Counts, Kind};
pub(crate) use extract::{allowed_size, safe_pdf_image};
use image::RgbImage;
pub(crate) use live::{Access, Live, Picture, SEARCH_DEADLINE, thumbnail_route};
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File},
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    time::Instant,
};
use store::{FileState, Hash, Store, Vectors};

pub(crate) async fn readable_images(
    db: &Db,
    index: std::sync::Arc<DocIndex>,
    user: AuthUser,
    sources: std::sync::Arc<Vec<crate::config::SourceConfig>>,
    ids: Vec<i64>,
    deadline: tokio::time::Instant,
) -> Result<std::collections::HashSet<i64>> {
    if index.table() != INDEX_TABLE || ids.is_empty() {
        return Ok(Default::default());
    }
    let Some(rows) = live::BLOCKING
        .rows(db, index.table(), ids, deadline)
        .await?
    else {
        return Ok(Default::default());
    };
    Ok(live::BLOCKING
        .run(deadline, move || {
            let mut readable = std::collections::HashSet::new();
            for (id, row) in rows {
                if live::original(&index, &user, &sources, &row)?.is_some() {
                    readable.insert(id);
                }
            }
            Ok(readable)
        })
        .await?
        .unwrap_or_default())
}

const MAX_BATCH: usize = 2;

pub(crate) fn cli(args: Vec<String>) -> Result<()> {
    let mut args = args.iter().map(String::as_str);
    let (mut gpu, mut batch, mut limit, mut config) = (false, 1, None, None);
    while let Some(arg) = args.next() {
        match arg {
            "--gpu" => gpu = true,
            "--batch" | "--maks-file" => {
                let value = args
                    .next()
                    .and_then(|s| s.parse::<usize>().ok())
                    .filter(|&n| n > 0)
                    .ok_or_else(|| anyhow!("{arg} harus bilangan bulat positif."))?;
                if arg == "--batch" {
                    ensure!(
                        value <= MAX_BATCH,
                        "--batch gambar paling banyak {MAX_BATCH}."
                    );
                    batch = value;
                } else {
                    limit = Some(value);
                }
            }
            _ if !arg.starts_with('-') && config.is_none() => config = Some(arg),
            _ => bail!("Opsi embed-gambar tidak dikenal. Lihat --help."),
        }
    }
    let cfg = Config::load(&crate::config_path(config))
        .map_err(|_| anyhow!("Config embed-gambar gagal dibaca atau tidak valid."))?;
    let Some(model) = cfg.gambar.as_ref() else {
        println!("Indeks gambar nonaktif.");
        return Ok(());
    };
    ensure!(
        !gpu || cfg!(feature = "cuda"),
        "Build CUDA diperlukan untuk --gpu."
    );
    let dll = model
        .ort_dylib
        .clone()
        .or_else(|| std::env::var_os("ORT_DYLIB_PATH").map(PathBuf::from))
        .ok_or_else(|| anyhow!("Isi gambar.ort_dylib atau ORT_DYLIB_PATH."))?;
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(move || run(&cfg, &dll, gpu, batch, limit))?
        .join()
        .map_err(|_| {
            anyhow!("Pembaca gambar berhenti. Indeks yang sudah tersimpan dapat dilanjutkan.")
        })?
}

fn store_path(data: &Path, model: &GambarConfig, tokens: Tokens) -> Result<PathBuf> {
    let mut hash = Sha256::new();
    hash.update(b"app-gambar-wal");
    hash.update(eg2::PREPROCESS_VERSION.to_le_bytes());
    hash.update((eg2::DIM as u32).to_le_bytes());
    for id in tokens.0 {
        hash.update(id.to_le_bytes());
    }
    for name in [
        "vision_encoder.onnx",
        "vision_encoder.onnx_data",
        "model.onnx",
        "model.onnx_data",
    ] {
        let meta = match fs::metadata(model.model.join(name)) {
            Ok(meta) if meta.is_file() => meta,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && name.ends_with("_data") =>
            {
                continue;
            }
            _ => bail!("Artefak model gambar tidak lengkap. Periksa folder ekspor EG2 FP32."),
        };
        let (size, mtime) = ingest::file_stamp(&meta);
        hash.update(name);
        hash.update(size.to_le_bytes());
        hash.update(mtime.to_le_bytes());
    }
    let folder = data.join(format!("embed-gambar-v1-index{INDEX_VERSION}"));
    Ok(folder.join(format!("{}.sqlite", hex::encode(hash.finalize()))))
}

fn building_marker(path: &Path) -> PathBuf {
    path.with_extension("sqlite.building")
}

fn published(path: &Path) -> bool {
    path.is_file()
        && building_marker(path)
            .try_exists()
            .is_ok_and(|building| !building)
}

fn remove_old_stores(path: &Path) -> Result<()> {
    let folder = path.parent().expect("indeks memiliki folder");
    let active = path.file_stem().expect("indeks memiliki nama");
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some((stamp, suffix)) = name.split_once('.') else {
            continue;
        };
        if stamp.len() == 64
            && stamp.bytes().all(|c| c.is_ascii_hexdigit())
            && stamp != active
            && suffix == "sqlite"
            && entry.file_type()?.is_file()
        {
            match fs::remove_file(entry.path()) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error)
                    if error.kind() == std::io::ErrorKind::PermissionDenied
                        || matches!(error.raw_os_error(), Some(32 | 33)) =>
                {
                    continue;
                }
                Err(error) => return Err(error.into()),
            }
            for suffix in ["sqlite-wal", "sqlite-shm", "sqlite.building"] {
                match fs::remove_file(folder.join(format!("{stamp}.{suffix}"))) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
    }
    Ok(())
}

fn stamp(root: &str, relative: &str, size: i64, mtime: i64) -> Hash {
    let mut hash = Sha256::new();
    hash.update(root);
    hash.update([0]);
    hash.update(relative);
    hash.update([0]);
    hash.update(size.to_le_bytes());
    hash.update(mtime.to_le_bytes());
    hash.finalize().into()
}

struct Target {
    id: i64,
    path: PathBuf,
    stamp: Hash,
    size: i64,
    mtime: i64,
    kind: Kind,
}

fn targets(cfg: &Config) -> Result<Vec<Target>> {
    let ingest_lock = ingest::open_ingest_lock(&cfg.data_dir)
        .map_err(|_| anyhow!("Kunci ingest gagal dibuka."))?;
    ingest_lock
        .try_lock_shared()
        .map_err(|_| anyhow!("Index sedang dipakai ingest. Coba lagi setelah selesai."))?;
    let (_version_lock, _index) = crate::eval::open_index(&cfg.data_dir)?;
    let db = Connection::open_with_flags(
        cfg.data_dir.join("rag-hub.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|_| anyhow!("Database dokumen gagal dibuka untuk dibaca."))?;
    let mut statement = db.prepare(&format!(
        "SELECT id, source, root, rel_path, size, mtime, visibility FROM {INDEX_TABLE}
         WHERE status != ? AND size >= 0 ORDER BY id"
    ))?;
    let mut rows = statement.query([ingest::status::PENDING])?;
    let data = cfg
        .data_dir
        .canonicalize()
        .map_err(|_| anyhow!("Folder data tidak tersedia."))?;
    let roots = cfg
        .sources
        .iter()
        .map(|source| {
            let root = source.path.canonicalize().map_err(|_| {
                anyhow!("Folder sumber tidak tersedia. Coba lagi setelah tersambung.")
            })?;
            Ok((source.name.as_str(), root))
        })
        .collect::<Result<HashMap<_, _>>>()?;
    let mut targets = Vec::new();
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let source: String = row.get(1)?;
        let root: String = row.get(2)?;
        let relative: String = row.get(3)?;
        let size = row.get(4)?;
        let mtime = row.get(5)?;
        let visibility: String = row.get(6)?;
        let Some(folder) = roots
            .get(source.as_str())
            .filter(|folder| folder.to_string_lossy() == root)
        else {
            continue;
        };
        if serde_json::from_str::<Visibility>(&visibility).is_err() {
            continue;
        }
        let Some(path) = documents::resolve(folder, &relative) else {
            continue;
        };
        if path.starts_with(&data) || !current(&path, size, mtime) {
            continue;
        }
        let Some(kind) = Kind::for_path(&path).unwrap_or(Some(Kind::Unreadable)) else {
            continue;
        };
        targets.push(Target {
            id,
            path,
            stamp: stamp(&root, &relative, size, mtime),
            size,
            mtime,
            kind,
        });
    }
    Ok(targets)
}

pub(crate) fn current(path: &Path, size: i64, mtime: i64) -> bool {
    fs::metadata(path).is_ok_and(|meta| ingest::file_stamp(&meta) == (size, mtime))
}

enum Outcome {
    Unchanged,
    Changed,
    Failed,
    Processed {
        counts: Counts,
        embedded: usize,
        failed: bool,
    },
}

struct Work<E> {
    store: Store,
    capacity: usize,
    encode: E,
    pending: Vec<(Hash, Prepared)>,
    vectors: Vectors,
}

fn image_hash(image: &RgbImage) -> Hash {
    let mut hash = Sha256::new();
    hash.update(image.width().to_le_bytes());
    hash.update(image.height().to_le_bytes());
    hash.update(image.as_raw());
    hash.finalize().into()
}

impl<E: FnMut(Vec<Prepared>) -> Result<Vec<Vec<f32>>> + Send> Work<E> {
    fn add(&mut self, location: String, image: RgbImage) -> Result<bool> {
        let hash = image_hash(&image);
        if let Some((locations, _)) = self.vectors.get_mut(&hash) {
            locations.push(location);
            return Ok(true);
        }
        let exists = self.store.has(&hash)?;
        self.vectors.insert(hash, (vec![location], Vec::new()));
        if exists {
            return Ok(true);
        }
        self.pending.push((hash, eg2::prepare(&image)));
        if self.pending.len() >= self.capacity {
            self.flush()?;
        }
        Ok(false)
    }

    fn flush(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let (keys, images): (Vec<_>, Vec<_>) = self.pending.drain(..).unzip();
        let rows = (self.encode)(images)?;
        ensure!(
            rows.len() == keys.len(),
            "Jumlah vektor gambar tidak cocok."
        );
        for (key, vector) in keys.into_iter().zip(rows) {
            self.vectors.get_mut(&key).expect("rujukan sudah dicatat").1 = vector;
        }
        Ok(())
    }

    fn process(&mut self, runtime: &tokio::runtime::Runtime, target: &Target) -> Result<Outcome> {
        let unchanged = || current(&target.path, target.size, target.mtime);
        if !unchanged() {
            return Ok(Outcome::Changed);
        }
        match self.store.begin(target.id, &target.stamp)? {
            FileState::Done => return Ok(Outcome::Unchanged),
            FileState::Failed => return Ok(Outcome::Failed),
            FileState::Work => {}
        }
        let request = worker::Request {
            path: target.path.clone(),
            selection: worker::Selection::All(target.kind),
        };
        let result = catch_unwind(AssertUnwindSafe(|| -> Result<Counts> {
            let result = runtime.block_on(worker::visit(request, &mut |location, image| {
                self.add(location, image)
            }))?;
            self.flush()?;
            Ok(result)
        }));
        let vectors = std::mem::take(&mut self.vectors);
        self.pending.clear();
        if !unchanged() {
            return Ok(Outcome::Changed);
        }
        let Ok(Ok(counts)) = result else {
            self.store.fail(target.id)?;
            return Ok(Outcome::Processed {
                counts: Counts::default(),
                embedded: 0,
                failed: true,
            });
        };
        if counts.pdf_parse_failed > 0 {
            self.store.fail(target.id)?;
            return Ok(Outcome::Processed {
                counts,
                embedded: 0,
                failed: true,
            });
        }
        let embedded = vectors
            .values()
            .filter(|(_, vector)| !vector.is_empty())
            .count();
        self.store.finish(target.id, vectors)?;
        Ok(Outcome::Processed {
            counts,
            embedded,
            failed: false,
        })
    }
}

fn run(cfg: &Config, dll: &Path, gpu: bool, batch: usize, limit: Option<usize>) -> Result<()> {
    let start = Instant::now();
    let model = cfg.gambar.as_ref().unwrap();
    let tokens = Tokens::load(&model.model)?;
    let path = store_path(&cfg.data_dir, model, tokens)?;
    let folder = path.parent().unwrap();
    fs::create_dir_all(folder).map_err(|_| anyhow!("Folder indeks gambar gagal disiapkan."))?;
    let lock = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(folder.join("lock"))
        .map_err(|_| anyhow!("Kunci gambar gagal dibuka."))?;
    lock.try_lock()
        .map_err(|_| anyhow!("Embed gambar lain sedang berjalan."))?;
    let targets = targets(cfg)?;
    let mut encoder = Encoder::open(model, dll, gpu, tokens)?;
    let marker = building_marker(&path);
    if !path.is_file() {
        File::create(&marker).map_err(|_| anyhow!("Penanda generasi gambar gagal dibuat."))?;
    }
    let stamps = targets
        .iter()
        .map(|target| (target.id, target.stamp))
        .collect();
    let mut store = Store::open(&path).map_err(|_| anyhow!("Simpanan gambar gagal dibuka."))?;
    store.reconcile(&stamps)?;
    let mut work = Work {
        store,
        capacity: batch,
        encode: move |images| encoder.embed(images),
        pending: Vec::new(),
        vectors: Vectors::new(),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut counts = [Counts::default(); 5];
    let (mut done, mut unchanged, mut changed) = (0_usize, 0, 0);
    let (mut failed, mut embedded) = (0, 0);
    let mut complete = true;
    println!("Target file={}", targets.len());
    for target in targets {
        if limit.is_some_and(|limit| done == limit) {
            complete = false;
            break;
        }
        let (result, fresh) = match work.process(&runtime, &target)? {
            Outcome::Unchanged => {
                unchanged += 1;
                continue;
            }
            Outcome::Changed => {
                changed += 1;
                continue;
            }
            Outcome::Failed => {
                failed += 1;
                continue;
            }
            Outcome::Processed {
                counts,
                embedded,
                failed: error,
            } => {
                failed += usize::from(error);
                (counts, embedded)
            }
        };
        done += 1;
        embedded += fresh;
        let sum = &mut counts[target.kind.group()];
        sum.taken += result.taken;
        sum.small += result.small;
        sum.duplicate += result.duplicate;
        sum.failed += result.failed;
        sum.pdf_skipped += result.pdf_skipped;
        sum.pdf_parse_failed += result.pdf_parse_failed;
        sum.pdf_text += result.pdf_text;
        sum.rendered += result.rendered;
        if done.is_multiple_of(25) {
            println!("Progres file={done} vektor_baru={embedded}");
        }
    }
    let (unique, refs) = work.store.counts()?;
    drop(work);
    if complete && changed == 0 {
        if marker.is_file() {
            fs::remove_file(marker).map_err(|_| anyhow!("Generasi gambar gagal diterbitkan."))?;
        }
        if remove_old_stores(&path).is_err() {
            eprintln!(
                "Simpanan gambar lama belum bisa dibersihkan. Coba lagi pada run berikutnya."
            );
        }
    }
    for (kind, count) in ["langsung", "docx", "pptx", "xlsx", "pdf"]
        .into_iter()
        .zip(counts)
    {
        println!(
            "Jenis {kind} diambil={} kecil={} duplikat={} gagal_decode={} pdf_dilewati={} pdf_gagal_baca={} pdf_ada_teks={} pdf_render={}",
            count.taken,
            count.small,
            count.duplicate,
            count.failed,
            count.pdf_skipped,
            count.pdf_parse_failed,
            count.pdf_text,
            count.rendered,
        );
    }
    let bytes = ["", "-wal", "-shm"]
        .iter()
        .filter_map(|suffix| fs::metadata(format!("{}{suffix}", path.display())).ok())
        .map(|meta| meta.len())
        .sum::<u64>();
    println!(
        "Total file={done} tetap={unchanged} berubah_saat_run={changed} gagal_file={failed} vektor_baru={embedded} unik={unique} rujukan={refs} byte={bytes} detik={:.2}",
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

// Langkah pencarian memakai jalur ini sebelum mengakses vektor atau rujukan.
#[allow(dead_code)]
pub(crate) async fn for_document(
    cfg: &Config,
    db: &Db,
    index: &std::sync::Arc<DocIndex>,
    user: &AuthUser,
    id: i64,
) -> Result<Vec<store::Entry>> {
    let Some(model) = cfg.gambar.as_ref().filter(|_| index.table() == INDEX_TABLE) else {
        return Ok(Vec::new());
    };
    let Some(document) = documents::rows(db, index.table(), vec![id])
        .await?
        .remove(&id)
    else {
        return Ok(Vec::new());
    };
    if document.visibility.is_none() || !document.allows_entry(user, &cfg.sources) {
        return Ok(Vec::new());
    }
    let (index, user, sources, data, model) = (
        index.clone(),
        user.clone(),
        cfg.sources.clone(),
        cfg.data_dir.clone(),
        model.clone(),
    );
    tokio::task::spawn_blocking(move || {
        if live::original(&index, &user, &sources, &document)?.is_none() {
            return Ok(Vec::new());
        }
        let stamp = stamp(
            &document.root,
            &document.rel_path,
            document.size,
            document.modified,
        );
        let tokens = Tokens::load(&model.model)?;
        let path = store_path(&data, &model, tokens)?;
        if !published(&path) {
            return Ok(Vec::new());
        }
        store::document(&path, id, &stamp)
    })
    .await?
}
