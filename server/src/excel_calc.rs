use super::{
    MAX_SHEET_BOX, MAX_WORKBOOK_SHEETS, XlsxPackage, cell_value, check_sheets, check_zip, data_ref,
    unreadable, worksheet_names, xls, xlsx_parts, xlsx_worksheet,
};
use crate::files::column_letter;
use calamine::{Cell, CellType, DataRef, Range, Reader, Xls, Xlsx};
use rusqlite::{
    Connection, Error, ErrorCode, PrepFlags, config::DbConfig, hooks::AuthAction,
    hooks::AuthContext, hooks::Authorization, limits::Limit, params_from_iter,
    types::Value as SqlValue, types::ValueRef,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io::Cursor,
    ops::Range as RowRange,
    sync::{Arc, Mutex},
};

#[path = "excel_calc/colors.rs"]
mod colors;
#[path = "excel_calc/numeric.rs"]
mod numeric;

const MAX_ROWS: usize = 100_000;
const MAX_COLUMNS: usize = 1_000;
const MAX_RESULT_ROWS: usize = 200;
const MAX_OUTPUT: usize = 40_000;
const MAX_VALUE_BYTES: i32 = 1024 * 1024;
const SQLITE_HEAP_BYTES: i64 = 192 * 1024 * 1024;
const PROGRESS_INTERVAL: i32 = 1_000;
const MAX_PROGRESS_CALLS: usize = 20_000;
pub(super) const MAX_SQL_BYTES: usize = 8_192;
pub(crate) const MIN_BUDGET: usize = 256;
pub(crate) const WORKER_ERROR: &str = "Excel tidak bisa dihitung dalam batas pembaca. Pilih sheet yang lebih kecil atau pecah file, lalu coba lagi.";
pub(crate) const BUDGET_ERROR: &str =
    "Anggaran hasil Excel giliran ini sudah habis. Minta perhitungan dalam giliran baru.";
const SMALLER_RESULT: &str =
    "Hasil Excel tidak muat. Pilih lebih sedikit kolom atau ringkas hasil dengan agregasi.";
const SQL_ERROR: &str =
    "SQL tidak valid. Gunakan satu SELECT, boleh diawali WITH, dan periksa nama kolom Excel.";
const SQL_DENIED: &str =
    "SQL hanya boleh membaca tabel data dan warna. Hapus perintah atau fungsi yang tidak didukung.";
const LOGIC_NOTE: &str = "Sel logika adalah teks 'TRUE' atau 'FALSE'. Contoh: A = 'TRUE'.";
const CELL_TYPES: [&str; 8] = [
    "angka",
    "teks",
    "tanggal",
    "logika",
    "galat",
    "kosong",
    "rumus_tanpa_nilai",
    "rumus_cache_kosong",
];

/// Tidak strict: `file_id` dan `sitasi` saling menggantikan. Dalam mode strict semua parameter
/// wajib diisi, sehingga model mengirim `sitasi: 0` bersama `file_id`.
pub(crate) fn tool() -> Value {
    json!({
        "type": "function",
        "strict": false,
        "name": "hitung_xlsx",
        "description": format!("Hitung dari Excel di percakapan atau Sumber Data (.xlsx, .xlsm, .xls). Isi salah satu dari file_id atau sitasi. Untuk Sumber Data, kutip nomor pada hasil hitung sesuai urutan sheet. Jika user tidak menyebut sheet, lihat ikhtisar dengan mengosongkan sheet, sheets, dan sql. Jika data tersebar, pakai sheet '*' atau pilih beberapa nama lewat sheets. Sebutkan jika ada sheet relevan yang belum dihitung. Sel kosong dan teks yang hanya spasi adalah NULL. {LOGIC_NOTE} Sampaikan catatan profil_kolom, catatan_tata_letak, catatan_warna, dan catatan_batas_warna dari hasil SQL kepada user."),
        "parameters": {
            "type": "object",
            "properties": {
                "file_id": { "type": "string", "description": "Id file dari riwayat percakapan" },
                "sitasi": { "type": "integer", "description": "Nomor [n] dokumen Excel dari search_documents di giliran ini. Pakai angkanya saja." },
                "sheet": { "type": "string", "description": "Nama sheet, atau '*' untuk semua. Kosong memilih sheet pertama jika ada SQL, atau ikhtisar workbook jika SQL juga kosong. Gunakan sheet atau sheets." },
                "sheets": { "type": "array", "items": { "type": "string" }, "description": "Daftar nama sheet yang dihitung bersama. Dengan SQL, gunakan kolom sheet untuk nama asal baris." },
                "header_row": { "type": "integer", "description": "Baris judul Excel untuk semua sheet terpilih. Bawaan 1, 0 jika tidak ada judul. Data dimulai setelah baris judul. Hitung terpisah jika tata letak antar sheet berbeda." },
                "simpan_ke_excel": { "type": "string", "description": "Opsional, bersama sql. Nama file Excel baru. Server menyalin semua baris hasil SQL (maks 10.000 baris, 60 kolom) ke file itu, dengan judul kolom asli untuk kolom huruf, lalu file tampil di chat. Pakai ini kalau user minta baris, hasil saringan, atau sebagian besar tabel jadi file Excel; jangan mengetik ulang barisnya ke create_xlsx." },
                "sql": { "type": "string", "description": "Opsional. Kosong dengan satu nama sheet menampilkan contoh baris dan isi tiap kolom: nilai unik beserta jumlah barisnya, atau rentangnya. Pakai itu untuk memilih saringan tanpa membaca semua baris. Untuk menghitung atau menyaring, tulis satu SELECT SQLite (boleh WITH) dari data. Gunakan huruf kolom, baris (nomor baris Excel), dan sheet untuk pilihan beberapa sheet. Untuk warna isi sel .xlsx/.xlsm pakai tabel warna(baris, kolom, warna, nama), dengan warna berbentuk #RRGGBB dan nama seperti abu-abu atau kuning. Gabungkan dengan data lewat baris dan juga sheet jika memilih beberapa sheet. Warna pola memakai warna depan, gradien memakai warna awal, dan pemformatan bersyarat diabaikan. SUM, AVG, dan TOTAL perlu angka. Lindungi ekspresi aritmetika dengan CASE WHEN typeof(A) IN ('integer','real') THEN A ELSE NULL END." }
            }
        }
    })
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Input {
    File(String),
    Citation(u32),
}

#[derive(Deserialize, Serialize)]
pub(crate) struct Options {
    pub file: String,
    pub sheet: Option<String>,
    pub sheets: Option<Vec<String>>,
    pub sql: Option<String>,
    pub budget: usize,
    pub pending: bool,
    #[serde(default = "default_header")]
    pub header_row: usize,
    /// Nama file Excel baru untuk semua baris hasil SQL (`simpan_ke_excel`).
    #[serde(default)]
    pub simpan: Option<String>,
}

fn default_header() -> usize {
    1
}

fn blank_arg(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(items) => items.iter().all(blank_arg),
        _ => false,
    }
}

impl Options {
    pub(crate) fn from_tool(arguments: &str, budget: usize) -> Result<(Input, Self), String> {
        let args: Value = serde_json::from_str(arguments).map_err(
            |_| "Argumen hitung_xlsx harus berupa objek JSON dengan file_id atau sitasi.",
        )?;
        let optional = |key: &str| args.get(key).filter(|arg| !blank_arg(arg));
        let string_arg = |key: &str| {
            optional(key)
                .map(|arg| {
                    arg.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| format!("Argumen {key} harus berupa teks."))
                })
                .transpose()
        };
        let file_id = string_arg("file_id")?;
        // Nomor sitasi mulai dari 1, jadi 0 sama dengan tidak diisi.
        let citation = optional("sitasi").filter(|arg| arg.as_f64() != Some(0.0));
        let input = match (file_id, citation) {
            (Some(id), None) => Input::File(id),
            (None, Some(number)) => Input::Citation(
                number
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|n| *n > 0)
                    .ok_or("Isi sitasi dengan angka dari hasil search_documents.")?,
            ),
            (Some(_), Some(_)) => {
                return Err("Pakai file_id saja atau sitasi saja, jangan keduanya.".into());
            }
            (None, None) => {
                return Err("Isi salah satu dari file_id atau sitasi dokumen Excel.".into());
            }
        };
        let sheet = string_arg("sheet")?;
        let sql = string_arg("sql")?;
        let simpan = string_arg("simpan_ke_excel")?
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty());
        let invalid_sheets = "Argumen sheets harus berupa daftar nama sheet.";
        let sheets = optional("sheets")
            .map(|arg| {
                arg.as_array()
                    .ok_or(invalid_sheets)?
                    .iter()
                    .filter(|name| !blank_arg(name))
                    .map(|name| name.as_str().map(str::to_owned).ok_or(invalid_sheets))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;
        let header_row = match optional("header_row") {
            None => default_header(),
            Some(arg) => arg.as_u64().and_then(|n| usize::try_from(n).ok())
                .or_else(|| arg.as_str()?.trim().parse().ok())
                .ok_or("Argumen header_row harus berupa nomor baris Excel atau 0 jika tidak ada judul.")?,
        };
        Ok((
            input,
            Self {
                file: String::new(),
                sheet,
                sheets,
                sql,
                budget,
                pending: false,
                header_row,
                simpan,
            },
        ))
    }

    fn sql(&self) -> Option<&str> {
        self.sql.as_deref().filter(|sql| !sql.trim().is_empty())
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.budget < MIN_BUDGET {
            return Err(BUDGET_ERROR.into());
        }
        if self.simpan.is_some() && self.sql().is_none() {
            return Err("simpan_ke_excel perlu sql yang memilih baris untuk disimpan.".into());
        }
        if self
            .simpan
            .as_ref()
            .is_some_and(|name| name.chars().count() > 120)
        {
            return Err("Nama file simpan_ke_excel maksimal 120 karakter.".into());
        }
        if let Some(sql) = self.sql()
            && (sql.len() > MAX_SQL_BYTES || sql.contains('\0'))
        {
            return Err(format!(
                "Argumen sql terlalu panjang atau mengandung karakter NUL. Tulis satu SELECT dengan maksimal {MAX_SQL_BYTES} byte."
            ));
        }
        if self.sheet.as_ref().is_some_and(|sheet| sheet.len() > 256) {
            return Err("Argumen sheet terlalu panjang. Pakai nama dari daftar sheet.".into());
        }
        if self.sheets.as_ref().is_some_and(|sheets| {
            sheets.len() > MAX_WORKBOOK_SHEETS || sheets.iter().any(|name| name.len() > 256)
        }) {
            return Err(format!(
                "Daftar sheets maksimal {MAX_WORKBOOK_SHEETS} nama. Pakai nama dari ikhtisar workbook."
            ));
        }
        if self.header_row > MAX_ROWS {
            return Err("Argumen header_row terlalu jauh. Pakai nomor baris Excel sampai 100.000 atau 0 jika tidak ada judul.".into());
        }
        Ok(())
    }
}

pub(super) fn calculate(
    kind: &str,
    bytes: &[u8],
    options: Options,
    limited_process: bool,
) -> Result<String, String> {
    options.validate()?;
    let mut book = Workbook::open(kind, bytes)?;
    let names = worksheet_names(book.sheets());
    let selection = Selection::new(&names, &options)?;
    let budget = options.budget.min(MAX_OUTPUT);
    let Some(sql) = options.sql() else {
        if let Some(name) = selection.detail {
            return book.read_sheet(name, options.pending, |bounds, notes, cell| {
                describe_sheet(&options, name, bounds, notes, cell, budget)
            });
        }
        return workbook_overview(&mut book, &options.file, &selection.names, budget);
    };

    let mut sheets = Vec::with_capacity(selection.names.len());
    let compare_layout = selection.names.len() > 1 && options.header_row > 0;
    let save = options.simpan.is_some();
    let read_titles = options.header_row > 0 && (compare_layout || save);
    let (mut total_cells, mut rows, mut columns) = (0_usize, 0_usize, 0_usize);
    let mut notes = SheetNotes::default();
    for name in selection.names {
        book.read_sheet(name, options.pending, |bounds, sheet_notes, cell| {
            let data = bounds.data_rows(options.header_row);
            bounds.validate(data.len()).map_err(|message| {
                if selection.multi {
                    format!("Sheet {name:?}. {message} Gunakan sheets untuk memilih sheet lain.")
                } else {
                    message
                }
            })?;
            total_cells += bounds.area();
            rows += data.len();
            columns = columns.max(bounds.columns);
            if total_cells > MAX_SHEET_BOX as usize || rows.saturating_mul(columns.max(1)) > MAX_SHEET_BOX as usize {
                return Err(format!(
                    "Sheet {name:?} membuat pilihan melewati batas total 2 juta sel, termasuk kolom yang diselaraskan. Gunakan sheets untuk memilih lebih sedikit sheet."
                ));
            }
            notes.recalculation |= sheet_notes.recalculation;
            notes.merged += sheet_notes.merged;
            let headings = if read_titles {
                read_headings(&bounds, options.header_row, cell)
            } else {
                Vec::new()
            };
            let cells = data.clone()
                .flat_map(|row| (0..bounds.columns).map(move |col| cell(row, col)))
                .collect();
            sheets.push(TableSheet { name, rows: data, columns: bounds.columns, headings, cells });
            Ok(())
        })?;
    }
    let mut output = json!({
        "file": options.file,
        "sheets": sheets.iter().map(|s| s.name).collect::<Vec<_>>(),
        "baris_data": rows,
        "baris_judul": options.header_row,
    });
    if !selection.multi {
        let sheet = &sheets[0];
        output["sheet"] = json!(sheet.name);
        output["rentang_baris"] = json!(excel_rows(&sheet.rows));
    }
    notes.add_to(&mut output);
    query(
        output,
        sql,
        &sheets,
        selection.multi,
        budget,
        limited_process,
        Some(&mut book),
        save,
    )
}

enum Workbook<'a> {
    Xlsx {
        book: Xlsx<Cursor<&'a [u8]>>,
        package: XlsxPackage<'a>,
        parts: super::XlsxParts,
    },
    Xls {
        book: Xls<Cursor<std::borrow::Cow<'a, [u8]>>>,
        info: xls::WorkbookInfo,
    },
}

impl<'a> Workbook<'a> {
    fn open(kind: &str, bytes: &'a [u8]) -> Result<Self, String> {
        match kind {
            "xlsx" | "xlsm" => {
                if bytes.starts_with(super::OLE_MAGIC) {
                    return Err(if xls::encrypted_package(bytes) {
                        super::ENCRYPTED_OFFICE.into()
                    } else {
                        super::unsupported_excel()
                    });
                }
                check_zip(bytes, kind)?;
                check_sheets(bytes)?;
                let book = Xlsx::new(Cursor::new(bytes)).map_err(|_| unreadable(kind))?;
                let mut package = XlsxPackage::open(bytes)?;
                let parts = xlsx_parts(&mut package)?;
                Ok(Self::Xlsx {
                    book,
                    package,
                    parts,
                })
            }
            "xls" => {
                let (prepared, info) =
                    xls::prepare(std::borrow::Cow::Borrowed(bytes), xls::Scope::Attachment)?;
                let book = Xls::new(Cursor::new(prepared)).map_err(|_| unreadable(kind))?;
                Ok(Self::Xls { book, info })
            }
            _ => Err("Pakai id lampiran atau file Excel dari percakapan ini.".into()),
        }
    }

    fn sheets(&self) -> &[calamine::Sheet] {
        match self {
            Self::Xlsx { book, .. } => book.sheets_metadata(),
            Self::Xls { book, .. } => book.sheets_metadata(),
        }
    }

    fn dimensions(&mut self, name: &str) -> Result<Bounds, String> {
        self.read_sheet(name, false, |bounds, _, _| Ok(bounds))
    }

    fn read_sheet<R>(
        &mut self,
        name: &str,
        pending: bool,
        use_sheet: impl FnOnce(
            Bounds,
            SheetNotes,
            &dyn Fn(usize, usize) -> TypedCell,
        ) -> Result<R, String>,
    ) -> Result<R, String> {
        match self {
            Self::Xlsx {
                book,
                package,
                parts,
            } => {
                let path = parts.sheets.get(name).ok_or_else(|| unreadable("xlsx"))?;
                let details =
                    xlsx_worksheet::inspect(package.member(path)?, xlsx_worksheet::Scan::Values)?;
                let mut reader = book
                    .worksheet_cells_reader(name)
                    .map_err(|_| unreadable("xlsx"))?;
                let recalculation = parts.pending || pending;
                let mut cells = Vec::new();
                let mut has_formulas = false;
                while let Some(cell) = reader
                    .next_cell_with_formula_metadata()
                    .map_err(|_| unreadable("xlsx"))?
                {
                    let formula = cell.formula.is_some();
                    has_formulas |= formula;
                    let empty_cache = recalculation && formula && empty_string(&cell.value);
                    let value = if formula && (pending || matches!(cell.value, DataRef::Empty)) {
                        InputCell::Uncached(empty_cache)
                    } else if empty_cache {
                        InputCell::EmptyCache
                    } else if matches!(cell.value, DataRef::Int(_) | DataRef::Float(_))
                        && let Some(&integer) = details.large_integers.get(&cell.pos)
                    {
                        InputCell::Exact(integer)
                    } else {
                        InputCell::Cached(cell.value)
                    };
                    if !matches!(value, InputCell::Cached(DataRef::Empty)) {
                        cells.push(Cell::new(cell.pos, value));
                    }
                }
                let range = Range::from_sparse(cells);
                let (rows, columns) = extent(&range);
                use_sheet(
                    Bounds {
                        last_row: if rows == 0 {
                            0
                        } else {
                            rows.max(details.row_count)
                        },
                        columns: if rows == 0 {
                            0
                        } else {
                            columns.max(details.merged_columns)
                        },
                    },
                    SheetNotes {
                        recalculation: has_formulas && recalculation,
                        merged: details.merged,
                    },
                    &|row, col| match range.get_value((row as u32, col as u32)) {
                        Some(InputCell::Exact(n)) => TypedCell::number((*n).into()),
                        Some(InputCell::Cached(value)) => TypedCell::from_data(value),
                        Some(InputCell::Uncached(empty_cache)) => {
                            TypedCell(SqlValue::Null, Kind::Uncached, *empty_cache)
                        }
                        Some(InputCell::EmptyCache) => TypedCell(SqlValue::Null, Kind::Empty, true),
                        None => TypedCell::empty(),
                    },
                )
            }
            Self::Xls { book, info } => {
                let error = |_| unreadable("xls");
                let sheet_info = book
                    .sheets_metadata()
                    .iter()
                    .position(|sheet| sheet.name == name)
                    .and_then(|index| info.sheets.get(index))
                    .copied()
                    .unwrap_or_default();
                let range = book.worksheet_range(name).map_err(error)?;
                let formulas = book.worksheet_formula(name).map_err(error)?;
                let merges = book.merge_cells_by_sheet_name(name).unwrap_or_default();
                let (r1, c1) = extent(&range);
                let (r2, c2) = extent(&formulas);
                let (rows, columns) = if r1.max(r2) == 0 {
                    (0, 0)
                } else {
                    merges.iter().fold(
                        (r1.max(r2).max(sheet_info.last_row), c1.max(c2)),
                        |(rows, columns), merge| {
                            (
                                rows.max(merge.end.0 as usize + 1),
                                columns.max(merge.end.1 as usize + 1),
                            )
                        },
                    )
                };
                use_sheet(
                    Bounds {
                        last_row: rows,
                        columns,
                    },
                    SheetNotes {
                        merged: merges.len(),
                        recalculation: sheet_info.recalculation,
                    },
                    &|row, col| {
                        let pos = (row as u32, col as u32);
                        let value = data_ref(range.get_value(pos));
                        if formulas.get_value(pos).is_some_and(|f| !f.is_empty()) {
                            if matches!(value, DataRef::Empty) {
                                return TypedCell(SqlValue::Null, Kind::Uncached, false);
                            }
                            if sheet_info.recalculation && empty_string(&value) {
                                return TypedCell(SqlValue::Null, Kind::Empty, true);
                            }
                        }
                        TypedCell::from_data(&value)
                    },
                )
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum InputCell<'a> {
    Cached(DataRef<'a>),
    Exact(i64),
    Uncached(bool),
    EmptyCache,
}

impl Default for InputCell<'_> {
    fn default() -> Self {
        Self::Cached(DataRef::Empty)
    }
}

impl CellType for InputCell<'_> {}

fn blank(value: &DataRef<'_>) -> bool {
    match value {
        DataRef::Empty => true,
        DataRef::String(text) => text.trim().is_empty(),
        DataRef::SharedString(text) => text.trim().is_empty(),
        _ => false,
    }
}

fn empty_string(value: &DataRef<'_>) -> bool {
    matches!(value, DataRef::SharedString(""))
        || matches!(value, DataRef::String(s) if s.is_empty())
}

fn extent<T: CellType>(range: &Range<T>) -> (usize, usize) {
    range
        .end()
        .map_or((0, 0), |(row, col)| (row as usize + 1, col as usize + 1))
}

#[derive(Default)]
struct SheetNotes {
    recalculation: bool,
    merged: usize,
}

impl SheetNotes {
    fn add_to(&self, output: &mut Value) {
        if self.recalculation {
            output["catatan_rumus"] = json!(
                "File meminta hitung ulang di Excel. Nilai rumus tersimpan mungkin belum mutakhir."
            );
        }
        if self.merged > 0 {
            output["rentang_sel_gabungan"] = json!(self.merged);
            output["catatan_sel_gabungan"] =
                json!("Sel gabungan dibaca dari sel kiri atas. Sel lain tetap kosong.");
        }
    }
}

struct Bounds {
    last_row: usize,
    columns: usize,
}

impl Bounds {
    fn data_rows(&self, header: usize) -> RowRange<usize> {
        header.min(self.last_row)..self.last_row
    }

    fn area(&self) -> usize {
        self.last_row.saturating_mul(self.columns)
    }

    fn validate(&self, data_rows: usize) -> Result<(), String> {
        let bound = if data_rows > MAX_ROWS {
            "100.000 baris data per sheet"
        } else if self.columns > MAX_COLUMNS {
            "1.000 kolom per sheet"
        } else if self.area() > MAX_SHEET_BOX as usize {
            "2 juta sel dari A1 sampai sel terakhir per sheet"
        } else {
            return Ok(());
        };
        Err(format!(
            "Sheet melewati batas hitung Excel ({bound}). Hapus baris atau kolom di ujung sheet, atau pecah filenya dan lampirkan lagi."
        ))
    }
}

fn excel_rows(rows: &RowRange<usize>) -> Option<[usize; 2]> {
    (!rows.is_empty()).then_some([rows.start + 1, rows.end])
}

struct Selection<'a> {
    names: Vec<&'a str>,
    detail: Option<&'a str>,
    multi: bool,
}

impl<'a> Selection<'a> {
    fn new(names: &'a [String], options: &Options) -> Result<Self, String> {
        if names.len() > MAX_WORKBOOK_SHEETS {
            return Err(format!(
                "Batas hitung Excel {MAX_WORKBOOK_SHEETS} sheet per file. Pecah filenya dan lampirkan lagi."
            ));
        }
        if names.is_empty() {
            return Err("File Excel ini tidak punya sheet.".into());
        }
        let sheet = options.sheet.as_deref().filter(|s| !s.trim().is_empty());
        let (name, all) = match sheet {
            None => (None, false),
            Some(s) if s.trim() == "*" => (None, true),
            Some(s) => {
                let alias = ["all", "semua"]
                    .iter()
                    .any(|alias| s.trim().eq_ignore_ascii_case(alias));
                match find_sheet(s, names)? {
                    Some(name) => (Some(name), false),
                    None if alias => (None, true),
                    None => return Err(missing_sheet(s, names)),
                }
            }
        };
        if let Some(list) = options.sheets.as_ref().filter(|list| !list.is_empty()) {
            let mut chosen = Vec::with_capacity(list.len());
            for requested in list {
                let found =
                    find_sheet(requested, names)?.ok_or_else(|| missing_sheet(requested, names))?;
                if !chosen.contains(&found) {
                    chosen.push(found);
                }
            }
            if name.is_some_and(|name| chosen != [name]) || (all && chosen.len() != names.len()) {
                return Err("Pilihan sheet dan sheets bertentangan. Isi salah satu saja.".into());
            }
            return Ok(Self {
                names: chosen,
                detail: None,
                multi: true,
            });
        }
        if all || (sheet.is_none() && options.sql().is_none()) {
            return Ok(Self {
                names: names.iter().map(String::as_str).collect(),
                detail: None,
                multi: all,
            });
        }
        let name = name.unwrap_or(&names[0]);
        Ok(Self {
            names: vec![name],
            detail: Some(name),
            multi: false,
        })
    }
}

/// Nama persis didahulukan. Excel membolehkan spasi di ujung nama sheet, sedangkan model dan user
/// hampir selalu menulisnya tanpa spasi itu.
fn find_sheet<'a>(requested: &str, names: &'a [String]) -> Result<Option<&'a str>, String> {
    if let Some(name) = names.iter().find(|name| *name == requested) {
        return Ok(Some(name));
    }
    let key = |name: &str| name.trim().to_lowercase();
    let wanted = key(requested);
    let mut matches = names.iter().filter(|name| key(name) == wanted);
    let first = matches.next();
    if let (Some(a), Some(b)) = (first, matches.next()) {
        return Err(format!(
            "Nama sheet {requested:?} cocok dengan {a:?} dan {b:?}. Pakai nama sheet persis dari ikhtisar workbook."
        ));
    }
    Ok(first.map(String::as_str))
}

fn missing_sheet(requested: &str, names: &[String]) -> String {
    let nearest = names
        .iter()
        .min_by_key(|name| edit_distance(requested, name))
        .unwrap();
    let shown = |name: &str| {
        let mut chars = name.chars();
        let mut bounded: String = chars.by_ref().take(40).collect();
        if chars.next().is_some() {
            bounded.push('…');
        }
        format!("{bounded:?}")
    };
    let available = names
        .iter()
        .take(12)
        .map(|name| shown(name))
        .collect::<Vec<_>>()
        .join(", ");
    let extra = names.len().saturating_sub(12);
    let suffix = if extra == 0 {
        String::new()
    } else {
        format!(" dan {extra} lainnya. Panggil tanpa sheet dan SQL untuk melihat semuanya.")
    };
    format!(
        "Sheet tidak ditemukan. Nama paling mirip {}. Sheet tersedia: {available}{suffix}",
        shown(nearest)
    )
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<_> = b.chars().take(128).flat_map(char::to_lowercase).collect();
    let mut costs: Vec<_> = (0..=b.len()).collect();
    for (i, left) in a.chars().take(128).flat_map(char::to_lowercase).enumerate() {
        let mut previous = costs[0];
        costs[0] = i + 1;
        for (j, &right) in b.iter().enumerate() {
            let top = costs[j + 1];
            costs[j + 1] = (previous + usize::from(left != right))
                .min(top + 1)
                .min(costs[j] + 1);
            previous = top;
        }
    }
    costs[b.len()]
}

fn workbook_overview(
    book: &mut Workbook<'_>,
    file: &str,
    names: &[&str],
    budget: usize,
) -> Result<String, String> {
    let mut page = Page::new(
        json!({
            "file": file,
            "jumlah_sheet": names.len(),
            "urutan_sheet": ["nama", "baris", "kolom"],
        }),
        "sheets",
        budget,
    )?;
    for &name in names {
        let bounds = book.dimensions(name)?;
        if !page.push(json!([name, bounds.last_row, bounds.columns])) {
            break;
        }
    }
    Ok(page.finish())
}

fn describe_sheet(
    options: &Options,
    name: &str,
    bounds: Bounds,
    notes: SheetNotes,
    cell: &dyn Fn(usize, usize) -> TypedCell,
    budget: usize,
) -> Result<String, String> {
    let rows = bounds.data_rows(options.header_row);
    bounds.validate(rows.len())?;
    let mut output = json!({
        "file": options.file,
        "sheet": name,
        "baris_data": rows.len(),
        "baris_judul": options.header_row,
        "rentang_baris": excel_rows(&rows),
        "urutan_contoh": ["baris", "[huruf, nilai]"],
        "contoh_baris": [],
        "contoh_terpotong": false,
        "nilai_kolom": [],
        "nilai_terpotong": false,
        "catatan_nilai": VALUES_NOTE,
    });
    notes.add_to(&mut output);
    let letters: Vec<_> = (0..bounds.columns).map(column_letter).collect();
    let mut counts: Vec<_> = (0..bounds.columns).map(|_| Counts::default()).collect();
    let mut values: Vec<_> = (0..bounds.columns)
        .map(|_| ColumnValues::default())
        .collect();
    // Di tabel lebar isi tiap kolom sudah menggantikan sebagian contoh baris.
    let sample_rows = if bounds.columns > 20 { 5 } else { 15 };
    let mut samples = Vec::new();
    let mut sample_bytes = (budget / 2).min(16_000);
    let row_budget = sample_bytes / sample_rows;
    let mut samples_cut = false;
    for row in 0..bounds.last_row {
        let sampling = samples.len() < sample_rows;
        let mut sample = Vec::new();
        let mut available = row_budget.min(sample_bytes).saturating_sub(20);
        let mut any = false;
        for (col, counts) in counts.iter_mut().enumerate() {
            let value = cell(row, col);
            if rows.contains(&row) {
                counts.add(&value);
                values[col].add(&value);
            }
            if sampling && value.0 != SqlValue::Null {
                any = true;
                let (preview, shortened) = value.preview();
                samples_cut |= shortened;
                let item = json!([letters[col], preview]);
                let size = item.to_string().len() + 1;
                if size <= available {
                    available -= size;
                    sample.push(item);
                } else {
                    samples_cut = true;
                }
            }
        }
        if any {
            let item = json!([row + 1, sample]);
            sample_bytes = sample_bytes.saturating_sub(item.to_string().len() + 1);
            samples.push(item);
        }
    }
    let mut page = Page::new(output, "kolom", budget)?;
    let shown = page.fit(&samples);
    page.output["contoh_baris"] = json!(&samples[..shown]);
    page.output["contoh_terpotong"] = json!(samples_cut || shown < samples.len());
    let values: Vec<_> = letters
        .iter()
        .zip(values)
        .filter_map(|(letter, values)| Some(json!([letter, values.summary()?])))
        .collect();
    for (col, (letter, counts)) in letters.into_iter().zip(counts).enumerate() {
        let title = if options.header_row > 0 && options.header_row <= bounds.last_row {
            cell(options.header_row - 1, col).title()
        } else {
            String::new()
        };
        let mut entry = vec![json!(letter), json!(title)];
        entry.extend(counts.0.into_iter().map(Value::from));
        if !page.push(Value::Array(entry)) {
            break;
        }
    }
    let shown = page.fit(&values);
    page.output["nilai_kolom"] = json!(&values[..shown]);
    page.output["nilai_terpotong"] = json!(shown < values.len());
    Ok(page.finish())
}

const MAX_UNIQUE: usize = 12;
const MAX_PREVIEW: usize = 48;
const VALUES_NOTE: &str = "nilai_kolom: unik berisi semua nilai dan jumlah barisnya. Kolom dengan lebih dari 12 nilai hanya menampilkan rentang atau contoh.";

/// Isi satu kolom, supaya model bisa memilih saringan tanpa membaca semua baris.
#[derive(Default)]
struct ColumnValues {
    unique: Vec<(SqlValue, usize)>,
    many: bool,
    numbers: Option<(f64, f64)>,
    dates: Option<(String, String)>,
    other: bool,
}

impl ColumnValues {
    fn add(&mut self, cell: &TypedCell) {
        match &cell.0 {
            SqlValue::Null => return,
            SqlValue::Integer(n) => self.number(*n as f64),
            SqlValue::Real(n) => self.number(*n),
            SqlValue::Text(day) if cell.1 as usize == Kind::Date as usize => {
                match &mut self.dates {
                    None => self.dates = Some((day.clone(), day.clone())),
                    Some((first, last)) => {
                        if day < first {
                            first.clone_from(day);
                        } else if day > last {
                            last.clone_from(day);
                        }
                    }
                }
            }
            _ => self.other = true,
        }
        // Teks yang akan terpotong di pratinjau tidak bisa dipakai sebagai nilai saringan.
        if matches!(&cell.0, SqlValue::Text(s) if s.len() > MAX_PREVIEW) {
            self.many = true;
        }
        if self.many {
            return;
        }
        if let Some((_, count)) = self.unique.iter_mut().find(|(value, _)| *value == cell.0) {
            *count += 1;
        } else if self.unique.len() < MAX_UNIQUE {
            self.unique.push((cell.0.clone(), 1));
        } else {
            self.many = true;
        }
    }

    fn number(&mut self, n: f64) {
        self.numbers = Some(
            self.numbers
                .map_or((n, n), |(min, max)| (min.min(n), max.max(n))),
        );
    }

    fn summary(mut self) -> Option<Value> {
        if self.unique.is_empty() && !self.many {
            return None;
        }
        if !self.many {
            self.unique
                .sort_by_key(|(_, count)| std::cmp::Reverse(*count));
            let unique: Vec<_> = self
                .unique
                .iter()
                .map(|(value, count)| json!([preview(value).0, count]))
                .collect();
            return Some(json!({ "unik": unique }));
        }
        let mut summary = json!({});
        if let Some((min, max)) = self.numbers {
            summary["angka"] = json!([whole(min), whole(max)]);
        }
        if let Some((first, last)) = self.dates {
            summary["tanggal"] = json!([first, last]);
        }
        if self.other {
            let examples: Vec<_> = self
                .unique
                .iter()
                .filter(|(value, _)| matches!(value, SqlValue::Text(_)))
                .take(3)
                .map(|(value, _)| preview(value).0)
                .collect();
            summary["contoh"] = json!(examples);
        }
        Some(summary)
    }
}

fn whole(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9e15 {
        json!(n as i64)
    } else {
        json!(n)
    }
}

struct TableSheet<'a> {
    name: &'a str,
    rows: RowRange<usize>,
    columns: usize,
    headings: Vec<Option<Heading>>,
    cells: Vec<TypedCell>,
}

impl TableSheet<'_> {
    fn heading(&self, col: usize) -> Option<&Heading> {
        self.headings.get(col).and_then(Option::as_ref)
    }
}

#[derive(PartialEq)]
struct Heading {
    row: usize,
    title: String,
}

fn read_headings(
    bounds: &Bounds,
    header: usize,
    cell: &dyn Fn(usize, usize) -> TypedCell,
) -> Vec<Option<Heading>> {
    let candidates = header.saturating_sub(15)..bounds.last_row.min(header + 14);
    (0..bounds.columns)
        .map(|col| {
            let title = cell(header - 1, col).into_title();
            if !title.trim().is_empty() {
                return Some(Heading { row: header, title });
            }
            // Saat baris judul kosong, cari contoh judul teks di baris sekitar.
            candidates.clone().find_map(|row| {
                if let TypedCell(SqlValue::Text(title), Kind::Text, _) = cell(row, col)
                    && !title.trim().is_empty()
                {
                    return Some(Heading {
                        row: row + 1,
                        title,
                    });
                }
                None
            })
        })
        .collect()
}

fn layout_note(sheets: &[TableSheet<'_>], letters: &[String], access: &Access) -> Option<Value> {
    let [first, rest @ ..] = sheets else {
        return None;
    };
    let matches = letters
        .iter()
        .enumerate()
        .filter(|(_, letter)| access.columns.contains(*letter));
    let mut differences = Vec::new();
    for (col, letter) in matches {
        let left = first.heading(col);
        if let Some(other) = rest.iter().find(|sheet| sheet.heading(col) != left) {
            let example = |sheet: &TableSheet<'_>| {
                let (row, title) = sheet.heading(col).map_or((None, String::new()), |head| {
                    (Some(head.row), short_title(head.title.clone()))
                });
                json!({ "sheet": sheet.name, "baris": row, "judul": title })
            };
            differences
                .push(json!({ "kolom": letter, "contoh": [example(first), example(other)] }));
            if differences.len() == 3 {
                break;
            }
        }
    }
    (!differences.is_empty()).then(|| json!({
        "catatan": "Judul atau letak judul berbeda. Jika tata letaknya berbeda, hitung setiap sheet terpisah.",
        "contoh_kolom": differences,
    }))
}

const ERROR_TAG: &[u8] = b"Example Corp Excel error\0";

#[derive(Clone, Copy)]
enum Kind {
    Number,
    Text,
    Date,
    Logical,
    Error,
    Empty,
    Uncached,
}

#[derive(Default)]
struct Counts([usize; 8]);

impl Counts {
    fn add(&mut self, cell: &TypedCell) {
        self.0[cell.1 as usize] += 1;
        self.0[7] += usize::from(cell.2);
    }

    fn get(&self, kind: Kind) -> usize {
        self.0[kind as usize]
    }

    fn non_numeric(&self) -> usize {
        self.0[Kind::Text as usize..=Kind::Error as usize]
            .iter()
            .sum()
    }

    fn mixed(&self) -> bool {
        self.0[..=Kind::Error as usize]
            .iter()
            .filter(|&&n| n > 0)
            .count()
            > 1
    }

    fn needs_note(&self) -> bool {
        self.non_numeric() > 0 || self.get(Kind::Uncached) > 0 || self.0[7] > 0
    }
}

struct TypedCell(SqlValue, Kind, bool);

impl TypedCell {
    fn empty() -> Self {
        Self(SqlValue::Null, Kind::Empty, false)
    }

    fn number(value: SqlValue) -> Self {
        Self(value, Kind::Number, false)
    }

    fn from_data(value: &DataRef<'_>) -> Self {
        match value {
            DataRef::Int(n) => Self::number((*n).into()),
            DataRef::Float(n) if n.is_finite() => {
                let bound = i64::MIN as f64;
                if n.fract() == 0.0 && (bound..-bound).contains(n) {
                    Self::number((*n as i64).into())
                } else {
                    Self::number((*n).into())
                }
            }
            DataRef::Float(n) => Self(n.to_string().into(), Kind::Text, false),
            DataRef::Bool(b) => Self(
                if *b { "TRUE" } else { "FALSE" }.to_owned().into(),
                Kind::Logical,
                false,
            ),
            DataRef::DateTime(d) if d.is_duration() => Self::number(d.as_f64().into()),
            DataRef::DateTime(_) | DataRef::DateTimeIso(_) | DataRef::DurationIso(_) => {
                let text = match cell_value(value) {
                    Value::String(s) => s,
                    other => other.to_string(),
                };
                Self(text.into(), Kind::Date, false)
            }
            DataRef::Error(error) => {
                let mut bytes = ERROR_TAG.to_vec();
                bytes.extend(error.to_string().as_bytes());
                Self(bytes.into(), Kind::Error, false)
            }
            _ if blank(value) => Self::empty(),
            DataRef::String(s) => Self(s.clone().into(), Kind::Text, false),
            DataRef::SharedString(s) => Self((*s).to_owned().into(), Kind::Text, false),
            DataRef::Empty => Self::empty(),
        }
    }

    fn preview(&self) -> (Value, bool) {
        preview(&self.0)
    }

    fn into_title(self) -> String {
        match self.0 {
            SqlValue::Null => String::new(),
            SqlValue::Integer(n) => n.to_string(),
            SqlValue::Real(n) => n.to_string(),
            SqlValue::Text(s) => s,
            SqlValue::Blob(bytes) => excel_error(&bytes).unwrap_or_default().into_owned(),
        }
    }

    fn title(self) -> String {
        short_title(self.into_title())
    }
}

fn preview(value: &SqlValue) -> (Value, bool) {
    match value {
        SqlValue::Null => (Value::Null, false),
        SqlValue::Integer(n) => (json!(n), false),
        SqlValue::Real(n) => (json!(n), false),
        SqlValue::Text(s) if s.len() > MAX_PREVIEW => {
            (json!(format!("{}…", &s[..s.floor_char_boundary(45)])), true)
        }
        SqlValue::Text(s) => (json!(s), false),
        SqlValue::Blob(bytes) => (json!({ "galat_excel": excel_error(bytes) }), false),
    }
}

fn short_title(mut title: String) -> String {
    const MAX_HEADER_BYTES: usize = 32;
    if title.len() > MAX_HEADER_BYTES {
        title.truncate(title.floor_char_boundary(MAX_HEADER_BYTES - '…'.len_utf8()));
        title.push('…');
    }
    title
}

fn excel_error(bytes: &[u8]) -> Option<std::borrow::Cow<'_, str>> {
    bytes.strip_prefix(ERROR_TAG).map(String::from_utf8_lossy)
}

#[allow(clippy::too_many_arguments)]
fn query(
    mut output: Value,
    sql: &str,
    sheets: &[TableSheet<'_>],
    multi: bool,
    budget: usize,
    limited_process: bool,
    book: Option<&mut Workbook<'_>>,
    save: bool,
) -> Result<String, String> {
    let width = sheets
        .iter()
        .map(|sheet| sheet.columns)
        .max()
        .unwrap_or_default();
    let letters: Vec<_> = (0..width).map(column_letter).collect();
    let add_schema = |message: String| {
        if message == SQL_ERROR {
            let columns = letters.last().map_or_else(String::new, |last| {
                if last == "A" {
                    ", A".into()
                } else {
                    format!(", A–{last}")
                }
            });
            let sheet = if multi { ", sheet" } else { "" };
            format!("{message} Kolom data yang tersedia: baris{sheet}{columns}.")
        } else {
            message
        }
    };
    let sql = single_select(sql).map_err(add_schema)?;
    let mut db = Connection::open_in_memory().map_err(sql_error)?;
    if limited_process {
        // Batas heap SQLite berlaku untuk seluruh proses. Di sini hanya ada satu perhitungan.
        db.pragma_update(None, "hard_heap_limit", SQLITE_HEAP_BYTES)
            .map_err(sql_error)?;
    }
    db.pragma_update(None, "temp_store", "MEMORY")
        .map_err(sql_error)?;
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_DQS_DML, false)
        .map_err(sql_error)?;
    numeric::register(&db).map_err(sql_error)?;
    let baris = if multi {
        "baris INTEGER"
    } else {
        "baris INTEGER PRIMARY KEY"
    };
    let fields = std::iter::once(baris.to_owned())
        .chain(multi.then(|| "sheet TEXT".to_owned()))
        .chain(letters.iter().map(|letter| format!("\"{letter}\"")))
        .collect::<Vec<_>>();
    db.execute(&format!("CREATE TABLE data ({})", fields.join(",")), [])
        .map_err(sql_error)?;
    let counts = write_rows(&mut db, sheets, width, multi).map_err(sql_error)?;
    let color_sheet = if multi { "sheet TEXT NOT NULL," } else { "" };
    let color_key = if multi {
        "sheet, baris, kolom"
    } else {
        "baris, kolom"
    };
    // View sementara membedakan pembacaan warna dari COUNT(*) atas CTE bernama warna.
    // LIMIT mencegah SQLite melebur view dan menghilangkan konteksnya dari authorizer.
    db.execute_batch(&format!("CREATE TABLE warna_sel (baris INTEGER, kolom TEXT, warna TEXT, nama TEXT, {color_sheet} PRIMARY KEY ({color_key})) WITHOUT ROWID;
        CREATE VIEW warna AS SELECT * FROM warna_sel LIMIT -1"))
        .map_err(sql_error)?;

    db.set_limit(Limit::SQLITE_LIMIT_LENGTH, MAX_VALUE_BYTES)
        .map_err(sql_error)?;
    db.set_limit(Limit::SQLITE_LIMIT_VDBE_OP, 100_000)
        .map_err(sql_error)?;
    db.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)
        .map_err(sql_error)?;
    let access = Arc::new(Mutex::new(Access::default()));
    let install_progress_handler = |db: &mut Connection| {
        let mut calls = 0;
        db.progress_handler(
            PROGRESS_INTERVAL,
            Some(move || {
                calls += 1;
                calls >= MAX_PROGRESS_CALLS
            }),
        )
        .map_err(sql_error)
    };
    let install_authorizer = |db: &mut Connection| {
        let observed = access.clone();
        db.authorizer(Some(move |ctx: AuthContext<'_>| {
            let authorized = authorize(ctx);
            if authorized == Authorization::Allow {
                match ctx.action {
                    AuthAction::Read {
                        table_name: "data",
                        column_name,
                    } if !column_name.is_empty() => {
                        observed
                            .lock()
                            .unwrap()
                            .columns
                            .insert(column_name.to_owned());
                    }
                    AuthAction::Read {
                        table_name: "warna_sel",
                        ..
                    } if ctx
                        .accessor
                        .is_some_and(|name| name.eq_ignore_ascii_case("warna")) =>
                    {
                        observed.lock().unwrap().colors = true;
                    }
                    AuthAction::Function {
                        function_name: "min" | "max",
                    } => {
                        observed.lock().unwrap().extremes = true;
                    }
                    _ => {}
                }
            }
            authorized
        }))
        .map_err(sql_error)
    };
    install_progress_handler(&mut db)?;
    install_authorizer(&mut db)?;
    let mut statement = prepare_select(&db, sql).map_err(add_schema)?;
    if access.lock().unwrap().colors {
        drop(statement);
        db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .map_err(sql_error)?;
        db.progress_handler(0, None::<fn() -> bool>)
            .map_err(sql_error)?;
        colors::populate(&mut db, book.ok_or(SQL_ERROR)?, sheets, multi, &mut output)?;
        db.execute_batch("DROP VIEW warna; ALTER TABLE warna_sel RENAME TO warna")
            .map_err(sql_error)?;
        install_progress_handler(&mut db)?;
        install_authorizer(&mut db)?;
        statement = prepare_select(&db, sql).map_err(add_schema)?;
    }
    let access = access.lock().unwrap();
    if let Some(note) = layout_note(sheets, &letters, &access) {
        output["catatan_tata_letak"] = note;
    }
    let columns: Vec<_> = letters
        .iter()
        .zip(&counts)
        .filter(|(letter, _)| access.columns.contains(*letter))
        .collect();
    let caution: Vec<_> = columns
        .iter()
        .filter_map(|(letter, counts)| {
            if !counts.needs_note() {
                return None;
            }
            let mut entry = vec![json!(letter)];
            entry.extend(counts.0.into_iter().map(Value::from));
            Some(Value::Array(entry))
        })
        .collect();
    if !caution.is_empty() {
        let fields: Vec<_> = ["huruf"].into_iter().chain(CELL_TYPES).collect();
        output["profil_kolom"] = json!({
            "urutan": fields,
            "kolom": [],
            "kolom_terpotong": caution.len(),
            "catatan": "Aritmetika atas nilai bukan angka bisa keliru. Rumus tanpa nilai dan cache string kosong memakai NULL. Lindungi ekspresi, misalnya CASE WHEN typeof(A) IN ('integer','real') THEN A * 1 END."
        });
    }
    let mixed: Vec<_> = columns
        .iter()
        .filter_map(|(letter, counts)| counts.mixed().then_some(letter))
        .collect();
    if access.extremes && !mixed.is_empty() {
        output["catatan_min_max"] = json!({
            "kolom_bercampur": mixed,
            "catatan": "MIN/MAX mengikuti urutan jenis SQLite. Untuk angka saja, saring dengan typeof()."
        });
    }
    drop(access);
    output["kolom"] = json!(statement.column_names().collect::<Vec<_>>());
    let width = statement.column_count();
    let query_error = |error| add_schema(aggregate_error(error, &columns));
    if save {
        // Judul asli hanya untuk kolom huruf yang diambil apa adanya dari tabel data. Kolom dengan
        // alias atau hasil ekspresi memakai nama hasilnya (kolom asal dari SQLite, bukan tebakan
        // dari teks SQL).
        let titles: Vec<String> = statement
            .columns_with_metadata()
            .map(|column| match (column.table_name(), column.origin_name()) {
                (Some("data"), Some(origin)) if origin == column.name() => {
                    save_title(origin, &letters, sheets)
                }
                _ => column.name().to_owned(),
            })
            .collect();
        let mut rows = Vec::new();
        let mut truncated = false;
        let mut bytes = 0_usize;
        let mut result = statement.query([]).map_err(query_error)?;
        while let Some(row) = result.next().map_err(query_error)? {
            if rows.len() == crate::files::MAX_ROWS {
                truncated = true;
                break;
            }
            let cells = (0..width)
                .map(|col| save_value(row.get_ref(col).map_err(sql_error)?))
                .collect::<Result<Vec<_>, _>>()?;
            // Ukuran setelah di-JSON-kan dua kali (hasil lalu amplop proses pembaca), jadi tanda
            // kutip dan backslash dihitung dobel.
            let row_bytes: usize = cells
                .iter()
                .map(|cell| {
                    let json = cell.to_string();
                    json.len() + json.bytes().filter(|b| matches!(b, b'"' | b'\\')).count() + 1
                })
                .sum();
            if bytes + row_bytes > SAVE_MAX_BYTES {
                truncated = true;
                break;
            }
            bytes += row_bytes;
            rows.push(Value::Array(cells));
        }
        output["simpan"] = json!({ "judul": titles, "baris": rows, "terpotong": truncated });
        return Ok(output.to_string());
    }
    let mut page = Page::new(output, "hasil", budget)?;
    let mut result = statement.query([]).map_err(query_error)?;
    if let Some(row) = result.next().map_err(query_error)? {
        append_sql_row(&mut page, row, width)?;
        if page.items.is_empty() {
            return Err("Anggaran hasil tidak cukup untuk menampilkan satu baris. Pilih lebih sedikit kolom atau pendekkan nilainya.".into());
        }
    }
    if !caution.is_empty() {
        let shown = page.fit(&caution);
        page.output["profil_kolom"]["kolom"] = json!(&caution[..shown]);
        page.output["profil_kolom"]["kolom_terpotong"] = json!(caution.len() - shown);
    }
    while !page.truncated {
        let Some(row) = result.next().map_err(query_error)? else {
            break;
        };
        if page.items.len() == MAX_RESULT_ROWS {
            page.truncated = true;
            break;
        }
        append_sql_row(&mut page, row, width)?;
    }
    Ok(page.finish())
}

fn prepare_select<'a>(db: &'a Connection, sql: &str) -> Result<rusqlite::Statement<'a>, String> {
    let statement = db
        .prepare_with_flags(sql, PrepFlags::SQLITE_PREPARE_NO_VTAB)
        .map_err(sql_error)?;
    if !statement.readonly() || statement.is_explain() != 0 || statement.column_count() == 0 {
        return Err(SQL_DENIED.into());
    }
    Ok(statement)
}

fn append_sql_row(page: &mut Page, row: &rusqlite::Row<'_>, width: usize) -> Result<(), String> {
    let mut cells = Vec::with_capacity(width);
    let mut text_bytes = 0;
    for col in 0..width {
        let value = match row.get_ref(col).map_err(sql_error)? {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(n) => json!(n),
            ValueRef::Real(n) if n.is_finite() => json!(n),
            ValueRef::Real(_) => {
                return Err(
                    "Hasil SQL bukan angka hingga. Periksa operasi hitung atau ubah skalanya."
                        .into(),
                );
            }
            ValueRef::Text(s) => {
                text_bytes += s.len();
                if text_bytes > page.remaining {
                    page.truncated = true;
                    return Ok(());
                }
                json!(String::from_utf8_lossy(s))
            }
            ValueRef::Blob(bytes) => match excel_error(bytes) {
                Some(error) => json!({ "galat_excel": error }),
                None => {
                    return Err(
                        "SQL menghasilkan BLOB. Gunakan hex() atau pilih kolom lain.".into(),
                    );
                }
            },
        };
        cells.push(value);
    }
    page.push(Value::Array(cells));
    Ok(())
}

/// Hasil simpan_ke_excel paling besar segini (perkiraan), supaya keluaran proses pembaca tetap di
/// bawah batasnya. Lewat dari ini, baris berikutnya tidak ikut dan hasilnya ditandai terpotong.
const SAVE_MAX_BYTES: usize = 24 * 1024 * 1024;

/// Judul kolom file simpanan: judul asli di baris judul untuk kolom huruf (sama di semua sheet
/// terpilih), selain itu nama kolom hasil SQL apa adanya.
fn save_title(name: &str, letters: &[String], sheets: &[TableSheet<'_>]) -> String {
    let Some(col) = letters.iter().position(|letter| letter == name) else {
        return name.to_owned();
    };
    let title = |sheet: &TableSheet<'_>| {
        sheet
            .heading(col)
            .filter(|heading| heading.row == sheet.rows.start)
            .map(|heading| heading.title.trim().to_owned())
            .filter(|title| !title.is_empty())
    };
    match sheets.split_first() {
        Some((first, rest)) => {
            let found = title(first);
            if rest.iter().all(|sheet| title(sheet) == found) {
                found.unwrap_or_else(|| name.to_owned())
            } else {
                name.to_owned()
            }
        }
        None => name.to_owned(),
    }
}

/// Nilai satu sel hasil SQL untuk file simpanan. Galat Excel disimpan sebagai teksnya.
fn save_value(value: ValueRef<'_>) -> Result<Value, String> {
    Ok(match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(n) => json!(n),
        ValueRef::Real(n) if n.is_finite() => json!(n),
        ValueRef::Real(_) => {
            return Err(
                "Hasil SQL bukan angka hingga. Periksa operasi hitung atau ubah skalanya.".into(),
            );
        }
        ValueRef::Text(s) => json!(String::from_utf8_lossy(s)),
        ValueRef::Blob(bytes) => match excel_error(bytes) {
            Some(error) => json!(error),
            None => {
                return Err("SQL menghasilkan BLOB. Gunakan hex() atau pilih kolom lain.".into());
            }
        },
    })
}

fn single_select(sql: &str) -> Result<&str, String> {
    let sql = skip_sql_trivia(sql);
    let keyword = sql
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or("");
    if !keyword.eq_ignore_ascii_case("SELECT") && !keyword.eq_ignore_ascii_case("WITH") {
        return Err(SQL_ERROR.into());
    }
    let mut at = 0;
    let bytes = sql.as_bytes();
    while at < bytes.len() {
        match bytes[at] {
            b';' => {
                return if skip_sql_trivia(&sql[at + 1..]).is_empty() {
                    Ok(&sql[..at])
                } else {
                    Err("Kirim satu SELECT saja. Pisahkan perhitungan lain ke panggilan berikutnya.".into())
                };
            }
            b'\'' | b'"' | b'\x60' | b'[' => {
                let close = if bytes[at] == b'[' { b']' } else { bytes[at] };
                at += 1;
                at += bytes[at..]
                    .iter()
                    .position(|&b| b == close)
                    .ok_or(SQL_ERROR)?
                    + 1;
            }
            b'-' if bytes.get(at + 1) == Some(&b'-') => {
                at += sql[at..].find('\n').unwrap_or(sql.len() - at);
            }
            b'/' if bytes.get(at + 1) == Some(&b'*') => {
                at += sql[at + 2..].find("*/").map_or(sql.len() - at, |n| n + 4);
            }
            _ => at += 1,
        }
    }
    Ok(sql)
}

fn skip_sql_trivia(mut sql: &str) -> &str {
    loop {
        sql = sql.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '\u{feff}');
        if let Some(comment) = sql.strip_prefix("--") {
            sql = comment.split_once('\n').map_or("", |(_, rest)| rest);
        } else if let Some(comment) = sql.strip_prefix("/*") {
            sql = comment.split_once("*/").map_or("", |(_, rest)| rest);
        } else {
            return sql;
        }
    }
}

#[derive(Default)]
struct Access {
    columns: BTreeSet<String>,
    extremes: bool,
    colors: bool,
}

fn aggregate_error(error: Error, columns: &[(&String, &Counts)]) -> String {
    let count = match numeric::failure(&error) {
        Some(Ok(count)) => count,
        Some(Err(message)) => return message.into(),
        None => return sql_error(error),
    };
    let candidates: Vec<_> = columns
        .iter()
        .filter_map(|(letter, counts)| {
            let n = counts.non_numeric();
            (n > 0).then(|| format!("{letter} ({n})"))
        })
        .take(12)
        .collect();
    let location = if candidates.is_empty() {
        String::new()
    } else {
        format!(
            " Kolom yang dibaca dengan nilai bukan angka (jumlah di sheet): {}.",
            candidates.join(", ")
        )
    };
    format!(
        "SUM, AVG, atau TOTAL menerima {count} nilai bukan angka.{location} Samng dengan typeof(A) IN ('integer','real') dan ganti A dengan kolom atau ekspresi yang dihitung."
    )
}

fn write_rows(
    db: &mut Connection,
    sheets: &[TableSheet<'_>],
    width: usize,
    multi: bool,
) -> rusqlite::Result<Vec<Counts>> {
    let tx = db.transaction()?;
    let parameters = vec!["?"; width + 1 + usize::from(multi)].join(",");
    let mut counts: Vec<_> = (0..width).map(|_| Counts::default()).collect();
    {
        let mut insert = tx.prepare(&format!("INSERT INTO data VALUES ({parameters})"))?;
        for sheet in sheets {
            let name = SqlValue::Text(sheet.name.to_owned());
            for (i, row) in sheet.rows.clone().enumerate() {
                let start = i * sheet.columns;
                let row_number = SqlValue::Integer(row as i64 + 1);
                let values = std::iter::once(&row_number)
                    .chain(multi.then_some(&name))
                    .chain(counts.iter_mut().enumerate().map(|(col, count)| {
                        if col < sheet.columns {
                            let cell = &sheet.cells[start + col];
                            count.add(cell);
                            &cell.0
                        } else {
                            count.0[Kind::Empty as usize] += 1;
                            &SqlValue::Null
                        }
                    }));
                insert.execute(params_from_iter(values))?;
            }
        }
    }
    tx.commit()?;
    Ok(counts)
}

fn authorize(ctx: AuthContext<'_>) -> Authorization {
    let allowed = match ctx.action {
        AuthAction::Select | AuthAction::Recursive => true,
        AuthAction::Read {
            table_name: "data" | "warna",
            ..
        } => ctx.database_name.is_none_or(|db| db == "main"),
        AuthAction::Read { table_name, .. } if table_name.eq_ignore_ascii_case("warna_sel") => {
            ctx.accessor
                .is_some_and(|name| name.eq_ignore_ascii_case("warna"))
                && ctx.database_name.is_none_or(|db| db == "main")
        }
        // SQLite mengirim nama kolom kosong dan database NULL untuk COUNT(*) dari CTE.
        AuthAction::Read {
            table_name,
            column_name: "",
        } if ctx.database_name.is_none() => !table_name
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("sqlite_")),
        AuthAction::Function { function_name } => matches!(
            function_name,
            "abs"
                | "avg"
                | "char"
                | "coalesce"
                | "concat"
                | "concat_ws"
                | "count"
                | "cume_dist"
                | "date"
                | "datetime"
                | "dense_rank"
                | "first_value"
                | "glob"
                | "group_concat"
                | "hex"
                | "if"
                | "ifnull"
                | "iif"
                | "instr"
                | "julianday"
                | "lag"
                | "last_value"
                | "lead"
                | "length"
                | "like"
                | "lower"
                | "ltrim"
                | "max"
                | "min"
                | "nth_value"
                | "ntile"
                | "nullif"
                | "percent_rank"
                | "rank"
                | "replace"
                | "round"
                | "row_number"
                | "rtrim"
                | "sign"
                | "strftime"
                | "string_agg"
                | "substr"
                | "substring"
                | "sum"
                | "time"
                | "total"
                | "trim"
                | "typeof"
                | "unicode"
                | "unixepoch"
                | "upper"
        ),
        _ => false,
    };
    if allowed {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}

fn sql_error(error: Error) -> String {
    match error.sqlite_error_code() {
        Some(ErrorCode::OperationInterrupted) =>
            "SQL melewati batas langkah. Kurangi gabungan tabel, rekursi, atau jumlah kolom yang diproses.",
        Some(ErrorCode::TooBig | ErrorCode::OutOfMemory) =>
            "SQL melewati batas ukuran nilai atau memori. Kurangi gabungan teks, jumlah kolom, atau baris yang diproses.",
        Some(ErrorCode::AuthorizationForStatementDenied) => SQL_DENIED,
        _ if matches!(error, Error::MultipleStatement) =>
            "Kirim satu SELECT saja. Pisahkan perhitungan lain ke panggilan berikutnya.",
        _ => SQL_ERROR,
    }.into()
}

struct Page {
    output: Value,
    field: &'static str,
    items: Vec<Value>,
    remaining: usize,
    truncated: bool,
}

impl Page {
    fn new(mut output: Value, field: &'static str, budget: usize) -> Result<Self, String> {
        output[field] = json!([]);
        if field == "kolom" {
            let fields: Vec<_> = ["huruf", "judul"].into_iter().chain(CELL_TYPES).collect();
            output["urutan_kolom"] = json!(fields);
            output["catatan_logika"] = json!(LOGIC_NOTE);
        }
        output["catatan_batas"] = json!(match field {
            "kolom" =>
                "Daftar kolom terpotong karena batas hasil. Pilih kolom yang diperlukan dengan SQL.",
            "sheets" =>
                "Daftar sheet terpotong karena batas hasil. Gunakan sheets untuk melihat daftar yang lebih kecil.",
            _ =>
                "Hasil terpotong karena batas baris atau ukuran. Tambahkan penyaring atau ringkas dengan agregasi.",
        });
        let remaining = budget
            .checked_sub(output.to_string().len())
            .ok_or(SMALLER_RESULT)?;
        Ok(Self {
            output,
            field,
            items: Vec::new(),
            remaining,
            truncated: false,
        })
    }

    fn push(&mut self, item: Value) -> bool {
        let size = item.to_string().len() + usize::from(!self.items.is_empty());
        if size > self.remaining {
            self.truncated = true;
            return false;
        }
        self.remaining -= size;
        self.items.push(item);
        true
    }

    fn fit(&mut self, items: &[Value]) -> usize {
        let mut shown = 0;
        for (i, item) in items.iter().enumerate() {
            let size = item.to_string().len() + usize::from(i > 0);
            if size > self.remaining {
                break;
            }
            self.remaining -= size;
            shown += 1;
        }
        shown
    }

    fn finish(mut self) -> String {
        self.output[self.field] = Value::Array(self.items);
        if !self.truncated {
            self.output.as_object_mut().unwrap().remove("catatan_batas");
        }
        self.output.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_xlsxwriter::{Formula, Workbook};

    fn options(sql: Option<&str>) -> Options {
        Options {
            file: "Contoh.xlsx".into(),
            sheet: None,
            sheets: None,
            sql: sql.map(str::to_owned),
            budget: MAX_OUTPUT,
            pending: false,
            header_row: 1,
            simpan: None,
        }
    }

    fn detail(name: &str) -> Options {
        Options {
            sheet: Some(name.into()),
            ..options(None)
        }
    }

    fn result(kind: &str, bytes: &[u8], params: Options) -> Value {
        serde_json::from_str(&calculate(kind, bytes, params, false).unwrap()).unwrap()
    }

    #[test]
    fn normalizes_empty_tool_arguments_and_names_invalid_fields() {
        for empty in [json!(""), Value::Null, json!([]), json!([""]), json!(" \t")] {
            let args = json!({
                "file_id": "uji",
                "sheet": empty,
                "sheets": empty,
                "sql": empty,
                "header_row": empty,
            });
            let (id, request) = Options::from_tool(&args.to_string(), MAX_OUTPUT).unwrap();
            assert_eq!(id, Input::File("uji".into()));
            assert!(request.sheet.is_none() && request.sheets.is_none() && request.sql.is_none());
            assert_eq!(request.header_row, 1);
        }
        for (header, expected) in [(json!(0), 0), (json!("0"), 0), (json!("1"), 1)] {
            let args = json!({"file_id": "uji", "header_row": header, "sheets": [" All ", " "]});
            let (_, request) = Options::from_tool(&args.to_string(), MAX_OUTPUT).unwrap();
            assert_eq!(request.header_row, expected);
            assert_eq!(request.sheets.unwrap(), [" All "]);
        }
        for (field, invalid) in [
            ("sheet", json!(["Data"])),
            ("sheets", json!("Data")),
            ("sql", json!(0)),
            ("header_row", json!("satu")),
        ] {
            let mut args = json!({"file_id": "uji"});
            args[field] = invalid;
            let error = Options::from_tool(&args.to_string(), MAX_OUTPUT)
                .err()
                .unwrap();
            assert!(error.contains(field), "{error}");
        }
        let args = json!({"sitasi": 3});
        assert_eq!(
            Options::from_tool(&args.to_string(), MAX_OUTPUT).unwrap().0,
            Input::Citation(3)
        );
        // Bentuk yang dikirim model saat semua parameter diisi.
        let filled = json!({"file_id": "uji", "sitasi": 0, "sheet": "Sheet1", "sheets": [], "header_row": 1, "sql": ""});
        let (id, request) = Options::from_tool(&filled.to_string(), MAX_OUTPUT).unwrap();
        assert_eq!(id, Input::File("uji".into()));
        assert_eq!(request.sheet.as_deref(), Some("Sheet1"));
        let filled = json!({"file_id": "", "sitasi": 3, "sheet": "", "sheets": [], "header_row": 1, "sql": ""});
        assert_eq!(
            Options::from_tool(&filled.to_string(), MAX_OUTPUT)
                .unwrap()
                .0,
            Input::Citation(3)
        );
        for (invalid, message) in [
            (json!({"sitasi": 0}), "Isi salah satu"),
            (json!({"file_id": "uji", "sitasi": 3}), "jangan keduanya"),
            (json!({}), "Isi salah satu"),
        ] {
            let error = Options::from_tool(&invalid.to_string(), MAX_OUTPUT)
                .err()
                .unwrap();
            assert!(error.contains(message), "{error}");
        }
    }

    #[test]
    fn distinguishes_locked_ooxml_from_damaged_workbooks() {
        let mut ole = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        ole.create_stream("/EncryptedPackage").unwrap();
        ole.create_stream("/EncryptionInfo").unwrap();
        let locked = ole.into_inner().into_inner();
        for kind in ["xlsx", "xlsm"] {
            assert!(
                calculate(kind, &locked, options(None), false)
                    .unwrap_err()
                    .contains("dikunci password")
            );
            assert!(
                calculate(kind, b"rusak", options(None), false)
                    .unwrap_err()
                    .contains("tidak bisa dibaca")
            );
        }
    }

    #[test]
    fn finds_sheet_names_that_end_with_a_space() {
        let names = ["Mesin 1", "ALL "].map(str::to_owned);
        for name in ["ALL", "all", "ALL "] {
            assert_eq!(
                Selection::new(&names, &detail(name)).unwrap().names,
                ["ALL "]
            );
        }
        let names = ["Data", "Data "].map(str::to_owned);
        assert_eq!(
            Selection::new(&names, &detail("Data")).unwrap().names,
            ["Data"]
        );
        assert!(Selection::new(&names, &detail("data")).is_err());
    }

    #[test]
    fn resolves_exact_sheet_names_before_aliases_and_reports_conflicts() {
        let names = ["ALL", "semua", "Data", "DATA"].map(str::to_owned);
        for name in ["ALL", "all", " all ", "semua", "DATA"] {
            let selection = Selection::new(&names, &detail(name)).unwrap();
            let expected = if name.trim() == "all" { "ALL" } else { name };
            assert_eq!(selection.names, [expected]);
            assert_eq!(selection.detail, Some(expected));
        }
        assert_eq!(Selection::new(&names, &detail("*")).unwrap().names, names);
        let error = Selection::new(&names, &detail("data")).err().unwrap();
        assert!(error.contains("persis") && error.contains("Data") && error.contains("DATA"));

        for name in ["ALL", "*"] {
            let mut request = detail(name);
            request.sheets = Some(vec!["Data".into()]);
            let error = Selection::new(&names, &request).err().unwrap();
            assert!(error.contains("sheet dan sheets bertentangan"), "{error}");
        }
        let mut request = detail("ALL");
        request.sheets = Some(vec!["ALL".into()]);
        assert_eq!(Selection::new(&names, &request).unwrap().names, ["ALL"]);
    }

    #[test]
    fn warns_about_different_headings_and_positions_in_columns_read_by_sql() {
        let mut book = Workbook::new();
        let first = book.add_worksheet().set_name("Pertama").unwrap();
        first.write_row(0, 0, ["Jumlah", "Kode", "Mesin"]).unwrap();
        first.write(1, 0, 10).unwrap();
        let second = book.add_worksheet().set_name("Kedua").unwrap();
        second.write(0, 0, "Berat").unwrap();
        second.write(0, 2, "Operator").unwrap();
        second.write(4, 1, "Kode").unwrap();
        second.write(5, 0, 30).unwrap();
        let bytes = book.save_to_buffer().unwrap();
        let mut request = detail("*");
        request.sql = Some("SELECT SUM(A), COUNT(B) FROM data".into());
        let output = result("xlsx", &bytes, request);
        assert_eq!(output["hasil"], json!([[40, 1]]));
        assert_eq!(
            output["catatan_tata_letak"]["contoh_kolom"],
            json!([
                {"kolom": "A", "contoh": [
                    {"sheet": "Pertama", "baris": 1, "judul": "Jumlah"},
                    {"sheet": "Kedua", "baris": 1, "judul": "Berat"},
                ]},
                {"kolom": "B", "contoh": [
                    {"sheet": "Pertama", "baris": 1, "judul": "Kode"},
                    {"sheet": "Kedua", "baris": 5, "judul": "Kode"},
                ]},
            ])
        );
        let mut request = detail("*");
        request.sql = Some("SELECT COUNT(*) FROM data".into());
        let output = result("xlsx", &bytes, request);
        assert!(output["catatan_tata_letak"].is_null());
    }

    #[test]
    fn xlsx_overview_and_sql_agree_on_format_only_sheets_and_trailing_rows() {
        let mut book = Workbook::new();
        let format = rust_xlsxwriter::Format::new().set_bold();
        let styles = book.add_worksheet().set_name("Format").unwrap();
        styles.write_blank(80, 7, &format).unwrap();
        let data = book.add_worksheet().set_name("Data").unwrap();
        data.write(0, 0, "Nilai").unwrap();
        data.write(1, 0, 9).unwrap();
        data.write_blank(4, 7, &format).unwrap();
        data.set_row_format(7, &format).unwrap();
        let bytes = book.save_to_buffer().unwrap();
        assert_eq!(
            result("xlsx", &bytes, options(None))["sheets"],
            // Baris dan sel yang hanya berformat (baris 5 dan 8) tidak ikut, seperti di calamine.
            json!([["Format", 0, 0], ["Data", 2, 1]])
        );
        let mut request = detail("*");
        request.sql =
            Some("SELECT sheet, COUNT(*), MAX(baris), SUM(A) FROM data GROUP BY sheet".into());
        assert_eq!(
            result("xlsx", &bytes, request)["hasil"],
            json!([["Data", 1, 2, 9]])
        );
        let output = result("xlsx", &bytes, options(Some("SELECT COUNT(*) FROM data")));
        assert_eq!(output["hasil"], json!([[0]]));
    }

    #[test]
    fn empty_arguments_and_multi_sheet_queries_follow_the_requested_scope() {
        let mut book = Workbook::new();
        for (name, value) in [("Mesin Satu", 10), ("Mesin Dua", 20), ("Mesin Tiga", 30)] {
            let sheet = book.add_worksheet().set_name(name).unwrap();
            sheet.write(0, 0, "Nilai").unwrap();
            sheet.write(1, 0, value).unwrap();
            if name == "Mesin Dua" {
                sheet.write(1, 1, "catatan").unwrap();
            }
        }
        let bytes = book.save_to_buffer().unwrap();
        for (sheet, sql) in [
            (None, None),
            (Some(""), Some("")),
            (Some(" \t"), Some("  ")),
            (Some("ALL"), Some("")),
        ] {
            let mut request = options(sql);
            request.sheet = sheet.map(str::to_owned);
            let info = result("xlsx", &bytes, request);
            assert_eq!(
                info["sheets"],
                json!([
                    ["Mesin Satu", 2, 1],
                    ["Mesin Dua", 2, 2],
                    ["Mesin Tiga", 2, 1]
                ])
            );
        }
        let sql =
            "SELECT sheet, baris, SUM(A), COUNT(B) FROM data GROUP BY sheet, baris ORDER BY SUM(A)";
        for alias in ["*", "ALL", "semua"] {
            let mut request = options(Some(sql));
            request.sheet = Some(alias.into());
            let all = result("xlsx", &bytes, request);
            assert_eq!(
                all["hasil"],
                json!([
                    ["Mesin Satu", 2, 10, 0],
                    ["Mesin Dua", 2, 20, 1],
                    ["Mesin Tiga", 2, 30, 0]
                ])
            );
            assert_eq!(
                all["profil_kolom"]["kolom"],
                json!([["B", 0, 1, 0, 0, 0, 2, 0, 0]])
            );
        }
        let mut request = options(Some(sql));
        request.sheets = Some(vec!["Mesin Tiga".into(), "Mesin Dua".into()]);
        let subset = result("xlsx", &bytes, request);
        assert_eq!(
            subset["hasil"],
            json!([["Mesin Dua", 2, 20, 1], ["Mesin Tiga", 2, 30, 0]])
        );
        assert_eq!(subset["sheets"], json!(["Mesin Tiga", "Mesin Dua"]));
        let mut request = options(Some("SELECT SUM(A) FROM data"));
        request.sheet = Some("".into());
        assert_eq!(result("xlsx", &bytes, request)["hasil"], json!([[10]]));

        let bad = calculate("xlsx", &bytes, detail("Mesin Satuu"), false).unwrap_err();
        assert!(
            bad.contains("paling mirip \"Mesin Satu\"") && bad.contains("Mesin Tiga"),
            "{bad}"
        );
        let mut request = options(Some("SELECTZ Z FROM data"));
        request.sheet = Some("*".into());
        let invalid = calculate("xlsx", &bytes, request, false).unwrap_err();
        assert!(invalid.contains("baris, sheet, A–B"), "{invalid}");
        let invalid =
            calculate("xlsx", &bytes, options(Some("SELECT Z FROM data")), false).unwrap_err();
        assert!(
            invalid.contains("baris, A") && !invalid.contains("baris, sheet"),
            "{invalid}"
        );
    }

    #[test]
    fn detail_shows_the_first_fifteen_nonempty_excel_rows_with_cell_addresses() {
        let mut book = Workbook::new();
        let sheet = book.add_worksheet().set_name("Formulir").unwrap();
        sheet
            .merge_range(
                0,
                0,
                0,
                2,
                "Judul formulir",
                &rust_xlsxwriter::Format::new(),
            )
            .unwrap();
        sheet.write(4, 1, "Tanggal").unwrap();
        sheet.write(4, 2, "Jumlah").unwrap();
        for row in 6..30 {
            sheet.write(row, 2, row + 1).unwrap();
        }
        let bytes = book.save_to_buffer().unwrap();
        let data = result("xlsx", &bytes, detail("Formulir"));
        assert_eq!(data["rentang_sel_gabungan"], 1);
        let preview = data["contoh_baris"].as_array().unwrap();
        assert_eq!(preview.len(), 15);
        assert_eq!(preview[0], json!([1, [["A", "Judul formulir"]]]));
        assert_eq!(preview[1], json!([5, [["B", "Tanggal"], ["C", "Jumlah"]]]));
        assert_eq!(preview[2], json!([7, [["C", 7]]]));
        assert_eq!(data["contoh_terpotong"], false);
    }

    #[test]
    fn detail_lists_column_values_for_choosing_a_filter() {
        let mut book = Workbook::new();
        let sheet = book.add_worksheet().set_name("Stok").unwrap();
        let day = rust_xlsxwriter::Format::new().set_num_format("yyyy-mm-dd");
        for (col, title) in [
            "Kode", "Panjang", "Tanggal", "Lot", "Campur", "Catatan", "Kosong",
        ]
        .iter()
        .enumerate()
        {
            sheet.write(0, col as u16, *title).unwrap();
        }
        for row in 1..=40_u32 {
            let kode = if row <= 25 { "SP" } else { "LP" };
            sheet.write(row, 0, kode).unwrap();
            sheet.write(row, 1, 4990 + row * 10).unwrap();
            let date = rust_xlsxwriter::ExcelDateTime::from_ymd(2026, 1, 1)
                .unwrap()
                .to_excel()
                + f64::from(row);
            sheet.write_with_format(row, 2, date, &day).unwrap();
            sheet.write(row, 3, format!("L{row:03}")).unwrap();
            let note = if row == 1 {
                "singkat".into()
            } else {
                "x".repeat(60)
            };
            sheet.write(row, 5, note).unwrap();
            if row % 2 == 0 {
                sheet.write(row, 4, row).unwrap();
            } else {
                sheet.write(row, 4, format!("P{row}")).unwrap();
            }
        }
        let bytes = book.save_to_buffer().unwrap();
        let data = result("xlsx", &bytes, detail("Stok"));
        assert_eq!(
            data["nilai_kolom"],
            json!([
                ["A", {"unik": [["SP", 25], ["LP", 15]]}],
                ["B", {"angka": [5000, 5390]}],
                ["C", {"tanggal": ["2026-01-02", "2026-02-10"]}],
                ["D", {"contoh": ["L001", "L002", "L003"]}],
                ["E", {"angka": [2, 40], "contoh": ["P1", "P3", "P5"]}],
                ["F", {"contoh": ["singkat"]}],
            ])
        );
        assert_eq!(data["nilai_terpotong"], false);
        assert_eq!(data["contoh_baris"].as_array().unwrap().len(), 15);
    }

    #[test]
    fn wide_detail_shows_fewer_rows_and_keeps_column_values() {
        let mut book = Workbook::new();
        let sheet = book.add_worksheet().set_name("Lebar").unwrap();
        for col in 0..47_u16 {
            sheet.write(0, col, format!("Kolom {col}")).unwrap();
            for row in 1..=60_u32 {
                sheet.write(row, col, format!("{col}-{}", row % 4)).unwrap();
            }
        }
        let bytes = book.save_to_buffer().unwrap();
        let data = result("xlsx", &bytes, detail("Lebar"));
        assert_eq!(data["contoh_baris"].as_array().unwrap().len(), 5);
        let values = data["nilai_kolom"].as_array().unwrap();
        assert_eq!(values.len(), 47);
        assert_eq!(values[46][0], "AU");
        assert_eq!(values[46][1]["unik"].as_array().unwrap().len(), 4);
        assert!(data.to_string().len() < 20_000);
    }

    #[test]
    fn combining_sheets_stops_at_the_total_cell_limit_and_names_the_sheet() {
        let mut book = Workbook::new();
        for (name, last) in [("Satu", 999), ("Dua", 1000)] {
            let sheet = book.add_worksheet().set_name(name).unwrap();
            sheet.write(0, 0, "Awal").unwrap();
            sheet.write(last, 999, 1).unwrap();
        }
        let bytes = book.save_to_buffer().unwrap();
        let mut request = options(Some("SELECT COUNT(*) FROM data"));
        request.sheet = Some("*".into());
        let error = calculate("xlsx", &bytes, request, false).unwrap_err();
        assert!(
            error.contains("\"Dua\"") && error.contains("2 juta") && error.contains("sheets"),
            "{error}"
        );
        let mut request = options(Some("SELECT COUNT(*) FROM data"));
        request.sheets = Some(vec!["Satu".into()]);
        assert_eq!(result("xlsx", &bytes, request)["hasil"], json!([[999]]));
    }

    #[test]
    fn reserves_the_first_wide_row_and_notifies_how_many_profiles_were_omitted() {
        let mut book = Workbook::new();
        let sheet = book.add_worksheet();
        for col in 0..1_000 {
            sheet.write(0, col, "Judul").unwrap();
            sheet.write(1, col, "Teks uji sepanjang 26 byte").unwrap();
        }
        let bytes = book.save_to_buffer().unwrap();
        let output = calculate(
            "xlsx",
            &bytes,
            options(Some("SELECT * FROM data LIMIT 1")),
            false,
        )
        .unwrap();
        assert!(output.len() <= MAX_OUTPUT);
        let result: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(result["hasil"].as_array().unwrap().len(), 1);
        assert_eq!(result["hasil"][0].as_array().unwrap().len(), 1001);
        let profile = &result["profil_kolom"];
        let omitted = profile["kolom_terpotong"].as_u64().unwrap() as usize;
        assert!(omitted > 0);
        assert_eq!(profile["kolom"].as_array().unwrap().len() + omitted, 1000);
        assert!(profile["catatan"].is_string());
    }

    #[test]
    fn uploaded_xlsx_keeps_coordinates_and_uses_cached_values() {
        let mut book = Workbook::new();
        let sheet = book.add_worksheet().set_name("Data").unwrap();
        sheet.write(0, 0, "Jenis").unwrap();
        sheet.write(0, 1, "Nilai").unwrap();
        sheet.write(0, 2, "Nilai").unwrap();
        sheet.write(1, 0, "uji").unwrap();
        sheet.write(1, 1, " \t ").unwrap();
        sheet.write(1, 2, 2).unwrap();
        sheet.write(3, 0, "uji").unwrap();
        sheet.write(3, 1, 8).unwrap();
        sheet
            .write_formula(3, 2, Formula::new("=1+2").set_result("3"))
            .unwrap();
        sheet.write(3, 3, 7).unwrap();
        let bytes = book.save_to_buffer().unwrap();
        let info = result("xlsx", &bytes, detail("Data"));
        assert_eq!(info["file"], "Contoh.xlsx");
        assert_eq!(info["sheet"], "Data");
        assert_eq!(info["baris_data"], 3);
        assert_eq!(
            info["kolom"],
            json!([
                ["A", "Jenis", 0, 2, 0, 0, 0, 1, 0, 0],
                ["B", "Nilai", 1, 0, 0, 0, 0, 2, 0, 0],
                ["C", "Nilai", 2, 0, 0, 0, 0, 1, 0, 0],
                ["D", "", 1, 0, 0, 0, 0, 2, 0, 0]
            ])
        );
        let sql = "WITH s AS (SELECT * FROM data WHERE A IS NOT NULL)
                   SELECT A, SUM(C), COUNT(*)-COUNT(B), MAX(baris) FROM s GROUP BY A";
        assert_eq!(
            result("xlsx", &bytes, options(Some(sql)))["hasil"],
            json!([["uji", 5, 1, 4]])
        );
        assert_eq!(
            result(
                "xlsx",
                &bytes,
                options(Some("SELECT count(*) FROM data WHERE B IS NULL"))
            )["hasil"],
            json!([[2]])
        );
        let mut missing = options(None);
        missing.sheet = Some("Tidak ada".into());
        assert!(
            calculate("xlsx", &bytes, missing, false)
                .unwrap_err()
                .contains("Sheet tidak ditemukan")
        );
    }

    #[test]
    fn counts_cached_xls_cells() {
        let bytes = super::super::xls_fixture(3, 2);
        let sql = "SELECT baris, A FROM data";
        assert_eq!(
            result("xls", &bytes, options(Some(sql)))["hasil"],
            json!([[2, 5]])
        );
    }

    #[test]
    fn edited_workbook_leaves_pending_formulas_out_of_aggregates() {
        let spec = json!({
            "filename": "Buatan",
            "sheets": [{"name":"Data", "columns":[{"header":"Nilai"}], "rows":[
                [4], [{"formula":"=A2+1", "value":99}], ["=A2+2"]
            ]}]
        });
        let spec = serde_json::from_value(spec).unwrap();
        let sql = "SELECT SUM(A), COUNT(A), COUNT(*) FROM data";
        let mut request = options(Some(sql));
        request.pending = true;
        let original = crate::files::build_xlsx(&spec).unwrap();
        let edited = crate::files::build_xlsx_with_recalculation(&spec, true).unwrap();
        let pending = result("xlsx", &original, request);
        assert_eq!(
            pending["profil_kolom"]["kolom"],
            json!([["A", 1, 0, 0, 0, 0, 0, 2, 1]])
        );
        let reuploaded = result("xlsx", &edited, options(Some(sql)));
        assert_eq!(
            reuploaded["profil_kolom"]["kolom"],
            json!([["A", 1, 0, 0, 0, 0, 2, 0, 2]])
        );
        for page in [pending, reuploaded] {
            assert_eq!(page["hasil"], json!([[4, 1, 3]]));
            assert!(
                page["catatan_rumus"]
                    .as_str()
                    .unwrap()
                    .contains("hitung ulang")
            );
        }
        assert_eq!(
            result("xlsx", &original, options(Some(sql)))["hasil"],
            json!([[103, 2, 3]])
        );
    }

    #[test]
    fn reports_non_selected_sheets_compactly_and_supports_wide_and_tall_sheets() {
        let mut book = Workbook::new();
        for i in 0..50 {
            let sheet = book.add_worksheet().set_name(format!("Data {i}")).unwrap();
            for col in 0..if i == 49 { 1_000 } else { 614 } {
                sheet.write(0, col, format!("Kolom {col}")).unwrap();
            }
        }
        book.worksheet_from_index(0)
            .unwrap()
            .write(1, 613, 9)
            .unwrap();
        book.worksheet_from_index(49)
            .unwrap()
            .write(1, 999, 7)
            .unwrap();
        let bytes = book.save_to_buffer().unwrap();
        let summary = calculate("xlsx", &bytes, options(None), false).unwrap();
        assert!(summary.len() < 28_000);
        let info: Value = serde_json::from_str(&summary).unwrap();
        assert_eq!(info["sheets"].as_array().unwrap().len(), 50);
        assert_eq!(info["sheets"][0], json!(["Data 0", 2, 614]));
        assert_eq!(info["sheets"][49], json!(["Data 49", 2, 1000]));
        assert!(info["catatan_batas"].is_null());
        let detail = result("xlsx", &bytes, detail("Data 0"));
        assert_eq!(detail["kolom"].as_array().unwrap().len(), 614);
        let mut wide = options(None);
        wide.sheet = Some("Data 49".into());
        wide.sql = Some(format!("SELECT SUM(\"{}\") FROM data", column_letter(999)));
        assert_eq!(result("xlsx", &bytes, wide)["hasil"], json!([[7]]));

        let mut book = Workbook::new();
        let sheet = book.add_worksheet();
        sheet.write(0, 0, "Nilai").unwrap();
        sheet.write(100_000, 0, 11).unwrap();
        let bytes = book.save_to_buffer().unwrap();
        let tall = result(
            "xlsx",
            &bytes,
            options(Some("SELECT COUNT(*), MAX(baris), SUM(A) FROM data")),
        );
        assert_eq!(tall["hasil"], json!([[100_000, 100_001, 11]]));
    }

    fn query_fixture(sql: &str, budget: usize) -> Result<String, String> {
        let sheet = TableSheet {
            name: "Data",
            rows: 1..206,
            columns: 1,
            headings: Vec::new(),
            cells: (1..206)
                .map(|row| TypedCell::number(SqlValue::Integer(row)))
                .collect(),
        };
        query(
            json!({ "file": "Uji", "sheet": "Data", "baris_data": 205 }),
            sql,
            &[sheet],
            false,
            budget,
            false,
            None,
            false,
        )
    }

    #[test]
    fn accepts_one_select_with_comments_but_denies_other_sql() {
        let allowed =
            query_fixture("/* ; */ SELECT A FROM data LIMIT 1; -- selesai", MAX_OUTPUT).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&allowed).unwrap()["hasil"],
            json!([[1]])
        );
        for sql in [
            "SELECT A FROM data; SELECT 2",
            "SELECT A FROM data; DELETE FROM data",
            "ATTACH DATABASE 'uji.sqlite' AS x",
            "PRAGMA user_version",
            "SELECT * FROM pragma_database_list",
            "UPDATE data SET A=0",
            "WITH s AS (SELECT 1) DELETE FROM data",
            "DROP TABLE data",
            "SELECT * FROM sqlite_schema",
            "SELECT count(*) FROM SQLITE_MASTER",
            "SELECT readfile('uji.sqlite')",
            "SELECT writefile('uji.sqlite','uji')",
            "SELECT load_extension('uji')",
            "SELECT randomblob(2000000000)",
            "SELECT zeroblob(2000000000)",
            "SELECT printf('%2000000000s','uji')",
            "VALUES (1)",
            "EXPLAIN SELECT A FROM data",
            "SELECT \"MISSING\" FROM data",
            "SELECT FROM data",
        ] {
            assert!(query_fixture(sql, MAX_OUTPUT).is_err(), "{sql}");
        }
        assert_eq!(
            single_select("SELECT 'a'';b' AS [x;y]; /* ; */").unwrap(),
            "SELECT 'a'';b' AS [x;y]"
        );
        assert!(
            single_select("SELECT 1; /* ; */ PRAGMA user_version")
                .unwrap_err()
                .contains("satu SELECT")
        );
        assert!(single_select("SELECT 1;;").is_err());
    }

    #[test]
    fn interrupts_unbounded_recursion_and_limits_aggregate_values() {
        let sql =
            "WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n FROM r) SELECT COUNT(*) FROM r";
        let error = query_fixture(sql, MAX_OUTPUT).unwrap_err();
        assert!(error.contains("batas langkah"), "{error}");
        let sql = "WITH RECURSIVE r(n,s) AS (SELECT 1,'0123456789' UNION ALL
                   SELECT n+1,s||s FROM r WHERE n<12)
                   SELECT group_concat(s) FROM r CROSS JOIN data";
        let error = query_fixture(sql, MAX_OUTPUT).unwrap_err();
        assert!(error.contains("batas ukuran"), "{error}");
    }

    #[test]
    fn names_the_sheet_limit_that_was_exceeded() {
        for (rows, columns, expected) in [
            (100_002, 1, "100.000 baris"),
            (1, 1_001, "1.000 kolom"),
            (2_001, 1_000, "2 juta sel"),
        ] {
            let error = Bounds {
                last_row: rows,
                columns,
            }
            .validate(rows.saturating_sub(1))
            .unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn numeric_aggregates_reject_text_dates_logic_and_excel_errors() {
        use rust_xlsxwriter::{ExcelDateTime, Format};
        let mut book = Workbook::new();
        let sheet = book.add_worksheet();
        sheet.write(0, 0, "Campuran").unwrap();
        sheet.write(1, 0, 10).unwrap();
        sheet
            .write_formula(2, 0, Formula::new("=1/0").set_result("#DIV/0!"))
            .unwrap();
        sheet.write_string(3, 0, "1.250,50").unwrap();
        let date_format = Format::new().set_num_format("yyyy-mm-dd");
        let date = ExcelDateTime::parse_from_str("2026-01-18").unwrap();
        sheet
            .write_datetime_with_format(4, 0, &date, &date_format)
            .unwrap();
        sheet
            .write_datetime_with_format(5, 0, &date, &date_format)
            .unwrap();
        sheet.write(6, 0, true).unwrap();
        sheet.write(7, 0, "#DIV/0!").unwrap();
        let bytes = book.save_to_buffer().unwrap();
        let summary = result("xlsx", &bytes, detail("Sheet1"));
        assert_eq!(
            summary["kolom"][0],
            json!(["A", "Campuran", 1, 2, 2, 1, 1, 0, 0, 0])
        );
        for sql in [
            "SELECT AVG(A) FROM data WHERE baris <= 3",
            "SELECT SUM(A) FROM data WHERE baris IN (2, 4)",
            "SELECT SUM(A) FROM data WHERE baris IN (5, 6)",
            "SELECT TOTAL(A) OVER (ORDER BY baris) FROM data",
        ] {
            let error = calculate("xlsx", &bytes, options(Some(sql)), false).unwrap_err();
            assert!(
                error.contains("nilai bukan angka")
                    && error.contains("A (6)")
                    && error.contains("typeof"),
                "{error}"
            );
        }
        let filtered = result(
            "xlsx",
            &bytes,
            options(Some(
                "SELECT SUM(A), AVG(A), TOTAL(A) FROM data WHERE typeof(A) IN ('integer','real')",
            )),
        );
        assert_eq!(filtered["hasil"], json!([[10, 10.0, 10.0]]));
        let types = result(
            "xlsx",
            &bytes,
            options(Some(
                "SELECT typeof(A), A FROM data WHERE baris IN (3, 8) ORDER BY baris",
            )),
        );
        assert_eq!(
            types["hasil"],
            json!([["blob", {"galat_excel":"#DIV/0!"}],["text","#DIV/0!"]])
        );
        let extrema = result(
            "xlsx",
            &bytes,
            options(Some("SELECT MIN(A), MAX(A) FROM data")),
        );
        assert_eq!(extrema["catatan_min_max"]["kolom_bercampur"], json!(["A"]));
        let coerced = result(
            "xlsx",
            &bytes,
            options(Some(
                "SELECT AVG(A*1), SUM(A+0) FROM data WHERE baris IN (2, 4)",
            )),
        );
        assert_eq!(coerced["hasil"], json!([[5.625, 11.25]]));
        assert_eq!(
            coerced["profil_kolom"]["kolom"],
            json!([["A", 1, 2, 2, 1, 1, 0, 0, 0]])
        );
        assert!(
            coerced["profil_kolom"]["catatan"]
                .as_str()
                .unwrap()
                .contains("CASE WHEN typeof(A)")
        );
        assert!(
            result("xlsx", &bytes, options(Some("SELECT COUNT(*) FROM data")))["profil_kolom"]
                .is_null()
        );
    }

    #[test]
    fn compares_boolean_cells_as_text_and_leaves_cached_empty_strings_empty() {
        let mut book = Workbook::new();
        let sheet = book.add_worksheet();
        sheet.write(0, 0, "Aktif").unwrap();
        sheet.write(0, 1, "Opsional").unwrap();
        sheet.write(1, 0, true).unwrap();
        sheet.write(2, 0, false).unwrap();
        sheet.set_formula_result_default("");
        sheet.write_formula(1, 1, Formula::new("=\"\"")).unwrap();
        let bytes = book.save_to_buffer().unwrap();
        let summary = result("xlsx", &bytes, detail("Sheet1"));
        assert!(
            summary["catatan_logika"]
                .as_str()
                .unwrap()
                .contains("A = 'TRUE'")
        );
        assert_eq!(
            summary["kolom"][1],
            json!(["B", "Opsional", 0, 0, 0, 0, 0, 2, 0, 1])
        );
        let sql = "SELECT CASE WHEN A = 'TRUE' THEN 1 ELSE 0 END, B FROM data ORDER BY baris";
        let result = result("xlsx", &bytes, options(Some(sql)));
        assert_eq!(result["hasil"], json!([[1, null], [0, null]]));
        assert_eq!(
            result["profil_kolom"]["kolom"],
            json!([["A", 0, 0, 0, 2, 0, 0, 0, 0], ["B", 0, 0, 0, 0, 0, 2, 0, 1]])
        );
    }

    #[test]
    fn ignores_empty_explicit_rows_and_reports_merges_with_custom_header() {
        let mut book = Workbook::new();
        let sheet = book.add_worksheet();
        sheet.write(0, 0, "Pengantar").unwrap();
        sheet
            .merge_range(
                1,
                0,
                1,
                1,
                &"judul panjang ".repeat(10),
                &rust_xlsxwriter::Format::new(),
            )
            .unwrap();
        sheet.write(2, 0, 4).unwrap();
        // Baris kosong yang hanya diberi tinggi tidak memperpanjang rentang data.
        sheet.set_row_height(5, 21).unwrap();
        let bytes = book.save_to_buffer().unwrap();
        let mut request = detail("Sheet1");
        request.header_row = 2;
        let info = result("xlsx", &bytes, request);
        assert_eq!(info["rentang_baris"], json!([3, 3]));
        // Tidak ada sel kosong di rentang data, karena baris 4 sampai 6 tidak ikut.
        assert_eq!(info["kolom"][0][7], 0);
        assert!(info["kolom"][0][1].as_str().unwrap().ends_with('…'));
        assert_eq!(info["kolom"][1][1], "");
        assert_eq!(info["rentang_sel_gabungan"], 1);
        let mut request = options(Some("SELECT MIN(baris), MAX(baris) FROM data"));
        request.header_row = 0;
        let all = result("xlsx", &bytes, request);
        assert_eq!(all["rentang_baris"], json!([1, 3]));
        assert_eq!(all["hasil"], json!([[1, 3]]));
    }

    #[tokio::test]
    async fn lists_only_worksheets_in_xlsx_with_chart_sheet_first() {
        use rust_xlsxwriter::{Chart, ChartType};
        let mut book = Workbook::new();
        let mut chart = Chart::new(ChartType::Column);
        chart.add_series().set_values(("Data", 1, 0, 1, 0));
        let chartsheet = book.add_chartsheet().set_name("Grafik").unwrap();
        chartsheet.insert_chart(0, 0, &chart).unwrap();
        let data = book.add_worksheet().set_name("Data").unwrap();
        data.write(0, 0, "Nilai").unwrap();
        data.write(1, 0, 5).unwrap();
        let bytes = book.save_to_buffer().unwrap();
        let uploaded = super::super::read_content("xlsx", "Grafik", &bytes)
            .await
            .unwrap();
        assert_eq!(uploaded["sheets"][0]["name"], "Data");
        let summary = result("xlsx", &bytes, options(None));
        assert_eq!(summary["sheets"], json!([["Data", 2, 1]]));
        let mut request = options(None);
        request.sheet = Some("Grafik".into());
        assert!(
            calculate("xlsx", &bytes, request, false)
                .unwrap_err()
                .contains("Sheet tidak ditemukan")
        );
    }

    #[test]
    fn keeps_exact_integers_from_xml_and_matches_generated_download() {
        let mut book = Workbook::new();
        let sheet = book.add_worksheet();
        sheet.write(0, 0, "Angka").unwrap();
        sheet
            .write_formula(1, 0, Formula::new("=1").set_result("9.007199254740993e15"))
            .unwrap();
        sheet.write(2, 0, 1).unwrap();
        let bytes = book.save_to_buffer().unwrap();
        let sql = "SELECT SUM(A) FROM data";
        assert_eq!(
            result("xlsx", &bytes, options(Some(sql)))["hasil"],
            json!([[9_007_199_254_740_994_i64]])
        );
        let spec = serde_json::from_value(json!({
            "filename":"Tepat", "sheets":[{"name":"Data", "columns":[{"header":"Angka"}],
            "rows":[[9_007_199_254_740_993_i64]]}]
        }))
        .unwrap();
        let bytes = crate::files::build_xlsx(&spec).unwrap();
        assert_eq!(
            result("xlsx", &bytes, options(Some(sql)))["hasil"],
            json!([[9_007_199_254_740_992_i64]])
        );
    }

    #[test]
    fn clips_whole_rows_at_result_and_byte_limits() {
        let sql = "SELECT baris, A FROM data ORDER BY baris";
        let full = query_fixture(sql, MAX_OUTPUT).unwrap();
        let full: Value = serde_json::from_str(&full).unwrap();
        assert_eq!(full["hasil"].as_array().unwrap().len(), MAX_RESULT_ROWS);
        assert!(
            full["catatan_batas"]
                .as_str()
                .unwrap()
                .contains("terpotong")
        );
        let short = query_fixture("SELECT baris, 'uji\\t\\n😀' FROM data", 350).unwrap();
        assert!(short.len() <= 350);
        let short: Value = serde_json::from_str(&short).unwrap();
        assert!(!short["hasil"].as_array().unwrap().is_empty());
        assert!(short["catatan_batas"].is_string());
    }

    #[test]
    fn distinguishes_a_row_too_large_to_return_from_no_rows() {
        let sql = format!("SELECT '{}' AS teks FROM data", "uji".repeat(350));
        let error = query_fixture(&sql, 350).unwrap_err();
        assert!(
            error.contains("Anggaran") && error.contains("satu baris"),
            "{error}"
        );
        let empty = query_fixture(&format!("{sql} WHERE 0"), 350).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&empty).unwrap()["hasil"],
            json!([])
        );
    }

    #[test]
    #[ignore]
    fn realistic_xlsx_stay_fast() {
        let mut book = Workbook::new();
        let sheet = book.add_worksheet();
        for col in 0..47 {
            sheet.write(0, col, format!("Kolom {col}")).unwrap();
            for row in 1..=7_000 {
                sheet.write(row, col, row + u32::from(col)).unwrap();
            }
        }
        let bytes = book.save_to_buffer().unwrap();
        let start = std::time::Instant::now();
        let page = result(
            "xlsx",
            &bytes,
            options(Some("SELECT COUNT(*), SUM(AU) FROM data")),
        );
        assert_eq!(page["hasil"], json!([[7_000, 24_825_500]]));
        eprintln!(
            "Hitung 7.000 × 47 dari XLSX sintetis: {:?}",
            start.elapsed()
        );
    }
}
