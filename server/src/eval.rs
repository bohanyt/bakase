//! Evaluasi pencarian lokal. Akses admin hanya dipakai oleh perintah ini.

mod prepare;
mod semantic;
#[cfg(test)]
mod tests;

use crate::acl::{AuthUser, Role};
use crate::config::{Config, SourceConfig};
use crate::db::Db;
use crate::documents;
use crate::embed::{Model, OrtEmbedder, default_threads};
use crate::hybrid::{self, NAME_ONLY_LIMIT};
use crate::search::{self, DocIndex, SearchFilters};
use anyhow::{Result, anyhow, bail, ensure};
use calamine::{DataType, Reader, Xlsx, open_workbook};
use semantic::{Ranked, Semantic};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SET: &str = "evaluation.xlsx";
const SEPARATOR: &str = " › ";
const HEADERS: [&str; 7] = [
    "ID",
    "Pakai",
    "Asal",
    "Pertanyaan",
    "Dokumen yang diharapkan",
    "Pertama di chat",
    "Catatan",
];

fn principal() -> AuthUser {
    AuthUser {
        id: 0,
        email: String::new(),
        name: String::new(),
        department: None,
        role: Role::Admin,
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Kata,
    Makna,
    Gabungan,
}

enum Ranker {
    Meaning(Semantic),
    Hybrid(Arc<hybrid::Ready>),
}

impl Mode {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "kata" => Ok(Self::Kata),
            "makna" => Ok(Self::Makna),
            "gabungan" => Ok(Self::Gabungan),
            _ => bail!("Mode evaluasi tidak dikenal. Pilih kata, makna, atau gabungan."),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Kata => "kata",
            Self::Makna => "makna",
            Self::Gabungan => "gabungan",
        }
    }

    async fn rank(
        self,
        index: &Arc<DocIndex>,
        db: &Db,
        sources: &[SourceConfig],
        question: &str,
        limit: usize,
        semantic: Option<&mut Ranker>,
    ) -> Result<Vec<DocumentKey>> {
        let ranked = match (self, semantic) {
            (Self::Kata, _) => documents::search_ranked(
                index.clone(),
                db,
                &principal(),
                sources,
                documents::keyword_query(question),
                limit,
                NAME_ONLY_LIMIT,
                false,
                SearchFilters::default(),
                true,
            )
            .await?
            .into_iter()
            .map(|(_, row)| Ranked {
                name_only: row.name_only(),
                key: (row.source, row.rel_path),
            })
            .collect(),
            (Self::Makna, Some(Ranker::Meaning(semantic))) => semantic.rank(question, limit)?,
            (Self::Gabungan, Some(Ranker::Hybrid(ready))) => {
                return Ok(hybrid::rank(
                    ready.clone(),
                    index.clone(),
                    db,
                    &principal(),
                    sources,
                    &documents::keyword_query(question),
                    limit,
                    true,
                    &SearchFilters::default(),
                )
                .await?
                .0
                .into_iter()
                .map(|(_, row)| (row.source, row.rel_path))
                .collect());
            }
            _ => bail!("Model evaluasi belum dimuat."),
        };
        Ok(ranked.into_iter().map(|doc| doc.key).collect())
    }
}

async fn load_ranker(
    mode: Mode,
    data: &Path,
    model: &Model,
    index: &DocIndex,
    db: &Db,
    sources: &[SourceConfig],
    open: impl FnOnce() -> Result<Box<dyn crate::embed::Embedder>>,
) -> Result<Ranker> {
    Ok(match mode {
        Mode::Makna => {
            Ranker::Meaning(Semantic::load(data, model, index, db, sources, open).await?)
        }
        Mode::Gabungan => Ranker::Hybrid(hybrid::Ready::for_eval(data, model, index, open()?)?),
        Mode::Kata => bail!("Mode kata tidak memakai model embedding."),
    })
}

pub async fn cli(command: &str, args: &[String]) -> Result<()> {
    let mut args = args.iter().map(String::as_str);
    let mut positional = Vec::new();
    let mut config = None;
    let mut overwrite = false;
    let mut mode = Mode::Kata;
    let mut model_folder = None;
    let mut dll = None;
    let preparing = command == "eval-siapkan";
    let mut limit = if preparing { 60 } else { 10 };
    while let Some(arg) = args.next() {
        match arg {
            "--timpa" if preparing => overwrite = true,
            "--maks" | "--k" if preparing == (arg == "--maks") => {
                limit = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .filter(|&n| n > 0)
                    .ok_or_else(|| anyhow!("{arg} harus diikuti bilangan bulat positif."))?;
            }
            "--config" if preparing => {
                config = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("Isi argumen --config."))?,
                );
            }
            "--mode" if !preparing => mode = Mode::parse(args.next().unwrap_or(""))?,
            "--model" if !preparing => {
                model_folder = Some(Path::new(
                    args.next().ok_or_else(|| anyhow!("Isi argumen --model."))?,
                ))
            }
            "--ort-dylib" if !preparing => {
                dll = Some(Path::new(
                    args.next()
                        .ok_or_else(|| anyhow!("Isi argumen --ort-dylib."))?,
                ))
            }
            _ if arg.starts_with('-') => bail!("Opsi evaluasi tidak dikenal. Lihat --help."),
            _ => positional.push(arg),
        }
    }
    if preparing {
        if config.is_none()
            && positional.len() > 1
            && positional.last().is_some_and(|arg| {
                Path::new(arg)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"))
            })
        {
            config = positional.pop();
        }
        ensure!(
            !positional.is_empty(),
            "Sertakan minimal satu folder ekspor."
        );
    } else {
        ensure!(positional.len() <= 1, "Sertakan paling banyak satu config.");
        config = positional.pop();
        if matches!(mode, Mode::Kata) {
            ensure!(
                model_folder.is_none(),
                "--model hanya untuk mode makna atau gabungan."
            );
        } else {
            ensure!(
                model_folder.is_some(),
                "Sertakan --model untuk mode makna atau gabungan."
            );
        }
    }
    let cfg = Config::load(&crate::config_path(config))
        .map_err(|_| anyhow!("Config evaluasi gagal dibaca atau tidak valid."))?;
    let dir = cfg.data_dir.join("eval");
    let set = dir.join(SET);
    if preparing {
        prepare::check_overwrite(&set, overwrite)?;
    }
    let _ingest = model_folder
        .map(|_| lock_ingest(&cfg.data_dir))
        .transpose()?;
    let db = Db::open_read_only(&cfg.data_dir)
        .map_err(|_| anyhow!("Database evaluasi gagal dibuka untuk dibaca."))?;
    let (_lock, index) = open_index(&cfg.data_dir)?;
    let index = Arc::new(index);
    if preparing {
        let exports = positional
            .into_iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        prepare::run(&cfg, &db, &index, &exports, limit, overwrite, &set).await
    } else {
        let model = model_folder.map(Model::load).transpose()?;
        let semantic = if let Some((folder, model)) = model_folder.zip(model.as_ref()) {
            Some(
                load_ranker(
                    mode,
                    &cfg.data_dir,
                    model,
                    &index,
                    &db,
                    &cfg.sources,
                    || {
                        Ok(Box::new(OrtEmbedder::open(
                            folder,
                            model,
                            dll,
                            default_threads(),
                            false,
                        )?))
                    },
                )
                .await?,
            )
        } else {
            None
        };
        run(
            (
                mode,
                model.as_ref().map(|model| model.name.as_str()),
                semantic,
            ),
            &index,
            &db,
            &cfg.sources,
            &set,
            limit,
        )
        .await
    }
}

fn lock_ingest(data: &Path) -> Result<File> {
    let lock =
        crate::ingest::open_ingest_lock(data).map_err(|_| anyhow!("Kunci ingest gagal dibuka."))?;
    match lock.try_lock_shared() {
        Ok(()) => Ok(lock),
        Err(std::fs::TryLockError::WouldBlock) => {
            bail!("Index sedang dikunci oleh ingest. Coba lagi setelah ingest selesai.")
        }
        Err(std::fs::TryLockError::Error(_)) => bail!("Kunci ingest gagal diambil."),
    }
}

pub(crate) fn open_index(data: &Path) -> Result<(File, DocIndex)> {
    let lock =
        search::version_lock(data).map_err(|_| anyhow!("Kunci versi index gagal dibuka."))?;
    match lock.try_lock_shared() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            bail!("Index sedang dikunci oleh ingest --ulang. Coba lagi setelah selesai.")
        }
        Err(std::fs::TryLockError::Error(_)) => bail!("Versi index gagal dikunci."),
    }
    let index_dir = search::version_dir(data, search::INDEX_VERSION);
    ensure!(
        index_dir.join(search::COMPLETE).is_file(),
        "Index saat ini belum lengkap. Jalankan ingest terlebih dahulu."
    );
    let index = DocIndex::open_current(&index_dir, false)
        .map_err(|_| anyhow!("Index gagal dibuka untuk dibaca."))?;
    Ok((lock, index))
}

type DocumentKey = (String, String);

struct Question {
    id: String,
    origin: String,
    text: String,
    expected: HashSet<DocumentKey>,
}

fn read_set(path: &Path) -> Result<Vec<Question>> {
    let invalid = || anyhow!("Set uji Excel gagal dibaca atau formatnya tidak valid.");
    let mut workbook: Xlsx<_> = open_workbook(path).map_err(|_| invalid())?;
    let range = workbook
        .worksheet_range_at(0)
        .ok_or_else(invalid)?
        .map_err(|_| invalid())?;
    let mut rows = range.rows();
    let header = rows.next().ok_or_else(invalid)?;
    let columns = HEADERS[..5]
        .iter()
        .map(|name| {
            header
                .iter()
                .position(|cell| cell.as_string().as_deref() == Some(name))
                .ok_or_else(invalid)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut questions = Vec::new();
    let mut ids = HashSet::new();
    for (i, row) in rows.enumerate() {
        let cell = |n: usize| {
            row.get(columns[n])
                .and_then(DataType::as_string)
                .unwrap_or_default()
        };
        if !cell(1).trim().eq_ignore_ascii_case("ya") {
            continue;
        }
        let bad_row = || anyhow!("Baris set uji {} tidak valid.", i + 2);
        let id = cell(0);
        ensure!(
            id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit()) && ids.insert(id.clone()),
            bad_row()
        );
        let origin = cell(2).trim().to_string();
        let text = cell(3);
        let expected = cell(4)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let (source, path) = line.split_once(SEPARATOR).ok_or_else(bad_row)?;
                let (source, path) = (source.trim(), path.trim());
                ensure!(!source.is_empty() && !path.is_empty(), bad_row());
                Ok((source.to_string(), path.replace('\\', "/")))
            })
            .collect::<Result<HashSet<_>>>()?;
        ensure!(
            !expected.is_empty() && !origin.is_empty() && !text.trim().is_empty(),
            bad_row()
        );
        questions.push(Question {
            id,
            origin,
            text,
            expected,
        });
    }
    Ok(questions)
}

#[derive(Default)]
struct Metrics {
    count: usize,
    hits: [usize; 3],
    reciprocal_sum: f64,
}

impl Metrics {
    fn add(&mut self, rank: Option<usize>) {
        self.count += 1;
        if let Some(rank) = rank {
            for (hit, k) in self.hits.iter_mut().zip([3, 6, 10]) {
                *hit += usize::from(rank <= k);
            }
            if rank <= 10 {
                self.reciprocal_sum += 1.0 / rank as f64;
            }
        }
    }

    fn print(&self, label: &str) {
        let n = self.count.max(1) as f64;
        let [hit3, hit6, hit10] = self.hits.map(|hits| hits as f64 / n);
        println!(
            "{label} | soal {} | hit@3 {hit3:.4} | hit@6 {hit6:.4} | hit@10 {hit10:.4} | MRR@10 {:.4}",
            self.count,
            self.reciprocal_sum / n
        );
    }
}

fn first_rank(expected: &HashSet<DocumentKey>, ranked: &[DocumentKey]) -> Option<usize> {
    ranked
        .iter()
        .position(|doc| expected.contains(doc))
        .map(|i| i + 1)
}

#[derive(Serialize)]
struct Detail {
    id: String,
    rank: Option<usize>,
    k: usize,
}

async fn run(
    (mode, model, mut semantic): (Mode, Option<&str>, Option<Ranker>),
    index: &Arc<DocIndex>,
    db: &Db,
    sources: &[SourceConfig],
    set: &Path,
    k: usize,
) -> Result<()> {
    let mut questions = read_set(set)?;
    ensure!(
        !questions.is_empty(),
        "Tidak ada baris set uji bertanda ya."
    );
    let keys = questions
        .iter()
        .flat_map(|q| q.expected.iter().cloned())
        .collect();
    let indexed = prepare::indexed_keys(db, index, keys)
        .await
        .map_err(|_| anyhow!("Status isi dokumen gagal dibaca dari database atau index."))?;
    let enabled = questions.len();
    questions.retain(|question| {
        let missing = question.expected.difference(&indexed).count();
        if missing == 0 {
            return true;
        }
        println!("ID {} | dokumen hilang {missing}", question.id);
        false
    });
    println!("Baris dilewati {}", enabled - questions.len());
    ensure!(
        !questions.is_empty(),
        "Semua baris set uji dilewati karena dokumen harapan tidak ditemukan di index."
    );

    let mut total = Metrics::default();
    let mut groups = BTreeMap::<String, Metrics>::new();
    let mut details = Vec::new();
    for question in questions {
        let ranked = mode
            .rank(
                index,
                db,
                sources,
                &question.text,
                k.max(10),
                semantic.as_mut(),
            )
            .await
            .map_err(|_| anyhow!("Pencarian gagal untuk ID {}.", question.id))?;
        let rank = first_rank(&question.expected, &ranked);
        total.add(rank);
        groups.entry(question.origin).or_default().add(rank);
        details.push(Detail {
            id: question.id,
            rank: rank.filter(|&rank| rank <= k),
            k,
        });
    }
    write_details(set.with_file_name(result_name(mode, model)), &details)?;
    total.print(&format!("Total {}", mode_label(mode, model)));
    for (i, (origin, metrics)) in groups.into_iter().enumerate() {
        let label = match origin.as_str() {
            "Review" | "Operasional" => origin,
            _ => format!("Asal {}", i + 1),
        };
        metrics.print(&label);
    }
    println!("ID meleset pada K={k}");
    for detail in details.iter().filter(|row| row.rank.is_none()) {
        println!("{}", detail.id);
    }
    Ok(())
}

fn mode_label(mode: Mode, model: Option<&str>) -> String {
    match model {
        Some(model) => format!("{}-{model}", mode.name()),
        None => mode.name().to_string(),
    }
}

fn result_name(mode: Mode, model: Option<&str>) -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "hasil-{:04}{:02}{:02}-{:02}{:02}{:02}-{}.json",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
        mode_label(mode, model)
    )
}

fn write_details(path: PathBuf, details: &[Detail]) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(details)?;
    let mut file = File::create_new(path).map_err(|_| {
        anyhow!(
            "Hasil evaluasi gagal dibuat. Periksa izin folder atau ulangi pada detik berikutnya."
        )
    })?;
    file.write_all(&bytes)
        .map_err(|_| anyhow!("Hasil evaluasi gagal ditulis."))
}
