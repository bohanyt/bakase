mod runtime;
mod store;

pub(crate) use runtime::{Embedder, OrtEmbedder, cosine, default_threads};
pub(crate) use store::Store;

#[cfg(test)]
pub(crate) use tests::{
    FakeEmbedder, write_test_artifacts, write_test_store, write_test_store_mode,
};

use crate::{config::MaknaConfig, search::DocIndex};
use anyhow::{Result, anyhow, bail, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

const MAX_BATCH: usize = 64;

#[derive(Debug)]
enum Failure {
    ModelMissing,
    Runtime,
    StoreBusy,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ModelMissing => "Model atau artefak model tidak ditemukan.",
            Self::Runtime => "Runtime ONNX gagal disiapkan.",
            Self::StoreBusy => "Simpanan vektor sedang dipakai. Coba lagi setelah selesai.",
        })
    }
}

impl std::error::Error for Failure {}

pub(crate) fn failure_category(error: &anyhow::Error) -> &'static str {
    match error.downcast_ref::<Failure>() {
        Some(Failure::ModelMissing) => "model tidak ditemukan",
        Some(Failure::Runtime) => "runtime/DLL",
        Some(Failure::StoreBusy) => "simpanan vektor sedang dipakai",
        None => "lainnya",
    }
}

fn model_file_error(error: std::io::Error, message: &'static str) -> anyhow::Error {
    if error.kind() == std::io::ErrorKind::NotFound {
        Failure::ModelMissing.into()
    } else {
        anyhow!("{message}")
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Pooling {
    Cls,
    Mean,
    Last,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_failures_keep_their_category_through_context() {
        for (error, category) in [
            (Failure::ModelMissing.into(), "model tidak ditemukan"),
            (Failure::Runtime.into(), "runtime/DLL"),
            (
                anyhow::Error::new(Failure::Runtime).context("membuka model sintetis"),
                "runtime/DLL",
            ),
            (Failure::StoreBusy.into(), "simpanan vektor sedang dipakai"),
            (anyhow!("kesalahan sintetis"), "lainnya"),
        ] {
            assert_eq!(failure_category(&error), category);
        }
    }

    pub(crate) struct FakeEmbedder;

    impl Embedder for FakeEmbedder {
        fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|text| {
                    if text.contains("torsi") || text.contains("pengencangan") {
                        vec![1., 0.]
                    } else {
                        vec![0., 1.]
                    }
                })
                .collect())
        }
    }

    pub(crate) fn write_test_store(
        data: &Path,
        model: &Model,
        index: &crate::search::DocIndex,
    ) -> Result<()> {
        write_test_store_mode(data, model, index, false)
    }

    pub(crate) fn write_test_store_mode(
        data: &Path,
        model: &Model,
        index: &crate::search::DocIndex,
        rebuild: bool,
    ) -> Result<()> {
        store::update(
            &model.store_dir(data),
            model,
            index,
            rebuild,
            2,
            Duration::ZERO,
            embed_fake_batch,
        )
    }

    pub(super) fn embed_fake_batch(texts: &[String]) -> Result<Vec<(Vec<f32>, bool)>> {
        Ok(FakeEmbedder
            .embed(texts)?
            .into_iter()
            .map(|vector| (vector, false))
            .collect())
    }

    pub(crate) fn write_test_artifacts(folder: &Path) {
        for name in ["model.onnx", "tokenizer.json"] {
            fs::write(folder.join(name), b"synthetic").unwrap();
        }
    }

    #[test]
    fn embed_rejects_batches_over_the_limit_before_opening_files() {
        let run = |batch: &str| {
            cli(&["--batch".into(), batch.into()])
                .unwrap_err()
                .to_string()
        };
        assert!(run("64").contains("Sertakan --model"));
        assert!(run("65").contains("--batch paling banyak 64"));
    }
}

#[derive(Clone, serde::Serialize, Deserialize, PartialEq, Eq)]
struct Artifact {
    bytes: u64,
    modified: SystemTime,
}

#[derive(Clone, serde::Serialize, Deserialize, PartialEq, Eq)]
struct Artifacts {
    onnx_in_subdir: bool,
    onnx: Artifact,
    onnx_data: Option<Artifact>,
    tokenizer: Artifact,
}

impl Artifacts {
    fn load(folder: &Path) -> Result<Self> {
        fn read(path: &Path) -> std::io::Result<Artifact> {
            let meta = fs::metadata(path)?;
            if !meta.is_file() {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
            Ok(Artifact {
                bytes: meta.len(),
                modified: meta.modified()?,
            })
        }
        let unavailable = || anyhow!("Metadata artefak model gagal dibaca.");
        let onnx_path = model_path(folder);
        let onnx_data = match read(&onnx_path.with_extension("onnx_data")) {
            Ok(artifact) => Some(artifact),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(unavailable()),
        };
        Ok(Self {
            onnx_in_subdir: onnx_path == folder.join("onnx/model.onnx"),
            onnx: read(&onnx_path)
                .map_err(|error| model_file_error(error, "Metadata artefak model gagal dibaca."))?,
            onnx_data,
            tokenizer: read(&folder.join("tokenizer.json"))
                .map_err(|error| model_file_error(error, "Metadata artefak model gagal dibaca."))?,
        })
    }
}

fn model_path(folder: &Path) -> PathBuf {
    let path = folder.join("model.onnx");
    if path.is_file() {
        path
    } else {
        // Ekspor unduhan dapat menyimpan ONNX dan bobot eksternal di onnx/.
        folder.join("onnx/model.onnx")
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Model {
    pub name: String,
    pooling: Pooling,
    pub dim: usize,
    #[serde(default = "max_tokens")]
    max_tokens: usize,
    #[serde(default)]
    pub query_prefix: String,
    #[serde(default)]
    pub document_prefix: String,
    normalize: bool,
    #[serde(skip)]
    hash: String,
    #[serde(skip)]
    artifacts: Option<Artifacts>,
}

fn max_tokens() -> usize {
    1024
}

fn valid_model_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default();
    let device = matches!(stem, "con" | "prn" | "aux" | "nul")
        || stem
            .strip_prefix("com")
            .or_else(|| stem.strip_prefix("lpt"))
            .is_some_and(|n| matches!(n.as_bytes(), [b'0'..=b'9']));
    !device
        && name.len() <= 80
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && !name.ends_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'.'))
}

impl Model {
    pub(crate) fn load(folder: &Path) -> Result<Self> {
        let raw = fs::read_to_string(folder.join("app-model.toml"))
            .map_err(|error| model_file_error(error, "app-model.toml gagal dibaca."))?;
        let mut model: Self =
            toml::from_str(&raw).map_err(|_| anyhow!("Format app-model.toml tidak valid."))?;
        ensure!(
            valid_model_name(&model.name)
                && model.dim > 0
                && model.dim <= 16384
                && model.max_tokens > 0
                && model.normalize,
            "Nama, dimensi, batas token, atau normalize pada config model tidak valid."
        );
        model.hash = hex::encode(Sha256::digest(raw.as_bytes()));
        model.artifacts = Some(Artifacts::load(folder)?);
        Ok(model)
    }

    pub(crate) fn store_dir(&self, data: &Path) -> PathBuf {
        data.join(format!("embed-v{}", crate::search::INDEX_VERSION))
            .join(&self.name)
    }
}

pub fn cli(args: &[String]) -> Result<()> {
    let mut args = args.iter().map(String::as_str);
    let (mut folder, mut config, mut dll) = (None, None, None);
    let (mut gpu, mut rebuild, mut batch, mut threads) = (false, false, 8, default_threads());
    while let Some(arg) = args.next() {
        match arg {
            "--model" => folder = Some(args.next().ok_or_else(|| anyhow!("Isi argumen --model."))?),
            "--ort-dylib" => {
                dll = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("Isi argumen --ort-dylib."))?,
                )
            }
            "--gpu" => gpu = true,
            "--ulang" => rebuild = true,
            "--batch" | "--threads" => {
                let value = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .filter(|&n| n > 0)
                    .ok_or_else(|| anyhow!("{arg} harus diikuti bilangan bulat positif."))?;
                if arg == "--batch" {
                    ensure!(value <= MAX_BATCH, "--batch paling banyak {MAX_BATCH}.");
                    batch = value;
                } else {
                    threads = value;
                }
            }
            _ if !arg.starts_with('-') && config.is_none() => config = Some(arg),
            _ => bail!("Opsi embed tidak dikenal. Lihat --help."),
        }
    }
    let folder = Path::new(folder.ok_or_else(|| anyhow!("Sertakan --model <folder>."))?);
    let model = Model::load(folder)?;
    let cfg = crate::config::Config::load(&crate::config_path(config))
        .map_err(|_| anyhow!("Config embed gagal dibaca atau tidak valid."))?;
    let (_lock, index) = crate::eval::open_index(&cfg.data_dir)?;
    update_index(
        &cfg.data_dir,
        &model,
        &index,
        rebuild,
        batch,
        Duration::ZERO,
        || OrtEmbedder::open(folder, &model, dll.map(Path::new), threads, gpu),
    )
}

pub(crate) fn after_ingest(data: &Path, cfg: &MaknaConfig, rebuild: bool) -> Result<()> {
    let model = Model::load(&cfg.model)?;
    let (_lock, index) = crate::eval::open_index(data)?;
    update_index(
        data,
        &model,
        &index,
        rebuild,
        8,
        Duration::from_secs(30),
        || {
            OrtEmbedder::open(
                &cfg.model,
                &model,
                cfg.ort_dylib.as_deref(),
                default_threads(),
                false,
            )
        },
    )
}

fn update_index(
    data: &Path,
    model: &Model,
    index: &DocIndex,
    rebuild: bool,
    batch: usize,
    lock_wait: Duration,
    open: impl FnOnce() -> Result<OrtEmbedder>,
) -> Result<()> {
    let mut open = Some(open);
    let mut encoder = None;
    store::update(
        &model.store_dir(data),
        model,
        index,
        rebuild,
        batch,
        lock_wait,
        |texts| {
            if encoder.is_none() {
                encoder = Some(open.take().unwrap()()?);
            }
            encoder.as_mut().unwrap().embed_counted(texts)
        },
    )
}
