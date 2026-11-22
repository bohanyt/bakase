//! Pembacaan lampiran lewat jalur server sungguhan (`rag-hub-server read-attachment`): server
//! menjalankan proses pembaca (`extract-attachment`) di dalam job dengan batas memori. File
//! yang dibuat untuk menghabiskan memori hanya mematikan proses pembaca; server tetap jalan
//! dan menjawab dengan pesan penolakan.

use std::io::Write;
use std::process::{Command, Output, Stdio};

#[cfg(windows)]
#[path = "../src/worker_job.rs"]
mod worker_job;

#[cfg(windows)]
#[tokio::test]
async fn image_worker_requires_a_job_and_the_job_enforces_its_deadline() {
    use tokio::io::AsyncWriteExt;
    let spawned = || {
        tokio::process::Command::new(env!("CARGO_BIN_EXE_rag-hub-server"))
            .arg("__baca-gambar")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap()
    };
    let path =
        std::env::temp_dir().join(format!("app-gambar-worker-{}.png", rand::random::<u64>()));
    image::RgbImage::new(128, 128).save(&path).unwrap();
    let request = serde_json::json!({"path":path,"selection":{"All":"Direct"}}).to_string() + "\n";

    let mut unprotected = spawned();
    unprotected
        .stdin
        .take()
        .unwrap()
        .write_all(request.as_bytes())
        .await
        .unwrap();
    let denied = unprotected.wait_with_output().await.unwrap();
    assert!(!denied.status.success() && denied.stdout.is_empty());

    let mut protected = spawned();
    let job = worker_job::WorkerJob::attach(&protected, 768 * 1024 * 1024).unwrap();
    protected
        .stdin
        .take()
        .unwrap()
        .write_all(request.as_bytes())
        .await
        .unwrap();
    let decoded = protected.wait_with_output().await.unwrap();
    drop(job);
    std::fs::remove_file(path).unwrap();
    assert!(
        decoded.status.success(),
        "status {:?}, keluaran {} byte, {}",
        decoded.status.code(),
        decoded.stdout.len(),
        String::from_utf8_lossy(&decoded.stderr),
    );
    let size = u32::from_le_bytes(decoded.stdout[..4].try_into().unwrap()) as usize;
    let header: serde_json::Value = serde_json::from_slice(&decoded.stdout[4..4 + size]).unwrap();
    assert_eq!(header["Image"], serde_json::json!(["", 128, 128]));

    let mut expired = spawned();
    let job = worker_job::WorkerJob::attach(&expired, 768 * 1024 * 1024).unwrap();
    let _deadline = job.deadline(std::time::Duration::ZERO).unwrap();
    let _open_stdin = expired.stdin.take().unwrap();
    assert_eq!(expired.wait().await.unwrap().code(), Some(1));
}

fn run(command: &str, kind: &str, bytes: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rag-hub-server"))
        .arg(command)
        .arg(kind)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // Proses bisa sudah berhenti sebelum semua isi terkirim.
    let _ = child.stdin.take().unwrap().write_all(bytes);
    child.wait_with_output().unwrap()
}

/// Hasil `read-attachment`: ringkasan JSON (jumlah karakter, atau pesan penolakan).
fn read(kind: &str, bytes: &[u8]) -> serde_json::Value {
    let out = run("read-attachment", kind, bytes);
    assert!(out.status.success(), "server ikut berhenti");
    serde_json::from_slice(&out.stdout).unwrap()
}

fn docx(text: &str) -> Vec<u8> {
    use docx_rs::{Docx, Paragraph, Run};
    let mut out = std::io::Cursor::new(Vec::new());
    Docx::new()
        .add_paragraph(Paragraph::new().add_run(Run::new().add_text(text)))
        .build()
        .pack(&mut out)
        .unwrap();
    out.into_inner()
}

/// Workbook minimal yang ditulis tangan: `sheet` adalah isi `<sheetData>`, `strings` isi
/// `xl/sharedStrings.xml` kalau ada.
fn xlsx(sheet: &str, strings: Option<&str>) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut out);
    let opts = zip::write::SimpleFileOptions::default();
    let mut put = |name: &str, body: &str| {
        zip.start_file(name, opts).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    };
    let sst = strings.is_some();
    put(
        "[Content_Types].xml",
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>{}</Types>"#,
            if sst {
                r#"<Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>"#
            } else {
                ""
            }
        ),
    );
    put(
        "_rels/.rels",
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
    );
    put(
        "xl/workbook.xml",
        r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
    );
    put(
        "xl/_rels/workbook.xml.rels",
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>{}</Relationships>"#,
            if sst {
                r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>"#
            } else {
                ""
            }
        ),
    );
    put(
        "xl/worksheets/sheet1.xml",
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{sheet}</sheetData></worksheet>"#
        ),
    );
    if let Some(strings) = strings {
        put("xl/sharedStrings.xml", strings);
    }
    zip.finish().unwrap();
    out.into_inner()
}

#[test]
fn documents_are_read_through_the_worker() {
    let summary = read("docx", &docx("Teks lampiran uji"));
    assert!(summary["chars"].as_u64().unwrap() > 0, "{summary}");
    let summary = read(
        "xlsx",
        &xlsx(r#"<row r="1"><c r="A1"><v>12</v></c></row>"#, None),
    );
    assert_eq!(summary["sheets"], 1, "{summary}");
}

#[test]
fn the_worker_refuses_to_run_without_the_server_limit() {
    let out = run("extract-attachment", "docx", &docx("Teks"));
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    let out = run("extract-attachment", "exe", b"MZ");
    assert!(!out.status.success());
}

#[test]
fn far_apart_cells_are_rejected_with_a_message() {
    let sheet =
        r#"<row r="1"><c r="A1"><v>1</v></c></row><row r="6000"><c r="XFD6000"><v>1</v></c></row>"#;
    let summary = read("xlsx", &xlsx(sheet, None));
    assert!(
        summary["rejected"]
            .as_str()
            .unwrap()
            .contains("terlalu besar"),
        "{summary}"
    );
}

/// Jumlah shared string raksasa: pembaca Excel memesan memori sebanyak itu saat membuka file.
/// Hanya proses pembaca yang mati.
#[test]
fn a_huge_shared_string_count_only_kills_the_worker() {
    let strings = r#"<?xml version="1.0" encoding="UTF-8"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="1000000000000" uniqueCount="1000000000000"><si><t>a</t></si></sst>"#;
    let sheet = r#"<row r="1"><c r="A1" t="s"><v>0</v></c></row>"#;
    let summary = read("xlsx", &xlsx(sheet, Some(strings)));
    assert!(summary["rejected"].is_string(), "{summary}");
}

/// Rumus shared dengan indeks raksasa: pembaca rumus memesan daftar sepanjang indeks itu.
/// Batas memori job mematikan proses pembaca.
#[cfg(windows)]
#[test]
fn a_huge_shared_formula_index_only_kills_the_worker() {
    let sheet =
        r#"<row r="1"><c r="A1"><f t="shared" si="100000000" ref="A1">1</f><v>1</v></c></row>"#;
    let summary = read("xlsx", &xlsx(sheet, None));
    assert!(summary["rejected"].is_string(), "{summary}");
}
