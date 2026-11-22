//! Dokumen Word buatan Bakase. Model mengirim isi terstruktur (judul, heading, paragraf,
//! daftar, tabel), server membuat .docx dengan docx-rs. Isi yang sama dipakai untuk preview.

use docx_rs::{
    BreakType, Docx, Paragraph, Run, RunFonts, Shading, SpecialIndentType, Style, StyleType, Table,
    TableCell, TableRow,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const DOCX_MIME: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DocxSpec {
    pub filename: String,
    #[serde(default)]
    pub title: Option<String>,
    pub blocks: Vec<Block>,
}

fn two() -> u8 {
    2
}

/// Satu bagian dokumen.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Heading {
        #[serde(default = "two")]
        level: u8,
        text: String,
    },
    Paragraph {
        text: String,
    },
    Bullets {
        items: Vec<String>,
    },
    Numbered {
        items: Vec<String>,
    },
    Table {
        columns: Vec<String>,
        #[serde(default)]
        rows: Vec<Vec<Value>>,
    },
    PageBreak,
}

const MAX_BLOCKS: usize = 2_000;
const MAX_TEXT: usize = 20_000;
const MAX_ITEMS: usize = 500;
const MAX_TABLE_COLUMNS: usize = 20;
const MAX_TABLE_ROWS: usize = 2_000;

/// Skema tool `create_docx` untuk model.
pub fn docx_tool() -> Value {
    json!({
        "type": "function",
        "name": "create_docx",
        "description": "Buat dokumen Word (.docx) yang bisa diunduh user: surat, laporan, SOP, notulen, atau dokumen lain yang perlu dikirim atau dicetak.",
        "parameters": {
            "type": "object",
            "properties": {
                "filename": { "type": "string", "description": "Nama file tanpa ekstensi" },
                "title": { "type": "string", "description": "Judul dokumen di bagian paling atas (opsional)" },
                "blocks": {
                    "type": "array",
                    "description": "Isi dokumen berurutan.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "type": { "type": "string", "enum": ["heading", "paragraph", "bullets", "numbered", "table", "page_break"] },
                            "level": { "type": "integer", "description": "heading: 1, 2, atau 3" },
                            "text": { "type": "string", "description": "heading atau paragraph. **teks** dicetak tebal." },
                            "items": { "type": "array", "items": { "type": "string" }, "description": "bullets atau numbered" },
                            "columns": { "type": "array", "items": { "type": "string" }, "description": "table: judul kolom" },
                            "rows": { "type": "array", "items": { "type": "array", "items": { "type": ["string", "number", "boolean", "null"] } }, "description": "table: isi baris" }
                        },
                        "required": ["type"]
                    }
                }
            },
            "required": ["filename", "blocks"]
        }
    })
}

/// Skema tool `read_docx`: isi dokumen Word buatan Bakase di percakapan yang sama.
pub fn read_docx_tool() -> Value {
    json!({
        "type": "function",
        "name": "read_docx",
        "description": "Baca isi dokumen Word yang dibuat Bakase sebelumnya di percakapan ini (id file ada di riwayat), per halaman. Untuk mengubahnya, buat versi baru dengan create_docx yang memuat seluruh isi, termasuk bagian yang tidak berubah.",
        "parameters": {
            "type": "object",
            "properties": {
                "file_id": { "type": "string", "description": "Id file dari riwayat percakapan" },
                "from_block": { "type": "integer", "description": "Bagian pertama yang dibaca, mulai 0" }
            },
            "required": ["file_id"]
        }
    })
}

/// Memeriksa spesifikasi. Pesan error ditulis untuk model, supaya bisa memperbaiki panggilannya.
pub fn validate_docx(spec: &DocxSpec) -> Result<(), String> {
    if spec.blocks.is_empty() {
        return Err("Dokumen minimal punya satu bagian.".into());
    }
    if spec.blocks.len() > MAX_BLOCKS {
        return Err(format!("Maksimal {MAX_BLOCKS} bagian."));
    }
    let long = |t: &str| t.chars().count() > MAX_TEXT;
    if spec.title.as_deref().is_some_and(long) {
        return Err(format!("Judul maksimal {MAX_TEXT} karakter."));
    }
    for (i, block) in spec.blocks.iter().enumerate() {
        let at = format!("Bagian ke-{}", i + 1);
        match block {
            Block::Heading { level, text } => {
                if !(1..=3).contains(level) {
                    return Err(format!("{at}: level heading harus 1, 2, atau 3."));
                }
                if long(text) {
                    return Err(format!("{at}: teks terlalu panjang."));
                }
            }
            Block::Paragraph { text } if long(text) => {
                return Err(format!(
                    "{at}: paragraf maksimal {MAX_TEXT} karakter; pecah jadi beberapa paragraf."
                ));
            }
            Block::Bullets { items } | Block::Numbered { items } => {
                if items.is_empty() || items.len() > MAX_ITEMS {
                    return Err(format!("{at}: daftar harus berisi 1–{MAX_ITEMS} butir."));
                }
                if items.iter().any(|t| long(t)) {
                    return Err(format!("{at}: ada butir yang terlalu panjang."));
                }
            }
            Block::Table { columns, rows } => {
                if columns.is_empty() || columns.len() > MAX_TABLE_COLUMNS {
                    return Err(format!(
                        "{at}: tabel harus punya 1–{MAX_TABLE_COLUMNS} kolom."
                    ));
                }
                if rows.len() > MAX_TABLE_ROWS {
                    return Err(format!(
                        "{at}: tabel maksimal {MAX_TABLE_ROWS} baris; untuk data sebanyak itu pakai create_xlsx."
                    ));
                }
                if let Some(r) = rows.iter().position(|r| r.len() > columns.len()) {
                    return Err(format!(
                        "{at}: baris tabel ke-{} punya nilai lebih banyak dari kolomnya.",
                        r + 1
                    ));
                }
                if rows.iter().flatten().any(|v| v.is_object() || v.is_array()) {
                    return Err(format!(
                        "{at}: isi sel tabel harus teks, angka, benar/salah, atau kosong."
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Karakter kontrol tidak valid di XML Word, kecuali tab dan baris baru.
fn clean(text: &str) -> String {
    // XML 1.0 hanya menerima tab, baris baru, dan karakter di atas kontrol, kecuali U+FFFE/U+FFFF.
    text.chars()
        .filter(|c| {
            (!c.is_control() || matches!(c, '\t' | '\n')) && !matches!(c, '\u{FFFE}' | '\u{FFFF}')
        })
        .collect()
}

/// Teks dengan `**tebal**` dijadikan beberapa run. Tanda `**` yang tidak berpasangan
/// dibiarkan apa adanya.
fn runs(text: &str) -> Vec<Run> {
    let parts: Vec<&str> = text.split("**").collect();
    if parts.len().is_multiple_of(2) {
        return vec![Run::new().add_text(text)];
    }
    parts
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.is_empty())
        .map(|(i, p)| {
            let run = Run::new().add_text(*p);
            if i % 2 == 1 { run.bold() } else { run }
        })
        .collect()
}

fn paragraph_with(text: &str) -> Paragraph {
    runs(text)
        .into_iter()
        .fold(Paragraph::new(), |p, r| p.add_run(r))
}

pub fn cell_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => clean(s),
        other => other.to_string(),
    }
}

fn styles(doc: Docx) -> Docx {
    let heading = |id: &str, name: &str, size: usize, level: usize| {
        Style::new(id, StyleType::Paragraph)
            .name(name)
            .based_on("Normal")
            .next("Normal")
            .bold()
            .size(size)
            .outline_lvl(level)
    };
    doc.add_style(
        Style::new("Title", StyleType::Paragraph)
            .name("Title")
            .bold()
            .size(36),
    )
    .add_style(heading("Heading1", "Heading 1", 32, 0))
    .add_style(heading("Heading2", "Heading 2", 28, 1))
    .add_style(heading("Heading3", "Heading 3", 24, 2))
}

pub fn build_docx(spec: &DocxSpec) -> Result<Vec<u8>, String> {
    validate_docx(spec)?;
    let font = RunFonts::new()
        .ascii("Calibri")
        .hi_ansi("Calibri")
        .cs("Calibri")
        .east_asia("Calibri");
    let mut doc = styles(Docx::new().default_fonts(font).default_size(22));
    if let Some(title) = spec.title.as_deref().filter(|t| !t.trim().is_empty()) {
        doc = doc.add_paragraph(
            Paragraph::new()
                .style("Title")
                .add_run(Run::new().add_text(clean(title))),
        );
    }
    for block in &spec.blocks {
        doc = match block {
            Block::Heading { level, text } => doc.add_paragraph(
                Paragraph::new()
                    .style(&format!("Heading{level}"))
                    .add_run(Run::new().add_text(clean(text))),
            ),
            Block::Paragraph { text } => clean(text)
                .split('\n')
                .fold(doc, |doc, line| doc.add_paragraph(paragraph_with(line))),
            Block::Bullets { items } => items
                .iter()
                .fold(doc, |doc, item| doc.add_paragraph(list_item("•", item))),
            Block::Numbered { items } => items.iter().enumerate().fold(doc, |doc, (n, item)| {
                doc.add_paragraph(list_item(&format!("{}.", n + 1), item))
            }),
            Block::Table { columns, rows } => doc.add_table(table(columns, rows)),
            Block::PageBreak => {
                doc.add_paragraph(Paragraph::new().add_run(Run::new().add_break(BreakType::Page)))
            }
        };
    }
    let mut buffer = std::io::Cursor::new(Vec::new());
    doc.build()
        .pack(&mut buffer)
        .map_err(|e| format!("Gagal menyimpan Word: {e}"))?;
    Ok(buffer.into_inner())
}

fn list_item(marker: &str, text: &str) -> Paragraph {
    let text = clean(text).replace('\n', " ");
    runs(&text).into_iter().fold(
        Paragraph::new()
            .indent(Some(720), Some(SpecialIndentType::Hanging(360)), None, None)
            .add_run(Run::new().add_text(format!("{marker}\t"))),
        |p, r| p.add_run(r),
    )
}

fn table(columns: &[String], rows: &[Vec<Value>]) -> Table {
    let header = TableRow::new(
        columns
            .iter()
            .map(|c| {
                TableCell::new()
                    .shading(Shading::new().fill("F2F2F2"))
                    .add_paragraph(Paragraph::new().add_run(Run::new().add_text(clean(c)).bold()))
            })
            .collect(),
    );
    let body = rows.iter().map(|row| {
        TableRow::new(
            (0..columns.len())
                .map(|c| {
                    let text = row.get(c).map(cell_text).unwrap_or_default();
                    TableCell::new()
                        .add_paragraph(Paragraph::new().add_run(Run::new().add_text(text)))
                })
                .collect(),
        )
    });
    Table::new(std::iter::once(header).chain(body).collect())
}

/// Potongan teks maksimal `max` byte, tidak memotong di tengah karakter.
fn truncate_bytes(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// Isi dokumen untuk model dalam batas `budget` byte JSON, mulai bagian `from_block`.
/// Halaman selalu maju: bagian yang terlalu besar dikirim terpotong, dan bagian terakhir
/// dipindah ke halaman berikutnya sampai JSON akhirnya muat.
pub fn read_page(spec: &DocxSpec, from_block: usize, budget: usize) -> Value {
    let base = json!({
        "filename": truncate_bytes(&spec.filename, 300),
        "title": spec.title.as_deref().map(|t| truncate_bytes(t, 1_000)),
        "total_blocks": spec.blocks.len(),
    });
    let block_json = |i: usize| json!({ "index": i, "block": spec.blocks[i] });
    let mut used = base.to_string().len() + 200;
    let mut blocks = Vec::new();
    let mut next = None;
    for i in from_block..spec.blocks.len() {
        let item = block_json(i);
        let size = item.to_string().len();
        if used + size > budget {
            if blocks.is_empty() {
                // Teks berkutip bisa membesar saat di-escape, jadi sisakan separuh anggaran.
                let cut = truncate_bytes(&item.to_string(), budget.saturating_sub(used) / 2);
                blocks.push(json!({ "index": i, "dipotong": cut }));
                next = (i + 1 < spec.blocks.len()).then_some(i + 1);
            } else {
                next = Some(i);
            }
            break;
        }
        used += size;
        blocks.push(item);
    }
    let mut cap = budget / 2;
    loop {
        let mut page = base.clone();
        page["blocks"] = json!(blocks);
        if let Some(next) = next {
            page["next_from_block"] = json!(next);
        }
        if page.to_string().len() <= budget || blocks.is_empty() {
            return page;
        }
        if blocks.len() > 1 {
            let last = blocks.pop().unwrap_or_default();
            next = last["index"].as_u64().map(|i| i as usize);
            continue;
        }
        // Tinggal satu bagian yang tetap tidak muat: potong lebih pendek, atau lewati.
        let index = blocks[0]["index"].as_u64().unwrap_or(0) as usize;
        next = next.or((index + 1 < spec.blocks.len()).then_some(index + 1));
        cap /= 2;
        if cap < 200 {
            blocks.clear();
            continue;
        }
        let cut = truncate_bytes(&block_json(index).to_string(), cap);
        blocks[0] = json!({ "index": index, "dipotong": cut });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(blocks: Value) -> DocxSpec {
        serde_json::from_value(
            json!({ "filename": "Laporan", "title": "Laporan Mingguan", "blocks": blocks }),
        )
        .unwrap()
    }

    #[test]
    fn builds_a_document_with_every_block_type() {
        let bytes = build_docx(&spec(json!([
            { "type": "heading", "level": 1, "text": "Ringkasan" },
            { "type": "paragraph", "text": "Operasional **naik 5%**.\nBaris kedua." },
            { "type": "bullets", "items": ["Line 1 normal", "Line 2 **perbaikan**"] },
            { "type": "numbered", "items": ["Cek mesin", "Isi laporan"] },
            { "type": "table", "columns": ["Line", "Output"], "rows": [["1", 120], ["2", null]] },
            { "type": "page_break" },
            { "type": "paragraph", "text": "Teks \u{0007} dengan karakter kontrol" }
        ])))
        .unwrap();
        assert!(bytes.starts_with(b"PK"), "docx adalah file zip");
        // Dibaca balik dengan pembaca docx-rs.
        let parsed = docx_rs::read_docx(&bytes).unwrap().json();
        assert!(
            parsed.contains("naik 5%") && parsed.contains("Heading1") && parsed.contains("Output")
        );
        assert!(!parsed.contains("\\u0007"));
    }

    #[test]
    fn mistakes_are_explained_to_the_model() {
        let err = |v: Value| validate_docx(&spec(v)).unwrap_err();
        assert!(err(json!([])).contains("minimal"));
        assert!(
            err(json!([{ "type": "heading", "level": 5, "text": "x" }])).contains("Bagian ke-1")
        );
        assert!(err(json!([{ "type": "bullets", "items": [] }])).contains("daftar"));
        assert!(
            err(json!([{ "type": "table", "columns": ["A"], "rows": [[1, 2]] }]))
                .contains("lebih banyak")
        );
    }

    #[test]
    fn unbalanced_bold_markers_stay_as_text() {
        assert_eq!(runs("a **b** c").len(), 3);
        assert_eq!(runs("5 ** 3").len(), 1);
    }

    #[test]
    fn invalid_xml_characters_and_object_cells_are_handled() {
        let bytes = build_docx(&spec(json!([
            { "type": "paragraph", "text": "a\u{FFFE}b\u{FFFF}c" }
        ])))
        .unwrap();
        let parsed = docx_rs::read_docx(&bytes).unwrap().json();
        assert!(parsed.contains("abc"));
        let err = validate_docx(&spec(json!([
            { "type": "table", "columns": ["A"], "rows": [[{ "toString": "x" }]] }
        ])))
        .unwrap_err();
        assert!(err.contains("isi sel"));
    }

    #[test]
    fn emoji_heavy_documents_fit_the_budget() {
        let emoji = "😀".repeat(20_000);
        let doc: DocxSpec = serde_json::from_value(json!({ "filename": "x", "title": emoji,
            "blocks": [{ "type": "paragraph", "text": emoji }, { "type": "paragraph", "text": emoji }] }))
        .unwrap();
        let mut from = 0;
        let mut seen = 0;
        for _ in 0..5 {
            let page = read_page(&doc, from, 40_000);
            assert!(page.to_string().len() <= 40_000);
            seen += page["blocks"].as_array().unwrap().len();
            match page["next_from_block"].as_u64() {
                Some(next) => {
                    assert!(next as usize > from);
                    from = next as usize;
                }
                None => break,
            }
        }
        assert_eq!(seen, 2, "kedua bagian terbaca, masing-masing terpotong");
    }

    #[test]
    fn documents_are_read_in_pages() {
        let blocks: Vec<Value> = (0..300)
            .map(|i| json!({ "type": "paragraph", "text": format!("Paragraf {i} {}", "x".repeat(200)) }))
            .collect();
        let doc = spec(json!(blocks));
        let page = read_page(&doc, 0, 10_000);
        let next = page["next_from_block"].as_u64().unwrap() as usize;
        assert!(next > 0 && next < 300);
        assert!(page.to_string().len() <= 10_500);
        assert_eq!(read_page(&doc, next, 10_000)["blocks"][0]["index"], next);
    }
}
