use super::{Artifacts, Model, cosine};
use crate::search::{DocIndex, Hit, INDEX_VERSION};
use anyhow::{Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions, TryLockError},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

#[derive(Deserialize, Serialize, PartialEq, Eq, Hash)]
struct ChunkKey {
    doc_id: i64,
    chunk_id: String,
    fingerprint: String,
}

#[derive(Deserialize, Serialize)]
struct Row {
    #[serde(flatten)]
    key: ChunkKey,
    truncated: bool,
}

impl ChunkKey {
    fn from_hit(hit: &Hit, text: &str) -> Self {
        Self {
            doc_id: hit.doc_id,
            chunk_id: hit.chunk_id.clone(),
            // ID Tantivy hanya mencakup isi. Judul juga bisa berubah saat file berganti nama.
            fingerprint: hex::encode(Sha256::digest(text.as_bytes())),
        }
    }
}

fn embedding_text(hit: &Hit) -> String {
    format!("{}\n{}", hit.title, hit.text)
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
struct Manifest {
    format: u32,
    index_version: i64,
    model: String,
    dim: usize,
    query_prefix: String,
    document_prefix: String,
    config_sha256: String,
    #[serde(default)]
    artifacts: Option<Artifacts>,
}

impl From<&Model> for Manifest {
    fn from(model: &Model) -> Self {
        Self {
            format: 2,
            index_version: INDEX_VERSION,
            model: model.name.clone(),
            dim: model.dim,
            query_prefix: model.query_prefix.clone(),
            document_prefix: model.document_prefix.clone(),
            config_sha256: model.hash.clone(),
            artifacts: model.artifacts.clone(),
        }
    }
}

pub(crate) struct Store {
    rows: Vec<Row>,
    vectors: Vec<f32>,
    dim: usize,
}

#[derive(Clone)]
pub(crate) struct Entry {
    pub doc_id: i64,
    pub source: String,
    pub name_only: bool,
}

fn invalid() -> anyhow::Error {
    anyhow!(
        "Simpanan vektor tidak valid atau gagal diakses. Periksa izin folder atau jalankan embed --ulang."
    )
}

fn missing() -> anyhow::Error {
    anyhow!("Vektor untuk model dan versi index ini belum ada. Jalankan embed.")
}

fn generation(dir: &Path) -> Result<Option<PathBuf>> {
    let pointer = match fs::read_to_string(dir.join("current")) {
        Ok(pointer) => pointer,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(invalid()),
    };
    ensure!(generation_name(&pointer), invalid());
    Ok(Some(dir.join(pointer)))
}

fn generation_name(name: &str) -> bool {
    name.strip_prefix("vektor-v")
        .is_some_and(|s| s.len() == 16 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_reader(std::io::BufReader::new(
        File::open(path).map_err(|_| invalid())?,
    ))
    .map_err(|_| invalid())
}

fn read_lock(dir: &Path) -> Result<File> {
    let lock = File::open(dir.join("lock")).map_err(|_| missing())?;
    lock.try_lock_shared()
        .map_err(|_| anyhow!("Embed model sedang berjalan. Coba lagi setelah selesai."))?;
    Ok(lock)
}

fn write_lock(dir: &Path, wait: Duration) -> Result<File> {
    fs::create_dir_all(dir).map_err(|_| invalid())?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("lock"))
        .map_err(|_| invalid())?;
    let deadline = Instant::now() + wait;
    loop {
        match lock.try_lock() {
            Ok(()) => return Ok(lock),
            Err(TryLockError::Error(_)) => return Err(invalid()),
            Err(TryLockError::WouldBlock) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(super::Failure::StoreBusy.into());
                }
                thread::sleep(remaining.min(Duration::from_millis(100)));
            }
        }
    }
}

impl Store {
    pub(crate) fn current(dir: &Path) -> Result<PathBuf> {
        generation(dir)?.ok_or_else(missing)
    }

    pub(crate) fn open_generation(dir: &Path, generation: &Path, model: &Model) -> Result<Self> {
        let _lock = read_lock(dir)?;
        ensure!(Self::current(dir)? == generation, invalid());
        Self::read_generation(generation, model)
    }

    #[cfg(test)]
    pub(crate) fn synthetic(vectors: Vec<f32>, dim: usize) -> Self {
        Self {
            rows: Vec::new(),
            vectors,
            dim,
        }
    }

    fn read(dir: &Path, model: &Model) -> Result<Option<Self>> {
        generation(dir)?
            .map(|dir| Self::read_generation(&dir, model))
            .transpose()
    }

    fn read_generation(dir: &Path, model: &Model) -> Result<Self> {
        let manifest: Manifest = read_json(&dir.join("manifest.json"))?;
        ensure!(
            manifest == Manifest::from(model),
            "Config model, artefak model, atau format simpanan berbeda. Jalankan embed --ulang."
        );
        let rows: Vec<Row> = read_json(&dir.join("rows.json"))?;
        let file = File::open(dir.join("vectors.f32")).map_err(|_| invalid())?;
        let length = file.metadata().map_err(|_| invalid())?.len();
        ensure!(
            rows.len()
                .checked_mul(model.dim)
                .and_then(|n| n.checked_mul(4))
                .map(|n| n as u64)
                == Some(length)
                && rows
                    .iter()
                    .map(|row| &row.key.chunk_id)
                    .collect::<HashSet<_>>()
                    .len()
                    == rows.len(),
            invalid()
        );
        let mut vectors = Vec::with_capacity(rows.len() * model.dim);
        let mut input = BufReader::new(file);
        let mut bytes = vec![0; model.dim * 4];
        for _ in &rows {
            input.read_exact(&mut bytes).map_err(|_| invalid())?;
            vectors.extend(
                bytes
                    .chunks_exact(4)
                    .map(|value| f32::from_le_bytes(value.try_into().unwrap())),
            );
        }
        ensure!(vectors.chunks_exact(model.dim).all(valid_vector), invalid());
        Ok(Self {
            rows,
            vectors,
            dim: model.dim,
        })
    }

    pub(crate) fn matching_vectors(
        &self,
        index: &DocIndex,
        snapshot: &tantivy::Searcher,
        mut visit: impl FnMut(tantivy::DocAddress, Hit, Option<usize>),
    ) -> Result<()> {
        let by_key: HashMap<_, _> = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, row)| (row.key.chunk_id.as_str(), i))
            .collect();
        index.visit_snapshot(snapshot, |address, hit| {
            let vector = by_key
                .get(hit.chunk_id.as_str())
                .copied()
                .filter(|&i| self.rows[i].key == ChunkKey::from_hit(&hit, &embedding_text(&hit)));
            visit(address, hit, vector);
            Ok(())
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    pub(crate) fn load_current(
        dir: &Path,
        model: &Model,
        index: &DocIndex,
    ) -> Result<(Self, Vec<Entry>)> {
        let _lock = read_lock(dir)?;
        let store = Self::read(dir, model)?.ok_or_else(missing)?;
        let mut by_key: HashMap<_, _> = store
            .rows
            .iter()
            .enumerate()
            .map(|(i, row)| (&row.key, i))
            .collect();
        let mut hits = vec![None; store.rows.len()];
        let stale = || anyhow!("Vektor belum sesuai dengan index saat ini. Jalankan embed lagi.");
        index.visit_chunks(|hit| {
            let key = ChunkKey::from_hit(&hit, &embedding_text(&hit));
            let i = by_key.remove(&key).ok_or_else(stale)?;
            hits[i] = Some(Entry {
                doc_id: hit.doc_id,
                source: hit.source,
                name_only: hit.text.is_empty(),
            });
            Ok(())
        })?;
        ensure!(by_key.is_empty(), stale());
        Ok((store, hits.into_iter().flatten().collect()))
    }

    pub(crate) fn scores<'a>(&'a self, query: &'a [f32]) -> impl Iterator<Item = f32> + 'a {
        self.vectors
            .chunks_exact(self.dim)
            .map(|row| cosine(row, query))
    }

    pub(crate) fn vector(&self, i: usize) -> &[f32] {
        &self.vectors[i * self.dim..(i + 1) * self.dim]
    }
}

fn valid_vector(v: &[f32]) -> bool {
    (cosine(v, v) - 1.0).abs() < 0.001
}

struct Pending {
    dir: PathBuf,
    pointer: PathBuf,
    name: String,
    published: bool,
}

impl Pending {
    fn new(root: &Path) -> Result<Self> {
        let name = format!("vektor-v{:016x}", rand::random::<u64>());
        let dir = root.join(&name);
        fs::create_dir(&dir).map_err(|_| invalid())?;
        let pointer = root.join(format!("{name}.tmp"));
        Ok(Self {
            dir,
            pointer,
            name,
            published: false,
        })
    }

    fn publish(&mut self, root: &Path) -> Result<()> {
        let mut file = File::create_new(&self.pointer).map_err(|_| invalid())?;
        file.write_all(self.name.as_bytes())
            .map_err(|_| invalid())?;
        file.sync_all().map_err(|_| invalid())?;
        drop(file);
        // A single rename switches readers to all three files together.
        fs::rename(&self.pointer, root.join("current")).map_err(|_| invalid())?;
        self.published = true;
        Ok(())
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_file(&self.pointer);
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut out = BufWriter::new(File::create_new(path).map_err(|_| invalid())?);
    serde_json::to_writer(&mut out, value).map_err(|_| invalid())?;
    out.flush().map_err(|_| invalid())?;
    out.get_ref().sync_all().map_err(|_| invalid())?;
    Ok(())
}

#[derive(Default, Debug, PartialEq, Eq)]
struct Counts {
    embedded: usize,
    skipped: usize,
    removed: usize,
    truncated: usize,
    stale: usize,
}

pub(super) fn update(
    dir: &Path,
    model: &Model,
    index: &DocIndex,
    rebuild: bool,
    batch_size: usize,
    lock_wait: Duration,
    mut embed: impl FnMut(&[String]) -> Result<Vec<(Vec<f32>, bool)>>,
) -> Result<()> {
    let started = Instant::now();
    let counts = update_store(
        dir, model, index, rebuild, batch_size, lock_wait, &mut embed,
    )?;
    println!(
        "Potongan {} | baru {} | dilewati {} | dihapus {} | terpangkas {} | detik {:.2}",
        counts.embedded + counts.skipped,
        counts.embedded,
        counts.skipped,
        counts.removed,
        counts.truncated,
        started.elapsed().as_secs_f64()
    );
    println!("Snapshot tertinggal {}", counts.stale);
    Ok(())
}

fn update_store(
    dir: &Path,
    model: &Model,
    index: &DocIndex,
    rebuild: bool,
    batch_size: usize,
    lock_wait: Duration,
    embed: &mut impl FnMut(&[String]) -> Result<Vec<(Vec<f32>, bool)>>,
) -> Result<Counts> {
    let _lock = write_lock(dir, lock_wait)?;
    let check_snapshot = || anyhow!("Snapshot index gagal diperiksa. Jalankan embed lagi.");
    // Catat sebelum reload agar commit yang bersamaan juga terdeteksi.
    let opstamp = index.commit_opstamp().map_err(|_| check_snapshot())?;
    index.reload().map_err(|_| check_snapshot())?;
    let mut counts = write_generation(dir, model, index, rebuild, batch_size, embed)?;
    counts.stale = usize::from(index.commit_opstamp().map_err(|_| check_snapshot())? != opstamp);
    Ok(counts)
}

fn write_generation(
    dir: &Path,
    model: &Model,
    index: &DocIndex,
    rebuild: bool,
    batch_size: usize,
    embed: &mut impl FnMut(&[String]) -> Result<Vec<(Vec<f32>, bool)>>,
) -> Result<Counts> {
    let previous = if rebuild {
        None
    } else {
        Store::read(dir, model)?
    };
    let mut old: HashMap<_, _> = previous
        .iter()
        .flat_map(|store| store.rows.iter().enumerate())
        .map(|(i, row)| (row.key.chunk_id.as_str(), (row, i)))
        .collect();
    let mut seen = HashSet::new();
    let mut counts = Counts::default();
    let mut pending = Pending::new(dir)?;
    let mut output =
        BufWriter::new(File::create_new(pending.dir.join("vectors.f32")).map_err(|_| invalid())?);
    let mut rows = Vec::new();
    let mut keys = Vec::new();
    let mut texts = Vec::new();
    let mut flush = |keys: &mut Vec<_>,
                     texts: &mut Vec<_>,
                     rows: &mut Vec<Row>,
                     output: &mut BufWriter<File>,
                     counts: &mut Counts|
     -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let vectors = embed(texts)?;
        ensure!(
            vectors.len() == keys.len()
                && vectors
                    .iter()
                    .all(|(row, _)| row.len() == model.dim && valid_vector(row)),
            "Model menghasilkan vektor yang tidak valid."
        );
        for (key, (vector, truncated)) in keys.drain(..).zip(vectors) {
            write_vector(output, &vector)?;
            rows.push(Row { key, truncated });
            counts.truncated += usize::from(truncated);
            counts.embedded += 1;
        }
        texts.clear();
        Ok(())
    };
    index.visit_chunks(|hit| {
        let text = embedding_text(&hit);
        let key = ChunkKey::from_hit(&hit, &text);
        ensure!(
            seen.insert(key.chunk_id.clone()),
            "ID potongan ganda di index."
        );
        if let Some((row, i)) = old
            .remove(key.chunk_id.as_str())
            .filter(|(prior, _)| prior.key == key)
        {
            write_vector(&mut output, previous.as_ref().unwrap().vector(i))?;
            rows.push(Row {
                key,
                truncated: row.truncated,
            });
            counts.truncated += usize::from(row.truncated);
            counts.skipped += 1;
        } else {
            keys.push(key);
            texts.push(format!("{}{text}", model.document_prefix));
            if keys.len() == batch_size {
                flush(&mut keys, &mut texts, &mut rows, &mut output, &mut counts)?;
            }
        }
        if seen.len().is_multiple_of(1000) {
            println!("Potongan diperiksa {}", seen.len());
        }
        Ok(())
    })?;
    flush(&mut keys, &mut texts, &mut rows, &mut output, &mut counts)?;
    counts.removed = old.len();
    if previous.is_some() && counts.embedded == 0 && counts.removed == 0 {
        return Ok(counts);
    }
    output.flush().map_err(|_| invalid())?;
    output.get_ref().sync_all().map_err(|_| invalid())?;
    drop(output);
    write_json(&pending.dir.join("rows.json"), &rows)?;
    write_json(&pending.dir.join("manifest.json"), &Manifest::from(model))?;
    pending.publish(dir)?;
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name();
        if name != pending.name.as_str()
            && name
                .to_str()
                .is_some_and(|s| generation_name(s.strip_suffix(".tmp").unwrap_or(s)))
        {
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                let _ = fs::remove_dir_all(entry.path());
            } else {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    Ok(counts)
}

fn write_vector(output: &mut BufWriter<File>, vector: &[f32]) -> Result<()> {
    for value in vector {
        output
            .write_all(&value.to_le_bytes())
            .map_err(|_| invalid())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        embed::tests::{embed_fake_batch, write_test_artifacts},
        search::NewChunk,
    };

    struct Temp(PathBuf);

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const SETTINGS: &str = "name = 'uji-0.6b'\npooling = 'cls'\ndim = 2\nnormalize = true\n";

    fn model_fixture() -> (Temp, Model) {
        let root =
            Temp(std::env::temp_dir().join(format!("app-embed-{:016x}", rand::random::<u64>())));
        fs::create_dir(&root.0).unwrap();
        fs::write(root.0.join("app-model.toml"), SETTINGS).unwrap();
        write_test_artifacts(&root.0);
        let model = Model::load(&root.0).unwrap();
        (root, model)
    }

    #[test]
    fn refuses_artifact_changes_until_rebuilt() {
        let (root, model) = model_fixture();
        let dir = model.store_dir(&root.0);
        let index = DocIndex::in_memory().unwrap();
        update_store(
            &dir,
            &model,
            &index,
            false,
            2,
            Duration::ZERO,
            &mut embed_fake_batch,
        )
        .unwrap();
        for artifact in ["model.onnx", "model.onnx_data", "tokenizer.json"] {
            fs::write(root.0.join(artifact), b"replacement artifact").unwrap();
            let changed = Model::load(&root.0).unwrap();
            let error = update_store(
                &dir,
                &changed,
                &index,
                false,
                2,
                Duration::ZERO,
                &mut embed_fake_batch,
            )
            .unwrap_err();
            assert!(error.to_string().contains("embed --ulang"));
            update_store(
                &dir,
                &changed,
                &index,
                true,
                2,
                Duration::ZERO,
                &mut embed_fake_batch,
            )
            .unwrap();
        }
        let model = Model::load(&root.0).unwrap();
        let modified = model.artifacts.as_ref().unwrap().tokenizer.modified;
        File::options()
            .write(true)
            .open(root.0.join("tokenizer.json"))
            .unwrap()
            .set_modified(modified - std::time::Duration::from_secs(60))
            .unwrap();
        let changed = Model::load(&root.0).unwrap();
        let error = update_store(
            &dir,
            &changed,
            &index,
            false,
            2,
            Duration::ZERO,
            &mut embed_fake_batch,
        )
        .unwrap_err();
        assert!(error.to_string().contains("embed --ulang"));
    }

    #[test]
    fn publishes_complete_generations_and_updates_incrementally() {
        let (root, model) = model_fixture();
        let dir = model.store_dir(&root.0);
        let index = DocIndex::in_memory().unwrap();
        let mut writer = index.writer().unwrap();
        let add = |writer: &tantivy::IndexWriter, id, title, body| {
            index
                .add_chunk(
                    writer,
                    NewChunk {
                        doc_id: id,
                        n: 0,
                        title,
                        text: body,
                        source: "Uji",
                        rel_path: "uji.txt",
                        acl: &["umum".into()],
                        page: None,
                        file_type: "txt",
                        years: &[],
                    },
                )
                .unwrap();
        };
        add(&writer, 1, "torsi", "uji");
        add(&writer, 2, "torsi", "tetap");
        add(&writer, 3, "contoh", "hapus");
        writer.commit().unwrap();
        index.reload().unwrap();
        let mut fake = embed_fake_batch;
        let first =
            update_store(&dir, &model, &index, false, 2, Duration::ZERO, &mut fake).unwrap();
        assert_eq!(
            first,
            Counts {
                embedded: 3,
                skipped: 0,
                ..Counts::default()
            }
        );
        let original = generation(&dir).unwrap().unwrap();
        let (store, _) = Store::load_current(&dir, &model, &index).unwrap();
        assert_eq!(
            store.scores(&[1., 0.]).filter(|&score| score == 1.).count(),
            2
        );
        let unchanged = update_store(&dir, &model, &index, false, 2, Duration::ZERO, &mut |_| {
            panic!("inference on unchanged index")
        })
        .unwrap();
        assert_eq!(
            unchanged,
            Counts {
                embedded: 0,
                skipped: 3,
                ..Counts::default()
            }
        );
        assert_eq!(generation(&dir).unwrap().unwrap(), original);

        index.delete_document(&writer, 1);
        index.delete_document(&writer, 3);
        add(&writer, 1, "judul baru", "uji");
        add(&writer, 4, "baru", "isi baru");
        writer.commit().unwrap();
        index.reload().unwrap();
        assert!(
            Store::load_current(&dir, &model, &index)
                .err()
                .unwrap()
                .to_string()
                .contains("belum sesuai")
        );
        assert!(
            update_store(
                &dir,
                &model,
                &index,
                false,
                1,
                Duration::ZERO,
                &mut |_| Err(anyhow!("gagal inferensi"))
            )
            .is_err()
        );
        assert_eq!(generation(&dir).unwrap().unwrap(), original);
        assert_eq!(Store::read(&dir, &model).unwrap().unwrap().rows.len(), 3);
        let updated =
            update_store(&dir, &model, &index, false, 1, Duration::ZERO, &mut fake).unwrap();
        assert_eq!(
            updated,
            Counts {
                embedded: 2,
                skipped: 1,
                removed: 1,
                ..Counts::default()
            }
        );
        assert!(!original.exists());
        let (store, _) = Store::load_current(&dir, &model, &index).unwrap();
        assert_eq!(
            store
                .rows
                .iter()
                .map(|row| row.key.doc_id)
                .collect::<HashSet<_>>(),
            [1, 2, 4].into()
        );
        assert_eq!(
            store.scores(&[1., 0.]).filter(|&score| score == 1.).count(),
            1
        );

        fs::write(
            root.0.join("app-model.toml"),
            format!("{SETTINGS}query_prefix = 'cari '\n"),
        )
        .unwrap();
        let changed = Model::load(&root.0).unwrap();
        let error =
            update_store(&dir, &changed, &index, false, 2, Duration::ZERO, &mut fake).unwrap_err();
        assert!(error.to_string().contains("embed --ulang"));
        assert_eq!(
            update_store(&dir, &changed, &index, true, 2, Duration::ZERO, &mut fake)
                .unwrap()
                .embedded,
            3
        );
        assert!(Store::load_current(&dir, &changed, &index).is_ok());
    }

    #[test]
    fn detects_a_commit_during_embedding_and_reuses_the_snapshot_vectors() {
        let (root, model) = model_fixture();
        let dir = model.store_dir(&root.0);
        let index_dir = root.0.join("index-uji");
        let ingest_index = DocIndex::open_current(&index_dir, true).unwrap();
        let mut writer = ingest_index.writer().unwrap();
        let add = |id| NewChunk {
            doc_id: id,
            n: 0,
            title: "Judul uji",
            text: "Isi sintetis",
            source: "Uji",
            rel_path: "uji.txt",
            acl: &[],
            page: None,
            file_type: "txt",
            years: &[],
        };
        ingest_index.add_chunk(&writer, add(1)).unwrap();
        writer.commit().unwrap();
        let index = DocIndex::open_current(&index_dir, false).unwrap();
        let counts = update_store(&dir, &model, &index, false, 1, Duration::ZERO, &mut |_| {
            ingest_index.add_chunk(&writer, add(2)).unwrap();
            writer.commit().unwrap();
            Ok(vec![(vec![1., 0.], true)])
        })
        .unwrap();
        assert_eq!(
            counts,
            Counts {
                embedded: 1,
                truncated: 1,
                stale: 1,
                ..Counts::default()
            }
        );
        index.reload().unwrap();
        let error = Store::load_current(&dir, &model, &index).err().unwrap();
        assert!(error.to_string().contains("belum sesuai"));
        let counts = update_store(
            &dir,
            &model,
            &index,
            false,
            1,
            Duration::ZERO,
            &mut embed_fake_batch,
        )
        .unwrap();
        assert_eq!(
            counts,
            Counts {
                embedded: 1,
                skipped: 1,
                truncated: 1,
                ..Counts::default()
            }
        );
        assert!(Store::load_current(&dir, &model, &index).is_ok());
    }
}
