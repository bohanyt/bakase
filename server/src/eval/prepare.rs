use super::*;
use crate::{documents::has_content, ingest};
use rust_xlsxwriter::{Format, Workbook};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{self, BufReader, Read};
use std::{collections::HashMap, fs};

#[derive(Deserialize)]
struct FileIndex {
    files: Vec<ExportFile>,
}

#[derive(Deserialize)]
struct ExportFile {
    document_id: String,
    bytes: Option<u64>,
    sha256: Option<String>,
    duplicate_of_document_id: Option<String>,
}

#[derive(Deserialize)]
struct Relations {
    answers: Vec<Answer>,
}

#[derive(Deserialize)]
struct Answer {
    question: String,
    question_number: u64,
    chat_id: String,
    question_message_id: String,
    #[serde(default)]
    ui_document_reference_cards: Vec<Card>,
}

#[derive(Deserialize)]
struct Card {
    match_status: String,
    #[serde(default)]
    document_ids: Vec<String>,
}

#[derive(Default)]
struct Counts {
    answers: usize,
    qualified: usize,
    mapped: usize,
    unmapped: usize,
    matched: usize,
    indexed: usize,
    selected: usize,
}

struct Export {
    origin: String,
    files: HashMap<String, ExportFile>,
    answers: Vec<Answer>,
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_reader(BufReader::new(File::open(path)?))?)
}

fn read_export(dir: &Path) -> Result<Option<Export>> {
    let relations = dir.join("answers");
    let entries = match fs::read_dir(relations) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound && dir.is_dir() => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut candidates = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry
                .file_name()
                .to_string_lossy()
                .ends_with("references.json")
        {
            candidates.push(entry.path());
        }
    }
    let [path] = candidates.as_slice() else {
        ensure!(candidates.is_empty(), "Lebih dari satu berkas relasi.");
        return Ok(None);
    };
    let Relations { answers } = read_json(path)?;
    let FileIndex { files } = read_json(&dir.join("files.json"))?;
    let folder = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Ekspor");
    let origin = match folder
        .split('_')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "review" => "Review",
        "production" | "operasional" => "Operasional",
        _ => folder,
    };
    Ok(Some(Export {
        origin: origin.to_string(),
        files: files
            .into_iter()
            .map(|f| (f.document_id.clone(), f))
            .collect(),
        answers,
    }))
}

fn resolve<'a>(files: &'a HashMap<String, ExportFile>, mut id: &'a str) -> Option<(String, u64)> {
    let mut visited = HashSet::new();
    while visited.insert(id) {
        let record = files.get(id)?;
        if let Some(sha) = record.sha256.as_deref().filter(|sha| !sha.is_empty()) {
            return (sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| sha.to_ascii_lowercase())
                .zip(record.bytes);
        }
        id = record
            .duplicate_of_document_id
            .as_deref()
            .filter(|id| !id.is_empty())?;
    }
    None
}

#[derive(Debug)]
struct Candidate {
    export: usize,
    origin: String,
    digest: [u8; 32],
    question: String,
    normalized: String,
    references: Vec<String>,
    expected: Vec<DocumentKey>,
    first: bool,
    dropped: usize,
}

fn first_questions<'a>(answers: impl IntoIterator<Item = &'a Answer>) -> HashMap<String, u64> {
    let mut first = HashMap::<String, u64>::new();
    for answer in answers {
        let number = first
            .entry(answer.chat_id.clone())
            .or_insert(answer.question_number);
        *number = (*number).min(answer.question_number);
    }
    first
}

fn is_japanese_letter(c: char) -> bool {
    c.is_alphabetic()
        && (search::is_japanese_script_block(c)
            || matches!(c as u32,
                0x3005 | 0x3007 | 0x31F0..=0x31FF
                | 0x1B000..=0x1B16F | 0x20000..=0x2FA1F | 0x30000..=0x323AF))
}

fn collect(
    export: Export,
    i: usize,
    counts: &mut Counts,
    sizes: &mut HashMap<u64, HashSet<String>>,
    first: &HashMap<String, u64>,
) -> Vec<Candidate> {
    counts.answers = export.answers.len();
    let mut candidates = Vec::new();
    for answer in export.answers {
        let words = answer.question.split_whitespace().collect::<Vec<_>>();
        if (words.len() < 3
            && answer
                .question
                .chars()
                .filter(|&c| is_japanese_letter(c))
                .take(6)
                .count()
                < 6)
            || answer.ui_document_reference_cards.is_empty()
            || answer.ui_document_reference_cards.iter().any(|card| {
                card.match_status != "matched_unique"
                    || card.document_ids.is_empty()
                    || card.document_ids.iter().any(|id| id.trim().is_empty())
            })
        {
            continue;
        }
        counts.qualified += 1;
        let mut references = Vec::new();
        let mut dropped = 0;
        for id in answer
            .ui_document_reference_cards
            .into_iter()
            .flat_map(|c| c.document_ids)
        {
            if let Some((sha, size)) = resolve(&export.files, &id) {
                counts.mapped += 1;
                sizes.entry(size).or_default().insert(sha.clone());
                references.push(sha);
            } else {
                counts.unmapped += 1;
                dropped += 1;
            }
        }
        let normalized = words.join(" ").to_lowercase();
        let mut hash = Sha256::new();
        hash.update((answer.chat_id.len() as u64).to_be_bytes());
        hash.update(answer.chat_id.as_bytes());
        hash.update(answer.question_message_id.as_bytes());
        candidates.push(Candidate {
            export: i,
            origin: export.origin.clone(),
            digest: hash.finalize().into(),
            question: answer.question,
            normalized,
            references,
            expected: Vec::new(),
            first: first[&answer.chat_id] == answer.question_number,
            dropped,
        });
    }
    candidates
}

fn matching_sha(path: &Path, sizes: &HashMap<u64, HashSet<String>>) -> io::Result<Option<String>> {
    let size = fs::metadata(path)?.len();
    let Some(needed) = sizes.get(&size) else {
        return Ok(None);
    };
    let mut file = File::open(path)?.take(size.saturating_add(1));
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    if file.limit() != 1 {
        return Ok(None);
    }
    let sha = hex::encode(digest.finalize());
    Ok(needed.contains(&sha).then_some(sha))
}

#[derive(Default)]
struct SourceMatches {
    paths: HashMap<String, Vec<DocumentKey>>,
    unavailable: usize,
}

fn match_sources(
    sources: &[SourceConfig],
    data: &Path,
    sizes: &HashMap<u64, HashSet<String>>,
) -> Result<SourceMatches> {
    let mut matches = SourceMatches::default();
    let skip = fs::canonicalize(data)?;
    for source in sources {
        let Ok(root) = fs::canonicalize(&source.path) else {
            matches.unavailable += 1;
            continue;
        };
        if root.starts_with(&skip) {
            continue;
        }
        if sizes.is_empty() {
            matches.unavailable += usize::from(fs::read_dir(&source.path).is_err());
            continue;
        }
        let mut files = Vec::new();
        if ingest::walk(&source.path, Some(&skip), &mut files).is_err() {
            matches.unavailable += 1;
            continue;
        }
        for file in files {
            if let Some(path) = ingest::rel_path(&source.path, &file)
                && let Ok(Some(sha)) = matching_sha(&file, sizes)
            {
                matches
                    .paths
                    .entry(sha)
                    .or_default()
                    .push((source.name.clone(), path));
            }
        }
    }
    Ok(matches)
}

pub(super) async fn indexed_keys(
    db: &Db,
    index: &Arc<DocIndex>,
    keys: HashSet<DocumentKey>,
) -> Result<HashSet<DocumentKey>> {
    let index = index.clone();
    db.call(move |conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT id, status FROM {} WHERE source = ?1 AND rel_path = ?2 AND chunks > 0",
            index.table()
        ))?;
        let user = principal();
        let mut indexed = HashSet::new();
        for key in keys {
            let mut rows = stmt.query([&key.0, &key.1])?;
            if let Some(row) = rows.next()?
                && has_content(&row.get::<_, String>(1)?)
                && index
                    .document_chunks(row.get(0)?, &user, 0, 1)?
                    .is_some_and(|(_, hits)| hits.iter().any(|(_, hit)| !hit.text.is_empty()))
            {
                indexed.insert(key);
            }
        }
        Ok(indexed)
    })
    .await
}

fn select(mut candidates: Vec<Candidate>, max: usize) -> Vec<Candidate> {
    candidates.sort_by(|a, b| {
        (!a.first, a.digest, &a.origin, &a.normalized, &a.expected).cmp(&(
            !b.first,
            b.digest,
            &b.origin,
            &b.normalized,
            &b.expected,
        ))
    });
    let mut seen = HashSet::new();
    candidates.retain(|c| seen.insert(c.normalized.clone()));

    let available = candidates.len();
    let target = max.min(available);
    let mut groups = BTreeMap::<usize, Vec<Candidate>>::new();
    for c in candidates {
        groups.entry(c.export).or_default().push(c);
    }
    let mut quotas: BTreeMap<_, usize> = groups.keys().map(|&i| (i, 0)).collect();
    let mut production_picks = 0;
    for _ in 0..target {
        let next_group = |production_only: bool| {
            groups
                .iter()
                .filter(|(i, group)| {
                    quotas[i] < group.len()
                        && (!production_only || group[0].origin == "Operasional")
                })
                .max_by_key(|(i, group)| {
                    // Selisih dari jatah proporsional, dalam satuan 1/available.
                    let deficit = group.len() as i128 * target as i128
                        - quotas[i] as i128 * available as i128;
                    (deficit, std::cmp::Reverse(group[quotas[i]].digest))
                })
        };
        let (i, group) = (production_picks < 10)
            .then(|| next_group(true))
            .flatten()
            .or_else(|| next_group(false))
            .expect("jatah selalu lebih kecil dari jumlah kandidat");
        *quotas.get_mut(i).expect("jatah per ekspor") += 1;
        production_picks += usize::from(group[0].origin == "Operasional");
    }
    let mut selected: Vec<_> = groups
        .into_iter()
        .flat_map(|(i, group)| group.into_iter().take(quotas[&i]))
        .collect();
    selected.sort_by_key(|c| (!c.first, c.digest));
    selected
}

pub(super) fn check_overwrite(path: &Path, overwrite: bool) -> Result<()> {
    ensure!(
        overwrite
            || !path
                .try_exists()
                .map_err(|_| anyhow!("Set uji gagal diperiksa."))?,
        "Set uji sudah ada. Gunakan --timpa untuk menggantinya."
    );
    Ok(())
}

fn workbook(rows: &[Candidate]) -> Result<Workbook> {
    let mut book = Workbook::new();
    let sheet = book.add_worksheet();
    let header = Format::new().set_bold().set_text_wrap();
    let wrapped = Format::new()
        .set_text_wrap()
        .set_align(rust_xlsxwriter::FormatAlign::Top);
    sheet.set_freeze_panes(1, 0)?;
    for (col, (label, width)) in HEADERS
        .into_iter()
        .zip([20, 9, 18, 65, 75, 20, 35])
        .enumerate()
    {
        sheet.set_column_width(col as u16, width)?;
        sheet.write_string_with_format(0, col as u16, label, &header)?;
    }
    for (row, c) in (1..).zip(rows) {
        let expected = c
            .expected
            .iter()
            .map(|(source, path)| format!("{source}{SEPARATOR}{path}"))
            .collect::<Vec<_>>()
            .join("\n");
        let id = hex::encode(&c.digest[..8]);
        for (col, value) in [
            id.as_str(),
            "ya",
            &c.origin,
            &c.question,
            &expected,
            if c.first { "ya" } else { "tidak" },
        ]
        .into_iter()
        .enumerate()
        {
            sheet.write_string_with_format(row, col as u16, value, &wrapped)?;
        }
        if c.dropped > 0 {
            sheet.write_string_with_format(
                row,
                6,
                format!("{} referensi lain tidak ditemukan di index", c.dropped),
                &wrapped,
            )?;
        }
    }
    Ok(book)
}

fn write_set(path: &Path, rows: &[Candidate], overwrite: bool) -> Result<()> {
    check_overwrite(path, overwrite)?;
    let bytes = workbook(rows)
        .and_then(|mut book| Ok(book.save_to_buffer()?))
        .map_err(|_| anyhow!("Set uji Excel gagal dibuat."))?;
    let failed = || anyhow!("Set uji Excel gagal ditulis. Periksa izin dan ruang folder evaluasi.");
    fs::create_dir_all(path.parent().expect("folder set uji")).map_err(|_| failed())?;
    let dest = if overwrite {
        path.with_extension(format!("{:016x}.tmp", rand::random::<u64>()))
    } else {
        path.to_path_buf()
    };
    let mut file = File::create_new(&dest).map_err(|e| {
        if !overwrite && e.kind() == io::ErrorKind::AlreadyExists {
            anyhow!("Set uji sudah ada. Gunakan --timpa untuk menggantinya.")
        } else {
            failed()
        }
    })?;
    let written = file.write_all(&bytes);
    drop(file);
    let written = written.and_then(|()| {
        if overwrite {
            fs::rename(&dest, path)
        } else {
            Ok(())
        }
    });
    if written.is_err() {
        let _ = fs::remove_file(dest);
        return Err(failed());
    }
    Ok(())
}

fn production_report<'a>(exports: impl IntoIterator<Item = &'a Export>) -> String {
    let count = exports
        .into_iter()
        .filter(|export| export.origin == "Operasional")
        .count();
    let mut report = format!("Folder ekspor Operasional {count}");
    if count == 0 {
        report.push_str("\nMinimum soal Operasional tidak diterapkan.");
    }
    report
}

pub(super) async fn run(
    cfg: &Config,
    db: &Db,
    index: &Arc<DocIndex>,
    dirs: &[PathBuf],
    max: usize,
    overwrite: bool,
    set: &Path,
) -> Result<()> {
    let mut exports = Vec::new();
    for (i, dir) in (1..).zip(dirs) {
        if let Some(export) = read_export(dir)
            .map_err(|_| anyhow!("Ekspor {i} gagal dibaca atau formatnya tidak valid."))?
        {
            exports.push((i, export));
        }
    }
    println!(
        "{}",
        production_report(exports.iter().map(|(_, export)| export))
    );
    let unsupported = dirs.len() - exports.len();
    let first = first_questions(exports.iter().flat_map(|(_, export)| &export.answers));
    let mut counts = BTreeMap::new();
    let mut candidates = Vec::new();
    let mut sizes = HashMap::new();
    for (i, export) in exports {
        candidates.extend(collect(
            export,
            i,
            counts.entry(i).or_default(),
            &mut sizes,
            &first,
        ));
    }
    let matches = match_sources(&cfg.sources, &cfg.data_dir, &sizes)
        .map_err(|_| anyhow!("Folder sumber gagal dipindai."))?;
    println!(
        "Folder sumber hilang atau tidak bisa dibaca {}",
        matches.unavailable
    );
    let keys = matches.paths.values().flatten().cloned().collect();
    let indexed = indexed_keys(db, index, keys)
        .await
        .map_err(|_| anyhow!("Status isi dokumen gagal dibaca dari database atau index."))?;
    for c in &mut candidates {
        let count = counts.get_mut(&c.export).expect("ekspor kandidat");
        for sha in &c.references {
            if let Some(paths) = matches.paths.get(sha) {
                count.matched += 1;
                let indexed_paths = paths.iter().filter(|&key| indexed.contains(key));
                if indexed_paths.clone().next().is_some() {
                    count.indexed += 1;
                    c.expected.extend(indexed_paths.cloned());
                    continue;
                }
            }
            c.dropped += 1;
        }
        c.expected.sort_unstable();
        c.expected.dedup();
    }
    candidates.retain(|c| !c.expected.is_empty());
    let selected = select(candidates, max);
    for c in &selected {
        counts.get_mut(&c.export).expect("ekspor terpilih").selected += 1;
    }
    for (i, c) in counts {
        println!(
            "Ekspor {i} | jawaban {} | memenuhi syarat {} | ref dengan SHA {} | tanpa SHA {} | SHA cocok {} | ref terindeks {} | soal dipilih {}",
            c.answers, c.qualified, c.mapped, c.unmapped, c.matched, c.indexed, c.selected
        );
    }
    println!("Ekspor tanpa format yang didukung {unsupported}");
    println!("Soal dipilih {}", selected.len());
    // Set yang sudah direview tidak boleh tertimpa set kosong, juga dengan --timpa.
    ensure!(
        !selected.is_empty(),
        "Tidak ada soal yang lolos, jadi set uji tidak ditulis."
    );
    write_set(set, &selected, overwrite)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::tests::{Temp, export_fixture};
    use serde_json::json;

    fn answer(chat: &str, number: u64, text: &str) -> serde_json::Value {
        json!({
            "question": text, "question_number": number,
            "chat_id": chat, "question_message_id": number.to_string(),
            "answer": "Isi buatan", "all_retrieval_reference_records": [],
            "ui_document_reference_cards": [
                { "match_status": "matched_unique", "document_ids": ["salinan"] }
            ]
        })
    }

    #[test]
    fn reads_exports_and_follows_duplicate_chains() {
        let temp = Temp::new();
        let production = temp.0.join("Production_Chat_Uji");
        let sha = "ab".repeat(32);
        let own_sha = "cd".repeat(32);
        export_fixture(
            &production,
            json!([
                { "document_id": "salinan", "duplicate_of_document_id": "antara" },
                { "document_id": "antara", "duplicate_of_document_id": "asli", "sha256": "" },
                { "document_id": "asli", "sha256": sha.to_uppercase(), "bytes": 25 },
                { "document_id": "beda", "sha256": own_sha, "bytes": 10, "duplicate_of_document_id": "asli" },
                { "document_id": "kosong", "duplicate_of_document_id": "" },
                { "document_id": "", "sha256": sha, "bytes": 25 },
                { "document_id": "siklus", "duplicate_of_document_id": "siklus" }
            ]),
            json!([answer("chat", 1, "satu dua tiga")]),
        );
        let export = read_export(&production).unwrap().unwrap();
        assert_eq!(export.origin, "Operasional");
        assert_eq!(production_report([&export]), "Folder ekspor Operasional 1");
        assert_eq!(
            production_report([]),
            "Folder ekspor Operasional 0\nMinimum soal Operasional tidak diterapkan."
        );
        assert_eq!(export.answers.len(), 1);
        assert_eq!(resolve(&export.files, "salinan"), Some((sha, 25)));
        assert_eq!(resolve(&export.files, "beda"), Some((own_sha, 10)));
        assert_eq!(resolve(&export.files, "kosong"), None);
        assert_eq!(resolve(&export.files, "siklus"), None);
        assert_eq!(resolve(&export.files, "hilang"), None);

        let example = temp.0.join("Contoh/answers");
        fs::create_dir_all(&example).unwrap();
        fs::write(example.join("answers-only.json"), r#"{"records":[]}"#).unwrap();
        assert!(read_export(example.parent().unwrap()).unwrap().is_none());
    }

    #[test]
    fn matches_only_needed_sizes_and_keeps_duplicate_paths() {
        let temp = Temp::new();
        let data = temp.0.join("runtime");
        fs::create_dir(&data).unwrap();
        let source = SourceConfig {
            name: "Uji".into(),
            path: temp.0.clone(),
            visibility: crate::acl::Visibility::Umum,
        };
        for path in [
            "a.txt",
            "b.txt",
            ".rahasia.txt",
            "~$sementara.txt",
            "runtime/lokal.txt",
        ] {
            fs::write(temp.0.join(path), b"abc").unwrap();
        }
        let sha = hex::encode(Sha256::digest(b"abc"));
        let sizes = HashMap::from([(4, HashSet::from([sha.clone()]))]);
        assert!(
            match_sources(std::slice::from_ref(&source), &data, &sizes)
                .unwrap()
                .paths
                .is_empty()
        );
        let sizes = HashMap::from([(3, HashSet::from([sha.clone()]))]);
        let matches = match_sources(&[source], &data, &sizes).unwrap();
        let mut paths = matches.paths[&sha].clone();
        paths.sort();
        assert_eq!(
            paths,
            [
                ("Uji".into(), "a.txt".into()),
                ("Uji".into(), "b.txt".into())
            ]
        );
    }

    #[test]
    fn counts_missing_and_unreadable_source_folders() {
        let temp = Temp::new();
        let data = temp.0.join("runtime");
        fs::create_dir(&data).unwrap();
        fs::write(temp.0.join("bukan-folder"), b"uji").unwrap();
        let sources = ["hilang", "bukan-folder"].map(|name| SourceConfig {
            name: name.into(),
            path: temp.0.join(name),
            visibility: crate::acl::Visibility::Umum,
        });
        for sizes in [HashMap::new(), HashMap::from([(3, HashSet::new())])] {
            let result = match_sources(&sources, &data, &sizes).unwrap();
            assert_eq!(result.unavailable, 2);
            assert!(result.paths.is_empty());
        }
    }

    fn candidates(
        origin: &str,
        i: usize,
        answers: Vec<serde_json::Value>,
    ) -> (Counts, Vec<Candidate>) {
        let mut counts = Counts::default();
        let mut sizes = HashMap::new();
        let answers: Vec<Answer> = serde_json::from_value(json!(answers)).unwrap();
        let first = first_questions(&answers);
        let mut rows = collect(
            Export {
                origin: origin.into(),
                files: HashMap::new(),
                answers,
            },
            i,
            &mut counts,
            &mut sizes,
            &first,
        );
        for row in &mut rows {
            row.expected = vec![
                ("Uji".into(), "a.txt".into()),
                ("Uji".into(), "b.txt".into()),
            ];
        }
        (counts, rows)
    }

    #[test]
    fn uses_first_question_before_filtering_and_dedupes_normalized_text() {
        let mut ambiguous = answer("chat", 1, "rujukan terlalu banyak");
        ambiguous["ui_document_reference_cards"][0]["match_status"] = json!("matched_multiple_ids");
        let mut empty = answer("chat", 1, "tanpa dokumen harapan");
        empty["ui_document_reference_cards"][0]["document_ids"] = json!([]);
        let (counts, rows) = candidates(
            "Review",
            1,
            vec![
                answer("chat", 3, "baris kedua chat"),
                answer("chat", 1, "dua kata"),
                ambiguous,
                empty,
                answer("chat", 2, " AWAL   percakapan INI "),
                answer("chat", 4, "awal percakapan ini"),
            ],
        );
        assert_eq!(
            (counts.answers, counts.qualified, counts.unmapped),
            (6, 3, 3)
        );
        let selected = select(rows, 10);
        assert_eq!(selected.len(), 2);
        assert!(selected.iter().all(|c| !c.first));
        assert_eq!(
            selected
                .iter()
                .filter(|c| c.normalized == "awal percakapan ini")
                .count(),
            1
        );
    }

    #[test]
    fn accepts_six_japanese_letters_without_spaces() {
        let (_, rows) = candidates(
            "Review",
            1,
            vec![
                answer("a", 1, "質をチェック"),
                answer("b", 1, "品質の確認？"),
                answer("c", 1, "가나다라마바"),
                answer("d", 1, "satu dua tiga"),
            ],
        );
        assert_eq!(
            rows.iter().map(|c| c.question.as_str()).collect::<Vec<_>>(),
            ["質をチェック", "satu dua tiga"]
        );
        assert!(rows.iter().all(|c| c.first));
    }

    #[test]
    fn selection_is_proportional_deterministic_and_reserves_production() {
        let build = |review, prod| {
            let mut rows = candidates(
                "Review",
                1,
                (0..review)
                    .map(|i| answer(&format!("review-{i}"), 1, &format!("soal mutu nomor {i}")))
                    .collect(),
            )
            .1;
            rows.extend(
                candidates(
                    "Operasional",
                    2,
                    (0..prod)
                        .map(|i| {
                            answer(
                                &format!("prod-{i}"),
                                1,
                                &format!("soal operasional nomor {i}"),
                            )
                        })
                        .collect(),
                )
                .1,
            );
            rows
        };
        let normal = select(build(80, 20), 60);
        assert_eq!(
            normal.iter().filter(|r| r.origin == "Operasional").count(),
            12
        );
        let selected = select(build(90, 10), 60);
        assert_eq!(
            selected
                .iter()
                .filter(|r| r.origin == "Operasional")
                .count(),
            10
        );
        let mut reversed = build(90, 10);
        reversed.reverse();
        let ids = |rows: Vec<Candidate>| rows.into_iter().map(|c| c.digest).collect::<Vec<_>>();
        assert_eq!(ids(selected), ids(select(reversed, 60)));
    }

    #[test]
    fn workbook_round_trip_and_overwrite_refusal() {
        let temp = Temp::new();
        let path = temp.0.join(SET);
        let rows = candidates(
            "Review",
            1,
            vec![
                answer("chat", 1, "satu dua tiga"),
                answer("chat", 2, "empat lima enam"),
            ],
        )
        .1;
        write_set(&path, &rows, false).unwrap();
        let original = fs::read(&path).unwrap();
        assert!(
            write_set(&path, &rows[..1], false)
                .unwrap_err()
                .to_string()
                .contains("--timpa")
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        write_set(&path, &rows[..1], true).unwrap();
        assert_eq!(read_set(&path).unwrap().len(), 1);
        let mut book = workbook(&rows).unwrap();
        let sheet = book.worksheet_from_index(0).unwrap();
        sheet.write_string(1, 1, " YA ").unwrap();
        sheet.write_string(2, 1, "tidak").unwrap();
        book.save(&path).unwrap();
        let questions = read_set(&path).unwrap();
        assert_eq!(questions.len(), 1);
        assert_eq!(
            questions[0].expected,
            rows[0].expected.iter().cloned().collect()
        );
    }
}
