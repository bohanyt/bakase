//! Presentasi PowerPoint dari isi terstruktur yang juga dipakai untuk preview.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod package;
#[cfg(test)]
mod tests;

pub use package::build;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PptxSpec {
    pub filename: String,
    pub slides: Vec<Slide>,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Slide {
    Title {
        title: String,
        subtitle: Option<String>,
    },
    Bullets {
        title: String,
        items: Vec<String>,
    },
    Table {
        title: String,
        columns: Vec<String>,
        rows: Vec<Vec<Cell>>,
    },
    Chart {
        title: String,
        chart_type: ChartKind,
        unit: String,
        categories: Vec<String>,
        series: Vec<Series>,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
pub enum Cell {
    Text(String),
    Number(serde_json::Number),
    Bool(bool),
    Blank,
}

impl std::fmt::Display for Cell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(text) => f.write_str(text),
            Self::Number(number) => number.fmt(f),
            Self::Bool(value) => f.write_str(if *value { "TRUE" } else { "FALSE" }),
            Self::Blank => Ok(()),
        }
    }
}

pub fn format_preview_table_numbers(spec: &mut Value) {
    let Some(slides) = spec.get_mut("slides").and_then(Value::as_array_mut) else {
        return;
    };
    for slide in slides.iter_mut().filter(|slide| slide["type"] == "table") {
        let Some(rows) = slide.get_mut("rows").and_then(Value::as_array_mut) else {
            continue;
        };
        for cell in rows.iter_mut().filter_map(Value::as_array_mut).flatten() {
            if let Value::Number(number) = cell {
                // Sel tabel di PPTX ditulis sebagai teks yang sama.
                *cell = Value::String(number.to_string());
            }
        }
    }
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    Bar,
    Line,
    Pie,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Series {
    name: String,
    values: Vec<f64>,
}

const MAX_SLIDES: usize = 25;
const MAX_BULLETS: usize = 6;
const MAX_COLUMNS: usize = 6;
const MAX_ROWS: usize = 8;
const MAX_CATEGORIES: usize = 12;
const MAX_SERIES: usize = 6;
const EXCEL_MAX_NUMBER: f64 = 9.999_999_999_999_99e307;

pub fn tool() -> Value {
    let title = json!({ "type": "string", "description": "Judul slide, maksimal 80 karakter." });
    let variant = |kind: &str, fields: Value| {
        let mut properties = fields.as_object().unwrap().clone();
        properties.insert("type".into(), json!({ "type": "string", "enum": [kind] }));
        properties.insert("title".into(), title.clone());
        let required: Vec<_> = properties.keys().cloned().collect();
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": required,
            "properties": properties
        })
    };
    json!({
        "type": "function",
        "name": "create_pptx",
        "strict": true,
        "description": format!("Buat presentasi .pptx yang bisa diunduh. Pakai hanya jika user meminta presentasi atau slide. Grafik dan tabel dapat diedit di PowerPoint. Gunakan 1–{MAX_SLIDES} slide."),
        "parameters": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "filename": { "type": "string", "description": "Nama file tanpa ekstensi, maksimal 100 karakter." },
                "slides": {
                    "type": "array",
                    "description": "Slide berurutan dengan satu jenis isi utama per slide.",
                    "items": { "anyOf": [
                        variant("title", json!({
                            "subtitle": { "type": ["string", "null"], "description": "Subjudul, maksimal 120 karakter. Isi null jika tidak dipakai." }
                        })),
                        variant("bullets", json!({
                            "items": { "type": "array", "items": { "type": "string" },
                                "description": format!("1–{MAX_BULLETS} poin pendek, masing-masing maksimal 120 karakter.") }
                        })),
                        variant("table", json!({
                            "columns": { "type": "array", "items": { "type": "string" }, "description": format!("1–{MAX_COLUMNS} judul kolom, masing-masing maksimal 24 karakter.") },
                            "rows": { "type": "array", "description": format!("Maksimal {MAX_ROWS} baris. Jumlah sel tiap baris sama dengan jumlah kolom, teks maksimal 32 karakter per sel. Bagi tabel panjang atau lebar ke beberapa slide."),
                                "items": { "type": "array", "items": { "type": ["string", "number", "boolean", "null"] } } }
                        })),
                        variant("chart", json!({
                            "chart_type": { "type": "string", "enum": ["bar", "line", "pie"] },
                            "unit": { "type": "string", "description": "Satuan yang tampil di slide, misalnya ton, %, atau pemeriksaan. Maksimal 24 karakter. Untuk %, kirim 97 untuk 97%." },
                            "categories": { "type": "array", "items": { "type": "string" }, "description": format!("1–{MAX_CATEGORIES} label kategori, masing-masing maksimal 24 karakter.") },
                            "series": { "type": "array", "description": format!("1–{MAX_SERIES} seri untuk batang atau garis. Pai memakai satu seri, semua angka nonnegatif dan minimal satu positif."),
                                "items": {
                                    "type": "object",
                                    "additionalProperties": false,
                                    "properties": {
                                        "name": { "type": "string", "description": "Nama seri, maksimal 24 karakter." },
                                        "values": { "type": "array", "items": { "type": "number" }, "description": "Satu angka per kategori, sesuai urutannya. Ambil dari sumber atau perhitungan yang jelas." }
                                    },
                                    "required": ["name", "values"]
                                }
                            }
                        }))
                    ] }
                }
            },
            "required": ["filename", "slides"]
        }
    })
}

pub fn read_tool() -> Value {
    json!({
        "type": "function",
        "name": "read_pptx",
        "strict": true,
        "description": "Baca judul, poin, tabel, dan data grafik dari PowerPoint buatan Bakase di percakapan ini. Mulai dari slide 1 atau nomor slide yang ditanya. Kalau next_from_slide berisi angka, pakai angka itu untuk halaman berikutnya. Untuk presentasi yang dilampirkan user, pakai read_attachment.",
        "parameters": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "file_id": { "type": "string", "description": "Id file PowerPoint dari riwayat atau ringkasan percakapan." },
                "from_slide": { "type": "integer", "description": "Nomor slide pertama yang dibaca, mulai dari 1." }
            },
            "required": ["file_id", "from_slide"]
        }
    })
}

pub fn read_page(spec: &PptxSpec, from_slide: usize, budget: usize) -> Result<String, String> {
    let total = spec.slides.len();
    if !(1..=total).contains(&from_slide) {
        return Err(format!("Nomor slide harus antara 1 dan {total}."));
    }
    let mut page = json!({
        "filename": spec.filename,
        "total_slides": total,
        "from_slide": from_slide,
        "slides": [],
        "next_from_slide": null,
    });
    let mut used = page.to_string().len();
    let mut slides = Vec::new();
    for slide in &spec.slides[from_slide - 1..] {
        let item = json!(slide);
        // Satu byte untuk koma. 'null' yang dicadangkan cukup untuk nomor slide berikutnya.
        let size = item.to_string().len() + 1;
        if used + size > budget {
            break;
        }
        used += size;
        slides.push(item);
    }
    if slides.is_empty() {
        return Err("Slide ini tidak muat dalam sisa batas baca giliran ini. Lanjutkan pada giliran berikutnya.".into());
    }
    let next = from_slide + slides.len();
    page["slides"] = Value::Array(slides);
    if next <= total {
        page["next_from_slide"] = json!(next);
    }
    Ok(page.to_string())
}

pub fn validate(spec: &PptxSpec) -> Result<(), String> {
    check_text(&spec.filename, "Nama file", 100)?;
    if !(1..=MAX_SLIDES).contains(&spec.slides.len()) {
        return Err(format!("Presentasi harus punya 1–{MAX_SLIDES} slide."));
    }
    for (index, slide) in spec.slides.iter().enumerate() {
        slide
            .validate()
            .map_err(|error| format!("Slide {}: {error}", index + 1))?;
    }
    Ok(())
}

fn check_text(text: &str, field: &str, max: usize) -> Result<(), String> {
    if text.trim().is_empty() || text.chars().count() > max {
        return Err(format!("{field} harus berisi 1–{max} karakter."));
    }
    if text
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, '\u{fffe}' | '\u{ffff}'))
    {
        return Err(format!("{field} memuat karakter kontrol atau baris baru."));
    }
    Ok(())
}

impl Slide {
    fn title(&self) -> &str {
        match self {
            Self::Title { title, .. }
            | Self::Bullets { title, .. }
            | Self::Table { title, .. }
            | Self::Chart { title, .. } => title,
        }
    }

    fn validate(&self) -> Result<(), String> {
        check_text(self.title(), "Judul slide", 80)?;
        match self {
            Self::Title { subtitle, .. } => {
                if let Some(text) = subtitle {
                    check_text(text, "Subjudul", 120)?;
                }
            }
            Self::Bullets { items, .. } => {
                if !(1..=MAX_BULLETS).contains(&items.len()) {
                    return Err(format!("Daftar harus berisi 1–{MAX_BULLETS} poin."));
                }
                for item in items {
                    check_text(item, "Poin", 120)?;
                }
            }
            Self::Table { columns, rows, .. } => {
                if !(1..=MAX_COLUMNS).contains(&columns.len()) || rows.len() > MAX_ROWS {
                    return Err(format!(
                        "Tabel harus punya 1–{MAX_COLUMNS} kolom dan maksimal {MAX_ROWS} baris. Bagi tabel panjang ke beberapa slide."
                    ));
                }
                for title in columns {
                    check_text(title, "Judul kolom", 24)?;
                }
                for row in rows {
                    if row.len() != columns.len() {
                        return Err(
                            "Jumlah sel di setiap baris harus sama dengan jumlah kolom.".into()
                        );
                    }
                    for cell in row {
                        if let Cell::Text(text) = cell
                            && !text.is_empty()
                        {
                            check_text(text, "Teks sel", 32)?;
                        }
                    }
                }
                if !package::table_fits(columns, rows) {
                    return Err("Teks tabel tidak muat di slide. Kurangi kolom atau baris per slide, atau pendekkan isi sel yang panjang.".into());
                }
            }
            Self::Chart {
                unit,
                chart_type,
                categories,
                series,
                ..
            } => {
                check_text(unit, "Satuan grafik", 24)?;
                let max_series = if *chart_type == ChartKind::Pie {
                    1
                } else {
                    MAX_SERIES
                };
                if !(1..=MAX_CATEGORIES).contains(&categories.len())
                    || !(1..=max_series).contains(&series.len())
                {
                    return Err(format!(
                        "Grafik harus punya 1–{MAX_CATEGORIES} kategori dan 1–{max_series} seri."
                    ));
                }
                for category in categories {
                    check_text(category, "Kategori grafik", 24)?;
                }
                for series in series {
                    check_text(&series.name, "Nama seri", 24)?;
                    if series.values.len() != categories.len() {
                        return Err(
                            "Setiap seri harus memuat satu angka untuk setiap kategori.".into()
                        );
                    }
                    if series.values.iter().any(|value| {
                        *value != 0.0
                            && !(f64::MIN_POSITIVE..=EXCEL_MAX_NUMBER).contains(&value.abs())
                    }) {
                        return Err("Angka grafik di luar rentang Excel. Untuk nilai selain nol, besarnya harus antara 2.2250738585072014e-308 dan 9.99999999999999e307. Pakai satuan lain agar nilainya masuk rentang.".into());
                    }
                    if *chart_type == ChartKind::Pie
                        && (series.values.iter().any(|value| *value < 0.0)
                            || !series.values.iter().any(|value| *value > 0.0))
                    {
                        return Err("Grafik pai memerlukan angka nonnegatif dan minimal satu angka positif.".into());
                    }
                }
            }
        }
        Ok(())
    }
}
