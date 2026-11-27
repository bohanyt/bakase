use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tantivy::directory::error::{DeleteError, LockError, OpenReadError, OpenWriteError};
use tantivy::directory::{
    Directory, DirectoryLock, FileHandle, Lock, MmapDirectory as TantivyDirectory, WatchCallback,
    WatchHandle, WritePtr,
};

const STALE_TEMP: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Debug)]
pub(super) struct MmapDirectory {
    dir: TantivyDirectory,
    path: PathBuf,
}

impl MmapDirectory {
    pub(super) fn open(path: &Path) -> tantivy::Result<Self> {
        Ok(Self {
            dir: TantivyDirectory::open(path)?,
            path: path.to_owned(),
        })
    }
}

impl Directory for MmapDirectory {
    fn get_file_handle(&self, path: &Path) -> Result<Arc<dyn FileHandle>, OpenReadError> {
        self.dir.get_file_handle(path)
    }

    fn delete(&self, path: &Path) -> Result<(), DeleteError> {
        self.dir.delete(path)
    }

    fn exists(&self, path: &Path) -> Result<bool, OpenReadError> {
        self.dir.exists(path)
    }

    fn open_write(&self, path: &Path) -> Result<WritePtr, OpenWriteError> {
        self.dir.open_write(path)
    }

    fn atomic_read(&self, path: &Path) -> Result<Vec<u8>, OpenReadError> {
        self.dir.atomic_read(path)
    }

    fn atomic_write(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        let target = self.path.join(path);
        let temp = target.with_file_name(format!(".app-index-{:032x}.tmp", rand::random::<u128>()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        // std handles an open destination on Windows. Tantivy's tempfile persist
        // fails with access denied if a reader still has meta.json open.
        let renamed = file
            .write_all(data)
            .and_then(|()| file.sync_data())
            .and_then(|()| fs::rename(&temp, target));
        // Flush lagi lewat handle yang sama supaya nama baru ikut tersimpan sebelum commit
        // dilaporkan berhasil. Windows tidak punya fsync folder.
        let result = renamed.and_then(|()| file.sync_all());
        drop(file);
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }

    fn sync_directory(&self) -> io::Result<()> {
        self.dir.sync_directory()
    }

    fn acquire_lock(&self, lock: &Lock) -> Result<DirectoryLock, LockError> {
        self.dir.acquire_lock(lock)
    }

    fn watch(&self, callback: WatchCallback) -> tantivy::Result<WatchHandle> {
        self.dir.watch(callback)
    }
}

/// Menghapus file sementara dari proses yang mati sebelum rename. Dipanggil sambil memegang
/// lock penulis; file yang masih baru dilewati karena bisa milik commit yang sedang berjalan.
pub(super) fn remove_stale_temps(dir: &Path) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let stale = name.starts_with(".app-index-")
            && name.ends_with(".tmp")
            && entry
                .metadata()
                .and_then(|meta| meta.modified())
                .is_ok_and(|at| at.elapsed().is_ok_and(|age| age > STALE_TEMP));
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
#[test]
fn stale_temps_are_removed_without_touching_other_files() {
    let dir = std::env::temp_dir().join(format!("rag-hub-temp-{:032x}", rand::random::<u128>()));
    fs::create_dir(&dir).unwrap();
    let old = std::time::SystemTime::now() - 2 * STALE_TEMP;
    for (name, stale) in [
        (".app-index-lama.tmp", true),
        (".app-index-baru.tmp", false),
        ("meta.json", true),
        ("segmen.tmp", true),
    ] {
        let file = fs::File::create(dir.join(name)).unwrap();
        if stale {
            file.set_modified(old).unwrap();
        }
    }
    remove_stale_temps(&dir);
    let mut left: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(left, [".app-index-baru.tmp", "meta.json", "segmen.tmp"]);
}

#[cfg(test)]
#[test]
fn commit_replaces_metadata_while_a_reader_has_it_open() {
    use std::io::Read;

    let dir = std::env::temp_dir().join(format!("rag-hub-meta-{:032x}", rand::random::<u128>()));
    fs::create_dir(&dir).unwrap();
    let result = (|| -> anyhow::Result<_> {
        let index = super::DocIndex::open_for_ingest(&dir, true)?;
        let mut writer = index.writer()?;
        let meta = dir.join("meta.json");
        let old = fs::read(&meta)?;
        let mut reader = fs::File::open(&meta)?;
        let mut commit = writer.prepare_commit()?;
        commit.set_payload("uji");
        commit.commit()?;
        let mut previous = Vec::new();
        reader.read_to_end(&mut previous)?;
        let current = index.index.load_metas()?.payload;
        writer.wait_merging_threads()?;
        Ok((old, previous, current))
    })();
    let _ = fs::remove_dir_all(&dir);
    let (old, previous, current) = result.unwrap();
    assert_eq!(previous, old);
    assert_eq!(current.as_deref(), Some("uji"));
}
