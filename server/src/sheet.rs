//! Spreadsheet jadi teks untuk index. Setiap baris data selalu ditulis utuh dan sejajar
//! dengan kolomnya; label dan kepala kolom hanya menambah konteks. Kalau ragu, tabel
//! digabung, bukan dipecah, dan lebih baik tanpa kepala kolom daripada kepala yang salah.
//!
//! - Baris kosong memisahkan bagian. Kolom kosong memisahkan tabel yang berdampingan hanya
//!   kalau kedua sisinya punya label sendiri atau kepala kolom yang sama.
//! - Baris satu sel berhuruf di kolom pertama sebuah tabel adalah label bagian. Tabel diberi
//!   judul `## Sheet › label › …`. Label yang terpotong atau tidak muat di judul tetap ditulis
//!   utuh di isi.
//! - Kepala kolom hanya dipakai kalau terbukti: baris pertama tabel mirip kepala dan baris
//!   sesudahnya berisi angka di kolom berkepala. Kepala ditulis sebagai tabel markdown, dan
//!   `chunk_text` mengulangnya di setiap potongan lanjutan.
//! - Tabel yang terlalu lebar dan baris yang terlalu panjang ditulis sebagai "kolom: nilai",
//!   dengan identitas baris di setiap potongannya.

use std::collections::HashMap;

/// Panjang maksimal nama sheet dan satu label di judul.
const MAX_LABEL_CHARS: usize = 60;
/// Judul tetap di bawah batas 200 karakter `chunk_text`; label teratas dibuang dulu.
const MAX_HEADING_CHARS: usize = 190;
/// Kepala tabel lebih panjang dari ini tidak diulang `chunk_text` (batasnya 600).
const MAX_HEADER_CHARS: usize = 500;
/// Baris tabel lebih panjang dari ini ditulis per sel, supaya tidak terbelah di tengah.
const MAX_ROW_CHARS: usize = 1_000;
/// Panjang potongan isi sel yang sangat panjang.
const PIECE_CHARS: usize = 800;
/// Panjang identitas baris yang diulang di depan setiap sel baris panjang.
const MAX_ID_CHARS: usize = 120;
/// Baris kepala kolom bertingkat yang digabung paling banyak sekian.
const MAX_HEADER_ROWS: usize = 3;
/// Teks pendek sendirian di atas tabel (dipisah baris kosong) yang dipakai sebagai label.
const MAX_STANDALONE_LABELS: usize = 3;

pub struct Sheet<'a> {
    pub name: String,
    pub hidden: bool,
    pub cells: &'a [Vec<String>],
}

/// Teks semua sheet, dan apakah dipotong karena melewati `max_chars` karakter.
pub fn to_text(sheets: &[Sheet], max_chars: usize) -> (String, bool) {
    let mut out = Out {
        text: String::new(),
        chars: 0,
        max: max_chars,
        full: false,
    };
    for sheet in sheets {
        let grid: Vec<Vec<String>> = sheet
            .cells
            .iter()
            .map(|row| row.iter().map(|c| clean_cell(c)).collect())
            .collect();
        if grid.iter().all(|row| row.iter().all(String::is_empty)) {
            continue;
        }
        let mut name = clip(&clean_cell(&sheet.name), MAX_LABEL_CHARS);
        if sheet.hidden {
            name.push_str(" (tersembunyi)");
        }
        out.push(&format!("## {name}\n\n"));
        let mut w = SheetWriter {
            out: &mut out,
            sheet: &name,
            last: HashMap::new(),
            pending: HashMap::new(),
            pending_region: HashMap::new(),
            region: 0,
            wrote_table: false,
            leading: 0,
            uncertain: false,
            several: false,
            loose: false,
        };
        for (r0, r1) in regions(&grid) {
            w.region += 1;
            w.wrote_table = false;
            let rows_of = |ra: usize, rb: usize, c0: usize, c1: usize| -> Vec<Vec<&str>> {
                grid[ra..rb]
                    .iter()
                    .map(|row| (c0..c1).map(|c| cell(row, c)).collect::<Vec<_>>())
                    .filter(|row| filled(row) > 0)
                    .collect()
            };
            let slice = |c0: usize, c1: usize| rows_of(r0, r1, c0, c1);
            // Blok yang muat di dalam jejak kolom tabel di atasnya (data jarang, record teks,
            // kolom kosong di tepi atau di tengah) dibaca dengan rentang tabel itu, dan beberapa
            // blok di jejak yang sama digabung jadi satu, supaya baris tetap utuh dan pengaman
            // tabel lanjutan berlaku. Pengecualian: label periode sejenis di kolom pertama
            // tabel itu ("FY 2025"), yang memulai periode berikutnya.
            let footprints: Vec<(usize, usize)> =
                w.last.iter().map(|(&s, ctx)| (s, s + ctx.width)).collect();
            let found = blocks(&grid, r0, r1, &footprints);
            let target = |&(c0, c1): &(usize, usize)| -> (usize, usize) {
                let Some((s, e, last)) = w.last.iter().find_map(|(&s, ctx)| {
                    let e = s + ctx.width;
                    (s <= c0 && c1 <= e).then(|| (s, e, ctx.labels.last().cloned()))
                }) else {
                    return (c0, c1);
                };
                let rows = slice(c0, c1);
                let period = c0 == s
                    && rows
                        .iter()
                        .all(|r| is_text_line(r) && r.first().is_some_and(|v| !v.is_empty()))
                    && rows
                        .iter()
                        .any(|r| last.as_ref().is_some_and(|l| shape(l) == shape(first(r))));
                if period { (c0, c1) } else { (s, e) }
            };
            let targets: Vec<(usize, usize)> = found.iter().map(target).collect();
            let mut merged: Vec<(usize, usize)> = Vec::new();
            for (i, &t) in targets.iter().enumerate() {
                // Rentang yang dilebarkan tidak boleh menimpa blok lain yang tidak ikut
                // dilebarkan (misalnya label periode di kolom pertama).
                let clash = t != found[i]
                    && found.iter().zip(&targets).any(|(&(a, b), &u)| {
                        u == (a, b) && (a, b) != found[i] && a < t.1 && t.0 < b
                    });
                let t = if clash { found[i] } else { t };
                if merged.last() == Some(&t) {
                    continue;
                }
                merged.push(t);
            }
            // Periode baru di salah satu tabel dicatat dulu untuk semua tabel di rentang ini,
            // supaya urutan penulisan (kiri dulu) tidak menentukan hasilnya.
            for &(start, end) in &merged {
                w.note_period(start, &slice(start, end));
            }
            // Baris label periode di kolom pertama setiap blok. Label periode yang muncul di
            // satu tabel saja (tabel berdampingan tanpa baris kosong di antaranya) memotong
            // tabel lain di baris itu: bagian sesudahnya ditulis sebagai lanjutan tanpa periode
            // lamanya.
            let rows_where = |pred: fn(&[&str]) -> bool| -> Vec<Vec<usize>> {
                merged
                    .iter()
                    .map(|&(a, b)| {
                        (r0..r1)
                            .filter(|&r| {
                                let row: Vec<&str> = (a..b).map(|c| cell(&grid[r], c)).collect();
                                pred(&row)
                            })
                            .collect()
                    })
                    .collect()
            };
            let period_rows = rows_where(|row| is_label(row) && may_be_period(first(row)));
            // Tabel hanya dipotong kalau sudah ada data sebelum baris itu: label di awal blok
            // ("FY 2025" lalu "Intake") tetap satu rangkaian label. Label sendiri di baris
            // itu ("Shift 2" di samping "FY 2025") tidak mencegah potongan, jadi lanjutannya
            // tetap membuang periode lama.
            let label_rows = rows_where(is_label);
            // Bentuk label di kolom pertama blok `i` pada baris `r`.
            let label_shape = |i: usize, r: usize| {
                let (a, b) = merged[i];
                let row: Vec<&str> = (a..b).map(|c| cell(&grid[r], c)).collect();
                is_label(&row).then(|| shape(&clip(first(&row), MAX_LABEL_CHARS)))
            };
            for (i, &(start, end)) in merged.iter().enumerate() {
                let uncertain = one_sided_label(&grid, r0, r1, start, end);
                let (tables, _, _, staggered) = table_runs(&grid, r0, r1, start, end);
                let several = tables >= 2;
                let used = |r: usize| (start..end).any(|c| !cell(&grid[r], c).is_empty());
                // Baris data (bukan label) pertama dan baris terisi terakhir blok ini, dihitung
                // sekali, supaya setiap calon potongan cukup dibandingkan dengan dua angka.
                let labels_here: std::collections::HashSet<usize> =
                    label_rows[i].iter().copied().collect();
                let first_data = (r0..r1)
                    .find(|&x| used(x) && !labels_here.contains(&x))
                    .unwrap_or(r1);
                let last_used = (r0..r1).rev().find(|&x| used(x));
                // Periode sejenis di kedua tabel pada baris yang sama ("FY 2025" di atas
                // keduanya) tidak memotong; masing-masing mengganti periodenya sendiri.
                let mut cuts: Vec<usize> = period_rows
                    .iter()
                    .enumerate()
                    .filter(|&(j, _)| j != i)
                    .flat_map(|(j, rs)| rs.iter().map(move |&r| (j, r)))
                    .filter(|&(j, r)| {
                        let same = label_shape(i, r).is_some_and(|s| Some(s) == label_shape(j, r));
                        !same && first_data < r && last_used.is_some_and(|l| l >= r)
                    })
                    .map(|(_, r)| r)
                    .collect();
                cuts.sort_unstable();
                cuts.dedup();
                let mut bounds = vec![r0];
                bounds.extend(cuts);
                bounds.push(r1);
                for (k, span) in bounds.windows(2).enumerate() {
                    let rows = rows_of(span[0], span[1], start, end);
                    if rows.is_empty() {
                        continue;
                    }
                    if k > 0 {
                        w.mark_stale(start);
                    }
                    w.uncertain = uncertain;
                    w.several = several;
                    w.loose = staggered;
                    w.block(start, &rows);
                    if w.out.full {
                        return (out.text, true);
                    }
                }
            }
            // Judul tertunda dari region sebelumnya yang tidak dipakai tabel di region ini
            // kedaluwarsa: judul di A1, tabel di C3, lalu formulir di kolom A tidak memakainya.
            if w.wrote_table {
                let region = w.region;
                w.pending_region.retain(|_, r| *r == region);
                let live: Vec<usize> = w.pending_region.keys().copied().collect();
                w.pending.retain(|c, _| live.contains(c));
            }
        }
    }
    let full = out.full;
    (out.text, full)
}

/// Teks keluaran dengan batas jumlah karakter (bukan byte).
struct Out {
    text: String,
    chars: usize,
    max: usize,
    full: bool,
}

impl Out {
    fn push(&mut self, s: &str) {
        if self.full {
            return;
        }
        self.chars += s.chars().count();
        self.text.push_str(s);
        if self.chars >= self.max {
            self.full = true;
        }
    }
}

/// Spasi dan baris baru di dalam sel jadi satu spasi. Backslash lalu `|` di-escape supaya
/// isi sel tidak memecah tabel markdown.
fn clean_cell(cell: &str) -> String {
    cell.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('\\', "\\\\")
        .replace('|', "\\|")
}

fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", text[..i].trim_end()),
        None => text.to_string(),
    }
}

/// Potongan teks paling panjang `max` karakter, sebisa mungkin di spasi.
fn pieces(text: &str, max: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some((cut, _)) = rest.char_indices().nth(max) {
        let at = match rest[..cut].rfind(' ') {
            Some(i) if i >= cut / 2 => i,
            _ => cut,
        };
        out.push(rest[..at].trim_end());
        rest = rest[at..].trim_start();
    }
    if !rest.is_empty() {
        out.push(rest);
    }
    out
}

fn cell(row: &[String], c: usize) -> &str {
    row.get(c).map_or("", String::as_str)
}

fn filled(row: &[&str]) -> usize {
    row.iter().filter(|c| !c.is_empty()).count()
}

fn first<'a>(row: &[&'a str]) -> &'a str {
    row.iter().find(|c| !c.is_empty()).copied().unwrap_or("")
}

fn has_letters(cell: &str) -> bool {
    cell.chars().any(char::is_alphabetic)
}

fn is_number(cell: &str) -> bool {
    let t: String = cell
        .chars()
        .filter(|c| !matches!(c, ',' | ' ' | '%' | '\u{a0}'))
        .collect();
    !t.is_empty() && t.parse::<f64>().is_ok()
}

fn is_year(cell: &str) -> bool {
    cell.len() == 4
        && cell
            .parse::<u32>()
            .is_ok_and(|y| (1900..=2100).contains(&y))
}

/// Satu sel teks (berhuruf, bukan angka) di mana pun posisinya, atau pembatas tahun ("2025")
/// di kolom pertama.
fn is_text_line(row: &[&str]) -> bool {
    let c = first(row);
    filled(row) == 1
        && ((has_letters(c) && !is_number(c) && !is_blankish(c))
            || row.first().is_some_and(|v| is_year(v.trim())))
}

/// Label bagian: satu sel teks di kolom pertama tabel.
fn is_label(row: &[&str]) -> bool {
    is_text_line(row) && row.first().is_some_and(|c| !c.is_empty())
}

/// Baris yang mirip kepala kolom: minimal dua sel terisi dan tanpa angka, kecuali tahun kalau
/// jelas kepala (dua tahun atau lebih, atau dua sel teks atau lebih). `Apr | 2025` bukan.
fn is_header_start(row: &[&str]) -> bool {
    let cells: Vec<&str> = row.iter().copied().filter(|c| !c.is_empty()).collect();
    let text = cells.iter().filter(|c| has_letters(c)).count();
    let years = cells.iter().filter(|c| is_year(c)).count();
    cells.len() >= 2
        && text >= 1
        && cells
            .iter()
            .all(|c| (!is_number(c) || is_year(c)) && !is_blankish(c) && !is_date_like(c))
        && (years == 0 || years >= 2 || text >= 2)
}

/// Tanggal lengkap dengan hari (`17 Nov 2026`, `17/11/2026`, `2026-11-17`) adalah nilai, bukan
/// nama kolom. Bulan dan tahun saja (`Mar 2026`, `Jan-26`) tetap boleh jadi nama kolom.
fn is_date_like(cell: &str) -> bool {
    const MONTHS: [&str; 33] = [
        "jan", "feb", "mar", "apr", "may", "mei", "jun", "jul", "aug", "agu", "agt", "sep", "oct",
        "okt", "nov", "dec", "des", "january", "januari", "february", "februari", "march", "maret",
        "april", "june", "juni", "july", "juli", "august", "agustus", "october", "oktober",
        "desember",
    ];
    let lower = cell.to_lowercase();
    let numbers = lower
        .split(|c: char| !c.is_ascii_digit())
        .filter(|w| !w.is_empty())
        .count();
    // Juga tanggal dengan jam: "2026-11-17 07:00", "2026-11-17T07:00".
    let numeric_date = lower.split([' ', 't']).any(|tok| {
        tok.split(|c: char| !c.is_ascii_digit())
            .filter(|w| !w.is_empty())
            .count()
            == 3
            && tok
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '/' | '-' | '.'))
    });
    let named_date = numbers >= 2
        && lower
            .split(|c: char| !c.is_alphabetic())
            .any(|w| MONTHS.contains(&w) || w == "september" || w == "november" || w == "december");
    numeric_date || named_date
}

/// Penanda nilai kosong di data (`N/A`, `-`); baris yang memuatnya bukan kepala kolom.
fn is_blankish(cell: &str) -> bool {
    matches!(
        cell.to_lowercase().as_str(),
        "-" | "–" | "—" | "n/a" | "na" | "#n/a" | "nil" | "none" | "tbd"
    )
}

/// Nama bulan saja (`Apr`, `April`, `Mei`): nilai di kolom bulan.
fn is_month(cell: &str) -> bool {
    const MONTHS: [&str; 36] = [
        "jan",
        "feb",
        "mar",
        "apr",
        "may",
        "mei",
        "jun",
        "jul",
        "aug",
        "agu",
        "agt",
        "sep",
        "oct",
        "okt",
        "nov",
        "dec",
        "des",
        "january",
        "januari",
        "february",
        "februari",
        "march",
        "maret",
        "april",
        "june",
        "juni",
        "july",
        "juli",
        "august",
        "agustus",
        "september",
        "october",
        "oktober",
        "november",
        "december",
        "desember",
    ];
    MONTHS.contains(&cell.trim().trim_end_matches('.').to_lowercase().as_str())
}

/// Nama kolom yang umum di lembar kerja (Inggris, Indonesia, Jepang).
const COLUMN_NAMES: &[&str] = &[
    // Inggris
    "no",
    "number",
    "item",
    "items",
    "name",
    "description",
    "desc",
    "code",
    "part",
    "qty",
    "quantity",
    "unit",
    "uom",
    "price",
    "amount",
    "total",
    "subtotal",
    "date",
    "month",
    "year",
    "week",
    "day",
    "time",
    "shift",
    "line",
    "machine",
    "operator",
    "inspector",
    "customer",
    "supplier",
    "vendor",
    "type",
    "size",
    "spec",
    "specification",
    "standard",
    "result",
    "status",
    "remark",
    "remarks",
    "note",
    "notes",
    "comment",
    "comments",
    "target",
    "actual",
    "plan",
    "output",
    "input",
    "production",
    "prod",
    "hours",
    "hour",
    "stock",
    "balance",
    "weight",
    "length",
    "width",
    "thickness",
    "diameter",
    "hardness",
    "reading",
    "value",
    "min",
    "max",
    "average",
    "avg",
    "sample",
    "lot",
    "batch",
    "model",
    "material",
    "grade",
    "color",
    "location",
    "department",
    "dept",
    "category",
    "process",
    "defect",
    "reject",
    "rate",
    "cost",
    "sales",
    "priority",
    "test",
    "piece",
    "od",
    "id",
    "uts",
    "ys",
    "tolerance",
    "nominal",
    "measured",
    "judgement",
    "judgment",
    "appearance",
    "product",
    "speed",
    "pressure",
    "temperature",
    "observation",
    "finding",
    // Indonesia
    "produk",
    "observasi",
    "temuan",
    "nama",
    "barang",
    "kode",
    "jumlah",
    "jml",
    "satuan",
    "harga",
    "tanggal",
    "tgl",
    "bulan",
    "tahun",
    "minggu",
    "hari",
    "jam",
    "mesin",
    "pelanggan",
    "pemasok",
    "jenis",
    "tipe",
    "ukuran",
    "hasil",
    "keterangan",
    "ket",
    "catatan",
    "rencana",
    "aktual",
    "operasional",
    "stok",
    "saldo",
    "berat",
    "panjang",
    "lebar",
    "tebal",
    "nilai",
    "sampel",
    "bahan",
    "warna",
    "lokasi",
    "tim",
    "departemen",
    "bagian",
    "proses",
    "cacat",
    "biaya",
    "penjualan",
    "nomor",
    "kecepatan",
    "tekanan",
    "suhu",
    "temperatur",
    // Jepang
    "品番",
    "品名",
    "数量",
    "単位",
    "単価",
    "金額",
    "合計",
    "日付",
    "月",
    "年",
    "備考",
    "実績",
    "計画",
    "目標",
    "生産",
    "在庫",
    "重量",
    "寸法",
    "板厚",
    "材質",
    "工程",
    "不良",
    "担当",
    "顧客",
    "項目",
    "外径",
    "内径",
    "長さ",
    "幅",
    "厚さ",
    "速度",
    "圧力",
    "温度",
    "硬度",
];

/// Sel ini nama kolom umum: satu kata dari daftar, atau dua kata yang keduanya dari daftar
/// atau satu dari daftar dengan penanda (`Part No`, `Nama Barang`, `Order Qty`, `Qty (pcs)`).
/// Kata lain di sekitarnya membatalkan (`Machine oil`, `High priority`, `On order`).
/// Nama parameter spesifikasi (kecepatan, tekanan, suhu, ukuran) dalam bahasa Inggris,
/// Indonesia, dan Jepang.
const PARAMETERS_EN: &[&str] = &[
    "speed",
    "pressure",
    "temperature",
    "diameter",
    "thickness",
    "width",
    "length",
    "weight",
    "hardness",
    "force",
    "torque",
    "voltage",
    "current",
    "height",
    "depth",
    "roughness",
    "frequency",
    "power",
    "flow",
    "density",
    "viscosity",
    "humidity",
    "strength",
    "elongation",
    "tensile",
    "flatness",
    "straightness",
    "roundness",
    "ovality",
    "camber",
    "burr",
    "uts",
    "ys",
    "ts",
    "od",
    "id",
    "wall",
    "elong",
    "ra",
    "rz",
    "hrc",
    "hrb",
    "hv",
    "hb",
    "runout",
    "concentricity",
    "eccentricity",
    "perpendicularity",
    "parallelism",
    "squareness",
    "waviness",
    "warpage",
    "bow",
    "twist",
    "pitch",
    "angle",
    "radius",
    "chamfer",
    "gap",
    "clearance",
    "mass",
    "volume",
    "resistance",
    "conductivity",
    "moisture",
    "gloss",
    "adhesion",
    "toughness",
    "modulus",
    "impact",
];
const PARAMETERS_ID: &[&str] = &[
    "diameter",
    "tinggi",
    "kedalaman",
    "kekasaran",
    "frekuensi",
    "daya",
    "debit",
    "densitas",
    "viskositas",
    "kelembapan",
    "kecepatan",
    "tekanan",
    "suhu",
    "temperatur",
    "tebal",
    "lebar",
    "panjang",
    "berat",
    "kekerasan",
    "gaya",
    "torsi",
    "tegangan",
    "arus",
    "kuat",
    "kekuatan",
    "regangan",
    "perpanjangan",
    "kerataan",
    "kelurusan",
    "kebulatan",
    "batas",
    "luluh",
    "ketebalan",
    "kelengkungan",
    "kemiringan",
    "sudut",
    "jari",
    "jarak",
    "celah",
    "kadar",
    "kelembaban",
    "massa",
    "volume",
    "hambatan",
    "tahanan",
    "ketangguhan",
    "modulus",
    "rasio",
    "pemuluran",
];
const PARAMETERS_JA: &[&str] = &[
    "速度",
    "圧力",
    "温度",
    "外径",
    "内径",
    "長さ",
    "幅",
    "厚さ",
    "硬度",
    "重量",
    "電圧",
    "電流",
    "トルク",
    "強さ",
    "強度",
    "降伏点",
    "耐力",
    "伸び",
    "平面度",
    "真直度",
    "真円度",
    "粗さ",
    "伸び率",
    "絞り",
    "硬さ",
    "厚み",
    "板厚",
    "肉厚",
    "高さ",
    "深さ",
    "角度",
    "半径",
    "曲がり",
    "反り",
    "振れ",
    "平行度",
    "直角度",
    "同軸度",
    "質量",
    "抵抗",
    "衝撃値",
    "ピッチ",
    "隙間",
];

/// Kunci metadata formulir (lihat `is_form_meta`), huruf kecil, kata dipisah spasi.
const FORM_META: &[&str] = &[
    // Inggris
    "date",
    "shift",
    "line",
    "machine",
    "machine no",
    "operator",
    "inspector",
    "lot",
    "lot no",
    "batch",
    "batch no",
    "prepared by",
    "checked by",
    "approved by",
    "inspected by",
    "pic",
    // Indonesia
    "tanggal",
    "tgl",
    "hari tanggal",
    "lini",
    "mesin",
    "no mesin",
    "nomor mesin",
    "no lot",
    "pemeriksa",
    "dibuat oleh",
    "diperiksa oleh",
    "disetujui oleh",
    // Jepang
    "日付",
    "シフト",
    "ライン",
    "号機",
    "ロット",
    "担当",
    "担当者",
    "作業者",
    "検査員",
    "作成",
    "作成者",
    "確認",
    "承認",
];

/// Kunci spesifikasi (lihat `is_spec_key`), huruf kecil, kata dipisah spasi.
const SPEC_KEYS: &[&str] = &[
    "material",
    "grade",
    "standard",
    "spec",
    "specification",
    "part name",
    "part no",
    "part number",
    "drawing no",
    "customer",
    "model",
    "size",
    "package no",
    "heat no",
    "bahan",
    "standar",
    "spesifikasi",
    "ukuran",
    "nama part",
    "no part",
    "pelanggan",
    "材質",
    "規格",
    "品名",
    "品番",
    "図番",
    "顧客",
    "サイズ",
    "寸法",
];

/// Kolom pertama kepala yang isinya memang nama parameter ("Parameter", "Characteristic", "特性";
/// bukan "Item", yang juga kepala daftar stok):
/// baris "Cutting speed | 120 | 119" di bawahnya record, bukan formulir baru.
fn is_parameter_header(name: &str) -> bool {
    matches!(
        name.trim().to_lowercase().as_str(),
        "parameter"
            | "parameters"
            | "parameter name"
            | "characteristic"
            | "characteristics"
            | "spec"
            | "specification"
            | "spesifikasi"
            | "karakteristik"
            | "特性"
    )
}

/// Kunci baris formulir atau spesifikasi: nama kolom umum, atau parameter majemuk
/// ("Cutting speed", "Kecepatan potong", "切断速度"). Hanya untuk mengenali formulir, bukan
/// bukti kepala.
fn is_form_key(cell: &str) -> bool {
    is_column_name(cell) || is_parameter_key(cell) || is_form_meta(cell)
}

/// Kunci metadata formulir ("Date", "Shift", "Line", "Machine No", "Prepared by", "担当"):
/// jarang jadi identitas record tabel biasa. Persis kuncinya saja; "Line 2" atau "Machine A"
/// identitas record.
fn is_form_meta(cell: &str) -> bool {
    FORM_META.contains(&key_words(cell).as_str())
}

/// Kunci spesifikasi yang bukan parameter terukur ("Material", "Grade", "Part No", "材質"):
/// baris pertama lembar spesifikasi, sebelum parameternya.
fn is_spec_key(cell: &str) -> bool {
    SPEC_KEYS.contains(&key_words(cell).as_str())
}

/// Kunci baris yang dinormalkan: huruf kecil, anotasi dalam kurung dibuang, kata dipisah satu
/// spasi ("Machine No.:" jadi "machine no").
fn key_words(cell: &str) -> String {
    let mut text = String::new();
    let mut depth = 0usize;
    for c in cell.to_lowercase().chars() {
        match c {
            '(' | '[' | '（' => depth += 1,
            ')' | ']' | '）' => depth = depth.saturating_sub(1),
            _ if depth == 0 => text.push(c),
            _ => {}
        }
    }
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || matches!(c, '.' | ':' | '：' | '/' | '_' | '-' | '#'))
        .filter(|w| !w.is_empty())
        .collect();
    words.join(" ")
}

/// Nama parameter, tunggal atau majemuk ("Speed", "Cutting speed (m/min)", "Diameter luar",
/// "切断速度").
fn is_parameter_key(cell: &str) -> bool {
    // Anotasi satuan dalam kurung ("Cutting speed (m/min)", "切断速度（m/min）") dibuang dulu.
    let mut text = String::new();
    let mut depth = 0usize;
    for c in cell.to_lowercase().chars() {
        match c {
            '(' | '[' | '（' => depth += 1,
            ')' | ']' | '）' => depth = depth.saturating_sub(1),
            _ if depth == 0 => text.push(c),
            _ => {}
        }
    }
    let lower = text.trim();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    // Kata yang juga nama barang ("Angle", profil generik) hanya parameter dalam frasa
    // ("Bending angle").
    let english = words
        .last()
        .is_some_and(|w| PARAMETERS_EN.contains(w) && (words.len() >= 2 || *w != "angle"));
    let indonesian = words.first().is_some_and(|w| PARAMETERS_ID.contains(w));
    let japanese = PARAMETERS_JA.iter().any(|p| lower.ends_with(p));
    words.len() <= 4 && (english || indonesian || japanese)
}

fn is_column_name(cell: &str) -> bool {
    let mut text = String::new();
    let mut depth = 0usize;
    for c in cell.to_lowercase().chars() {
        match c {
            '(' | '[' | '（' => depth += 1,
            ')' | ']' | '）' => depth = depth.saturating_sub(1),
            _ if depth == 0 => text.push(c),
            _ => {}
        }
    }
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || matches!(c, '.' | ':' | '/' | '_' | '-' | '#'))
        .filter(|w| !w.is_empty() && !w.chars().all(|c| c.is_ascii_digit()))
        .collect();
    let known = |w: &str| COLUMN_NAMES.contains(&w);
    // Penanda yang sering menempel pada nama kolom ("Part No", "Order Qty", "No Urut").
    let marker = |w: &str| matches!(w, "no" | "nomor" | "number" | "order" | "urut" | "ke");
    match words.as_slice() {
        [w] => known(w),
        [a, b] => (known(a) && known(b)) || (known(a) && marker(b)) || (marker(a) && known(b)),
        _ => false,
    }
}

/// Bukti tambahan bahwa `names` kepala kolom, bukan record teks (`Module | On order | Urgent` di
/// atas `Bolt | 12 | Normal`), roster (`Production | Day | Ann | Pending`), atau formulir (`Material | AX100 | STD` di atas `OD | 60.5 | mm`):
/// minimal dua kolom bernama yang masing-masing terbukti. Kolom terbukti kalau isinya di
/// baris-baris data (mulai `start`, label dilewati) semuanya nama bulan atau tanggal, atau
/// namanya nama kolom umum (`Item`, `Qty`, `Tanggal`) dan isinya bukan nama kolom umum. Yang
/// terakhir menolak formulir kunci-nilai yang kuncinya nama kolom umum (`Machine`, `Shift`).
/// Satu kecocokan saja tidak cukup (`Machine oil | On order | Urgent`).
fn header_evidence(
    names: &[String],
    band: &[&Vec<&str>],
    body: &[Vec<&str>],
    start: usize,
) -> bool {
    // Baris ringkasan ("Total | 19 | pcs") adalah data, tapi kata di kolom pertamanya tidak
    // membatalkan nama kolom. Kepala yang diulang (kepala halaman) juga dilewati.
    let repeat = |r: &Vec<&str>| band.iter().any(|b| trimmed(b) == trimmed(r));
    let rows: Vec<&Vec<&str>> = body
        .iter()
        .skip(start)
        .filter(|r| !is_label(r) && !is_total_row(r) && !repeat(r))
        .take(6)
        .collect();
    // Per kolom bernama yang terbukti: (kolom angka, bukti teks). Bukti teks = nama kolom
    // umum atau kolom bulan/tanggal; nama kolom periode saja ("2000 | 2000" di atas stok) bisa
    // juga record berisi angka.
    let periods = names.iter().filter(|n| is_period_name(n)).count();
    // Tahun saja sebagai nama kolom hanya bukti kalau tahun-tahunnya deret yang masuk akal
    // (berbeda, urut, rentang paling lama 30 tahun) dan kolom identitasnya jelas kepala
    // ("Item", "Description", "Month"). "Production | 2000 | 1920" di atas jam kerja per
    // departemen bisa record.
    let years: Vec<i64> = names
        .iter()
        .filter(|n| is_year(n.trim()))
        .filter_map(|n| n.trim().parse().ok())
        .collect();
    let sequence = years.len() >= 2
        && (years.windows(2).all(|w| w[0] < w[1]) || years.windows(2).all(|w| w[0] > w[1]))
        && years
            .iter()
            .max()
            .zip(years.iter().min())
            .is_some_and(|(a, b)| a - b <= 30);
    let anchored = names.iter().any(|n| is_identity_header(n));
    let years_ok = periods >= 2 && sequence && anchored;
    let proven: Vec<(bool, bool)> = names
        .iter()
        .enumerate()
        .filter_map(|(c, name)| {
            let values: Vec<&str> = rows
                .iter()
                .filter_map(|r| r.get(c).copied())
                .filter(|v| !v.is_empty())
                .collect();
            if name.is_empty() || values.is_empty() {
                return None;
            }
            let dated = values.iter().all(|v| is_month(v) || is_date_like(v))
                && !is_month(name)
                && !is_date_like(name);
            // Kolom per periode (`2025`, `Mar 2026`, `FY 2025`) di atas angka yang bukan tahun.
            // Tahun saja sebagai nama kolom hanya bukti kalau ada kolom periode lain
            // ("Item | 2025 | 2026") dan syarat `years_ok`; "Production | 2000 | Ann" bisa
            // record jam kerja.
            let per_period = is_period_name(name)
                && (!is_year(name.trim()) || years_ok)
                && values.iter().all(|v| is_number(v) && !is_year(v));
            let named = is_column_name(name) && !values.iter().any(|v| is_column_name(v));
            // Kolom angka: ada nilai angka (hasil Review boleh campur "Pass", penanda "-", "N/A").
            let numeric = values.iter().any(|v| is_number(v));
            (dated || per_period || named).then_some((numeric, dated || named))
        })
        .collect();
    // Dua kolom terbukti, salah satunya kolom angka (nama kolom teks saja, "Production | Day"
    // di atas "Review | Night", bisa juga record roster), dan salah satunya terbukti dari teks.
    proven.len() >= 2
        && proven.iter().any(|&(numeric, _)| numeric)
        && proven.iter().any(|&(_, textual)| textual)
}

/// Nama kolom identitas yang jelas kepala, bukan isi record: "Item", "Description", "Month",
/// "Department", "Uraian", "項目". Nama departemen ("Production", "Sales") bukan.
fn is_identity_header(name: &str) -> bool {
    is_parameter_header(name)
        || matches!(
            name.trim().trim_end_matches(':').to_lowercase().as_str(),
            "item"
                | "items"
                | "item name"
                | "description"
                | "desc"
                | "name"
                | "product"
                | "product name"
                | "part"
                | "part name"
                | "part no"
                | "part number"
                | "material"
                | "category"
                | "department"
                | "dept"
                | "month"
                | "customer"
                | "supplier"
                | "account"
                | "uraian"
                | "keterangan"
                | "deskripsi"
                | "nama"
                | "nama barang"
                | "barang"
                | "produk"
                | "bahan"
                | "kategori"
                | "departemen"
                | "bagian"
                | "bulan"
                | "pelanggan"
                | "pemasok"
                | "akun"
                | "項目"
                | "品名"
                | "品番"
                | "材質"
                | "部門"
                | "月"
                | "顧客"
        )
}

/// Judul bagian yang jelas: berisi kata judul ("Production Report", "Laporan Harian",
/// "Daftar Stok", "生産日報"). Kata yang juga nama barang ("Sheet", "Form", "Log"), kata sifat
/// ("Weekly planner") atau nama pelanggan ("PT") tidak dihitung.
fn is_heading_text(text: &str) -> bool {
    const WORDS: &[&str] = &[
        "report",
        "summary",
        "schedule",
        "recap",
        "roster",
        "checklist",
        "laporan",
        "rekap",
        "rekapitulasi",
        "ringkasan",
        "daftar",
        "jadwal",
        "formulir",
        "opname",
    ];
    const JA: &[&str] = &["一覧", "報告", "集計", "月報", "日報", "週報", "報告書"];
    let lower = text.trim().to_lowercase();
    lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| WORDS.contains(&w))
        || JA.iter().any(|j| lower.contains(j))
}

/// Teks yang mirip judul, bukan identitas record: judul bagian yang jelas, panjang (lebih dari
/// empat kata), diakhiri titik dua, periode ("FY 2026", "November 2026"), atau nama bagian
/// ("Shift 1", "Gudang A", "Week 45").
fn is_title_like(text: &str) -> bool {
    const SECTIONS: &[&str] = &[
        "shift",
        "group",
        "grup",
        "regu",
        "section",
        "seksi",
        "plant",
        "pabrik",
        "gudang",
        "warehouse",
        "week",
        "minggu",
        "periode",
        "period",
    ];
    let lower = text.trim().to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    is_heading_text(text)
        || words.len() > 4
        || lower.ends_with(':')
        || is_period_name(&lower)
        || words.iter().any(|w| is_month(w) || SECTIONS.contains(w))
}

/// Baris judul bagian: judul yang jelas, sendirian atau dengan periode di sel lain
/// ("Production Report | 2026", "Laporan Operasional | November 2026").
fn is_heading_row(row: &[&str]) -> bool {
    let cells: Vec<&str> = row.iter().copied().filter(|c| !c.is_empty()).collect();
    (1..=3).contains(&cells.len())
        && is_heading_text(cells[0])
        && cells[1..]
            .iter()
            .all(|c| is_period_name(c) || is_month(c) || is_date_like(c))
}

/// Judul di atas record jarang: mirip judul dan tanpa kode huruf-angka ("A5", "M8",
/// "AX100"), atau periode ("Q1 2026"). "Daily planner A5" nama barang.
fn is_title(text: &str) -> bool {
    let code = text
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| w.chars().any(char::is_alphabetic) && w.chars().any(|c| c.is_ascii_digit()));
    is_period_name(text) || (is_title_like(text) && !code)
}

/// Jenis identitas record: bulan, tanggal, kode berangka ("Bolt M8", "Line 1"), atau teks.
fn key_kind(text: &str) -> u8 {
    if is_month(text.trim()) {
        0
    } else if is_date_like(text.trim()) {
        1
    } else if text.chars().any(|c| c.is_ascii_digit()) {
        2
    } else {
        3
    }
}

/// Baris ringkasan: sel pertamanya "Total", "Subtotal", "Grand total", "Jumlah", "合計", ….
fn is_total_row(row: &[&str]) -> bool {
    matches!(
        first(row)
            .trim()
            .trim_end_matches(':')
            .to_lowercase()
            .as_str(),
        "total" | "subtotal" | "sub total" | "grand total" | "jumlah" | "sum" | "合計" | "小計"
    )
}

/// Mungkin label periode, untuk membatalkan periode lama di tabel sebelah: berisi angka atau
/// nama bulan ("FY 2025", "November", "Shift 2"). Hanya untuk membuang label yang mungkin
/// basi; tidak pernah membuat baris jadi label.
fn may_be_period(label: &str) -> bool {
    is_period(label)
        || label
            .split(|c: char| !c.is_alphabetic())
            .any(|w| !w.is_empty() && is_month(w))
}

/// Nama kolom per periode yang jelas: tahun (`2025`), tahun buku (`FY 2025`, `FY25`), kuartal
/// atau semester (`Q1`, `Q1 2026`, `H2`), bulan dan tahun (`Mar 2026`, `Jan-26`), atau dua tahun
/// (`2025/2026`). Kode seperti `AX100` atau `STD X1000` bukan.
fn is_period_name(cell: &str) -> bool {
    let lower = cell.trim().to_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| c.is_whitespace() || matches!(c, '-' | '/' | '.' | '\''))
        .filter(|t| !t.is_empty())
        .collect();
    let year = |t: &str| is_year(t) || (t.len() == 2 && t.chars().all(|c| c.is_ascii_digit()));
    let part = |t: &str| {
        matches!(t, "q1" | "q2" | "q3" | "q4" | "h1" | "h2")
            || (t.len() > 2 && t.starts_with("fy") && year(&t[2..]))
    };
    match tokens.as_slice() {
        [t] => is_year(t) || part(t),
        [a, b] => {
            ((is_month(a) || *a == "fy" || part(a)) && year(b))
                || (is_year(a) && (part(b) || is_year(b)))
        }
        _ => false,
    }
}

/// Label periode: berisi angka ("FY 2025", "Q1", "November 2026"). Nama bulan saja ("May")
/// bisa nama orang atau record, jadi bukan bukti periode.
fn is_period(label: &str) -> bool {
    label.chars().any(|c| c.is_ascii_digit())
}

/// Baris kepala tingkat bawah dari sel gabungan vertikal. Kolom pertamanya kosong (kepala
/// baris seperti "Month" atau "Item" digabung ke bawah). Setiap nama di baris atas membentang
/// ke kanan sampai nama berikutnya, dan setiap sel baris bawah harus berada di bawah bentangan
/// selebar dua kolom atau lebih, dengan satu bentangan berisi dua sel atau lebih. Tidak ada
/// angka selain tahun, dan selnya tidak sejenis dengan data di bawahnya (`below`).
fn is_subheader(upper: &[&str], lower: &[&str], below: Option<&Vec<&str>>) -> bool {
    if filled(lower) < 2
        || lower.first().is_none_or(|c| !c.is_empty())
        || lower
            .iter()
            .any(|c| (is_number(c) && !is_year(c)) || is_blankish(c))
        || !lower.iter().any(|c| has_letters(c) || is_year(c))
        || below.is_some_and(|b| like_data(lower, b))
    {
        return false;
    }
    let width = upper.len().max(lower.len());
    let starts: Vec<usize> = (0..width)
        .filter(|&c| upper.get(c).is_some_and(|u| !u.is_empty()))
        .collect();
    let mut per_span = vec![0usize; starts.len()];
    for (c, v) in lower.iter().enumerate() {
        if v.is_empty() {
            continue;
        }
        let Some(k) = starts.iter().rposition(|&s| s <= c) else {
            return false;
        };
        let end = starts.get(k + 1).copied().unwrap_or(width);
        if end - starts[k] < 2 {
            return false;
        }
        per_span[k] += 1;
    }
    per_span.iter().any(|&n| n >= 2)
}

/// Dua sel sejenis: keduanya berhuruf dan bentuknya sama setelah angka disamarkan
/// (`Review-001`, `Review-002`), atau keduanya campuran huruf dan angka (`Bolt M8`, `Nut M10`). Nama
/// kolom jarang begitu.
fn same_kind(a: &str, b: &str) -> bool {
    if !has_letters(a) || !has_letters(b) {
        return false;
    }
    let mixed = |s: &str| s.chars().any(|c| c.is_ascii_digit());
    shape(a) == shape(b) || (mixed(a) && mixed(b))
}

/// Baris `row` tampak seperti data yang sejenis dengan `data` di kolom yang sama.
fn like_data(row: &[&str], data: &[&str]) -> bool {
    row.iter()
        .zip(data)
        .any(|(x, y)| !x.is_empty() && !y.is_empty() && same_kind(x, y))
}

/// Kepala kolom yang dimulai di baris `i` beserta jumlah barisnya, kalau terbukti. Baris `i`
/// mirip kepala, boleh diikuti baris kepala tingkat bawah, lalu baris berikutnya berisi angka di
/// kolom yang punya nama, dan nama kolomnya tidak sejenis dengan data itu. Tanpa bukti itu
/// (tabel teks, baris formulir, record), tidak ada kepala: data tetap utuh, hanya tanpa nama
/// kolom yang diulang.
fn header_band(body: &[Vec<&str>], i: usize) -> Option<usize> {
    // Baris ringkasan ("Total | 2000 | 2000") adalah data, bukan kepala.
    if !is_header_start(body.get(i)?) || is_total_row(&body[i]) {
        return None;
    }
    let mut n = 1;
    while n < MAX_HEADER_ROWS
        && body
            .get(i + n)
            .is_some_and(|r| is_subheader(&body[i + n - 1], r, body.get(i + n + 1)))
    {
        n += 1;
    }
    // Setiap tingkat bawah harus menaungi kolom angka di data; kalau tidak, baris itu mungkin
    // record (`| Pass | No cracks |`), jadi hanya baris pertama yang dicoba sebagai kepala.
    if n > 1 {
        let data = body.get(i + n)?;
        let numeric_under = |r: &Vec<&str>| {
            r.iter()
                .enumerate()
                .any(|(c, v)| !v.is_empty() && data.get(c).is_some_and(|d| is_number(d)))
        };
        if !(i + 1..i + n).all(|j| numeric_under(&body[j])) {
            n = 1;
        }
    }
    let data = body.get(i + n)?;
    let band: Vec<&Vec<&str>> = body[i..i + n].iter().collect();
    let names = combine_header(&band);
    // Kepala dua kolom tidak bisa dibedakan dari formulir kunci-nilai ("Machine | Stasiun",
    // "Shift | 2") atau record ("Module | On order"), jadi minimal tiga kolom bernama.
    if names.iter().filter(|n| !n.is_empty()).count() < 3 {
        return None;
    }
    let numeric = data
        .iter()
        .enumerate()
        .any(|(c, v)| is_number(v) && names.get(c).is_some_and(|n| !n.is_empty()));
    if !numeric || band.iter().any(|r| like_data(r, data)) {
        return None;
    }
    // Kepala juga butuh bukti nama kolom, termasuk kepala bertingkat: bentangan saja bisa
    // berasal dari formulir ("Material | AX100" di atas "Nominal | Actual").
    if !header_evidence(&names, &band, body, i + n) {
        return None;
    }
    // Data sesudahnya harus berlanjut (6 baris berikutnya, label dilewati): kalau ada kepala
    // lain, kandidat ini kemungkinan baris formulir ("Machine | Stasiun" di atas "Shift | 2",
    // "Line | 3", label "Production", lalu "Item | Qty"). Tabel pendek yang langsung diikuti
    // bagian lain jadi tanpa kepala; itu lebih aman daripada kepala yang salah.
    // Kandidat dua kolom bisa baris formulir kunci-nilai yang panjang ("Shift | 2", "Line | 3",
    // …), jadi diperiksa sampai 50 baris; kepala yang lebih lebar cukup 6 baris.
    let reach = if filled(&body[i]) == 2 { 50 } else { 6 };
    for after in body.iter().skip(i + n + 1).take(reach) {
        if is_label(after) || is_total_row(after) {
            continue;
        }
        let repeat = band.iter().any(|r| trimmed(r) == trimmed(after));
        // Record teks yang sejenis dengan data (`S-5 | Pass | OK` setelah `S-4 | 12 | ok`)
        // bukan kepala lain.
        if is_header_start(after) && !repeat && !like_data(after, data) {
            return None;
        }
    }
    // Kolom tanpa nama yang kosong di baris data pertama tapi terisi di baris sesudahnya
    // (sampai 50 baris) menandakan tabel lain yang lebih lebar: formulir dua kolom
    // ("Machine | Stasiun", "Shift | 2", …) di atas tabel tiga kolom gugur di sini, berapa pun
    // panjang formulirnya. Kolom satuan tanpa nama yang terisi sejak baris pertama tetap boleh.
    let unnamed = body
        .iter()
        .skip(i + n + 1)
        .take(50)
        .filter(|r| !is_label(r))
        .any(|r| {
            r.iter().enumerate().any(|(c, v)| {
                !v.is_empty()
                    && names.get(c).is_none_or(|n| n.is_empty())
                    && data.get(c).is_none_or(|d| d.is_empty())
            })
        });
    if unnamed {
        return None;
    }
    Some(n)
}

/// Bentuk label tanpa angka, untuk mengenali label sejenis ("FY 2025" dan "FY 2026").
/// Nama bulan juga disamarkan kalau labelnya berisi angka, jadi "October 2026" dan
/// "November 2026" sejenis, tapi "April" dan "May" (bisa nama orang) tidak.
fn shape(label: &str) -> String {
    let dated = is_period(label);
    let flush = |word: &mut String, out: &mut String| {
        if dated && is_month(word) {
            out.push('@');
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    let mut out = String::new();
    let mut word = String::new();
    for c in label.to_lowercase().chars() {
        if c.is_alphabetic() {
            word.push(c);
            continue;
        }
        flush(&mut word, &mut out);
        let c = if c.is_ascii_digit() { '#' } else { c };
        if !(c == '#' && out.ends_with('#')) {
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// Sel tanpa sel kosong di ujung kanan, untuk membandingkan baris kepala.
fn trimmed<S: AsRef<str>>(row: &[S]) -> Vec<&str> {
    let mut v: Vec<&str> = row.iter().map(AsRef::as_ref).collect();
    while v.last().is_some_and(|c| c.is_empty()) {
        v.pop();
    }
    v
}

/// Rentang baris yang dipisahkan baris kosong.
fn regions(grid: &[Vec<String>]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, row) in grid.iter().enumerate() {
        let empty = row.iter().all(String::is_empty);
        match (start, empty) {
            (None, false) => start = Some(i),
            (Some(s), true) => {
                out.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, grid.len()));
    }
    out
}

/// Blok gabungan dari beberapa tabel yang dipisah kolom kosong, tapi hanya sebagian punya
/// label sendiri di atasnya ("Intake" di atas A:C, tabel stok tanpa label di E:G): label
/// itu mungkin hanya milik tabel di bawahnya, jadi tidak dipakai untuk seluruh blok.
fn one_sided_label(grid: &[Vec<String>], r0: usize, r1: usize, c0: usize, c1: usize) -> bool {
    let (_, labeled, bare, _) = table_runs(grid, r0, r1, c0, c1);
    labeled && bare
}

/// Tabel-tabel (bagian dengan baris dua sel atau lebih) yang dipisah kolom kosong di blok
/// `c0..c1`: jumlahnya, ada yang berlabel sendiri, ada yang tidak, dan apakah baris pertamanya
/// yang berisi dua sel atau lebih berbeda-beda (tabel berdampingan yang mulainya bergeser).
fn table_runs(
    grid: &[Vec<String>],
    r0: usize,
    r1: usize,
    c0: usize,
    c1: usize,
) -> (usize, bool, bool, bool) {
    let rows = &grid[r0..r1];
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    for c in c0..=c1 {
        let used = c < c1 && rows.iter().any(|row| !cell(row, c).is_empty());
        match (start, used) {
            (None, true) => start = Some(c),
            (Some(s), false) => {
                runs.push((s, c));
                start = None;
            }
            _ => {}
        }
    }
    let (mut tables, mut labeled, mut bare) = (0, false, false);
    let mut starts: Vec<usize> = Vec::new();
    for (a, b) in runs {
        let counts: Vec<usize> = rows
            .iter()
            .map(|row| (a..b).filter(|&c| !cell(row, c).is_empty()).count())
            .collect();
        if !counts.iter().any(|&n| n >= 2) {
            continue;
        }
        tables += 1;
        starts.extend(counts.iter().position(|&n| n >= 2));
        let label = counts.iter().position(|&n| n > 0).is_some_and(|i| {
            let v = cell(&rows[i], a);
            counts[i] == 1 && has_letters(v) && !is_number(v)
        });
        if label {
            labeled = true;
        } else {
            bare = true;
        }
    }
    if tables < 2 {
        return (tables, false, false, false);
    }
    let staggered = starts.iter().any(|&s| s != starts[0]);
    (tables, labeled, bare, staggered)
}

/// Nama kolom identitas record: nama kolom identitas umum (`is_identity_header`), atau sampel,
/// tanggal, nomor, kode, lot, mesin, lini, karyawan.
fn is_identity_column(name: &str) -> bool {
    is_identity_header(name)
        || matches!(
            name.trim().trim_end_matches(':').to_lowercase().as_str(),
            "sample"
                | "sample no"
                | "sample id"
                | "date"
                | "day"
                | "no"
                | "no."
                | "code"
                | "lot"
                | "lot no"
                | "machine"
                | "line"
                | "employee"
                | "sampel"
                | "tanggal"
                | "tgl"
                | "hari"
                | "kode"
                | "mesin"
                | "lini"
                | "karyawan"
                | "nama karyawan"
                | "日付"
                | "番号"
                | "ロット"
        )
}

/// Teks lepas di luar tabel: sel yang diawali `#` ("# Intake") di-escape supaya tidak
/// terbaca sebagai judul potongan oleh `chunk_text`.
fn free_text(text: &str) -> String {
    if text.starts_with('#') {
        format!("\\{text}")
    } else {
        text.to_string()
    }
}

/// Ringkasan satu rentang kolom: jumlah sel terisi per baris.
struct Part {
    c0: usize,
    c1: usize,
    counts: Vec<u32>,
}

/// Rentang kolom tabel-tabel yang berdampingan di baris `r0..r1`. Kolom kosong hanya
/// memisahkan kalau kedua sisinya tabel (ada baris dengan dua sel atau lebih) dengan kepala
/// kolom yang sama persis (Intake dan Dispatch, masing-masing `Month | Output | Hours`).
/// Selain itu, juga kalau keduanya berlabel, kolom kosong dianggap pemisah visual di satu
/// tabel dan baris tetap utuh: kelompok hasil uji di sebelah kolom sampel ("Sample | OD",
/// lalu "Test Date | UTS") adalah satu record, dan kalau ragu label dibuang, bukan barisnya
/// yang dipotong.
/// `footprints`: rentang kolom tabel-tabel sebelumnya. Dua bagian yang masing-masing berada di
/// jejak tabel berbeda (lanjutan tabel berdampingan) tetap dipisah.
fn blocks(
    grid: &[Vec<String>],
    r0: usize,
    r1: usize,
    footprints: &[(usize, usize)],
) -> Vec<(usize, usize)> {
    let rows = &grid[r0..r1];
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let used: Vec<bool> = (0..width)
        .map(|c| rows.iter().any(|row| !cell(row, c).is_empty()))
        .collect();
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    for (c, &u) in used.iter().chain(std::iter::once(&false)).enumerate() {
        match (start, u) {
            (None, true) => start = Some(c),
            (Some(s), false) => {
                runs.push((s, c));
                start = None;
            }
            _ => {}
        }
    }
    let summary = |(c0, c1): (usize, usize)| Part {
        c0,
        c1,
        counts: rows
            .iter()
            .map(|row| (c0..c1).filter(|&c| !cell(row, c).is_empty()).count() as u32)
            .collect(),
    };
    let is_table = |p: &Part| p.counts.iter().any(|&n| n >= 2);
    let first_multi = |p: &Part| -> Option<Vec<&str>> {
        p.counts.iter().position(|&n| n >= 2).map(|i| {
            (p.c0..p.c1)
                .map(|c| cell(&rows[i], c))
                .filter(|v| !v.is_empty())
                .collect()
        })
    };
    let mut out: Vec<Part> = Vec::new();
    for run in runs {
        let right = summary(run);
        let Some(left) = out.last_mut() else {
            out.push(right);
            continue;
        };
        let fp = |c0: usize, c1: usize| footprints.iter().find(|&&(s, e)| s <= c0 && c1 <= e);
        let own_footprints = fp(left.c0, left.c1)
            .zip(fp(right.c0, right.c1))
            .is_some_and(|(a, b)| a != b);
        // Bagian di jejak tabel berbeda selalu dipisah, juga kalau isinya hanya label
        // (`FY 2025` di atas kedua tabel berdampingan).
        let separate = own_footprints
            || (is_table(left) && is_table(&right) && first_multi(left) == first_multi(&right));
        if separate {
            out.push(right);
        } else {
            left.c1 = right.c1;
            for (a, b) in left.counts.iter_mut().zip(&right.counts) {
                *a += b;
            }
        }
    }
    out.iter().map(|p| (p.c0, p.c1)).collect()
}

/// Kepala kolom aktif sebuah tabel.
#[derive(Clone)]
struct Header {
    /// Baris-baris kepala apa adanya, untuk mengenali kepala yang diulang.
    raw: Vec<Vec<String>>,
    names: Vec<String>,
    /// Terlalu lebar untuk diulang sebagai kepala tabel; baris ditulis "kolom: nilai".
    wide: bool,
}

impl Header {
    /// Kepala ini diulang persis (semua barisnya) mulai baris `i`.
    fn repeats_at(&self, body: &[Vec<&str>], i: usize) -> bool {
        self.raw
            .iter()
            .enumerate()
            .all(|(k, r)| body.get(i + k).is_some_and(|b| trimmed(r) == trimmed(b)))
    }
}

/// Konteks terakhir di satu posisi kolom, untuk tabel lanjutan di bawahnya.
#[derive(Clone, Default)]
struct Context {
    labels: Vec<String>,
    header: Option<Header>,
    width: usize,
    /// Blok ini hanya berisi label (misalnya periode berikutnya): tabel sesudahnya boleh
    /// punya kepala baru.
    fresh: bool,
    /// Label periode terakhir mungkin sudah tidak berlaku: ada periode baru di atas tabel lain
    /// (berdampingan) tapi tidak di atas tabel ini. Lanjutan tanpa label sendiri tidak
    /// mewarisinya.
    stale: bool,
    /// Kolom yang isinya angka semua di tabel ini; lanjutannya harus tetap angka di sana.
    numeric: Vec<bool>,
    /// Tabel tanpa kepala: baris pertamanya yang mirip kepala (kepala tak terbukti).
    lead: Option<Vec<String>>,
    /// Tabel parameter: nama parameter di kolom pertama adalah record.
    params: bool,
    /// Deret identitas bernomor tabel ini ("line #"): lanjutannya harus sejenis.
    series: Option<String>,
}

/// Profil kolom tabel yang sedang ditulis, diperbarui per baris data.
#[derive(Clone)]
struct Profile {
    /// Per kolom: angka semua (`Some(true)`), ada teks (`Some(false)`), belum ada nilai.
    columns: Vec<Option<bool>>,
    /// Kolom pertama baris data sejauh ini: ada yang bukan nama kolom umum, ada yang nama
    /// kolom umum.
    keys: (bool, bool),
    /// Bentuk identitas (kolom pertama) baris data terakhir, misalnya `s-#` untuk `S-4`.
    last_key: Option<String>,
    /// Baris yang sudah ditulis di bawah profil ini.
    rows: usize,
    /// Isi kolom pertama yang sudah muncul (huruf kecil): departemen yang berulang
    /// ("Production" lagi) bukan kunci formulir baru.
    seen: std::collections::HashSet<String>,
    /// Kepala tabel ini menamai kolom pertamanya parameter: kunci parameter di datanya boleh.
    keys_ok: bool,
    /// Ada baris data yang kolom pertamanya metadata formulir ("Date"): tabel ini formulir.
    meta: bool,
    /// Ada baris data yang kolom pertamanya parameter, kunci spesifikasi, atau metadata
    /// formulir ("Speed", "Material", "Date"): tabel ini spesifikasi atau formulir.
    spec: bool,
    /// Ada baris data yang kolom pertamanya bukan parameter, kunci spesifikasi, atau metadata.
    plain: bool,
    /// Bentuk identitas yang sama di semua baris data sejauh ini (`line #` untuk `Line 1`,
    /// `Line 2`); `None` kalau belum ada atau sudah campur (`mixed`).
    series: Option<String>,
    mixed: bool,
    /// Profil lanjutan yang mewarisi deret identitas tabel di atasnya.
    seeded: bool,
}

impl Profile {
    /// Profil awal: syarat kolom angka dari tabel yang kepalanya dipinjam (`prior`).
    fn new(width: usize, prior: &[bool]) -> Self {
        Self {
            columns: (0..width)
                .map(|c| prior.get(c).copied().unwrap_or(false).then_some(true))
                .collect(),
            keys: (false, false),
            last_key: None,
            rows: 0,
            seen: Default::default(),
            keys_ok: false,
            meta: false,
            spec: false,
            plain: false,
            series: None,
            mixed: false,
            seeded: false,
        }
    }

    fn numeric(&self) -> Vec<bool> {
        self.columns.iter().map(|c| *c == Some(true)).collect()
    }

    /// Semua baris data sejauh ini parameter, kunci spesifikasi, atau metadata formulir: satu
    /// record stok biasa ("Bolt") membatalkannya, juga kalau ada barang bernama parameter.
    fn formish(&self) -> bool {
        self.spec && !self.plain
    }

    /// Tabel parameter: kepalanya menamai kolom pertama parameter, atau semua barisnya
    /// parameter/spesifikasi ("Item | Nominal | Actual" di atas "Speed", "Pressure").
    fn parameters(&self) -> bool {
        self.keys_ok || (self.rows >= 2 && self.formish())
    }

    /// Deret identitas bernomor yang berlaku ("line #" untuk "Line 1", "Line 2").
    fn active_series(&self) -> Option<String> {
        self.series
            .clone()
            .filter(|s| s.contains('#') && !self.mixed && (self.rows >= 2 || self.seeded))
    }

    /// Profil lanjutan tabel `prev` (sesudah baris kosong): syarat kolom angka, tabel
    /// parameter, dan deret identitasnya tetap berlaku.
    fn seeded(width: usize, prev: &Context) -> Self {
        let mut p = Self::new(width, &prev.numeric);
        p.keys_ok = prev.params;
        if let Some(series) = &prev.series {
            p.series = Some(series.clone());
            p.seeded = true;
        }
        p
    }

    /// Baris `row` boleh ditulis di bawah kepala tabel ini: kolom angka tetap angka, dan
    /// kolom pertamanya bukan kunci formulir ("Date", "Shift") kalau baris-baris sebelumnya
    /// bukan.
    /// Semua baris data dua sel atau lebih dicek, juga yang mirip kepala (`Width | 2000 | mm`):
    /// kepala sungguhan sudah dipakai sebelum sampai ke sini.
    fn fits(&self, row: &[&str]) -> bool {
        if filled(row) < 2 || is_total_row(row) {
            return true;
        }
        // Identitas sejenis dengan baris sebelumnya (`S-5` setelah `S-4`) berarti record
        // tabel yang sama, walau nilainya campur (`Pass` di kolom angka).
        if let (Some(key), Some(v)) = (&self.last_key, row.first())
            && has_letters(v)
            && !is_column_name(v)
            && shape(v) == *key
        {
            return true;
        }
        let typed = self.columns.iter().enumerate().all(|(c, kind)| {
            *kind != Some(true)
                || row
                    .get(c)
                    .is_none_or(|v| v.is_empty() || is_blankish(v) || is_number(v))
        });
        // Kunci formulir baru ("Date") setelah identitas yang bukan nama kolom umum ("Apr",
        // "Review"), juga kalau sebagian identitas sebelumnya kebetulan nama kolom ("Production").
        let new_key = row.first().is_some_and(|v| {
            let fresh = !self.seen.contains(&v.trim().to_lowercase());
            // Metadata formulir ("Shift | 2") di bawah record yang bukan formulir, juga kalau
            // identitas record itu kebetulan nama kolom umum ("Production", "Sales").
            if is_form_meta(v) && fresh && self.rows > 0 && !self.meta {
                return true;
            }
            // Parameter baru ("Speed", "Tensile strength") di bawah record yang bukan
            // spesifikasi, juga kalau identitas record itu nama kolom umum ("Production",
            // "Line 1"). Di tabel parameter (`keys_ok`) nama parameter adalah record.
            if !self.keys_ok && is_parameter_key(v) && fresh && self.rows > 0 && !self.formish() {
                return true;
            }
            // Identitas lain ("UTS", "Wall") sesudah deret identitas bernomor ("Line 1",
            // "Line 2", "S-4") bukan record deret itu.
            if fresh
                && (self.rows >= 2 || self.seeded)
                && has_letters(v)
                && self
                    .series
                    .as_ref()
                    .is_some_and(|s| s.contains('#') && *s != shape(v))
            {
                return true;
            }
            if self.keys_ok {
                // Di tabel parameter, nama parameter adalah record; metadata formulir ("Shift",
                // "Line") selalu memutus konteks, juga kalau semua baris sebelumnya parameter.
                return is_form_key(v) && !is_parameter_key(v) && fresh;
            }
            // Kunci spesifikasi ("Drawing No") lalu parameter ("Thickness") satu lembar.
            is_form_key(v) && self.keys.0 && fresh && !self.formish()
        });
        typed && !new_key
    }

    /// `headed`: tabel ini punya kepala (terbukti atau dipinjam). Tanpa kepala, baris pertama
    /// yang mirip kepala ("Day | Output" yang tidak terbukti) tidak dicatat; baris lain,
    /// termasuk record mirip kepala ("Apr | 2000 | pcs", record teks), dicatat.
    fn add(&mut self, row: &[&str], headed: bool) {
        if filled(row) < 2 || is_total_row(row) {
            return;
        }
        self.rows += 1;
        // Kepala tak terbukti ("Day | Output") tidak menentukan jenis kolom, tapi kolom
        // pertamanya tetap dicatat (record "Bolt | 2000 | pcs" juga mirip kepala).
        let types = headed || self.rows > 1 || !is_header_start(row);
        for (c, v) in row.iter().enumerate() {
            if !types {
                break;
            }
            if v.is_empty() || is_blankish(v) || c >= self.columns.len() {
                continue;
            }
            let numeric = is_number(v);
            self.columns[c] = Some(self.columns[c].unwrap_or(true) && numeric);
        }
        match row.first() {
            Some(v) if !v.is_empty() && is_form_key(v) => self.keys.1 = true,
            Some(v) if !v.is_empty() => self.keys.0 = true,
            _ => {}
        }
        self.last_key = row.first().filter(|v| !v.is_empty()).map(|v| shape(v));
        if let Some(v) = row.first().filter(|v| !v.is_empty()) {
            self.seen.insert(v.trim().to_lowercase());
            self.meta |= is_form_meta(v);
            let spec = is_form_meta(v) || is_parameter_key(v) || is_spec_key(v);
            self.spec |= spec;
            self.plain |= !spec;
            // Kepala tak terbukti tidak ikut deret identitas.
            if types {
                let s = shape(v);
                match &self.series {
                    None if !self.mixed => self.series = Some(s),
                    Some(x) if *x != s => {
                        self.series = None;
                        self.mixed = true;
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Baris-baris `rows` boleh ditulis di bawah kepala tabel dengan profil `numeric`: di baris
/// berisi dua sel atau lebih, kolom yang tadinya angka tetap angka, dan bukan formulir
/// kunci-nilai (kolom pertama berisi nama kolom umum seperti "Date", "Shift", "Line", "OD").
/// Baris total dan baris satu sel (judul, record teks sejajar) dilewati.
/// Kepala `header` yang diulang persis (kepala halaman) bukan data.
/// `lead`: kepala tak terbukti tabel sebelumnya ("Month | Prod" yang diulang di setiap
/// periode), dilewati seperti kepala yang diulang.
/// `params`: tabel itu tabel parameter (lihat `is_parameter_header`).
/// `series`: deret identitas bernomor tabel itu ("line #"); identitas lain ("Rm") bukan
/// lanjutannya.
fn fits_rows(
    numeric: &[bool],
    rows: &[Vec<&str>],
    header: Option<&Header>,
    lead: Option<&[String]>,
    params: bool,
    series: Option<&str>,
) -> bool {
    let repeat = |r: &Vec<&str>| {
        header.is_some_and(|h| h.raw.iter().any(|raw| trimmed(raw) == trimmed(r)))
            || lead.is_some_and(|l| trimmed(l) == trimmed(r))
    };
    let data: Vec<&Vec<&str>> = rows
        .iter()
        .filter(|r| filled(r) >= 2 && !is_total_row(r) && !repeat(r))
        .collect();
    // Tanpa kepala, baris pertama yang mirip kepala tidak menentukan jenis kolom (sama seperti
    // di `Profile`), tapi tetap dicek sebagai kunci formulir ("Year | FY 2025").
    let skip_types = header.is_none() && data.first().is_some_and(|r| is_header_start(r));
    let typed = numeric.iter().enumerate().all(|(c, &numeric)| {
        !numeric
            || data.iter().skip(usize::from(skip_types)).all(|r| {
                r.get(c)
                    .is_none_or(|v| v.is_empty() || is_blankish(v) || is_number(v))
            })
    });
    let form = data.iter().any(|r| {
        r.first().is_some_and(|v| {
            // Identitas sejenis deret tabel itu ("Line 3" setelah "Line 1", "Line 2")
            // record lanjutannya.
            is_form_key(v)
                && !(params && is_parameter_key(v))
                && series.is_none_or(|s| shape(v) != s)
        })
    });
    let other = series.is_some_and(|s| {
        data.iter()
            .any(|r| r.first().is_some_and(|v| has_letters(v) && shape(v) != s))
    });
    typed && !form && !other
}

/// `fits_rows` terhadap tabel `prev`: blok ini boleh memakai kepala dan labelnya.
fn fits_previous(prev: &Context, rows: &[Vec<&str>]) -> bool {
    fits_rows(
        &prev.numeric,
        rows,
        prev.header.as_ref(),
        prev.lead.as_deref(),
        prev.params,
        prev.series.as_deref(),
    )
}

struct SheetWriter<'a> {
    out: &'a mut Out,
    sheet: &'a str,
    last: HashMap<usize, Context>,
    /// Teks pendek sendirian (dipisah baris kosong) yang menunggu tabel di bawahnya.
    pending: HashMap<usize, Vec<String>>,
    /// Region tempat judul tertunda di `pending` terakhir ditambah.
    pending_region: HashMap<usize, usize>,
    /// Region yang sedang ditulis, dan apakah region ini sudah menulis tabel.
    region: usize,
    wrote_table: bool,
    /// Baris satu sel di awal tabel berikutnya yang sudah diputuskan sebagai data (record
    /// dengan nilai kosong), dipakai sekali oleh `table`.
    leading: usize,
    /// Blok berikutnya gabungan tabel yang hanya sebagian berlabel (`one_sided_label`): label
    /// di awalnya ditulis sebagai teks, bukan label. Dipakai sekali oleh `block`.
    uncertain: bool,
    /// Blok berikutnya gabungan beberapa tabel yang dipisah kolom kosong: label tertunda di
    /// atasnya ("Intake Production Report" lalu baris kosong) mungkin hanya milik salah
    /// satunya, jadi tidak dipakai. Dipakai sekali oleh `block`.
    several: bool,
    /// Blok berikutnya gabungan tabel berdampingan yang mulainya bergeser ("Intake" di
    /// atas A:C, "Dispatch" satu baris lebih bawah di E:G): baris kepala satu tabel sebaris
    /// dengan label atau data tabel lain, jadi semua baris blok ini ditulis sebagai data, tanpa
    /// kepala dan tanpa label (juga label tertunda di atasnya). Dipakai sekali oleh `block`.
    loose: bool,
}

impl SheetWriter<'_> {
    /// Menulis judul dan mengembalikan berapa label teratas yang tidak muat.
    fn heading(&mut self, labels: &[String]) -> usize {
        let len = |ls: &[String]| {
            self.sheet.chars().count() + ls.iter().map(|l| l.chars().count() + 3).sum::<usize>()
        };
        let mut dropped = 0;
        while labels.len() - dropped > 1 && len(&labels[dropped..]) > MAX_HEADING_CHARS {
            dropped += 1;
        }
        let mut line = format!("## {}", self.sheet);
        for l in &labels[dropped..] {
            line.push_str(" › ");
            line.push_str(l);
        }
        line.push('\n');
        self.out.push(&line);
        dropped
    }

    /// Label yang terpotong atau tidak muat di judul ditulis utuh di isi.
    fn full_label(&mut self, text: &str, labels: &[String], dropped: usize) {
        let shown = labels[dropped..].iter().any(|l| l == text);
        if !shown {
            self.out.push(&free_text(text));
            self.out.push("\n");
        }
    }

    fn header(&mut self, h: &Header) {
        if !h.wide {
            let mut line = markdown_row(&h.names);
            line.push('|');
            for _ in &h.names {
                line.push_str(" --- |");
            }
            line.push('\n');
            self.out.push(&line);
        }
    }

    /// Blok `rows` di kolom `c0` memuat label periode di kolom pertamanya ("FY 2025",
    /// "November 2026"), sendirian, di atas data, atau di tengah tabel. Periode di tabel lain
    /// belum tentu ikut berganti, jadi lanjutan tabel itu tanpa label sendiri tidak mewarisi
    /// periodenya.
    fn note_period(&mut self, c0: usize, rows: &[Vec<&str>]) {
        if !rows.iter().any(|r| is_label(r) && may_be_period(first(r))) {
            return;
        }
        for (&k, ctx) in self.last.iter_mut() {
            if k != c0 && ctx.labels.iter().any(|l| may_be_period(l)) {
                ctx.stale = true;
            }
        }
    }

    /// Periode tabel di `c0` mungkin sudah tidak berlaku (lihat `note_period`).
    fn mark_stale(&mut self, c0: usize) {
        if let Some(ctx) = self.last.get_mut(&c0)
            && ctx.labels.iter().any(|l| may_be_period(l))
        {
            ctx.stale = true;
        }
    }

    fn block(&mut self, c0: usize, rows: &[Vec<&str>]) {
        let uncertain = std::mem::take(&mut self.uncertain);
        let several = std::mem::take(&mut self.several);
        let loose = std::mem::take(&mut self.loose);
        let width = rows.iter().map(Vec::len).max().unwrap_or(0);
        let same_layout = self.last.get(&c0).is_some_and(|p| p.width == width);
        let prev = self.last.get(&c0).cloned().unwrap_or_default();
        // Teks yang sejajar dengan tabel di atasnya (sudah dilebarkan ke rentang kolomnya)
        // diperlakukan sebagai baris tabel, bukan teks lepas atau label tertunda, juga kalau
        // kepala tabel itu sudah dibuang.
        let aligned = same_layout;
        if !aligned && rows.iter().all(|r| is_text_line(r)) {
            let lines: Vec<&str> = rows.iter().map(|r| first(r)).collect();
            for l in &lines {
                self.out.push(&free_text(l));
                self.out.push("\n");
            }
            self.out.push("\n");
            if lines.iter().all(|l| l.chars().count() <= MAX_LABEL_CHARS) {
                self.pending_region.insert(c0, self.region);
                let p = self.pending.entry(c0).or_default();
                p.extend(lines.iter().map(|l| l.to_string()));
                let extra = p.len().saturating_sub(MAX_STANDALONE_LABELS);
                p.drain(..extra);
            } else {
                self.pending.remove(&c0);
            }
            return;
        }
        let mut lead = rows.iter().take_while(|r| is_label(r)).count();
        // Baris satu sel di awal blok yang langsung diikuti data, selebar tabel di atasnya,
        // bisa jadi record dengan nilai kosong ("Bolt |", "Nut |") alih-alih label. Kecuali
        // label periode sejenis, baris itu ditulis sebagai data di bawah kepala tabel di atasnya,
        // dan label lama dibuang karena tidak jelas masih berlaku.
        let period = lead > 0
            && prev
                .labels
                .last()
                .is_some_and(|p| shape(p) == shape(&clip(first(&rows[lead - 1]), MAX_LABEL_CHARS)));
        // Juga kalau sesudahnya kepala tabel di atasnya diulang persis (kepala halaman), dan
        // untuk blok yang hanya berisi teks sejajar di kolom mana pun (`| Fail |`).
        let text_block = rows.iter().all(|r| is_text_line(r));
        let text_period = rows.iter().any(|r| {
            r.first().is_some_and(|c| !c.is_empty())
                && prev
                    .labels
                    .last()
                    .is_some_and(|p| shape(p) == shape(&clip(first(r), MAX_LABEL_CHARS)))
        });
        // Judul bagian yang jelas ("Production Report") di awal blok sejajar: bisa judul, bisa
        // record ("Daily planner A5 |"). Barisnya tetap data, tapi kepala tabel di atasnya
        // tidak dipinjam.
        let titled = aligned
            && rows
                .iter()
                .take_while(|r| is_text_line(r) || is_heading_row(r))
                .any(|r| is_heading_text(first(r)));
        let records = aligned
            && ((lead > 0
                && !period
                && (lead == rows.len()
                    || rows.get(lead).is_some_and(|r| {
                        (!is_header_start(r) && !is_label(r))
                            || prev
                                .header
                                .as_ref()
                                .is_some_and(|h| h.repeats_at(rows, lead))
                    })))
                || (text_block && !text_period));
        // Label (bukan periode) yang diikuti kepala berbeda, di susunan kolom tabel di atasnya:
        // bisa bagian baru, bisa record (`Bolt |` lalu `Elbow | Pass | Bob`). Tidak jelas, jadi
        // baris-barisnya data tanpa kepala dan tanpa label.
        let ambiguous_lead = aligned
            && lead > 0
            && !period
            && !records
            && rows.get(lead).is_some_and(|r| is_header_start(r));
        // Di awal blok tanpa tabel di atasnya: baris satu sel yang langsung diikuti data
        // (bukan kepala terbukti) bisa record dengan nilai kosong ("Bolt M8 |", "Nut |", lalu
        // "Washer M8 | 12 | pcs"). Judul di depannya ("Stock Report") tetap label; baris
        // sesudah judul ditulis sebagai data kalau salah satunya sejenis dengan identitas data
        // sesudahnya (kalau ragu, data). "Production", "Dispatch" di atas data bulanan tetap
        // label.
        let sparse = if aligned || lead == 0 {
            0
        } else {
            rows.get(lead).map_or(0, |next| {
                let key = next.first().copied().unwrap_or("");
                let tail = rows[..lead]
                    .iter()
                    .rev()
                    .take_while(|r| !is_title(first(r)))
                    .count();
                let texts: Vec<&str> = rows[lead - tail..lead].iter().map(|r| first(r)).collect();
                // Identitas baris-baris data sesudahnya ("Alex", "Cici"): kalau semuanya sejenis
                // dengan baris di atasnya ("Taylor"), baris mirip kepala ("Alex | Pagi | Review")
                // sebenarnya record juga.
                let below: Vec<&str> = rows[lead..]
                    .iter()
                    .filter(|r| filled(r) >= 2 && !first(r).is_empty())
                    .map(|r| first(r))
                    .collect();
                let same_below = !is_identity_column(key)
                    && !below.is_empty()
                    && texts
                        .iter()
                        .all(|t| below.iter().all(|b| key_kind(b) == key_kind(t)));
                let records = filled(next) >= 2
                    && !is_label(next)
                    && header_band(rows, lead).is_none()
                    && !texts.is_empty()
                    && texts.iter().any(|t| key_kind(t) == key_kind(key))
                    // Satu baris di atas baris mirip kepala ("Intake" lalu "Month | Qty")
                    // lebih mungkin judul.
                    && (texts.len() >= 2 || !is_header_start(next) || same_below);
                if records { tail } else { 0 }
            })
        };
        if sparse > 0 {
            self.leading = sparse;
            lead -= sparse;
        }
        // Blok yang bergeser: baris awalnya juga ditulis sebagai baris tabel di kolomnya, bukan
        // teks lepas (`# Intake` di A1 tidak boleh terbaca sebagai judul potongan).
        if records || ambiguous_lead || loose {
            lead = 0;
        }
        let lead_text: Vec<&str> = rows[..lead].iter().map(|r| first(r)).collect();
        // Label tabel ini: teks sendirian di atasnya (sudah ditulis utuh), lalu label di
        // baris-baris pertamanya.
        let mut own: Vec<String> = self.pending.remove(&c0).unwrap_or_default();
        if several {
            own.clear();
        }
        // Blok gabungan beberapa tabel: judul di baris awal ("Intake Production Report" di
        // atas tabel kiri saja) mungkin hanya milik salah satunya, jadi hanya ditulis sebagai
        // teks, bukan label.
        for t in lead_text
            .iter()
            .filter(|_| !uncertain && !loose && !several)
        {
            let l = clip(t, MAX_LABEL_CHARS);
            if !own.contains(&l) {
                own.push(l);
            }
        }
        let body = &rows[lead..];
        let same_header = |h: &Option<Header>| h.as_ref().is_some_and(|h| h.repeats_at(body, 0));
        // Data yang berlanjut setelah baris kosong: tanpa label, tanpa kepala sendiri, dan
        // selebar tabel di atasnya. Kalau ragu, tabel ditulis tanpa kepala pinjaman.
        // Kepala dan label tabel di atasnya hanya dipinjam kalau isinya cocok: formulir
        // ("Date | 17 Nov 2026", "Shift | 2") di bawah tabel operasional bukan lanjutannya.
        let fits = fits_previous(&prev, body);
        let continues = own.is_empty()
            && prev.header.is_some()
            && width == prev.width
            && fits
            && body.first().is_some_and(|r| !is_header_start(r));
        // Periode berikutnya setelah baris kosong ("FY 2025" lalu langsung data) memakai lagi
        // kepala tabel periode sebelumnya.
        let next_period = !own.is_empty()
            && prev.header.is_some()
            && width == prev.width
            && fits
            && body
                .first()
                .is_some_and(|r| !is_header_start(r) && !is_label(r))
            && prev
                .labels
                .last()
                .zip(own.last())
                .is_some_and(|(p, o)| shape(p) == shape(o));
        let own_is_empty = own.is_empty();
        let mut labels = if records || ambiguous_lead {
            own
        } else if own.is_empty() {
            if continues || (same_header(&prev.header) && fits) {
                let mut l = prev.labels.clone();
                // Periode di tingkat mana pun ("FY 2026 › Dispatch") dibuang; nama bagian
                // tetap.
                if prev.stale {
                    l.retain(|x| !may_be_period(x));
                }
                l
            } else {
                Vec::new()
            }
        } else {
            // Label sejenis dengan label di tingkat yang sama di tabel sebelumnya ("FY 2025"
            // setelah "Intake › FY 2026") menggantikannya dan mewarisi label di atasnya.
            match prev.labels.len().checked_sub(own.len()) {
                // Hanya kalau isinya cocok dengan tabel itu: "Shift 2" lalu formulir tidak
                // mewarisi "November 2026" dari "November 2026 › Shift 1".
                Some(k) if k > 0 && fits && shape(&own[0]) == shape(&prev.labels[k]) => {
                    let mut l = prev.labels[..k].to_vec();
                    // Periode di atasnya mungkin sudah berganti di tabel sebelah.
                    if prev.stale {
                        l.retain(|x| !may_be_period(x));
                    }
                    l.extend(own.iter().cloned());
                    l
                }
                _ => own,
            }
        };
        // Blok gabungan tabel yang mulainya bergeser: label tertunda dan label tabel di atasnya
        // mungkin hanya milik salah satu tabel, jadi tidak dipakai (teksnya sudah ditulis).
        if loose {
            labels.clear();
        }
        // Blok tanpa label dengan susunan kolom yang sama seperti tabel di atasnya, diawali baris
        // mirip kepala yang berbeda: bisa tabel baru, bisa record teks (`Elbow | Pass | Bob`).
        // Tidak bisa dibedakan dengan aman, jadi blok ini tanpa kepala. Kalau tepat di atasnya
        // ada label sendiri (periode berikutnya), kepala baru tetap boleh.
        let ambiguous_start = !records
            && own_is_empty
            && same_layout
            && !prev.fresh
            && body.first().is_some_and(|r| is_header_start(r))
            && !same_header(&prev.header);
        let dropped = self.heading(&labels);
        for t in lead_text {
            self.full_label(t, &labels, dropped);
        }
        // Record sejajar memakai kepala tabel di atasnya juga hanya kalau baris datanya cocok
        // (judul "Setup" lalu formulir "Date | 17 Nov 2026" tidak).
        let inherited = (continues || next_period || (records && fits)) && !titled && !loose;
        let header = if inherited { prev.header.clone() } else { None };
        if let Some(h) = &header {
            self.header(h);
        }
        // Blok yang hanya berisi label (periode berikutnya sebelum tabelnya) tidak mengubah
        // kepala tabel di atasnya.
        let fresh = body.is_empty() && aligned;
        let (header, numeric, params, series) = if fresh {
            (
                prev.header.clone(),
                prev.numeric.clone(),
                prev.params,
                prev.series.clone(),
            )
        } else {
            // Blok yang bergeser: semua baris data apa adanya, termasuk baris satu sel di
            // tengahnya ("Intake Report 2026"), tanpa label dan tanpa kepala baru.
            if loose {
                self.leading = body.len();
            }
            let (header, profile) = self.table(
                body,
                &mut labels,
                header,
                !ambiguous_start && !ambiguous_lead && !loose,
                inherited.then_some(&prev),
            );
            // Profil seksi terakhir yang ditulis (bukan seluruh blok): lanjutan sesudah baris
            // kosong dicek terhadap tabel yang sedang berlaku.
            (
                header,
                profile.numeric(),
                profile.parameters(),
                profile.active_series(),
            )
        };
        self.out.push("\n");
        // Konteks lama yang kolomnya tertimpa tabel ini tidak berlaku lagi untuk tabel di
        // bawahnya.
        self.last
            .retain(|&s, ctx| s + ctx.width <= c0 || s >= c0 + width);
        // Blok label saja bagian dari tabel di atasnya: profil kolomnya tetap milik tabel itu.
        // Lanjutan yang memakai kepala yang sama menambah profilnya; kolom tanpa nilai ("-")
        // tidak menghapus syarat yang sudah ada.
        if !fresh {
            self.wrote_table = true;
        }
        let lead = if fresh {
            prev.lead.clone()
        } else if header.is_none() {
            body.iter()
                .find(|r| filled(r) >= 2)
                .filter(|r| is_header_start(r))
                .map(|r| r.iter().map(|c| c.to_string()).collect())
        } else {
            None
        };
        self.last.insert(
            c0,
            Context {
                labels,
                header,
                width,
                fresh,
                stale: false,
                numeric,
                lead,
                params,
                series,
            },
        );
    }

    /// Baris tabel setelah label. Mengembalikan kepala kolom yang aktif di akhir tabel.
    /// `detect`: kepala di awal tabel boleh dicari (tidak untuk blok yang awalnya ambigu).
    /// `prior`: tabel yang kepalanya dipinjam, untuk profil awalnya (lihat `Profile::seeded`).
    fn table(
        &mut self,
        body: &[Vec<&str>],
        labels: &mut Vec<String>,
        mut header: Option<Header>,
        detect: bool,
        prior: Option<&Context>,
    ) -> (Option<Header>, Profile) {
        let mut headed = true;
        let mut expect_header = false;
        // Sudah ada baris data di tabel ini.
        let mut wrote_data = false;
        // Akhir rangkaian baris satu sel yang sedang diperiksa, dan batas baris yang sudah
        // diputuskan sebagai data (supaya rangkaian panjang tidak dipindai ulang per baris).
        let mut run_end = 0;
        let mut data_until = std::mem::take(&mut self.leading);
        // Profil kolom tabel yang sedang ditulis (baris demi baris; mulai lagi saat kepala baru
        // dipakai), untuk memastikan kepala aktif hanya menaungi baris yang cocok.
        let width = body.iter().map(Vec::len).max().unwrap_or(0);
        let mut profile = match prior {
            Some(prev) => Profile::seeded(width, prev),
            None => Profile::new(width, &[]),
        };
        // Kepala `h` boleh tetap dipakai untuk baris-baris mulai `from` (sampai baris satu sel
        // berikutnya): isinya cocok dengan tabel ini, bukan formulir.
        let fits_after = |from: usize, h: &Option<Header>, profile: &Profile| -> bool {
            let from = from.min(body.len());
            let to = from + body[from..].iter().take_while(|r| filled(r) >= 2).count();
            fits_rows(
                &profile.numeric(),
                &body[from..to],
                h.as_ref(),
                None,
                profile.keys_ok,
                profile.active_series().as_deref(),
            )
        };
        let mut i = 0;
        while i < body.len() && !self.out.full {
            let row = &body[i];
            let repeat_len = |at: usize, h: &Option<Header>| {
                h.as_ref()
                    .filter(|h| h.repeats_at(body, at))
                    .map(|h| h.raw.len())
            };
            // Baris satu sel di tengah tabel: satu label atau beberapa berurutan. Panjang
            // rangkaian dihitung sekali; baris yang sudah diputuskan sebagai data dilewati.
            if i >= data_until && is_label(row) {
                if i >= run_end {
                    run_end = i + body[i..].iter().take_while(|r| is_label(r)).count();
                }
                let run = run_end - i;
                let texts: Vec<&str> = body[i..run_end].iter().map(|r| first(r)).collect();
                let new: Vec<String> = texts.iter().map(|t| clip(t, MAX_LABEL_CHARS)).collect();
                let after = body.get(run_end);
                let header_after = after.is_some_and(|r| is_header_start(r));
                let data_after = after.is_some_and(|r| !is_header_start(r) && !is_label(r));
                let period = run == 1 && labels.last().is_some_and(|l| shape(l) == shape(&new[0]));
                let repeat_after = repeat_len(run_end, &header).is_some();
                let proven_after = !repeat_after && header_band(body, run_end).is_some();
                if period && (header_after || data_after) {
                    // Periode berikutnya ("FY 2025" setelah "FY 2026"): ganti label terakhir;
                    // langsung diikuti data berarti kepala aktif dipakai lagi.
                    labels.pop();
                    labels.extend(new);
                    self.out.push("\n");
                    let dropped = self.heading(labels);
                    self.full_label(texts[0], labels, dropped);
                    if data_after {
                        // Kepala aktif dipakai lagi hanya kalau data sesudah periode baru cocok
                        // dengan tabel ini (bukan formulir "Date | 17 Nov 2025").
                        if !fits_after(run_end, &header, &profile) {
                            header = None;
                        }
                        if let Some(h) = header.clone() {
                            self.header(&h);
                        }
                        headed = false;
                    } else {
                        headed = true;
                        expect_header = true;
                        profile = Profile::new(width, &[]);
                    }
                    i = run_end;
                    continue;
                }
                if proven_after && !wrote_data && detect {
                    // Bagian baru dengan kepala baru yang terbukti ("Dispatch" lalu
                    // "Month | Hours"), hanya sebelum ada data di tabel ini. Di tengah tabel yang
                    // sudah berisi data, label dan kepala baru bisa saja record (`Bolt |` lalu
                    // `Elbow | Pass | Bob`), jadi diperlakukan sebagai kasus tidak jelas di bawah.
                    // Label lama hanya diwarisi kalau label pertama sejenis dengan label di
                    // tingkat yang sama; kalau tidak, diganti seluruhnya.
                    *labels = match labels.len().checked_sub(new.len()) {
                        Some(k) if k > 0 && shape(&new[0]) == shape(&labels[k]) => {
                            let mut l = labels[..k].to_vec();
                            l.extend(new);
                            l
                        }
                        _ => new,
                    };
                    self.out.push("\n");
                    let dropped = self.heading(labels);
                    for t in texts {
                        self.full_label(t, labels, dropped);
                    }
                    header = None;
                    headed = true;
                    expect_header = true;
                    profile = Profile::new(width, &[]);
                    i = run_end;
                    continue;
                }
                // Tidak jelas: bisa record dengan nilai kosong ("Bolt |"), bisa bagian baru
                // dengan kepala yang sama. Baris ditulis sebagai data dan label lama dibuang
                // supaya tidak menempel ke data yang mungkin milik bagian lain. Kepala
                // berbeda yang tidak terbukti juga membuang kepala aktif.
                let drop_header = header_after && !repeat_after && header.is_some();
                // Judul lalu formulir ("Setup" lalu "Date | 17 Nov 2026"): kepala aktif tidak
                // dipakai untuk formulir itu.
                let unfit =
                    data_after && header.is_some() && !fits_after(run_end, &header, &profile);
                // Judul bagian yang jelas ("Production Report"), juga di akhir tabel sebelum
                // baris kosong: bisa juga record ("Daily planner A5 |"), jadi tidak jadi label,
                // tapi kepala aktif tidak dipakai lagi sesudahnya.
                let titled = header.is_some() && texts.iter().any(|t| is_heading_text(t));
                if unfit || titled {
                    header = None;
                }
                if !labels.is_empty() || drop_header || unfit || titled {
                    // Judul baru tanpa label lama; chunk_text juga berhenti mengulang kepala
                    // lama sejak judul ini.
                    labels.clear();
                    if drop_header {
                        header = None;
                    }
                    self.out.push("\n");
                    self.heading(labels);
                    if data_after && let Some(h) = header.clone() {
                        self.header(&h);
                    }
                }
                data_until = run_end;
            }
            // Teks tunggal di kolom dalam (`| Dispatch |`) bisa judul bagian, bisa record: label
            // lama dibuang supaya tidak menempel ke data sesudahnya; kepala aktif tetap.
            // Judul bagian dengan periode di sel lain ("Production Report | 2026"): kepala dan
            // label lama tidak berlaku lagi; barisnya tetap data.
            if (header.is_some() || !labels.is_empty()) && filled(row) >= 2 && is_heading_row(row) {
                header = None;
                labels.clear();
                self.out.push("\n");
                self.heading(labels);
                profile = Profile::new(width, &[]);
            }
            // Judul bagian yang jelas di kolom dalam (`| Production Report |`) juga memutus
            // kepala aktif; barisnya tetap data.
            let unfit_text = !is_label(row)
                && is_text_line(row)
                && header.is_some()
                && (is_heading_text(first(row)) || !fits_after(i + 1, &header, &profile));
            if unfit_text {
                header = None;
            }
            if !is_label(row) && is_text_line(row) && (!labels.is_empty() || unfit_text) {
                labels.clear();
                self.out.push("\n");
                self.heading(labels);
                if let Some(h) = header.clone() {
                    self.header(&h);
                }
                headed = false;
            }
            // Baris mirip kepala yang berbeda di tengah tabel: bisa tabel lain yang menempel
            // tanpa baris kosong, bisa record teks (`Elbow | Pass | Bob`). Keduanya tidak bisa
            // dibedakan dengan aman, jadi kepala lama dan label lama dibuang, dan tidak ada
            // kepala baru yang dipakai di tengah tabel. Pengecualian: identitas di kolom
            // pertama sejenis dengan baris di atas/bawahnya (`S-5` setelah `S-4`), yang jelas
            // record tabel yang sama.
            let same_id = |other: Option<&Vec<&str>>| {
                other.is_some_and(|o| {
                    let (a, b) = (row.first().copied(), o.first().copied());
                    // Bentuk yang sama persis (`S-9`, `S-10`); campuran huruf-angka saja tidak
                    // cukup (`Point 1` bukan record `S-10`).
                    a.zip(b).is_some_and(|(a, b)| {
                        has_letters(a) && has_letters(b) && shape(a) == shape(b)
                    })
                })
            };
            let years = row.iter().filter(|v| is_year(v)).count();
            if !expect_header
                && let Some(h) = &header
                && !h.repeats_at(body, i)
                && is_header_start(row)
                && !is_total_row(row)
                // Angka di kolom yang punya nama berarti data (`Apr | 2025 | pcs`), kecuali
                // dua tahun atau lebih yang berarti kepala tahun (`Item | 2025 | 2026`).
                && !row.iter().enumerate().any(|(c, v)| {
                    is_number(v)
                        && !(is_year(v) && years >= 2)
                        && h.names.get(c).is_some_and(|n| !n.is_empty())
                })
                && !same_id(body.get(i + 1))
                && !same_id(i.checked_sub(1).map(|k| &body[k]))
            {
                header = None;
                labels.clear();
                self.out.push("\n");
                self.heading(labels);
                headed = false;
            }
            // Kepala yang diulang persis (kepala halaman) melanjutkan tabel yang sama: profilnya
            // tetap.
            let repeated = repeat_len(i, &header).is_some();
            let band = if let Some(n) = repeat_len(i, &header) {
                Some(n)
            } else if (header.is_none() && !wrote_data && detect) || expect_header {
                header_band(body, i)
            } else {
                None
            };
            if let Some(n) = band {
                if !headed {
                    self.out.push("\n");
                    self.heading(labels);
                }
                let rows: Vec<&Vec<&str>> = body[i..i + n].iter().collect();
                let names = combine_header(&rows);
                let h = Header {
                    raw: rows
                        .iter()
                        .map(|r| r.iter().map(|c| c.to_string()).collect())
                        .collect(),
                    wide: markdown_row(&names).chars().count() > MAX_HEADER_CHARS,
                    names,
                };
                self.header(&h);
                let keys_ok = h.names.first().is_some_and(|n| is_parameter_header(n));
                header = Some(h);
                if !repeated {
                    profile = Profile::new(width, &[]);
                    profile.keys_ok = keys_ok;
                }
                i += n;
                headed = false;
                expect_header = false;
                continue;
            }
            if expect_header && is_header_start(row) {
                // Kepala baru yang tidak terbukti: kepala lama tidak boleh dipakai untuk
                // bagian ini, jadi bagian ini ditulis tanpa kepala.
                header = None;
            }
            // Baris yang tidak cocok dengan tabel ini (formulir "Date | 17 Nov 2026" langsung di
            // bawah data operasional, tanpa baris kosong): kepala dan label dibuang, dengan judul
            // baru supaya chunk_text berhenti mengulang kepala lama. Juga untuk tabel tanpa
            // kepala yang masih membawa label.
            if (header.is_some() || !labels.is_empty()) && !profile.fits(row) {
                header = None;
                labels.clear();
                self.out.push("\n");
                self.heading(labels);
                profile = Profile::new(width, &[]);
            }
            profile.add(row, header.is_some());
            let line = markdown_row(row);
            match &header {
                Some(h) if h.wide => self.out.push(&key_values(Some(&h.names), row)),
                h if line.chars().count() > MAX_ROW_CHARS => {
                    self.out
                        .push(&key_values(h.as_ref().map(|h| h.names.as_slice()), row));
                    // Baris tabel sesudahnya tetap punya kepala di potongannya.
                    if let Some(h) = h.clone() {
                        self.header(&h);
                    }
                }
                _ => self.out.push(&line),
            }
            wrote_data = true;
            headed = false;
            expect_header = false;
            i += 1;
        }
        (header, profile)
    }
}

/// Nama kolom dari satu atau beberapa baris kepala. Sel gabungan hanya mengisi sel
/// pertamanya, jadi nama induk diteruskan ke kanan selama baris di bawahnya terisi:
/// "Production" di atas "Actual | Plan" jadi "Production Actual | Production Plan".
fn combine_header(rows: &[&Vec<&str>]) -> Vec<String> {
    let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut names = vec![String::new(); width];
    for (level, row) in rows.iter().enumerate() {
        let below = |c: usize| {
            rows[level + 1..]
                .iter()
                .any(|r| r.get(c).is_some_and(|v| !v.is_empty()))
        };
        let mut parent = "";
        for (c, name) in names.iter_mut().enumerate() {
            let own = row.get(c).copied().unwrap_or("");
            let part = if !own.is_empty() {
                parent = own;
                own
            } else if below(c) {
                parent
            } else {
                parent = "";
                ""
            };
            if !part.is_empty() {
                if !name.is_empty() {
                    name.push(' ');
                }
                name.push_str(part);
            }
        }
    }
    names
}

fn markdown_row<S: AsRef<str>>(row: &[S]) -> String {
    let dashes = |c: &str| c.contains('-') && c.chars().all(|ch| matches!(ch, '-' | ':'));
    // Baris yang semua selnya tanda minus (slot kosong, garis) mirip pemisah kepala tabel,
    // baik untuk markdown maupun untuk `chunk_text`; tanda minusnya di-escape.
    let looks_like_separator = row
        .iter()
        .any(|c| dashes(c.as_ref()) && c.as_ref().matches('-').count() >= 3)
        && row
            .iter()
            .all(|c| c.as_ref().is_empty() || dashes(c.as_ref()));
    let mut out = String::from("|");
    for c in row {
        let c = c.as_ref();
        out.push(' ');
        if looks_like_separator && dashes(c) {
            out.push('\\');
        }
        out.push_str(c);
        out.push_str(" |");
    }
    out.push('\n');
    out
}

/// Baris sebagai "kolom: nilai". Kalau terlalu panjang untuk satu baris, setiap sel ditulis
/// di barisnya sendiri dengan identitas baris (sel pertama, dipendekkan) di depannya. Sel
/// pertama sendiri ditulis utuh, dan isi sel yang sangat panjang dipecah per potongan.
fn key_values(names: Option<&[String]>, row: &[&str]) -> String {
    let name = |c: usize| -> String {
        names
            .and_then(|n| n.get(c))
            .filter(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("Kolom {}", c + 1))
    };
    let cells: Vec<(usize, &str)> = row
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, v)| !v.is_empty())
        .collect();
    let one_line = format!(
        "- {}\n",
        cells
            .iter()
            .map(|(c, v)| format!("{}: {v}", name(*c)))
            .collect::<Vec<_>>()
            .join("; ")
    );
    if one_line.chars().count() <= MAX_ROW_CHARS {
        return one_line;
    }
    let Some(&(id_col, id_val)) = cells.first() else {
        return String::new();
    };
    let id = clip(&format!("{}: {id_val}", name(id_col)), MAX_ID_CHARS);
    let mut out = String::new();
    for (c, v) in &cells {
        for (k, piece) in pieces(v, PIECE_CHARS).into_iter().enumerate() {
            if *c == id_col && k == 0 {
                out.push_str(&format!("- {}: {piece}\n", name(*c)));
            } else {
                out.push_str(&format!("- {id} · {}: {piece}\n", name(*c)));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|r| r.iter().map(|c| c.to_string()).collect())
            .collect()
    }

    fn text(name: &str, rows: &[&[&str]]) -> String {
        let cells = grid(rows);
        to_text(
            &[Sheet {
                name: name.into(),
                hidden: false,
                cells: &cells,
            }],
            usize::MAX,
        )
        .0
    }

    #[test]
    fn side_by_side_tables_get_their_own_labels_and_headers() {
        let t = text(
            "PCS per JAM",
            &[
                &["PCS PER JAM", "", "", "", "", "", "", ""],
                &["", "", "", "", "", "", "", ""],
                &["Intake", "", "", "", "", "Dispatch", "", ""],
                &["FY 2026", "", "", "", "", "FY 2026", "", ""],
                &["Month", "Prod", "Note", "", "", "Month", "Prod", "Note"],
                &["Apr", "37611", "1", "", "", "Apr", "24009", "1"],
                &["May", "28271", "1", "", "", "May", "23335", "1"],
                &["", "", "", "", "", "", "", ""],
                &["FY 2025", "", "", "", "", "FY 2025", "", ""],
                &["Month", "Prod", "Note", "", "", "Month", "Prod", "Note"],
                &["Apr", "33107", "1", "", "", "Apr", "42676", "1"],
            ],
        );
        assert_eq!(
            t,
            "## PCS per JAM\n\n\
             PCS PER JAM\n\n\
             ## PCS per JAM › PCS PER JAM › Intake › FY 2026\n\
             | Month | Prod | Note |\n| --- | --- | --- |\n| Apr | 37611 | 1 |\n| May | 28271 | 1 |\n\n\
             ## PCS per JAM › Dispatch › FY 2026\n\
             | Month | Prod | Note |\n| --- | --- | --- |\n| Apr | 24009 | 1 |\n| May | 23335 | 1 |\n\n\
             ## PCS per JAM › PCS PER JAM › Intake › FY 2025\n\
             | Month | Prod | Note |\n| --- | --- | --- |\n| Apr | 33107 | 1 |\n\n\
             ## PCS per JAM › Dispatch › FY 2025\n\
             | Month | Prod | Note |\n| --- | --- | --- |\n| Apr | 42676 | 1 |\n\n"
        );
    }

    #[test]
    fn spacer_columns_and_shared_titles_keep_rows_together() {
        let t = text(
            "Stok",
            &[
                &["Code", "", "Qty"],
                &["P-001", "", "12"],
                &["P-002", "", "7"],
            ],
        );
        assert_eq!(
            t,
            "## Stok\n\n## Stok\n| Code |  | Qty |\n| P-001 |  | 12 |\n| P-002 |  | 7 |\n\n"
        );
        let t = text(
            "Review",
            &[
                &["Review report", "", "", "", ""],
                &["Sample", "Part", "", "Diameter", "Hardness"],
                &["S-1", "Widget", "", "60.5", "210"],
            ],
        );
        assert!(
            t.contains("| Sample | Part |  | Diameter | Hardness |\n| --- |"),
            "{t}"
        );
        assert!(t.contains("| S-1 | Widget |  | 60.5 | 210 |\n"), "{t}");
    }

    /// Baris-baris ini harus ada berurutan dan tidak ada kepala tabel sama sekali.
    fn assert_plain(t: &str, rows: &[&str]) {
        assert!(!t.contains("| --- |"), "{t}");
        assert!(t.contains(&rows.join("\n")), "{t}");
    }

    #[test]
    fn unproven_headers_are_not_used() {
        // Tabel teks tanpa angka: tanpa kepala, baris utuh.
        let t = text(
            "Review",
            &[
                &["Sample", "Result", "Reading"],
                &["Review-001", "Pass", ""],
                &["Review-002", "Fail", "0.7"],
            ],
        );
        assert_plain(
            &t,
            &[
                "| Sample | Result | Reading |",
                "| Review-001 | Pass |  |",
                "| Review-002 | Fail | 0.7 |",
            ],
        );
        // Record di bawah kepala dengan sel kosong.
        let t = text(
            "Review",
            &[
                &["Sample", "", "Result", ""],
                &["Elbow", "Galvanized", "Pass", "Visual only"],
                &["Tee", "Plain", "Fail", "Crack"],
            ],
        );
        assert_plain(
            &t,
            &[
                "| Sample |  | Result |  |",
                "| Elbow | Galvanized | Pass | Visual only |",
            ],
        );
        // Formulir di atas tabel: tidak ada kepala yang salah.
        let t = text(
            "Prod",
            &[
                &["Date", "17 Nov 2026", ""],
                &["Shift", "Night", ""],
                &["Item", "Qty", "Reject"],
                &["Module", "10", "1"],
            ],
        );
        assert_plain(
            &t,
            &[
                "| Date | 17 Nov 2026 |  |",
                "| Shift | Night |  |",
                "| Item | Qty | Reject |",
                "| Module | 10 | 1 |",
            ],
        );
        // Daftar stok dengan nilai teks di baris pertama.
        for second in [&["Bolt M8", "N/A", ""][..], &["Bolt", "N/A", "Hold"][..]] {
            let rows: Vec<&[&str]> = vec![&["Item", "Qty", ""], second, &["Nut", "12", "Ready"]];
            let t = text("Stok", &rows);
            assert!(!t.contains("| --- |"), "{t}");
            assert!(t.contains("| Item | Qty |  |\n"), "{t}");
        }
        // Kepala gabungan horizontal tidak dipakai, data tetap sejajar.
        let t = text(
            "Gudang",
            &[
                &["Incoming", "", "Outgoing", ""],
                &["Item", "Qty", "Item", "Qty"],
                &["Bolt", "95", "Nut", "5"],
            ],
        );
        assert_plain(
            &t,
            &[
                "| Incoming |  | Outgoing |  |",
                "| Item | Qty | Item | Qty |",
                "| Bolt | 95 | Nut | 5 |",
            ],
        );
    }

    #[test]
    fn round_seven_cases() {
        // Formulir dengan nilai angka tidak jadi kepala.
        let t = text(
            "Prod",
            &[
                &["Date", "17 Nov 2026", ""],
                &["Shift", "2", ""],
                &["Item", "Qty", "Reject"],
                &["Module", "10", "1"],
            ],
        );
        assert!(!t.contains("| Date | 17 Nov 2026 |  |\n| --- |"), "{t}");
        assert!(t.contains("| Item | Qty | Reject |\n"), "{t}");
        // Pergantian periode dikenali walau tabel pertama tanpa kepala.
        let t = text(
            "Prod",
            &[
                &["FY 2026", ""],
                &["Month", "Status"],
                &["Apr", "Closed"],
                &["FY 2025", ""],
                &["Month", "Qty"],
                &["Apr", "8"],
            ],
        );
        assert!(
            t.ends_with("## Prod › FY 2025\n| Month | Qty |\n| Apr | 8 |\n\n"),
            "{t}"
        );
        // Bagian lain dengan kepala berbeda di tengah tabel: tidak bisa dibedakan dari record
        // (`Bolt |` lalu `Elbow | Pass | Bob`), jadi label dan kepala lama dibuang dan tidak ada
        // kepala baru; data tetap utuh.
        let t = text(
            "Prod",
            &[
                &["Intake", ""],
                &["Month", "Qty"],
                &["Apr", "10"],
                &["Dispatch", ""],
                &["Month", "Hours"],
                &["Apr", "7"],
            ],
        );
        assert!(
            t.ends_with("## Prod\n| Dispatch |  |\n| Month | Hours |\n| Apr | 7 |\n\n"),
            "{t}"
        );
        // Record yang jarang tidak terserap ke kepala bertingkat.
        let t = text(
            "Review",
            &[
                &["Sample", "Visual", "", "Reading"],
                &["", "Pass", "No cracks", ""],
                &["L-2", "Fail", "Scratch", "0.7"],
            ],
        );
        assert!(!t.contains("Visual Pass"), "{t}");
        assert!(t.contains("|  | Pass | No cracks |  |"), "{t}");
        // Kepala bertingkat yang diulang tetap memakai nama gabungannya.
        let t = text(
            "Prod",
            &[
                &["Month", "Production", ""],
                &["", "Actual", "Plan"],
                &["Apr", "10", "12"],
                &["Month", "Production", ""],
                &["", "Actual", "Plan"],
                &["May", "11", "13"],
            ],
        );
        assert_eq!(
            t.matches("| Month | Production Actual | Production Plan |\n| --- |")
                .count(),
            2,
            "{t}"
        );
        assert!(!t.contains("|  | Actual | Plan |"), "{t}");
        // Nilai N/A di data tidak membuat baris itu jadi kepala.
        let t = text(
            "Stok",
            &[
                &["Item", "Qty", "Unit"],
                &["Widget", "10", "pcs"],
                &["Bolt", "N/A", "pcs"],
                &["Nut", "12", "pcs"],
            ],
        );
        assert!(
            t.contains(
                "| Item | Qty | Unit |\n| --- | --- | --- |\n| Widget | 10 | pcs |\n| Bolt | N/A | pcs |"
            ),
            "{t}"
        );
    }

    #[test]
    fn round_eight_cases() {
        // Beberapa field formulir berangka tidak membuat Date jadi kepala.
        let t = text(
            "Prod",
            &[
                &["Date", "17 Nov 2026", ""],
                &["Shift", "2", ""],
                &["Line", "3", ""],
                &["Item", "Qty", "Reject"],
                &["Module", "10", "1"],
            ],
        );
        assert!(!t.contains("| --- |"), "{t}");
        // Dua label berurutan di tengah tabel: label lama tidak menempel ke data bagian baru.
        let t = text(
            "Prod",
            &[
                &["Intake", ""],
                &["FY 2026", ""],
                &["Month", "Qty"],
                &["Apr", "10"],
                &["Dispatch", ""],
                &["FY 2025", ""],
                &["Month", "Hours"],
                &["Apr", "7"],
            ],
        );
        assert!(
            t.ends_with(
                "## Prod\n| Dispatch |  |\n| FY 2025 |  |\n| Month | Hours |\n| Apr | 7 |\n\n"
            ),
            "{t}"
        );
        assert!(!t.contains("Intake › FY 2025"), "{t}");
        // N/A dan - di baris kedua bukan sub-kepala.
        let t = text(
            "Review",
            &[
                &["Sample", "Dimensions", ""],
                &["", "N/A", "-"],
                &["S-002", "60.5", "2.3"],
            ],
        );
        assert!(!t.contains("Dimensions N/A"), "{t}");
        assert!(t.contains("|  | N/A | - |"), "{t}");
    }

    #[test]
    fn round_nine_cases() {
        // Record dengan nilai kosong "-" bukan garis pemisah tabel (dicek di ingest).
        let t = text(
            "Review",
            &[
                &["Sample", "OD", "Wall"],
                &["S-001", "60.5", "2.3"],
                &["", "-", "-"],
                &["S-002", "60.4", "2.2"],
            ],
        );
        assert!(t.contains("|  | - | - |\n| S-002 |"), "{t}");
        // Record berurutan dengan qty kosong tetap data.
        let t = text(
            "Stok",
            &[
                &["Item", "Qty"],
                &["Module", "10"],
                &["Bolt", ""],
                &["Nut", ""],
                &["Plate", "7"],
            ],
        );
        assert!(
            t.contains("| Module | 10 |\n| Bolt |  |\n| Nut |  |\n| Plate | 7 |"),
            "{t}"
        );
        assert!(!t.contains("› Bolt"), "{t}");
        // Bagian saudara dengan kepala yang sama: tidak bersarang, dan label Intake tidak
        // menempel ke data Dispatch (barisnya tetap ada sebagai data).
        let t = text(
            "Production",
            &[
                &["Intake", ""],
                &["Month", "Qty"],
                &["Apr", "10"],
                &["Dispatch", ""],
                &["Month", "Qty"],
                &["Apr", "7"],
            ],
        );
        assert!(
            t.ends_with("| Dispatch |  |\n| Month | Qty |\n| Apr | 7 |\n\n"),
            "{t}"
        );
        assert!(t.contains("| Dispatch |  |"), "{t}");
        assert!(!t.contains("Intake › Dispatch"), "{t}");
        // Formulir berangka dipisah label dari tabelnya: Date tidak jadi kepala.
        let t = text(
            "Prod",
            &[
                &["Date", "17 Nov 2026", ""],
                &["Shift", "2", ""],
                &["Line", "3", ""],
                &["Production", "", ""],
                &["Item", "Qty", "Reject"],
                &["Module", "10", "1"],
            ],
        );
        assert!(!t.contains("| Date | 17 Nov 2026 |  |\n| --- |"), "{t}");
        // Kepala bulan-tahun tetap kepala.
        let t = text(
            "Rekap",
            &[&["Item", "Mar 2026", "Apr 2026"], &["Widget", "10", "12"]],
        );
        assert!(t.contains("| Item | Mar 2026 | Apr 2026 |\n| --- |"), "{t}");
        assert!(is_date_like("17 Nov 2026") && is_date_like("17/11/2026"));
        assert!(is_date_like("2026-11-17") && !is_date_like("Mar 2026"));
        assert!(!is_date_like("Jan-26") && !is_date_like("Margin 2025"));
    }

    #[test]
    fn round_ten_cases() {
        // Record dengan qty kosong sebelum kepala halaman yang diulang tetap data.
        let t = text(
            "Stock",
            &[
                &["Item", "Qty"],
                &["Module", "10"],
                &["Bolt", ""],
                &["Nut", ""],
                &["Item", "Qty"],
                &["Plate", "7"],
            ],
        );
        assert!(t.contains("| Bolt |  |\n| Nut |  |"), "{t}");
        assert!(!t.contains("› Bolt") && !t.contains("› Nut"), "{t}");
        assert!(t.contains("| Item | Qty |\n| Plate | 7 |"), "{t}");
        // Sama, tapi setelah baris kosong.
        let t = text(
            "Stock",
            &[
                &["Item", "Qty", "Unit"],
                &["Module", "10", "pcs"],
                &["", "", ""],
                &["Bolt", "", ""],
                &["Nut", "", ""],
                &["Plate", "7", "pcs"],
            ],
        );
        assert!(!t.contains("› Bolt") && !t.contains("› Nut"), "{t}");
        assert!(
            t.ends_with(
                "## Stock\n| Item | Qty | Unit |\n| --- | --- | --- |\n| Bolt |  |  |\n| Nut |  |  |\n| Plate | 7 | pcs |\n\n"
            ),
            "{t}"
        );
        // Formulir teks + angka, dipisah label dari tabelnya.
        for first_row in [
            &["Machine", "Stasiun", ""][..],
            &["Date", "2026-11-17 07:00", ""][..],
        ] {
            let rows: Vec<&[&str]> = vec![
                first_row,
                &["Shift", "2", ""],
                &["Line", "3", ""],
                &["Production", "", ""],
                &["Item", "Qty", "Reject"],
                &["Module", "10", "1"],
            ];
            let t = text("Prod", &rows);
            let wrong = format!("{}\n| --- |", markdown_row(first_row).trim_end());
            assert!(!t.contains(&wrong), "{t}");
        }
        assert!(is_date_like("2026-11-17 07:00") && is_date_like("2026-11-17T07:00"));
    }

    #[test]
    #[ignore = "tes performa: cargo test --release -- --ignored stay_fast"]
    fn empty_qty_records_stay_fast() {
        // Rangkaian panjang record ber-qty kosong tetap cepat (tidak kuadratik).
        let mut rows: Vec<Vec<String>> = vec![vec!["Item".into(), "Qty".into()]];
        rows.push(vec!["Module".into(), "10".into()]);
        rows.extend((0..20_000).map(|k| vec![format!("Barang {k}"), String::new()]));
        let t0 = std::time::Instant::now();
        let (t, _) = to_text(
            &[Sheet {
                name: "Stock".into(),
                hidden: false,
                cells: &rows,
            }],
            usize::MAX,
        );
        assert!(t.contains("| Barang 19999 |  |"));
        assert!(t0.elapsed().as_secs() < 3, "{:?}", t0.elapsed());
    }

    #[test]
    fn round_eleven_cases() {
        // Record ber-qty kosong setelah baris kosong, lalu kepala halaman diulang.
        let t = text(
            "Stock",
            &[
                &["Item", "Qty"],
                &["Module", "10"],
                &["", ""],
                &["Bolt", ""],
                &["Nut", ""],
                &["Item", "Qty"],
                &["Plate", "7"],
            ],
        );
        assert!(!t.contains("› Bolt") && !t.contains("› Nut"), "{t}");
        assert!(t.contains("| Bolt |  |\n| Nut |  |"), "{t}");
        assert!(t.ends_with("| Item | Qty |\n| Plate | 7 |\n\n"), "{t}");

        // Kepala dibatalkan: judul baru ditulis supaya kepala lama tidak diulang chunk_text.
        let mut rows: Vec<Vec<&str>> = vec![vec!["Month", "Qty", "Hours"]];
        rows.extend((0..10).map(|_| vec!["Apr", "10", "8"]));
        rows.push(vec!["Machine summary", "", ""]);
        rows.push(vec!["Machine", "Status", "Note"]);
        rows.extend((0..5).map(|_| vec!["Stasiun", "Ready", "ok"]));
        let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
        let t = text("Prod", &rows);
        assert!(t.contains("| Month | Qty | Hours |\n| --- |"), "{t}");
        let tail = &t[t.rfind("| Apr | 10 | 8 |").unwrap()..];
        assert!(tail.contains("\n## Prod\n| Machine summary |"), "{t}");
        assert!(!tail.contains("| Month | Qty |"), "{t}");

        // Dua tabel menempel tanpa baris kosong: kepala lama dibuang, kepala kedua tidak
        // dipakai (tidak bisa dibedakan dari record teks dengan aman).
        let mut rows: Vec<Vec<&str>> = vec![vec!["Month", "Qty", "Hours"]];
        rows.extend((0..10).map(|_| vec!["Apr", "10", "8"]));
        rows.push(vec!["Machine", "Status", "Note"]);
        rows.extend((0..5).map(|_| vec!["Stasiun", "8", "ok"]));
        let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
        let t = text("Prod", &rows);
        assert!(
            t.contains("\n## Prod\n| Machine | Status | Note |\n| Stasiun | 8 | ok |"),
            "{t}"
        );
        // Record teks dengan semua kolom terisi tidak membatalkan kepala.
        let t = text(
            "Review",
            &[
                &["Sample", "Result", "Remark"],
                &["S-4", "12", "ok"],
                &["S-5", "Pass", "OK"],
                &["S-6", "13", "ok"],
            ],
        );
        assert!(t.contains("| Sample | Result | Remark |\n| --- | --- | --- |\n| S-4 | 12 | ok |\n| S-5 | Pass | OK |\n| S-6 | 13 | ok |"), "{t}");

        // Formulir panjang di atas tabel yang lebih lebar.
        let mut rows: Vec<Vec<&str>> = vec![vec!["Machine", "Stasiun", ""]];
        for k in [
            "Shift",
            "Line",
            "Order",
            "Target",
            "Speed",
            "Package width",
            "Package weight",
        ] {
            rows.push(vec![k, "3", ""]);
        }
        rows.push(vec!["Production", "", ""]);
        rows.push(vec!["Item", "Qty", "Reject"]);
        rows.push(vec!["Module", "10", "1"]);
        let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
        let t = text("Prod", &rows);
        assert!(!t.contains("| Machine | Stasiun |  |\n| --- |"), "{t}");

        // N/A sendirian setelah baris kosong tetap di kolomnya.
        let t = text(
            "Review",
            &[
                &["Sample", "OD", "Wall"],
                &["S-1", "60.5", "2.3"],
                &["", "", ""],
                &["", "N/A", ""],
            ],
        );
        assert!(t.contains("|  | N/A |  |"), "{t}");
    }

    #[test]
    fn round_twelve_cases() {
        fn after<'a>(t: &'a str, marker: &str) -> &'a str {
            &t[t.find(marker).unwrap()..]
        }
        // Record teks Review tidak pernah jadi kepala.
        let mut rows: Vec<Vec<&str>> = vec![vec!["Item", "Reading", "Inspector"]];
        rows.extend((0..10).map(|_| vec!["Module", "60.5", "Ann"]));
        rows.push(vec!["Elbow", "Pass", "Bob"]);
        rows.push(vec!["Tee", "60.4", "Carol"]);
        let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
        let t = text("Review", &rows);
        assert!(!t.contains("| Elbow | Pass | Bob |\n| --- |"), "{t}");
        assert!(!after(&t, "| Elbow |").contains("| --- |"), "{t}");
        // Tabel menempel yang lebih sempit: kepala lebar lama tidak terbawa.
        let mut rows: Vec<Vec<&str>> = vec![vec!["Sample", "OD", "Wall", "Length"]];
        rows.extend((0..10).map(|_| vec!["S-1", "60.5", "2.3", "6000"]));
        rows.push(vec!["Machine", "Hours", "", ""]);
        rows.extend((0..5).map(|_| vec!["Stasiun", "8", "", ""]));
        let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
        let t = text("Review", &rows);
        assert!(
            after(&t, "| Machine |").starts_with("| Machine | Hours |  |  |\n| Stasiun"),
            "{t}"
        );
        assert!(t.contains("\n## Review\n| Machine |"), "{t}");
        // Satuan yang sama tidak membuat kepala lama terbawa.
        let mut rows: Vec<Vec<&str>> = vec![vec!["Month", "Output", "Unit"]];
        rows.extend((0..10).map(|_| vec!["Apr", "10", "kg"]));
        rows.push(vec!["Item", "Weight", "kg"]);
        rows.push(vec!["Module", "30", "kg"]);
        let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
        let t = text("Prod", &rows);
        assert!(
            t.contains("\n## Prod\n| Item | Weight | kg |\n| Module | 30 | kg |"),
            "{t}"
        );
        // Konteks lama yang tertimpa tabel lain tidak dipakai lagi.
        let t = text(
            "Gudang",
            &[
                &["Sample", "OD", "Wall", "Length"],
                &["S-1", "60.5", "2.3", "6000"],
                &["", "", "", ""],
                &["", "", "Item", "Qty"],
                &["", "", "Module", "N/A"],
                &["", "", "Nut", "10"],
                &["", "", "", ""],
                &["", "", "", "7"],
            ],
        );
        assert!(!after(&t, "| Nut |").contains("Length"), "{t}");
    }

    #[test]
    fn round_thirteen_cases() {
        // Kepala tahun di tengah tabel membuang label dan kepala lama.
        let mut rows: Vec<Vec<&str>> = vec![vec!["Intake", "", ""]];
        rows.push(vec!["Month", "Output", "Hours"]);
        rows.extend((0..10).map(|_| vec!["Apr", "100", "8"]));
        rows.push(vec!["Item", "2025", "2026"]);
        rows.push(vec!["Widget", "10", "12"]);
        let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
        let t = text("Prod", &rows);
        assert!(
            t.contains("\n## Prod\n| Item | 2025 | 2026 |\n| Widget | 10 | 12 |"),
            "{t}"
        );
        // Kepala bernomor bukan record sampel.
        let mut rows: Vec<Vec<String>> = vec![vec!["Sample".into(), "OD".into(), "Wall".into()]];
        rows.extend((1..=10).map(|k| vec![format!("S-{k}"), "60.5".into(), "2.3".into()]));
        rows.push(vec!["Point 1".into(), "Point 2".into(), "Point 3".into()]);
        rows.push(vec!["60.1".into(), "60.2".into(), "60.3".into()]);
        let (t, _) = to_text(
            &[Sheet {
                name: "Review".into(),
                hidden: false,
                cells: &rows,
            }],
            usize::MAX,
        );
        assert!(
            t.contains("\n## Review\n| Point 1 | Point 2 | Point 3 |\n| 60.1 |"),
            "{t}"
        );
        // Record terisolasi tidak jadi label item berikutnya.
        let t = text(
            "Report",
            &[
                &["Item", "Qty"],
                &["Module", "10"],
                &["", ""],
                &["Bolt", ""],
                &["", ""],
                &["Nut", "12"],
            ],
        );
        assert!(!t.contains("› Bolt"), "{t}");
        assert!(t.contains("| Bolt |  |"), "{t}");
        assert!(t.ends_with("| Nut | 12 |\n\n"), "{t}");
    }

    #[test]
    fn round_fourteen_cases() {
        // Record Review setelah baris kosong tidak jadi kepala.
        let t = text(
            "Review",
            &[
                &["Item", "Reading", "Inspector"],
                &["Module", "60.5", "Ann"],
                &["", "", ""],
                &["Elbow", "Pass", "Bob"],
                &["Tee", "60.4", "Carol"],
            ],
        );
        assert!(!t.contains("| Elbow | Pass | Bob |\n| --- |"), "{t}");
        assert!(
            t.contains("| Elbow | Pass | Bob |\n| Tee | 60.4 | Carol |"),
            "{t}"
        );
        // Teks di kolom dalam membuang label lama dan tetap sejajar.
        let t = text(
            "Prod",
            &[
                &["Intake", "", ""],
                &["Month", "Output", "Hours"],
                &["Apr", "100", "8"],
                &["", "", ""],
                &["", "Dispatch", ""],
                &["", "", ""],
                &["May", "200", "9"],
            ],
        );
        assert!(t.contains("|  | Dispatch |  |"), "{t}");
        assert!(
            t.ends_with(
                "## Prod\n| Month | Output | Hours |\n| --- | --- | --- |\n| May | 200 | 9 |\n\n"
            ),
            "{t}"
        );
        let t = text(
            "Review",
            &[
                &["Sample", "Result", "Note"],
                &["S-1", "12", "ok"],
                &["", "", ""],
                &["", "Fail", ""],
                &["", "", ""],
                &["", "Visual", "Remark"],
                &["", "Crack", "7"],
            ],
        );
        assert!(t.contains("|  | Fail |  |"), "{t}");
        assert!(!t.contains("› Fail"), "{t}");
    }

    #[test]
    fn round_fifteen_cases() {
        // Kolom Note kosong di blok lanjutan tidak membuat record jadi kepala.
        let t = text(
            "Review",
            &[
                &["Item", "Reading", "Inspector", "Note"],
                &["Module", "60.5", "Ann", "ok"],
                &["", "", "", ""],
                &["Elbow", "Pass", "Bob", ""],
                &["Tee", "60.4", "Carol", ""],
            ],
        );
        assert!(!t.contains("| Elbow | Pass | Bob |  |\n| --- |"), "{t}");
        assert!(t.contains("| Elbow | Pass | Bob |  |"), "{t}");
        // Record jarang lalu record teks: bukan label dan bukan kepala, mid-table maupun setelah
        // baris kosong.
        for blank in [false, true] {
            let mut rows: Vec<Vec<&str>> = vec![vec!["Item", "Reading", "Inspector"]];
            rows.extend((0..10).map(|_| vec!["Module", "60.5", "Ann"]));
            if blank {
                rows.push(vec!["", "", ""]);
            }
            rows.push(vec!["Bolt", "", ""]);
            rows.push(vec!["Elbow", "Pass", "Bob"]);
            rows.push(vec!["Tee", "60.4", "Carol"]);
            let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
            let t = text("Review", &rows);
            assert!(!t.contains("› Bolt"), "{t}");
            assert!(!t.contains("| Elbow | Pass | Bob |\n| --- |"), "{t}");
            assert!(t.contains("| Bolt |  |  |"), "{t}");
        }
        // Judul di kolom dalam yang langsung diikuti data membuang label lama.
        let t = text(
            "Prod",
            &[
                &["Intake", "", ""],
                &["Month", "Output", "Hours"],
                &["Apr", "100", "8"],
                &["", "", ""],
                &["", "Dispatch", ""],
                &["May", "200", "9"],
            ],
        );
        let tail = &t[t.find("|  | Dispatch |  |").unwrap()..];
        assert!(!tail.contains("Intake"), "{t}");
        assert!(tail.contains("| May | 200 | 9 |"), "{t}");
    }

    #[test]
    fn round_sixteen_cases() {
        // Kolom tengah yang kosong tidak memecah record jadi label dan kepala.
        let t = text(
            "Review",
            &[
                &["Item", "Reading", "Note", "Inspector", "Shift"],
                &["Module", "60.5", "ok", "Ann", "1"],
                &["", "", "", "", ""],
                &["Bolt", "", "", "Bob", ""],
                &["Elbow", "Pass", "", "Carol", "Night"],
                &["Tee", "60.4", "", "Dave", "2"],
            ],
        );
        assert!(!t.contains("› Bolt") && !t.contains("› Bob"), "{t}");
        assert!(t.contains("| Bolt |  |  | Bob |  |"), "{t}");
        assert!(t.contains("| Elbow | Pass |  | Carol | Night |"), "{t}");
        assert!(!t.contains("| Elbow | Pass |\n| --- |"), "{t}");
        // Label periode di kolom B membuang label FY lama.
        let t = text(
            "Prod",
            &[
                &["Intake", "", ""],
                &["FY 2026", "", ""],
                &["Month", "Output", "Hours"],
                &["Apr", "100", "8"],
                &["", "", ""],
                &["", "FY 2025", ""],
                &["", "", ""],
                &["Month", "Output", "Hours"],
                &["May", "200", "9"],
            ],
        );
        let tail = &t[t.find("| FY 2025 |").unwrap()..];
        assert!(!tail.contains("FY 2026"), "{t}");
        // Setelah kepala dibuang, record jarang tetap bukan label.
        let t = text(
            "Synthetic",
            &[
                &["Item", "Qty"],
                &["Module", "10"],
                &["", ""],
                &["Elbow", "Hold"],
                &["", ""],
                &["Bolt", ""],
                &["", ""],
                &["Nut", "7"],
            ],
        );
        assert!(!t.contains("› Bolt"), "{t}");
        assert!(t.contains("| Bolt |  |"), "{t}");
        // Formulir panjang dua kolom tidak jadi kepala tabel.
        let mut rows: Vec<Vec<&str>> = vec![vec!["Machine", "Stasiun"]];
        for k in [
            "Shift",
            "Line",
            "Order",
            "Target",
            "Speed",
            "Package width",
            "Package weight",
        ] {
            rows.push(vec![k, "3"]);
        }
        rows.push(vec!["Production", ""]);
        rows.push(vec!["Item", "Qty"]);
        rows.push(vec!["Module", "10"]);
        let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
        let t = text("Prod", &rows);
        assert!(!t.contains("| Machine | Stasiun |\n| --- |"), "{t}");
    }

    #[test]
    fn round_seventeen_cases() {
        // Periode di kolom dalam blok campuran tidak jadi label.
        let t = text(
            "Review",
            &[
                &["FY 2026", "", ""],
                &["Item", "Reading", "Note"],
                &["Module", "60.5", "ok"],
                &["", "", ""],
                &["Bolt", "", ""],
                &["", "FY 2025", ""],
                &["", "", ""],
                &["Elbow", "60.2", "ok"],
            ],
        );
        assert!(!t.contains("› Bolt"), "{t}");
        assert!(!t.contains("Bolt › FY 2025"), "{t}");
        assert!(t.contains("| Bolt |  |  |"), "{t}");
        assert!(t.contains("|  | FY 2025 |  |"), "{t}");
        assert!(t.contains("| Elbow | 60.2 | ok |"), "{t}");
        // Lanjutan tabel berdampingan tidak mewarisi label tabel kiri.
        let t = text(
            "Prod",
            &[
                &["Intake", "", "", "", "Dispatch", "", ""],
                &["FY 2026", "", "", "", "FY 2026", "", ""],
                &["Month", "Output", "Hours", "", "Month", "Output", "Hours"],
                &["Apr", "10", "8", "", "Apr", "20", "9"],
                &["", "", "", "", "", "", ""],
                &["FY 2025", "", "", "", "", "", ""],
                &["", "", "", "", "", "", ""],
                &["May", "50", "7", "", "May", "60", "6"],
            ],
        );
        assert!(!t.contains("| May | 50 | 7 |  | May |"), "{t}");
        assert!(
            t.contains("## Prod › Intake › FY 2025\n| Month | Output | Hours |\n| --- | --- | --- |\n| May | 50 | 7 |"),
            "{t}"
        );
        // Periode kanan tidak jelas (FY 2026 lama atau FY 2025 baru): dibuang, bagian tetap.
        assert!(
            t.ends_with("## Prod › Dispatch\n| Month | Output | Hours |\n| --- | --- | --- |\n| May | 60 | 6 |\n\n"),
            "{t}"
        );
        // Formulir dua kolom dan daftar tanpa kepala tidak dapat kepala palsu.
        let t = text(
            "Form",
            &[
                &["Machine", "Stasiun"],
                &["Shift", "2"],
                &["Line", "3"],
                &["Target", "5000"],
                &["Actual", "4800"],
            ],
        );
        assert!(!t.contains("| --- |"), "{t}");
        assert!(t.contains("| Machine | Stasiun |\n| Shift | 2 |"), "{t}");
        let t = text(
            "Stock",
            &[&["Module", "On order"], &["Bolt", "12"], &["Nut", "7"]],
        );
        assert!(!t.contains("| --- |"), "{t}");
        assert!(t.contains("| Module | On order |\n| Bolt | 12 |"), "{t}");
    }

    #[test]
    fn round_eighteen_cases() {
        let side = |extra: &[&[&str]]| {
            let mut rows: Vec<&[&str]> = vec![
                &["Intake", "", "", "", "Dispatch", "", ""],
                &["FY 2026", "", "", "", "FY 2026", "", ""],
                &["Month", "Output", "Hours", "", "Month", "Output", "Hours"],
                &["Apr", "10", "8", "", "Apr", "20", "9"],
                &["", "", "", "", "", "", ""],
            ];
            rows.extend_from_slice(extra);
            text("Prod", &rows)
        };
        let head = "| Month | Output | Hours |\n| --- | --- | --- |\n";
        // Periode di kiri langsung di atas data: periode kanan tidak diwarisi.
        let t = side(&[
            &["FY 2025", "", "", "", "", "", ""],
            &["May", "50", "7", "", "May", "60", "6"],
        ]);
        assert!(
            t.contains(&format!(
                "## Prod › Intake › FY 2025\n{head}| May | 50 | 7 |"
            )),
            "{t}"
        );
        assert!(
            t.ends_with(&format!("## Prod › Dispatch\n{head}| May | 60 | 6 |\n\n")),
            "{t}"
        );
        // Periode hanya di kanan: tabel kiri (ditulis lebih dulu) juga tidak mewarisi.
        let t = side(&[
            &["", "", "", "", "FY 2025", "", ""],
            &["May", "50", "7", "", "May", "60", "6"],
        ]);
        assert!(
            t.contains(&format!("## Prod › Intake\n{head}| May | 50 | 7 |")),
            "{t}"
        );
        assert!(
            t.ends_with(&format!(
                "## Prod › Dispatch › FY 2025\n{head}| May | 60 | 6 |\n\n"
            )),
            "{t}"
        );
        // Periode di kedua sisi, dipisah baris kosong: setiap tabel tetap punya label dan
        // kepalanya sendiri.
        let t = side(&[
            &["FY 2025", "", "", "", "FY 2025", "", ""],
            &["", "", "", "", "", "", ""],
            &["May", "50", "7", "", "May", "60", "6"],
        ]);
        assert!(
            t.contains(&format!(
                "## Prod › Intake › FY 2025\n{head}| May | 50 | 7 |"
            )),
            "{t}"
        );
        assert!(
            t.ends_with(&format!(
                "## Prod › Dispatch › FY 2025\n{head}| May | 60 | 6 |\n\n"
            )),
            "{t}"
        );
        // Daftar stok tanpa kepala dan formulir tiga kolom tidak dapat kepala palsu.
        let t = text(
            "Stock",
            &[
                &["Module", "On order", "Urgent"],
                &["Bolt", "12", "Normal"],
                &["Nut", "7", "Normal"],
            ],
        );
        assert!(!t.contains("| --- |"), "{t}");
        assert!(
            t.contains("| Module | On order | Urgent |\n| Bolt | 12 | Normal |"),
            "{t}"
        );
        let t = text(
            "Spec",
            &[
                &["Material", "AX100", "STD"],
                &["Thickness", "2.3", "mm"],
                &["Width", "1219", "mm"],
            ],
        );
        assert!(!t.contains("| --- |"), "{t}");
        // Kepala dengan nama kolom umum, termasuk kolom angkanya, tetap dikenali.
        let t = text(
            "Stock",
            &[
                &["Item", "Qty", "Priority"],
                &["Bolt", "12", "Normal"],
                &["Nut", "7", "Normal"],
            ],
        );
        assert!(
            t.contains("| Item | Qty | Priority |\n| --- | --- | --- |\n| Bolt | 12 | Normal |"),
            "{t}"
        );
        // Kolom angkanya tanpa nama kolom umum ("On order"): sama bentuknya dengan record
        // roster, jadi tanpa kepala.
        let t = text(
            "Stock",
            &[
                &["Item", "On order", "Priority"],
                &["Bolt", "12", "Normal"],
                &["Nut", "7", "Normal"],
            ],
        );
        assert!(!t.contains("| --- |"), "{t}");
        assert!(is_column_name("Part No.") && is_column_name("Qty (pcs)"));
        assert!(is_column_name("Nama Barang") && is_column_name("数量"));
        assert!(!is_column_name("On order") && !is_column_name("Urgent"));
    }

    #[test]
    fn round_nineteen_cases() {
        let head = "| Month | Output | Hours |\n| --- | --- | --- |\n";
        // Periode baru di tengah satu tabel, tanpa baris kosong: tabel sebelahnya dipotong di
        // baris itu dan lanjutannya tanpa periode lama. Kiri atau kanan sama saja.
        for left_side in [true, false] {
            let fy = if left_side {
                ["FY 2025", "", "", "", "", "", ""]
            } else {
                ["", "", "", "", "FY 2025", "", ""]
            };
            let rows: Vec<&[&str]> = vec![
                &["Intake", "", "", "", "Dispatch", "", ""],
                &["FY 2026", "", "", "", "FY 2026", "", ""],
                &["Month", "Output", "Hours", "", "Month", "Output", "Hours"],
                &["Apr", "10", "8", "", "Apr", "20", "9"],
                &fy,
                &["May", "50", "7", "", "May", "60", "6"],
            ];
            let t = text("Prod", &rows);
            let (changed, other, values) = if left_side {
                ("Intake", "Dispatch", "| May | 60 | 6 |")
            } else {
                ("Dispatch", "Intake", "| May | 50 | 7 |")
            };
            assert!(
                t.contains(&format!("## Prod › {other}\n{head}{values}")),
                "{t}"
            );
            assert!(
                t.contains(&format!("## Prod › {changed} › FY 2025\n")),
                "{t}"
            );
            let stale = format!("## Prod › {other} › FY 2026\n{head}{values}");
            assert!(!t.contains(&stale), "{t}");
        }
        // Pergantian nama bulan di satu sisi.
        let t = text(
            "Prod",
            &[
                &["Intake", "", "", "", "Dispatch", "", ""],
                &["October 2026", "", "", "", "October 2026", "", ""],
                &["Day", "Output", "Hours", "", "Day", "Output", "Hours"],
                &["1", "10", "8", "", "1", "20", "9"],
                &["", "", "", "", "", "", ""],
                &["November 2026", "", "", "", "", "", ""],
                &["1", "50", "7", "", "1", "60", "6"],
            ],
        );
        assert!(
            !t.contains(
                "Dispatch › October 2026\n| Day | Output | Hours |\n| --- | --- | --- |\n| 1 | 60 |"
            ),
            "{t}"
        );
        assert!(t.contains("## Prod › Intake › November 2026\n"), "{t}");
        assert!(t.contains("## Prod › Dispatch\n"), "{t}");
        // Formulir dan record dengan satu nama kolom umum saja: tanpa kepala.
        let t = text(
            "Review",
            &[
                &["Material", "AX100", "STD"],
                &["OD", "60.5", "mm"],
                &["ID", "55.9", "mm"],
            ],
        );
        assert!(!t.contains("| --- |"), "{t}");
        let t = text(
            "Stock",
            &[
                &["Machine oil", "On order", "Urgent"],
                &["Grease", "12", "Normal"],
                &["Coolant", "7", "Normal"],
            ],
        );
        assert!(!t.contains("| --- |"), "{t}");
        // Kepala Review khusus tetap dikenali.
        let t = text(
            "Review",
            &[
                &["Test piece", "OD", "UTS"],
                &["S-1", "60.5", "410"],
                &["S-2", "60.4", "420"],
            ],
        );
        assert!(
            t.contains("| Test piece | OD | UTS |\n| --- | --- | --- |\n| S-1 | 60.5 | 410 |"),
            "{t}"
        );
        assert!(is_period("FY 2025") && !is_period("November") && !is_period("Dispatch"));
        assert_eq!(shape("October 2026"), shape("November 2026"));
    }

    #[test]
    fn round_twenty_cases() {
        let head = "| Month | Output | Hours |
| --- | --- | --- |
";
        // Periode di atas nama bagian: lanjutan tabel sebelah membuang periodenya, bagian tetap.
        let t = text(
            "Prod",
            &[
                &["FY 2026", "", "", "", "FY 2026", "", ""],
                &["Intake", "", "", "", "Dispatch", "", ""],
                &["Month", "Output", "Hours", "", "Month", "Output", "Hours"],
                &["Apr", "10", "8", "", "Apr", "20", "9"],
                &["FY 2025", "", "", "", "", "", ""],
                &["May", "50", "7", "", "May", "60", "6"],
            ],
        );
        assert!(
            !t.contains(&format!(
                "FY 2026 › Dispatch
{head}| May | 60 | 6 |"
            )),
            "{t}"
        );
        assert!(
            t.contains(&format!(
                "## Prod › Dispatch
{head}| May | 60 | 6 |"
            )),
            "{t}"
        );
        // Nama staf "May" bukan periode setelah label "April".
        for blank in [false, true] {
            let mut rows: Vec<&[&str]> = vec![
                &["April", "", ""],
                &["Name", "Output", "Hours"],
                &["Ann", "10", "8"],
            ];
            if blank {
                rows.push(&["", "", ""]);
            }
            rows.push(&["May", "", ""]);
            rows.push(&["Bob", "20", "9"]);
            let t = text("Staff", &rows);
            assert!(!t.contains("› May"), "{t}");
            assert!(t.contains("| May |  |  |"), "{t}");
            assert!(t.contains("| Bob | 20 | 9 |"), "{t}");
        }
        // Dua kata kosakata yang kebetulan ada di record bukan kepala.
        let t = text(
            "Stock",
            &[
                &["Machine oil", "On order", "High priority"],
                &["Grease", "12", "Normal"],
                &["Coolant", "7", "Normal"],
            ],
        );
        assert!(!t.contains("| --- |"), "{t}");
        // Baris total tidak membatalkan kepala.
        let t = text(
            "Stok",
            &[
                &["Nama barang", "Jumlah", "Satuan"],
                &["Baut", "12", "pcs"],
                &["Mur", "7", "pcs"],
                &["Total", "19", "pcs"],
            ],
        );
        assert!(
            t.contains(
                "| Nama barang | Jumlah | Satuan |
| --- | --- | --- |
| Baut | 12 | pcs |"
            ),
            "{t}"
        );
        assert!(t.contains("| Total | 19 | pcs |"), "{t}");
        assert!(is_column_name("Part No.") && is_column_name("Order Qty"));
        assert!(!is_column_name("Machine oil") && !is_column_name("High priority"));
        assert_ne!(shape("April"), shape("May"));
    }

    #[test]
    fn round_twenty_one_cases() {
        // Kode material berangka bukan kolom periode.
        let t = text(
            "Review",
            &[
                &["Material", "AX100", "STD X1000"],
                &["Thickness", "2.3", "2.2"],
                &["Width", "1219", "1218"],
            ],
        );
        assert!(!t.contains("| --- |"), "{t}");
        assert!(is_period_name("2025") && is_period_name("FY 2025") && is_period_name("FY25"));
        assert!(is_period_name("Q1") && is_period_name("Q1 2026") && is_period_name("Mar 2026"));
        assert!(is_period_name("Jan-26") && is_period_name("2025/2026"));
        assert!(!is_period_name("AX100") && !is_period_name("STD X1000"));
        assert!(!is_period_name("Shift 2") && !is_period_name("Line 3"));
        // Label bagian baru di tabel sebelah tidak mewarisi periode lama di atasnya.
        let t = text(
            "Prod",
            &[
                &["FY 2026", "", "", "", "FY 2026", "", ""],
                &["Intake", "", "", "", "Shift 1", "", ""],
                &["Day", "Output", "Hours", "", "Day", "Output", "Hours"],
                &["1", "10", "8", "", "1", "20", "9"],
                &["", "", "", "", "", "", ""],
                &["FY 2025", "", "", "", "", "", ""],
                &["Intake", "", "", "", "Shift 2", "", ""],
                &["1", "50", "7", "", "1", "60", "6"],
            ],
        );
        assert!(!t.contains("FY 2026 › Shift 2"), "{t}");
        assert!(t.contains("› Shift 2\n"), "{t}");
        // Tabel kiri tidak dipotong di baris "Shift 2" karena punya labelnya sendiri.
        assert!(
            t.contains("## Prod › FY 2025 › Intake\n| Day | Output | Hours |"),
            "{t}"
        );
        // Nama bulan saja juga membatalkan bulan lama di tabel sebelah, dengan atau tanpa baris
        // kosong.
        for blank in [true, false] {
            let mut rows: Vec<&[&str]> = vec![
                &["October", "", "", "", "October", "", ""],
                &["Day", "Output", "Hours", "", "Day", "Output", "Hours"],
                &["1", "10", "8", "", "1", "20", "9"],
            ];
            if blank {
                rows.push(&["", "", "", "", "", "", ""]);
            }
            rows.push(&["November", "", "", "", "", "", ""]);
            rows.push(&["1", "50", "7", "", "1", "60", "6"]);
            let t = text("Prod", &rows);
            let at = t.rfind("| 1 | 60 | 6 |").unwrap();
            let heading = &t[t[..at].rfind("## ").unwrap()..at];
            assert!(!heading.contains("October"), "{t}");
        }
        assert!(may_be_period("November") && may_be_period("Shift 2"));
        assert!(!may_be_period("Dispatch"));
    }

    #[test]
    fn round_twenty_two_cases() {
        // Periode di kiri di samping label bagian baru di kanan, tanpa baris kosong: kanan
        // tidak membawa tahun lama.
        let t = text(
            "Prod",
            &[
                &["FY 2026", "", "", "", "FY 2026", "", ""],
                &["Intake", "", "", "", "Shift 1", "", ""],
                &["Day", "Output", "Hours", "", "Day", "Output", "Hours"],
                &["1", "10", "8", "", "1", "20", "9"],
                &["FY 2025", "", "", "", "Shift 2", "", ""],
                &["2", "50", "7", "", "2", "60", "6"],
            ],
        );
        assert!(!t.contains("FY 2026 › Shift 2"), "{t}");
        let at = t.find("| 2 | 60 | 6 |").unwrap();
        let heading = &t[t[..at].rfind("## ").unwrap()..at];
        assert!(!heading.contains("FY 2026"), "{t}");
        // Roster tanpa kepala: nama departemen dan shift bukan bukti kepala.
        let t = text(
            "Lembur",
            &[
                &["Production", "Day", "Ann", "Pending"],
                &["Review", "Night", "Bob", "2"],
                &["Warehouse", "Day", "Cici", "3"],
            ],
        );
        assert!(!t.contains("| --- |"), "{t}");
        // Baris `---` di data tidak menggantikan kepala tabel di potongan lanjutan.
        let t = text(
            "Review",
            &[
                &["Sample", "OD", "UTS"],
                &["S-001", "60.5", "410"],
                &["---", "---", "---"],
                &["S-002", "60.4", "420"],
            ],
        );
        assert!(
            t.contains("| Sample | OD | UTS |\n| --- | --- | --- |\n"),
            "{t}"
        );
        assert!(t.contains("| \\--- | \\--- | \\--- |\n"), "{t}");
        assert_eq!(
            t.lines().filter(|l| *l == "| --- | --- | --- |").count(),
            1,
            "{t}"
        );
    }

    #[test]
    fn round_twenty_three_cases() {
        // Stok tanpa kepala dengan jumlah berbentuk tahun: tanpa bukti nama kolom teks, bukan
        // kepala.
        let t = text(
            "Stock",
            &[
                &["Module", "2000", "2000"],
                &["Bolt", "500", "480"],
                &["Nut", "600", "600"],
            ],
        );
        assert!(!t.contains("| --- |"), "{t}");
        // Formulir Review dua baris (bahan, lalu Nominal/Actual) bukan kepala bertingkat.
        let t = text(
            "Review",
            &[
                &["Material", "AX100", "", "STD X1000"],
                &["", "Nominal", "Actual", ""],
                &["OD", "60.5", "60.4", "mm"],
                &["Wall", "2.3", "2.2", "mm"],
            ],
        );
        assert!(!t.contains("| --- |"), "{t}");
        assert!(!t.contains("AX100 Nominal"), "{t}");
        // Kepala bertingkat yang sungguhan tetap dikenali.
        let t = text(
            "Prod",
            &[
                &["Month", "Production", ""],
                &["", "Actual", "Plan"],
                &["Apr", "10", "12"],
                &["May", "11", "13"],
            ],
        );
        assert!(
            t.contains("| Month | Production Actual | Production Plan |\n| --- |"),
            "{t}"
        );
    }

    #[test]
    fn round_twenty_four_cases() {
        // Baris ringkasan bukan kepala.
        for total in ["Total", "Jumlah", "合計"] {
            let t = text(
                "Stock",
                &[
                    &[total, "2000", "2000"],
                    &["Bolt", "500", "480"],
                    &["Nut", "600", "600"],
                    &["Washer", "900", "920"],
                ],
            );
            assert!(!t.contains("| --- |"), "{t}");
        }
        // Formulir di bawah tabel operasional tidak meminjam kepala tabel itu.
        let t = text(
            "Prod",
            &[
                &["Month", "Output", "Hours"],
                &["Apr", "10", "8"],
                &["", "", ""],
                &["Date", "17 Nov 2026", ""],
                &["Shift", "2", ""],
                &["Line", "3", ""],
            ],
        );
        assert_eq!(t.matches("| Month | Output | Hours |").count(), 1, "{t}");
        assert!(t.contains("| Date | 17 Nov 2026 |  |"), "{t}");
        // Spesifikasi di bawah daftar stok juga tidak.
        let t = text(
            "Stock",
            &[
                &["Item", "Qty", "Unit"],
                &["Bolt", "12", "pcs"],
                &["", "", ""],
                &["OD", "60.5", "mm"],
                &["Wall", "2.3", "mm"],
            ],
        );
        assert_eq!(t.matches("| Item | Qty | Unit |").count(), 1, "{t}");
        assert!(t.contains("| OD | 60.5 | mm |"), "{t}");
        // Lanjutan data biasa setelah baris kosong tetap memakai kepalanya.
        let t = text(
            "Prod",
            &[
                &["Month", "Output", "Hours"],
                &["Apr", "10", "8"],
                &["", "", ""],
                &["May", "11", "9"],
                &["Total", "21", "17"],
            ],
        );
        assert_eq!(t.matches("| Month | Output | Hours |").count(), 2, "{t}");
    }

    #[test]
    fn round_twenty_five_cases() {
        let head = "| Month | Output | Hours |";
        // Judul satu sel lalu formulir: tidak meminjam kepala tabel operasional.
        let t = text(
            "Prod",
            &[
                &["Month", "Output", "Hours"],
                &["Apr", "10", "8"],
                &["", "", ""],
                &["Setup", "", ""],
                &["Date", "17 Nov 2026", ""],
                &["Shift", "2", ""],
            ],
        );
        assert_eq!(t.matches(head).count(), 1, "{t}");
        assert!(t.contains("| Date | 17 Nov 2026 |  |"), "{t}");
        // Pergantian periode di tengah tabel lalu formulir: kepala tidak diulang di atasnya.
        let t = text(
            "Prod",
            &[
                &["FY 2026", "", ""],
                &["Month", "Output", "Hours"],
                &["Apr", "10", "8"],
                &["FY 2025", "", ""],
                &["Date", "17 Nov 2025", ""],
                &["Shift", "2", ""],
            ],
        );
        assert_eq!(t.matches(head).count(), 1, "{t}");
        assert!(t.contains("| Date | 17 Nov 2025 |  |"), "{t}");
        // Lanjutan berisi "-" tidak menghapus syarat kolom angka: spesifikasi sesudahnya tidak
        // meminjam kepala.
        let t = text(
            "Prod",
            &[
                &["Month", "Output", "Hours"],
                &["Apr", "10", "8"],
                &["", "", ""],
                &["May", "-", "-"],
                &["", "", ""],
                &["Speed", "120", "m/min"],
                &["Pressure", "5", "bar"],
            ],
        );
        assert_eq!(t.matches(head).count(), 2, "{t}");
        let speed = t.find("| Speed |").unwrap();
        assert!(
            !t[t[..speed].rfind("## ").unwrap()..speed].contains(head),
            "{t}"
        );
        // Periode berikutnya dengan data biasa tetap memakai kepalanya.
        let t = text(
            "Prod",
            &[
                &["FY 2026", "", ""],
                &["Month", "Output", "Hours"],
                &["Apr", "10", "8"],
                &["FY 2025", "", ""],
                &["Apr", "9", "7"],
            ],
        );
        assert_eq!(t.matches(head).count(), 2, "{t}");
    }

    #[test]
    fn round_twenty_six_cases() {
        let head = "| Month | Output | Hours |";
        // Judul di tengah tabel tanpa baris kosong lalu formulir: formulir tidak di bawah kepala
        // operasional, di kolom pertama maupun kolom kedua, dengan atau tanpa label periode.
        for (title, with_period) in [
            (["Setup", "", ""], false),
            (["Setup", "", ""], true),
            (["", "Setup", ""], false),
        ] {
            let mut rows: Vec<&[&str]> = Vec::new();
            if with_period {
                rows.push(&["FY 2026", "", ""]);
            }
            rows.extend_from_slice(&[
                &["Month", "Output", "Hours"],
                &["Apr", "10", "8"],
                &title,
                &["Date", "17 Nov 2026", ""],
                &["Shift", "2", ""],
            ]);
            let t = text("Prod", &rows);
            let date = t.find("| Date |").unwrap();
            let section = &t[t[..date].rfind("## ").unwrap()..date];
            assert!(!section.contains(head), "{t}");
        }
        // Judul satu sel di blok sendiri tidak menghapus syarat kolom angka.
        let t = text(
            "Stock",
            &[
                &["Item", "Qty", "Unit"],
                &["Bolt", "12", "pcs"],
                &["", "", ""],
                &["", "Setup", ""],
                &["", "", ""],
                &["Coating", "Galvanized", "-"],
                &["Lubrication", "Manual", "-"],
            ],
        );
        let coating = t.find("| Coating |").unwrap();
        let section = &t[t[..coating].rfind("## ").unwrap()..coating];
        assert!(!section.contains("| Item | Qty | Unit |"), "{t}");
        // Kepala yang diulang persis di lanjutan tidak membuat lanjutannya ditolak.
        let t = text(
            "Prod",
            &[
                &["Month", "Output", "Hours"],
                &["Apr", "10", "8"],
                &["", "", ""],
                &["May", "11", "9"],
                &["Month", "Output", "Hours"],
                &["Jun", "12", "10"],
            ],
        );
        let may = t.find("| May | 11 | 9 |").unwrap();
        assert!(t[t[..may].rfind("## ").unwrap()..may].contains(head), "{t}");
    }

    #[test]
    #[ignore = "tes performa: cargo test --release -- --ignored stay_fast"]
    fn many_period_blocks_stay_fast() {
        // 4.000 blok hari berurutan (label periode lalu 10 baris mesin): pengecekan kepala di
        // setiap pergantian periode tidak boleh kuadratik.
        let mut cells: Vec<Vec<String>> = vec![
            vec!["Day 1".into(), String::new(), String::new()],
            vec!["Machine".into(), "Output".into(), "Hours".into()],
        ];
        for day in 1..=4_000 {
            if day > 1 {
                cells.push(vec![format!("Day {day}"), String::new(), String::new()]);
            }
            for m in 0..10 {
                cells.push(vec![format!("M-{m}"), format!("{}", day + m), "8".into()]);
            }
        }
        let t0 = std::time::Instant::now();
        let (t, _) = to_text(
            &[Sheet {
                name: "Harian".into(),
                hidden: false,
                cells: &cells,
            }],
            usize::MAX,
        );
        assert!(t.contains("| M-9 | 4009 | 8 |"));
        assert!(t0.elapsed().as_secs() < 5, "{:?}", t0.elapsed());
    }

    #[test]
    fn round_twenty_seven_cases() {
        let head = "| Month | Output | Hours |";
        let section_of = |t: &str, needle: &str| -> String {
            let at = t.find(needle).unwrap();
            t[t[..at].rfind("## ").unwrap()..at].to_string()
        };
        // Formulir langsung di bawah data operasional (tanpa baris kosong) tidak di bawah kepala
        // dan label operasional.
        let mut rows: Vec<&[&str]> = vec![&["FY 2026", "", ""], &["Month", "Output", "Hours"]];
        for _ in 0..8 {
            rows.push(&["Apr", "10", "8"]);
        }
        rows.push(&["Date", "17 Nov 2026", ""]);
        rows.push(&["Shift", "2", ""]);
        let t = text("Prod", &rows);
        let section = section_of(&t, "| Date |");
        assert!(
            !section.contains(head) && !section.contains("FY 2026"),
            "{t}"
        );
        // Judul, kepala diulang persis, lalu formulir; dan judul, sel angka tunggal, lalu
        // spesifikasi.
        for middle in [
            vec![&["Setup", "", ""][..], &["Month", "Output", "Hours"][..]],
            vec![&["Setup", "", ""][..], &["", "5", ""][..]],
        ] {
            let mut rows: Vec<&[&str]> = vec![&["Month", "Output", "Hours"], &["Apr", "10", "8"]];
            rows.extend(middle);
            rows.push(&["Date", "17 Nov 2026", ""]);
            rows.push(&["Speed", "120", "m/min"]);
            let t = text("Prod", &rows);
            assert!(!section_of(&t, "| Date |").contains(head), "{t}");
        }
        // Baris total tidak menghapus syarat kolom angka.
        let t = text(
            "Stock",
            &[
                &["Item", "Qty", "Amount"],
                &["Bolt", "12", "120"],
                &["Nut", "7", "70"],
                &["Total", "19 pcs", "190"],
                &["", "", ""],
                &["Coating", "Galvanized", "-"],
                &["Lubrication", "Manual", "-"],
            ],
        );
        assert!(
            !section_of(&t, "| Coating |").contains("| Item | Qty | Amount |"),
            "{t}"
        );
        // Tabel spesifikasi sungguhan yang kolom pertamanya nama kolom umum tetap utuh.
        let t = text(
            "Spec",
            &[
                &["Parameter", "Value", "Unit"],
                &["Thickness", "2.3", "mm"],
                &["Width", "1219", "mm"],
            ],
        );
        assert!(t.contains("| Width | 1219 | mm |"), "{t}");
    }

    #[test]
    fn round_twenty_eight_cases() {
        let section_of = |t: &str, needle: &str| -> String {
            let at = t.find(needle).unwrap();
            t[t[..at].rfind("## ").unwrap()..at].to_string()
        };
        // Spesifikasi yang mirip kepala (angka seperti tahun) langsung di bawah data operasional.
        let mut rows: Vec<&[&str]> = vec![&["Month", "Output", "Hours"]];
        for _ in 0..8 {
            rows.push(&["Apr", "10", "8"]);
        }
        rows.push(&["Width", "2000", "mm"]);
        rows.push(&["Length", "2000", "mm"]);
        let t = text("Prod", &rows);
        assert!(
            !section_of(&t, "| Width |").contains("| Month | Output | Hours |"),
            "{t}"
        );
        // Tabel tanpa kepala (dua kolom) tidak membawa label lamanya ke formulir.
        let mut rows: Vec<&[&str]> = vec![&["November 2026", ""], &["Day", "Output"]];
        let days: Vec<String> = (1..=8).map(|d| d.to_string()).collect();
        let grid_rows: Vec<[&str; 2]> = days.iter().map(|d| [d.as_str(), "50"]).collect();
        for r in &grid_rows {
            rows.push(r);
        }
        rows.push(&["Date", "17 Nov 2025"]);
        rows.push(&["Shift", "2"]);
        let t = text("Prod", &rows);
        assert!(!section_of(&t, "| Date |").contains("November 2026"), "{t}");
    }

    #[test]
    fn round_twenty_nine_cases() {
        let section_of = |t: &str, needle: &str| -> String {
            let at = t.find(needle).unwrap();
            t[t[..at].rfind("## ").unwrap()..at].to_string()
        };
        let owned = |rows: &[Vec<String>]| -> String {
            let refs: Vec<Vec<&str>> = rows
                .iter()
                .map(|r| r.iter().map(String::as_str).collect())
                .collect();
            let refs: Vec<&[&str]> = refs.iter().map(Vec::as_slice).collect();
            text("Prod", &refs)
        };
        let row = |cells: &[&str]| cells.iter().map(|c| c.to_string()).collect::<Vec<_>>();
        // Kunci formulir "Tim 1" bukan identitas record: setup di bawah data operasional tidak
        // memakai kepalanya.
        let mut rows = vec![
            row(&["November 2026", "", ""]),
            row(&["Line", "Output", "Hours"]),
        ];
        for n in 1..=8 {
            rows.push(row(&[&format!("Tim {n}"), "50", "8"]));
        }
        rows.push(row(&["Tim 1", "Stasiun A", "Active"]));
        rows.push(row(&["Tim 2", "Stasiun B", "Standby"]));
        let t = owned(&rows);
        let section = section_of(&t, "| Tim 1 | Stasiun A |");
        assert!(
            !section.contains("| Line | Output | Hours |") && !section.contains("November 2026"),
            "{t}"
        );
        // Record dengan jumlah seperti tahun tetap membentuk profil.
        let mut rows = vec![
            row(&["November 2026", "", ""]),
            row(&["Month", "Output", "Unit"]),
        ];
        for n in 0..8 {
            rows.push(row(&["Apr", &format!("{}", 2000 + n), "pcs"]));
        }
        rows.push(row(&["Width", "2000", "mm"]));
        let t = owned(&rows);
        assert!(
            !section_of(&t, "| Width |").contains("| Month | Output | Unit |"),
            "{t}"
        );
        // Profil lanjutan hanya dari seksi terakhir, bukan seluruh blok.
        let mut rows = vec![row(&["FY 2026", "", ""]), row(&["Month", "Output", "Unit"])];
        for _ in 0..8 {
            rows.push(row(&["Apr", "10", "pcs"]));
        }
        rows.push(row(&["FY 2025", "", ""]));
        rows.push(row(&["Month", "Output", "Hours"]));
        for _ in 0..8 {
            rows.push(row(&["Apr", "10", "8"]));
        }
        rows.push(row(&["", "", ""]));
        rows.push(row(&["Cutting speed", "120", "m/min"]));
        rows.push(row(&["Air pressure", "5", "bar"]));
        let t = owned(&rows);
        let section = section_of(&t, "| Cutting speed |");
        assert!(
            !section.contains("| Month | Output | Hours |") && !section.contains("FY 2025"),
            "{t}"
        );
        // Baris total berunit tidak membatalkan kepala.
        let t = text(
            "Stock",
            &[
                &["Item", "Qty", "Unit"],
                &["Bolt", "12", "pcs"],
                &["Nut", "7", "pcs"],
                &["Total", "19 pcs", ""],
            ],
        );
        assert!(t.contains("| Item | Qty | Unit |\n| --- |"), "{t}");
    }

    #[test]
    fn round_thirty_cases() {
        let section_of = |t: &str, needle: &str| -> String {
            let at = t.find(needle).unwrap();
            t[t[..at].rfind("## ").unwrap()..at].to_string()
        };
        // Roster dengan nama departemen campuran: formulir sesudahnya bukan bagian roster, tapi
        // departemen yang berulang tetap record roster.
        let t = text(
            "Roster",
            &[
                &["November 2026", "", "", ""],
                &["Department", "Shift", "Name", "Hours"],
                &["Production", "Day", "Ann", "8"],
                &["Sales", "Day", "Bob", "8"],
                &["Review", "Night", "Cici", "8"],
                &["Production", "Night", "Dan", "8"],
                &["Date", "17 Nov 2025", "", ""],
            ],
        );
        let roster = "| Department | Shift | Name | Hours |";
        assert!(
            section_of(&t, "| Production | Night | Dan |").contains(roster),
            "{t}"
        );
        let form = section_of(&t, "| Date |");
        assert!(
            !form.contains(roster) && !form.contains("November 2026"),
            "{t}"
        );
        // Spesifikasi berbahasa Indonesia dan Jepang tidak meminjam kepala stok.
        for (head, rows) in [
            (
                "| Nama Barang | Jumlah | Satuan |",
                [
                    ["Nama Barang", "Jumlah", "Satuan"],
                    ["Baut", "12", "pcs"],
                    ["Kecepatan", "120", "m/min"],
                    ["Tekanan", "5", "bar"],
                ],
            ),
            (
                "| 品番 | 数量 | 単位 |",
                [
                    ["品番", "数量", "単位"],
                    ["A-01", "12", "pcs"],
                    ["外径", "60.5", "mm"],
                    ["内径", "55.9", "mm"],
                ],
            ),
        ] {
            let grid: Vec<&[&str]> = vec![
                &rows[0][..],
                &rows[1][..],
                &["", "", ""][..],
                &rows[2][..],
                &rows[3][..],
            ];
            let t = text("Stok", &grid);
            assert!(
                !section_of(&t, &format!("| {} |", rows[2][0])).contains(head),
                "{t}"
            );
        }
        // Judul di A1 tidak dipakai formulir di kolom A setelah tabel di kolom C.
        let t = text(
            "Gudang",
            &[
                &["November 2026", "", "", "", ""],
                &["", "", "", "", ""],
                &["", "", "Item", "Qty", "Unit"],
                &["", "", "Bolt", "12", "pcs"],
                &["", "", "", "", ""],
                &["Date", "17 Nov 2025", "", "", ""],
                &["Shift", "2", "", "", ""],
            ],
        );
        assert!(!section_of(&t, "| Date |").contains("November 2026"), "{t}");
        // Total berunit tidak membuang kepala untuk baris sesudahnya.
        let t = text(
            "Stock",
            &[
                &["Item", "Qty", "Unit"],
                &["Bolt", "12", "pcs"],
                &["Nut", "7", "pcs"],
                &["Total", "19 pcs", ""],
                &["Washer", "6", "pcs"],
            ],
        );
        assert!(
            section_of(&t, "| Washer |").contains("| Item | Qty | Unit |"),
            "{t}"
        );
        // Kepala yang diulang lalu formulir tidak memulihkan label lama.
        let t = text(
            "Prod",
            &[
                &["November 2026", "", ""],
                &["Month", "Output", "Hours"],
                &["Apr", "10", "8"],
                &["", "", ""],
                &["Month", "Output", "Hours"],
                &["Date", "17 Nov 2025", ""],
                &["Shift", "2", ""],
            ],
        );
        assert!(!section_of(&t, "| Date |").contains("November 2026"), "{t}");
    }

    #[test]
    fn round_thirty_one_cases() {
        let section_of = |t: &str, needle: &str| -> String {
            let at = t.find(needle).unwrap();
            t[t[..at].rfind("## ").unwrap()..at].to_string()
        };
        // Kunci parameter majemuk (Inggris, Indonesia, Jepang), dengan dan tanpa baris kosong,
        // tidak meminjam kepala stok.
        for (a, b) in [
            ("Cutting speed", "Air pressure"),
            ("Kecepatan potong", "Tekanan udara"),
            ("切断速度", "空気圧力"),
        ] {
            for blank in [true, false] {
                let mut rows: Vec<&[&str]> = vec![&["Item", "Qty", "Unit"], &["Bolt", "12", "pcs"]];
                if blank {
                    rows.push(&["", "", ""]);
                }
                let (ra, rb) = ([a, "120", "m/min"], [b, "5", "bar"]);
                rows.push(&ra);
                rows.push(&rb);
                let t = text("Stock", &rows);
                assert!(
                    !section_of(&t, &format!("| {a} |")).contains("| Item | Qty | Unit |"),
                    "{t}"
                );
            }
        }
        assert!(is_form_key("Cutting speed") && is_form_key("Kecepatan potong"));
        assert!(is_form_key("切断速度") && !is_form_key("Bolt"));
        // Label anak tidak mewarisi tanggal induk untuk formulir.
        let t = text(
            "Prod",
            &[
                &["November 2026", "", ""],
                &["Shift 1", "", ""],
                &["Month", "Output", "Hours"],
                &["Apr", "10", "8"],
                &["", "", ""],
                &["Shift 2", "", ""],
                &["Date", "17 Nov 2025", ""],
                &["Machine", "Stasiun A", ""],
                &["Speed", "120", ""],
            ],
        );
        assert!(!section_of(&t, "| Date |").contains("November 2026"), "{t}");
        // Record pertama tabel tanpa kepala yang mirip kepala tetap menjaga label.
        for first in [
            &["Bolt", "2000", "pcs", ""][..],
            &["Review", "Day", "Ann", "Pending"][..],
        ] {
            let t = text(
                "Prod",
                &[
                    &["November 2026", "", "", ""],
                    first,
                    &["Date", "17 Nov 2025", "", ""],
                    &["Shift", "2", "", ""],
                ],
            );
            assert!(!section_of(&t, "| Date |").contains("November 2026"), "{t}");
        }
    }

    #[test]
    fn round_thirty_two_cases() {
        let section_of = |t: &str, needle: &str| -> String {
            let at = t.find(needle).unwrap();
            t[t[..at].rfind("## ").unwrap()..at].to_string()
        };
        let stock = "| Item | Qty | Unit |";
        // Kunci parameter dengan anotasi satuan, dengan dan tanpa baris kosong.
        for (a, b) in [
            ("Cutting speed (m/min)", "Air pressure (bar)"),
            ("切断速度 (m/min)", "空気圧力 (bar)"),
        ] {
            for blank in [true, false] {
                let mut rows: Vec<&[&str]> = vec![&["Item", "Qty", "Unit"], &["Bolt", "12", "pcs"]];
                if blank {
                    rows.push(&["", "", ""]);
                }
                let (ra, rb) = ([a, "120", "m/min"], [b, "5", "bar"]);
                rows.push(&ra);
                rows.push(&rb);
                let t = text("Stock", &rows);
                assert!(!section_of(&t, &format!("| {a} |")).contains(stock), "{t}");
            }
        }
        // Nama produk berawalan kata parameter Inggris bukan kunci formulir.
        assert!(!is_form_key("Pressure gauge") && !is_form_key("Torque wrench"));
        let t = text(
            "Stock",
            &[
                &["Item", "Qty", "Unit"],
                &["Pressure gauge", "12", "pcs"],
                &["Torque wrench", "3", "pcs"],
                &["Cutting speed", "120", "m/min"],
                &["Air pressure", "5", "bar"],
            ],
        );
        assert!(section_of(&t, "| Torque wrench |").contains(stock), "{t}");
        assert!(!section_of(&t, "| Cutting speed |").contains(stock), "{t}");
        // Baris pertama mirip kepala tetap dicek sebagai kunci formulir saat mewarisi label.
        let t = text(
            "Gudang",
            &[
                &["November 2026", "", ""],
                &["Shift 1", "", ""],
                &["Bolt", "12", "pcs"],
                &["", "", ""],
                &["Shift 2", "", ""],
                &["Year", "FY 2025", ""],
                &["Bolt", "10", "pcs"],
            ],
        );
        assert!(!section_of(&t, "| Year |").contains("November 2026"), "{t}");
        // Tabel parameter sungguhan mempertahankan kepalanya.
        let t = text(
            "Review",
            &[
                &["Parameter", "Nominal", "Actual"],
                &["Feed rate", "100", "99"],
                &["Cutting speed", "120", "119"],
            ],
        );
        assert!(
            section_of(&t, "| Cutting speed |").contains("| Parameter | Nominal | Actual |"),
            "{t}"
        );
    }

    #[test]
    fn round_thirty_three_cases() {
        let section_of = |t: &str, needle: &str| -> String {
            let at = t.find(needle).unwrap();
            t[t[..at].rfind("## ").unwrap()..at].to_string()
        };
        let param = "| Parameter | Nominal | Actual |";
        // Parameter dengan kata serapan Indonesia di depan, dengan dan tanpa baris kosong.
        for blank in [true, false] {
            let mut rows: Vec<&[&str]> = vec![
                &["November 2026", "", ""],
                &["Item", "Qty", "Unit"],
                &["Bolt", "12", "pcs"],
            ];
            if blank {
                rows.push(&["", "", ""]);
            }
            rows.push(&["Diameter luar (mm)", "60.5", "mm"]);
            rows.push(&["Diameter dalam (mm)", "55.9", "mm"]);
            let t = text("Stok", &rows);
            let section = section_of(&t, "| Diameter luar (mm) |");
            assert!(
                !section.contains("| Item | Qty | Unit |") && !section.contains("November 2026"),
                "{t}"
            );
        }
        // Di tabel parameter, metadata formulir ("Shift", "Line") tetap memutus konteks.
        let t = text(
            "Review",
            &[
                &["Parameter", "Nominal", "Actual"],
                &["Feed rate", "100", "99"],
                &["Cutting speed", "120", "119"],
                &["Shift", "2", ""],
                &["Line", "3", ""],
            ],
        );
        assert!(section_of(&t, "| Cutting speed |").contains(param), "{t}");
        assert!(!section_of(&t, "| Shift |").contains(param), "{t}");
        // Lanjutan tabel parameter sesudah baris kosong tetap memakai kepalanya.
        let t = text(
            "Review",
            &[
                &["Parameter", "Nominal", "Actual"],
                &["Feed rate", "100", "99"],
                &["Cutting speed", "120", "119"],
                &["", "", ""],
                &["Air pressure", "5", "4.9"],
                &["Temperature", "70", "68"],
            ],
        );
        assert!(section_of(&t, "| Air pressure |").contains(param), "{t}");
    }

    #[test]
    fn round_thirty_four_cases() {
        let section_of = |t: &str, needle: &str| -> String {
            let at = t.find(needle).unwrap();
            t[t[..at].rfind("## ").unwrap()..at].to_string()
        };
        // Tabel parameter yang semua baris sebelumnya parameter: metadata tetap memutus.
        let t = text(
            "Review",
            &[
                &["Parameter", "Nominal", "Actual"],
                &["Speed", "120", "119"],
                &["Pressure", "5", "4.9"],
                &["Shift", "2", ""],
                &["Line", "3", ""],
            ],
        );
        let param = "| Parameter | Nominal | Actual |";
        assert!(section_of(&t, "| Pressure |").contains(param), "{t}");
        assert!(!section_of(&t, "| Shift |").contains(param), "{t}");
        // Roster jam kerja tahunan tanpa kepala: jam 2000 bukan kolom tahun.
        let t = text(
            "Roster",
            &[
                &["Production", "2000", "Ann"],
                &["Review", "1800", "Bob"],
                &["Engineering", "1840", "Cici"],
                &["HR", "1760", "Dan"],
            ],
        );
        assert!(!t.contains("| --- |"), "{t}");
        // Kepala dua tahun tetap dikenali.
        let t = text(
            "Rekap",
            &[
                &["Item", "2025", "2026"],
                &["Widget", "10", "12"],
                &["Bolt", "7", "9"],
            ],
        );
        assert!(t.contains("| Item | 2025 | 2026 |\n| --- |"), "{t}");
    }

    #[test]
    fn round_thirty_five_cases() {
        let section_of = |t: &str, needle: &str| -> String {
            let at = t.find(needle).unwrap();
            t[t[..at].rfind("## ").unwrap()..at].to_string()
        };
        // Metadata formulir di bawah tabel departemen yang namanya kebetulan nama kolom umum.
        for (head, rows, meta) in [
            (
                ["Department", "Hours", "Cost"],
                [["Production", "160", "800"], ["Sales", "144", "720"]],
                [["Shift", "2", ""], ["Line", "3", ""]],
            ),
            (
                ["Departemen", "Jam", "Biaya"],
                [["Operasional", "160", "800"], ["Penjualan", "144", "720"]],
                [["Shift", "2", ""], ["Lini", "3", ""]],
            ),
        ] {
            let mut grid: Vec<&[&str]> = vec![&["November 2026", "", ""], &head];
            grid.extend(rows.iter().map(|r| r.as_slice()));
            grid.extend(meta.iter().map(|r| r.as_slice()));
            let t = text("Jam", &grid);
            let header = format!("| {} |", head.join(" | "));
            let before = section_of(&t, &format!("| {} |", rows[1][0]));
            assert!(
                before.contains(&header) && before.contains("November 2026"),
                "{t}"
            );
            let after = section_of(&t, "| Shift |");
            assert!(
                !after.contains(&header) && !after.contains("November 2026"),
                "{t}"
            );
        }
        // Formulir di bawah judulnya tetap berjudul; deret "Line 1", "Line 2" tetap record.
        let t = text(
            "Form",
            &[
                &["Inspection Report", ""],
                &["Date", "17 Nov 2026"],
                &["Shift", "2"],
                &["Line", "3"],
            ],
        );
        assert!(
            section_of(&t, "| Line |").contains("Inspection Report"),
            "{t}"
        );
        let t = text(
            "Line",
            &[
                &["Line", "Output", "Reject"],
                &["Line 1", "100", "2"],
                &["Line 2", "98", "1"],
                &["Line 3", "97", "3"],
            ],
        );
        assert!(
            section_of(&t, "| Line 3 |").contains("| Line | Output | Reject |"),
            "{t}"
        );
        // Jam kerja tahunan per departemen: bukan kepala tahun.
        for first in [
            ["Production", "2000", "1920"],
            ["Production", "2000", "2010"],
        ] {
            let t = text(
                "Jam",
                &[
                    &first,
                    &["Review", "1800", "1840"],
                    &["Engineering", "1840", "1880"],
                    &["HR", "1760", "1800"],
                ],
            );
            assert!(!t.contains("| --- |"), "{t}");
        }
        // Kepala tahun sungguhan tetap dikenali, juga dengan kolom bulan.
        let t = text(
            "Rekap",
            &[
                &["Month", "2025", "2026"],
                &["Jan", "10", "12"],
                &["Feb", "7", "9"],
            ],
        );
        assert!(t.contains("| Month | 2025 | 2026 |\n| --- |"), "{t}");
        // Pembatas tahun angka saja membuang periode lama, juga sesudah baris kosong.
        for blank in [false, true] {
            let mut grid: Vec<&[&str]> = vec![
                &["FY 2026", "", ""],
                &["Month", "Output", "Hours"],
                &["Apr", "100", "8"],
            ];
            if blank {
                grid.push(&["", "", ""]);
            }
            grid.extend([
                &["2025", "", ""][..],
                &["Month", "Output", "Hours"],
                &["May", "80", "7"],
            ]);
            let t = text("Output", &grid);
            assert!(section_of(&t, "| Apr |").contains("FY 2026"), "{t}");
            assert!(!section_of(&t, "| May |").contains("FY 2026"), "{t}");
        }
    }

    #[test]
    fn round_thirty_six_cases() {
        let section_of = |t: &str, needle: &str| -> String {
            let at = t.find(needle).unwrap();
            t[t[..at].rfind("## ").unwrap()..at].to_string()
        };
        // Parameter di bawah record yang identitasnya nama kolom umum.
        for (head, rows) in [
            (
                ["Department", "Hours", "Cost"],
                [["Production", "160", "800"], ["Sales", "144", "720"]],
            ),
            (
                ["Line", "Output", "Reject"],
                [["Line 1", "100", "2"], ["Line 2", "98", "1"]],
            ),
        ] {
            let mut grid: Vec<&[&str]> = vec![&["November 2026", "", ""], &head];
            grid.extend(rows.iter().map(|r| r.as_slice()));
            grid.extend([&["Speed", "120", "119"][..], &["Pressure", "5", "4.9"]]);
            let t = text("Jam", &grid);
            let header = format!("| {} |", head.join(" | "));
            let before = section_of(&t, &format!("| {} |", rows[1][0]));
            assert!(
                before.contains(&header) && before.contains("November 2026"),
                "{t}"
            );
            let after = section_of(&t, "| Pressure |");
            assert!(
                !after.contains(&header) && !after.contains("November 2026"),
                "{t}"
            );
        }
        // Hasil uji tarik sesudah baris kosong bukan lanjutan daftar stok.
        for spec in [
            [
                ["Tensile strength", "490", "MPa"],
                ["Yield strength", "245", "MPa"],
                ["Elongation", "22", "%"],
            ],
            [
                ["Kuat tarik", "490", "MPa"],
                ["Kuat luluh", "245", "MPa"],
                ["Regangan", "22", "%"],
            ],
            [
                ["引張強さ", "490", "MPa"],
                ["降伏点", "245", "MPa"],
                ["伸び", "22", "%"],
            ],
        ] {
            let mut grid: Vec<&[&str]> = vec![
                &["November 2026", "", ""],
                &["Item", "Qty", "Unit"],
                &["Bolt", "12", "pcs"],
                &["Nut", "7", "pcs"],
                &["", "", ""],
            ];
            grid.extend(spec.iter().map(|r| r.as_slice()));
            let t = text("Stok", &grid);
            assert!(
                section_of(&t, "| Nut |").contains("| Item | Qty | Unit |"),
                "{t}"
            );
            let after = section_of(&t, &format!("| {} |", spec[2][0]));
            assert!(
                !after.contains("| Item | Qty | Unit |") && !after.contains("November 2026"),
                "{t}"
            );
        }
        // Lembar spesifikasi tetap berjudul, tabel parameter tetap berkepala.
        let t = text(
            "Spec",
            &[
                &["Spec sheet", ""],
                &["Material", "AX100"],
                &["Thickness", "2.0"],
                &["Tensile strength", "490"],
            ],
        );
        assert!(
            section_of(&t, "| Tensile strength |").contains("Spec sheet"),
            "{t}"
        );
        let t = text(
            "Spec",
            &[
                &["Item", "Nominal", "Actual"],
                &["Speed", "120", "119"],
                &["Pressure", "5", "4.9"],
                &["Tensile strength", "490", "495"],
            ],
        );
        assert!(
            section_of(&t, "| Tensile strength |").contains("| Item | Nominal | Actual |"),
            "{t}"
        );
    }

    #[test]
    fn round_thirty_seven_cases() {
        let section_of = |t: &str, needle: &str| -> String {
            let at = t.find(needle).unwrap();
            t[t[..at].rfind("## ").unwrap()..at].to_string()
        };
        let line = "| Line | Output | Reject |";
        // Kunci Review sesudah deret bernomor "Line 1", "Line 2".
        for (a, b) in [("UTS", "YS"), ("OD", "ID"), ("Wall", "Runout")] {
            let t = text(
                "Review",
                &[
                    &["November 2026", "", ""],
                    &["Line", "Output", "Reject"],
                    &["Line 1", "100", "2"],
                    &["Line 2", "98", "1"],
                    &[a, "490", "495"],
                    &[b, "245", "250"],
                ],
            );
            assert!(section_of(&t, "| Line 2 |").contains(line), "{t}");
            let after = section_of(&t, &format!("| {b} |"));
            assert!(
                !after.contains(line) && !after.contains("November 2026"),
                "{t}"
            );
        }
        // Kunci ukur sesudah baris kosong bukan lanjutan daftar stok.
        for key in ["Wall", "Batas luluh", "伸び率"] {
            let t = text(
                "Stok",
                &[
                    &["November 2026", "", ""],
                    &["Item", "Qty", "Unit"],
                    &["Bolt", "12", "pcs"],
                    &["Nut", "7", "pcs"],
                    &["", "", ""],
                    &[key, "3.2", "mm"],
                ],
            );
            let after = section_of(&t, &format!("| {key} |"));
            assert!(
                !after.contains("| Item | Qty | Unit |") && !after.contains("November 2026"),
                "{t}"
            );
        }
        // Record jarang di awal daftar tetap baris data, bukan label.
        let t = text(
            "Stock",
            &[
                &["Bolt M8", "", ""],
                &["Nut M8", "", ""],
                &["Washer M8", "12", "pcs"],
            ],
        );
        assert!(!t.contains("› Bolt M8"), "{t}");
        assert!(
            t.contains("| Bolt M8 |  |  |\n| Nut M8 |  |  |\n| Washer M8 | 12 | pcs |"),
            "{t}"
        );
        let t = text(
            "Stock",
            &[
                &["Bolt M8", "", ""],
                &["Nut M8", "7", "pcs"],
                &["Pin M8", "3", "pcs"],
            ],
        );
        assert!(!t.contains("› Bolt M8") && t.contains("| Bolt M8 |"), "{t}");
        // Judul sungguhan tetap label.
        let t = text(
            "Stock",
            &[
                &["PT Example Corp", "", ""],
                &["Stock Report", "", ""],
                &["Bolt", "12", "pcs"],
                &["Nut", "7", "pcs"],
            ],
        );
        assert!(section_of(&t, "| Nut |").contains("Stock Report"), "{t}");
        let t = text(
            "Output",
            &[
                &["November 2026", "", ""],
                &["Apr", "100", "8"],
                &["May", "80", "7"],
            ],
        );
        assert!(section_of(&t, "| May |").contains("November 2026"), "{t}");
        // Kunci spesifikasi lalu parameter: judul laporan tetap.
        for key in ["Drawing No.", "Standar", "規格"] {
            let t = text(
                "Spec",
                &[
                    &["Inspection Report", ""],
                    &[key, "DWG-001"],
                    &["Thickness", "2.0"],
                    &["Tensile strength", "490"],
                ],
            );
            assert!(
                section_of(&t, "| Tensile strength |").contains("Inspection Report"),
                "{t}"
            );
        }
        // Tabel parameter berkepala "Item" berlanjut sesudah baris kosong.
        let t = text(
            "Spec",
            &[
                &["Review November", "", ""],
                &["Item", "Nominal", "Actual"],
                &["Speed", "120", "119"],
                &["Pressure", "5", "4.9"],
                &["", "", ""],
                &["Tensile strength", "490", "495"],
                &["Elongation", "22", "23"],
            ],
        );
        assert!(
            section_of(&t, "| Elongation |").contains("| Item | Nominal | Actual |"),
            "{t}"
        );
    }

    #[test]
    fn round_thirty_eight_cases() {
        let section_of = |t: &str, needle: &str| -> String {
            let at = t.find(needle).unwrap();
            t[t[..at].rfind("## ").unwrap()..at].to_string()
        };
        let stock = "| Item | Qty | Unit |";
        // Barang bernama "Angle" tidak membuat daftar stok jadi lembar spesifikasi.
        let t = text(
            "Stok",
            &[
                &["Stock November", "", ""],
                &["Item", "Qty", "Unit"],
                &["Angle", "12", "pcs"],
                &["Bolt", "7", "pcs"],
                &["UTS", "490", "MPa"],
                &["YS", "245", "MPa"],
            ],
        );
        assert!(section_of(&t, "| Bolt |").contains(stock), "{t}");
        let after = section_of(&t, "| YS |");
        assert!(
            !after.contains(stock) && !after.contains("Stock November"),
            "{t}"
        );
        // "Angle" di baris kedua daftar stok tetap record.
        let t = text(
            "Stok",
            &[
                &["Item", "Qty", "Unit"],
                &["Bolt", "7", "pcs"],
                &["Angle", "12", "pcs"],
                &["Nut", "5", "pcs"],
            ],
        );
        assert!(section_of(&t, "| Nut |").contains(stock), "{t}");
        // Judul di atas record jarang: judul tetap label, record tetap data.
        let t = text(
            "Sheet",
            &[
                &["Stock Report", "", ""],
                &["Bolt M8", "", ""],
                &["Nut M8", "", ""],
                &["Washer M8", "12", "pcs"],
            ],
        );
        assert!(!t.contains("› Bolt M8") && !t.contains("› Nut M8"), "{t}");
        assert!(
            section_of(&t, "| Washer M8 |").contains("Stock Report"),
            "{t}"
        );
        assert!(
            t.contains("| Bolt M8 |  |  |\n| Nut M8 |  |  |\n| Washer M8 | 12 | pcs |"),
            "{t}"
        );
        // Deret "Line 1", "Line 2" tetap berlaku sesudah baris kosong.
        let line = "| Line | Output | Reject |";
        let grid = |rows: &[&'static [&'static str]]| -> String {
            let mut g: Vec<&[&str]> = vec![
                &["November 2026", "", ""],
                &["Line", "Output", "Reject"],
                &["Line 1", "100", "2"],
                &["Line 2", "98", "1"],
                &["", "", ""],
            ];
            g.extend(rows.iter().copied());
            text("Review", &g)
        };
        let t = grid(&[&["Rm", "490", "495"], &["ReH", "245", "250"]]);
        let after = section_of(&t, "| ReH |");
        assert!(
            !after.contains(line) && !after.contains("November 2026"),
            "{t}"
        );
        let t = grid(&[&["Line 3", "97", "3"], &["Line 4", "99", "0"]]);
        let after = section_of(&t, "| Line 4 |");
        assert!(
            after.contains(line) && after.contains("November 2026"),
            "{t}"
        );
        // Judul bagian di atas data bulanan tanpa kepala tetap label.
        let t = text(
            "Output",
            &[
                &["Production", "", ""],
                &["Dispatch", "", ""],
                &["Apr", "100", "8"],
                &["May", "80", "7"],
            ],
        );
        let may = section_of(&t, "| May |");
        assert!(
            may.contains("Production") && may.contains("Dispatch"),
            "{t}"
        );
    }

    #[test]
    fn round_thirty_nine_cases() {
        let section_of = |t: &str, needle: &str| -> String {
            let at = t.find(needle).unwrap();
            t[t[..at].rfind("## ").unwrap()..at].to_string()
        };
        // Record jarang campuran di bawah judul tetap baris data.
        let t = text(
            "Synthetic",
            &[
                &["Stock Report", "", ""],
                &["Bolt M8", "", ""],
                &["Nut", "", ""],
                &["Washer M8", "12", "pcs"],
            ],
        );
        assert!(!t.contains("› Bolt M8") && !t.contains("› Nut"), "{t}");
        assert!(
            section_of(&t, "| Washer M8 |").contains("Stock Report"),
            "{t}"
        );
        assert!(
            t.contains("| Bolt M8 |  |  |\n| Nut |  |  |\n| Washer M8 | 12 | pcs |"),
            "{t}"
        );
        // Judul bagian yang jelas memutus kepala stok, dengan atau tanpa baris kosong.
        let stock = "| Item | Qty | Unit |";
        for blank in [true, false] {
            let mut grid: Vec<&[&str]> = vec![
                &["Item", "Qty", "Unit"],
                &["Bolt", "12", "pcs"],
                &["Nut", "7", "pcs"],
            ];
            if blank {
                grid.push(&["", "", ""]);
            }
            grid.extend([
                &["Production Report", "", ""][..],
                &["Apr", "100", "8"],
                &["May", "80", "7"],
            ]);
            let t = text("Sheet", &grid);
            assert!(section_of(&t, "| Nut |").contains(stock), "{t}");
            let may = section_of(&t, "| May |");
            assert!(
                !may.contains(stock) && may.contains("Production Report"),
                "{t}"
            );
        }
        // Label tabel kiri tidak menaungi tabel stok di sebelahnya: baris tetap utuh, labelnya
        // ditulis sebagai teks, bukan judul.
        let t = text(
            "Prod",
            &[
                &["Intake", "", "", "", "", "", ""],
                &["Month", "Output", "Hours", "", "Item", "Qty", "Unit"],
                &["Apr", "100", "8", "", "Bolt", "12", "pcs"],
                &["May", "80", "7", "", "Nut", "7", "pcs"],
            ],
        );
        assert!(!t.contains("› Intake") && t.contains("Intake\n"), "{t}");
        assert!(t.contains("| May | 80 | 7 |  | Nut | 7 | pcs |"), "{t}");
    }

    #[test]
    fn round_forty_cases() {
        let section_of = |t: &str, needle: &str| -> String {
            let at = t.find(needle).unwrap();
            t[t[..at].rfind("## ").unwrap()..at].to_string()
        };
        // Tabel Review lebar dengan kolom spasi tetap satu tabel: hasil tarik tetap bersama sampelnya.
        let t = text(
            "Review",
            &[
                &["Sample", "OD", "Wall", "", "UTS", "YS", "Elongation"],
                &["S-001", "60.5", "2.0", "", "490", "245", "22"],
                &["S-002", "60.4", "2.1", "", "495", "250", "23"],
            ],
        );
        assert!(
            t.contains("| S-001 | 60.5 | 2.0 |  | 490 | 245 | 22 |"),
            "{t}"
        );
        // Judul di akhir tabel stok sebelum baris kosong: data sesudahnya tidak memakai kepala
        // stok.
        let stock = "| Item | Qty | Unit |";
        let t = text(
            "Sheet",
            &[
                &["Item", "Qty", "Unit"],
                &["Bolt", "12", "pcs"],
                &["Nut", "7", "pcs"],
                &["Production Report", "", ""],
                &["", "", ""],
                &["Apr", "100", "8"],
                &["May", "80", "7"],
            ],
        );
        assert!(section_of(&t, "| Nut |").contains(stock), "{t}");
        assert!(!section_of(&t, "| May |").contains(stock), "{t}");
        // Record jarang sesudah tabel lain, dengan atau tanpa baris kosong: tidak jadi label.
        for blank in [false, true] {
            let mut grid: Vec<&[&str]> = vec![
                &["Month", "Output", "Hours"],
                &["Apr", "100", "8"],
                &["May", "80", "7"],
            ];
            if blank {
                grid.push(&["", "", ""]);
            }
            grid.extend([
                &["Stock Report", "", ""][..],
                &["Bolt M8", "", ""],
                &["Nut", "", ""],
                &["Washer M8", "12", "pcs"],
            ]);
            let t = text("Synthetic", &grid);
            assert!(!t.contains("› Bolt M8") && !t.contains("› Nut"), "{t}");
            assert!(
                t.contains("| Bolt M8 |  |  |\n| Nut |  |  |\n| Washer M8 | 12 | pcs |"),
                "{t}"
            );
            assert!(
                !section_of(&t, "| Washer M8 |").contains("| Month |"),
                "{t}"
            );
        }
        // Kata judul di nama barang tidak membuat label.
        let t = text(
            "Stok",
            &[
                &["Item", "Qty", "Unit"],
                &["Bolt", "12", "pcs"],
                &["Daily planner A5", "", ""],
                &["Washer", "15", "pcs"],
            ],
        );
        assert!(!t.contains("› Daily planner A5"), "{t}");
        assert!(t.contains("| Daily planner A5 |  |  |"), "{t}");
    }

    #[test]
    fn round_forty_one_cases() {
        let section_of = |t: &str, needle: &str| -> String {
            let at = t.find(needle).unwrap();
            t[t[..at].rfind("## ").unwrap()..at].to_string()
        };
        // Judul bagian di kolom dalam memutus kepala stok, dengan atau tanpa baris kosong.
        let stock = "| Item | Qty | Unit |";
        for blank in [false, true] {
            let mut grid: Vec<&[&str]> = vec![
                &["Item", "Qty", "Unit"],
                &["Bolt", "12", "pcs"],
                &["Nut", "7", "pcs"],
                &["", "Production Report", ""],
            ];
            if blank {
                grid.push(&["", "", ""]);
            }
            grid.extend([&["Apr", "100", "8"][..], &["May", "80", "7"]]);
            let t = text("Sheet", &grid);
            assert!(section_of(&t, "| Nut |").contains(stock), "{t}");
            assert!(!section_of(&t, "| May |").contains(stock), "{t}");
        }
        // Label tertunda di atas dua tabel berdampingan tidak jadi judul keduanya.
        let t = text(
            "Prod",
            &[
                &["Intake", "", "", "", "", "", ""],
                &["", "", "", "", "", "", ""],
                &["Month", "Output", "Hours", "", "Item", "Qty", "Unit"],
                &["Apr", "100", "8", "", "Bolt", "12", "pcs"],
                &["May", "80", "7", "", "Nut", "7", "pcs"],
            ],
        );
        assert!(!t.contains("› Intake"), "{t}");
        assert!(t.contains("| May | 80 | 7 |  | Nut | 7 | pcs |"), "{t}");
        // Judul grup di atas tabel Review lebar: hasil uji tetap bersama sampelnya.
        let t = text(
            "Review",
            &[
                &["Dimensions", "", "", "", "Mechanical properties", "", ""],
                &["Sample", "OD", "Wall", "", "UTS", "YS", "Elongation"],
                &["S-001", "60.5", "2.0", "", "490", "245", "22"],
                &["S-002", "60.4", "2.1", "", "495", "250", "23"],
            ],
        );
        assert!(
            t.contains("| S-001 | 60.5 | 2.0 |  | 490 | 245 | 22 |"),
            "{t}"
        );
        // Nama barang berkode dengan kata judul tetap record jarang.
        let t = text(
            "Stock",
            &[
                &["Stock Report", "", ""],
                &["Daily planner A5", "", ""],
                &["Weekly planner A5", "", ""],
                &["Washer M8", "12", "pcs"],
            ],
        );
        assert!(!t.contains("› Daily planner A5"), "{t}");
        assert!(
            section_of(&t, "| Washer M8 |").contains("Stock Report"),
            "{t}"
        );
        assert!(
            t.contains("| Daily planner A5 |  |  |\n| Weekly planner A5 |  |  |\n| Washer M8 |"),
            "{t}"
        );
    }

    #[test]
    fn round_forty_two_cases() {
        let section_of = |t: &str, needle: &str| -> String {
            let at = t.find(needle).unwrap();
            t[t[..at].rfind("## ").unwrap()..at].to_string()
        };
        // Grup Review dengan N/A atau kepala yang diulang tetap satu tabel.
        for (row2, row3) in [
            (
                ["S-002", "60.4", "2.1", "", "N/A", "250", "23"],
                ["S-003", "60.6", "2.0", "", "492", "247", "22"],
            ),
            (
                ["Sample", "OD", "Wall", "", "UTS", "YS", "Elongation"],
                ["S-003", "60.6", "2.0", "", "492", "247", "22"],
            ),
        ] {
            let t = text(
                "Review",
                &[
                    &["Dimensions", "", "", "", "Mechanical properties", "", ""],
                    &["Sample", "OD", "Wall", "", "UTS", "YS", "Elongation"],
                    &["S-001", "60.5", "2.0", "", "490", "245", "22"],
                    &row2,
                    &row3,
                ],
            );
            assert!(
                t.contains("| S-003 | 60.6 | 2.0 |  | 492 | 247 | 22 |"),
                "{t}"
            );
        }
        // Judul dengan tahun di sel lain memutus kepala stok, dengan atau tanpa baris kosong.
        let stock = "| Item | Qty | Unit |";
        for blank in [false, true] {
            let mut grid: Vec<&[&str]> = vec![
                &["Item", "Qty", "Unit"],
                &["Bolt", "12", "pcs"],
                &["Nut", "7", "pcs"],
                &["Production Report", "2026", ""],
            ];
            if blank {
                grid.push(&["", "", ""]);
            }
            grid.extend([&["Apr", "100", "8"][..], &["May", "80", "7"]]);
            let t = text("Sheet", &grid);
            assert!(section_of(&t, "| Nut |").contains(stock), "{t}");
            assert!(!section_of(&t, "| May |").contains(stock), "{t}");
            assert!(t.contains("| Production Report | 2026 |  |"), "{t}");
        }
        // Nama barang tanpa kode dengan kata sifat judul tetap record jarang.
        let t = text(
            "Stock",
            &[
                &["Stock Report", "", ""],
                &["Daily planner A5", "", ""],
                &["Weekly planner", "", ""],
                &["Ballpoint pen", "12", "pcs"],
            ],
        );
        assert!(
            !t.contains("› Daily planner A5") && !t.contains("› Weekly planner"),
            "{t}"
        );
        assert!(
            section_of(&t, "| Ballpoint pen |").contains("Stock Report"),
            "{t}"
        );
    }

    #[test]
    fn round_forty_three_cases() {
        let section_of = |t: &str, needle: &str| -> String {
            let at = t.find(needle).unwrap();
            t[t[..at].rfind("## ").unwrap()..at].to_string()
        };
        // Judul dengan tahun memutus label tabel tanpa kepala.
        let t = text(
            "Sheet",
            &[
                &["Stock Report November 2026", "", ""],
                &["Bolt", "12", "pcs"],
                &["Nut", "7", "pcs"],
                &["Production Report", "2025", ""],
                &["Apr", "100", "8"],
                &["May", "80", "7"],
            ],
        );
        assert!(
            section_of(&t, "| Nut |").contains("Stock Report November 2026"),
            "{t}"
        );
        assert!(!section_of(&t, "| May |").contains("Stock Report"), "{t}");
        // Grup Review dengan nilai bersatuan atau hasil deskriptif tetap satu tabel.
        for (a, b) in [
            ("490 MPa", "495 MPa"),
            ("No cracks", "No cracks"),
            ("Tidak retak", "Tidak retak"),
            ("異常なし", "異常なし"),
        ] {
            let t = text(
                "Review",
                &[
                    &["Dimensions", "", "", "", "Mechanical properties", "", ""],
                    &["Sample", "OD", "Wall", "", "UTS", "YS", "Elongation"],
                    &["S-001", "60.5", "2.0", "", a, "245", "22"],
                    &["S-002", "60.4", "2.1", "", b, "250", "23"],
                ],
            );
            assert!(
                t.contains(&format!("| S-002 | 60.4 | 2.1 |  | {b} | 250 | 23 |")),
                "{t}"
            );
        }
        // Dua tabel berlabel yang masing-masing punya kolom identitas tetap terpisah.
        let t = text(
            "Prod",
            &[
                &["Intake", "", "", "", "Dispatch", "", ""],
                &["Month", "Output", "Hours", "", "Month", "Output", "Hours"],
                &["Apr", "100", "8", "", "Apr", "90", "7"],
            ],
        );
        assert!(t.contains("› Intake") && t.contains("› Dispatch"), "{t}");
    }

    #[test]
    fn round_forty_four_cases() {
        // Hasil uji di sebelah kolom sampel tetap di baris sampelnya, apa pun kolom pertamanya.
        for (head, a, b) in [
            ("Date", "2025-11-08", "2025-11-08"),
            ("Hardness", "HRB 72", "HRB 75"),
            ("Keterangan", "Tidak retak", "Retak halus"),
            ("材質", "AX100", "AX100"),
        ] {
            let t = text(
                "Review",
                &[
                    &["Sample Details", "", "", "", "Test Results", "", ""],
                    &["Sample", "OD", "Wall", "", head, "UTS", "Elongation"],
                    &["S-001", "60.5", "2.0", "", a, "490", "22"],
                    &["S-002", "60.4", "2.1", "", b, "495", "23"],
                ],
            );
            assert!(
                t.contains(&format!("| S-002 | 60.4 | 2.1 |  | {b} | 495 | 23 |")),
                "{head}: {t}"
            );
        }
        // Dua tabel stok berlabel dengan kepala berbeda: bisa juga satu record, jadi baris tetap
        // utuh dan labelnya dibuang.
        let t = text(
            "Stok",
            &[
                &["Raw Materials", "", "", "", "Finished Goods", "", ""],
                &[
                    "Material Code",
                    "Quantity",
                    "Unit",
                    "",
                    "Product Code",
                    "Quantity",
                    "Unit",
                ],
                &["1001", "22", "t", "", "2001", "60", "pcs"],
                &["1002", "8", "kg", "", "2002", "45", "pcs"],
            ],
        );
        assert!(
            !t.contains("› Raw Materials") && !t.contains("› Finished"),
            "{t}"
        );
        assert!(t.contains("| 1002 | 8 | kg |  | 2002 | 45 | pcs |"), "{t}");
        // Dua tabel berlabel dengan kepala sama tetap terpisah.
        let t = text(
            "Stok",
            &[
                &["Raw", "", "", "", "Finished", "", ""],
                &["Code", "Qty", "Unit", "", "Code", "Qty", "Unit"],
                &["RM-001", "22", "kg", "", "FG-001", "60", "pcs"],
                &["RM-002", "8", "kg", "", "FG-002", "45", "pcs"],
            ],
        );
        assert!(t.contains("› Raw") && t.contains("› Finished"), "{t}");
        // Record jarang pertama sesudah judul tetap data, bukan label record sesudahnya.
        let t = text(
            "Roster",
            &[
                &["Daftar Karyawan", "", ""],
                &["Taylor", "", ""],
                &["Alex", "Pagi", "Review"],
                &["Cici", "Malam", "Gudang"],
            ],
        );
        assert!(!t.contains("› Taylor"), "{t}");
        assert!(t.contains("| Taylor |"), "{t}");
        assert!(t.contains("| Cici | Malam | Gudang |"), "{t}");
        let t = text(
            "Stok",
            &[
                &["Stock Report", "", ""],
                &["Bolt", "", ""],
                &["Nut", "OK", "Rak A"],
                &["Washer", "Rusak", "Rak B"],
            ],
        );
        assert!(!t.contains("› Bolt"), "{t}");
        assert!(t.contains("| Bolt |"), "{t}");
        // Judul proses di atas kepala tetap label.
        let t = text(
            "Prod",
            &[
                &["Intake", "", ""],
                &["Operator", "Shift", "Area"],
                &["Alex", "Pagi", "Review"],
            ],
        );
        assert!(t.contains("| Alex | Pagi | Review |"), "{t}");
    }

    #[test]
    fn round_forty_five_cases() {
        // Dua kelompok berlabel dengan identitas sejenis tapi kepala berbeda: satu record
        // operasional dan hasil ujinya, baris tetap utuh melewati kolom kosong.
        for (left, right, a, b, c, d) in [
            (
                "Production Date",
                "Test Date",
                "2025-11-06",
                "2025-11-07",
                "2025-11-08",
                "2025-11-09",
            ),
            ("Sample No", "Test No", "1", "2", "101", "102"),
            ("Sample", "Hardness", "ABC001", "ABC002", "HRB72", "HRB75"),
        ] {
            let t = text(
                "Review",
                &[
                    &["Production Details", "", "", "", "Test Results", "", ""],
                    &[left, "Line", "Shift", "", right, "UTS", "Elongation"],
                    &[a, "A", "2", "", c, "490", "22"],
                    &[b, "B", "3", "", d, "495", "23"],
                ],
            );
            assert!(
                t.contains(&format!("| {b} | B | 3 |  | {d} | 495 | 23 |")),
                "{left}: {t}"
            );
        }
        // Daftar dua orang: record pertama tetap data.
        let t = text(
            "Roster",
            &[
                &["Daftar Karyawan", "", ""],
                &["Taylor", "", ""],
                &["Alex", "Pagi", "Review"],
            ],
        );
        assert!(!t.contains("› Taylor"), "{t}");
        assert!(
            t.contains("| Taylor |") && t.contains("| Alex | Pagi | Review |"),
            "{t}"
        );

        // Tabel berdampingan yang mulainya bergeser satu baris: tanpa kepala dan tanpa label
        // dari baris awalnya, setiap baris utuh.
        let t = text(
            "Review",
            &[
                &["Intake", "", "", "", "", "", ""],
                &["Production Date", "Line", "Shift", "", "Dispatch", "", ""],
                &["2025-11-06", "A", "2", "", "Test Date", "UTS", "Elongation"],
                &["2025-11-07", "B", "3", "", "2025-11-08", "490", "22"],
                &["2025-11-08", "A", "2", "", "2025-11-09", "495", "23"],
            ],
        );
        assert!(!t.contains("› Intake") && !t.contains("| --- |"), "{t}");
        assert!(
            t.contains("| 2025-11-08 | A | 2 |  | 2025-11-09 | 495 | 23 |"),
            "{t}"
        );
        // Blok bergeser dengan judul tertunda di atasnya dan judul periode di tengahnya: tanpa
        // label dan tanpa kepala, judul tetap tertulis sebagai teks.
        let t = text(
            "Review",
            &[
                &["Intake Report 2025", "", "", "", "", "", ""],
                &["", "", "", "", "", "", ""],
                &["Intake", "", "", "", "", "", ""],
                &["Month", "Output", "Hours", "", "Dispatch", "", ""],
                &["Apr", "100", "8", "", "Month", "Qty", "Hours"],
                &["May", "120", "9", "", "Apr", "90", "7"],
                &["Intake Report 2026", "", "", "", "", "", ""],
                &["Month", "Output", "Hours", "", "", "", ""],
                &["Jan", "101", "8", "", "May", "95", "8"],
                &["Feb", "104", "9", "", "Jun", "91", "7"],
            ],
        );
        assert!(!t.contains("› Intake") && !t.contains("| --- |"), "{t}");
        assert!(t.contains("Intake Report 2026"), "{t}");
        assert!(t.contains("| Feb | 104 | 9 |  | Jun | 91 | 7 |"), "{t}");
        // Judul tertunda di atas blok gabungan dua tabel berkepala beda: tidak menaungi keduanya.
        let t = text(
            "Review",
            &[
                &["Intake Production Report", "", "", "", "", "", ""],
                &["", "", "", "", "", "", ""],
                &["Intake", "", "", "", "Dispatch", "", ""],
                &["Period", "Output", "Hours", "", "Month", "Qty", "Hours"],
                &["Apr", "100", "8", "", "Apr", "90", "7"],
                &["May", "120", "9", "", "May", "95", "8"],
            ],
        );
        assert!(!t.contains("› Intake"), "{t}");
        assert!(t.contains("| May | 120 | 9 |  | May | 95 | 8 |"), "{t}");
        // Sel pertama blok bergeser tetap di kolomnya, bukan teks lepas yang mirip judul.
        let t = text(
            "Review",
            &[
                &["# Intake", "", "", "", "", "", ""],
                &["Month", "Output", "Hours", "", "Dispatch", "", ""],
                &["Apr", "100", "8", "", "Month", "Qty", "Hours"],
                &["May", "120", "9", "", "Apr", "90", "7"],
            ],
        );
        assert!(t.contains("| # Intake |  |  |  |  |  |  |"), "{t}");
        assert!(!t.lines().any(|l| l.starts_with("# ")), "{t}");
        // Judul di atas tabel kiri saja di blok gabungan sejajar: data di kolomnya, bukan label
        // kedua tabel.
        let t = text(
            "Review",
            &[
                &["Intake Production Report", "", "", "", "", "", ""],
                &["Intake", "", "", "", "Dispatch", "", ""],
                &["Apr", "100", "8", "", "Apr", "90", "7"],
                &["May", "120", "9", "", "May", "95", "8"],
            ],
        );
        assert!(!t.contains("› Intake"), "{t}");
        assert!(t.contains("\nIntake Production Report\n"), "{t}");
        assert!(t.contains("| May | 120 | 9 |  | May | 95 | 8 |"), "{t}");
        let t = text(
            "Catatan",
            &[&["# Catatan shift"], &["Mesin 3 berhenti 10 menit"]],
        );
        assert!(t.contains("\\# Catatan shift\n"), "{t}");
        assert!(!t.lines().any(|l| l.starts_with("# ")), "{t}");
        // Kontrol: mulai sebaris dengan kepala sama tetap dua tabel berlabel.
        let t = text(
            "Prod",
            &[
                &["Intake", "", "", "", "Dispatch", "", ""],
                &["Month", "Output", "Hours", "", "Month", "Output", "Hours"],
                &["Apr", "100", "8", "", "Apr", "90", "7"],
                &["May", "120", "9", "", "May", "95", "8"],
            ],
        );
        assert!(t.contains("› Intake") && t.contains("› Dispatch"), "{t}");
        assert_eq!(
            t.matches("| Month | Output | Hours |\n| --- | --- | --- |")
                .count(),
            2,
            "{t}"
        );
    }

    #[test]
    #[ignore = "tes performa: cargo test --release -- --ignored stay_fast"]
    fn staggered_side_by_side_lots_stay_fast() {
        // Dua tabel berdampingan dengan label lot bergantian di kiri dan kanan: pemotongan di
        // setiap label tabel sebelah tidak boleh kuadratik.
        let mut cells: Vec<Vec<String>> = vec![
            ["Intake", "", "", "", "Dispatch", "", ""]
                .map(String::from)
                .to_vec(),
            ["Month", "Output", "Hours", "", "Month", "Output", "Hours"]
                .map(String::from)
                .to_vec(),
        ];
        for lot in 0..8_000 {
            let left = lot % 2 == 0;
            let mut label = vec![String::new(); 7];
            label[if left { 0 } else { 4 }] = format!("Lot {lot}");
            cells.push(label);
            for _ in 0..3 {
                cells.push(
                    ["Apr", "10", "8", "", "Apr", "20", "9"]
                        .map(String::from)
                        .to_vec(),
                );
            }
        }
        let t0 = std::time::Instant::now();
        let (t, _) = to_text(
            &[Sheet {
                name: "Lot".into(),
                hidden: false,
                cells: &cells,
            }],
            4_500_000,
        );
        assert!(t.contains("| Apr | 20 | 9 |"));
        assert!(t0.elapsed().as_secs() < 5, "{:?}", t0.elapsed());
    }

    #[test]
    fn proven_headers_with_year_values() {
        let t = text(
            "Prod",
            &[
                &["Month", "Production", "Unit"],
                &["Apr", "2025", "pcs"],
                &["May", "4321", "pcs"],
            ],
        );
        assert!(
            t.contains("| Month | Production | Unit |\n| --- | --- | --- |\n| Apr | 2025 | pcs |"),
            "{t}"
        );
        let t = text(
            "Rekap",
            &[&["Item", "2025", "2026"], &["Widget", "10", "12"]],
        );
        assert!(t.contains("| Item | 2025 | 2026 |\n| --- |"), "{t}");
    }

    #[test]
    fn sparse_rows_keep_their_columns() {
        let t = text(
            "Stok",
            &[
                &["Item", "Qty"],
                &["Widget", "4"],
                &["", "10"],
                &["Item", "Qty"],
                &["Plat", "3"],
            ],
        );
        assert!(t.contains("|  | 10 |"), "{t}");
        assert!(!t.contains("› 10"), "{t}");
        // Setelah baris kosong, hanya kolom Qty dan Reject yang terisi.
        let t = text(
            "Review",
            &[
                &["Item", "Qty", "Reject"],
                &["Widget", "5", "1"],
                &["", "", ""],
                &["", "10", ""],
                &["", "", "2"],
            ],
        );
        assert!(
            t.ends_with(
                "| Item | Qty | Reject |\n| --- | --- | --- |\n|  | 10 |  |\n|  |  | 2 |\n\n"
            ),
            "{t}"
        );
    }

    #[test]
    fn period_labels_replace_each_other_and_do_not_leak() {
        let t = text(
            "Prod",
            &[
                &["Intake", "", ""],
                &["FY 2026", "", ""],
                &["Month", "Prod", "Note"],
                &["Apr", "10", "1"],
                &["FY 2025", "", ""],
                &["Month", "Hours", "Note"],
                &["Apr", "9", "1"],
                &["", "", ""],
                &["Month", "Hours", "Note"],
                &["May", "8", "1"],
                &["", "", ""],
                &["Dispatch", "", ""],
                &["Month", "Prod", "Note"],
                &["Apr", "5", "1"],
            ],
        );
        assert!(
            t.contains("## Prod › Intake › FY 2026\n| Month | Prod | Note |"),
            "{t}"
        );
        assert!(
            t.contains("## Prod › Intake › FY 2025\n| Month | Hours | Note |"),
            "{t}"
        );
        // Tabel berikutnya dengan kepala aktif terakhir yang sama tetap di FY 2025.
        assert_eq!(t.matches("## Prod › Intake › FY 2025\n").count(), 2, "{t}");
        // Dispatch setelah baris kosong dengan kepala berbeda: tanpa label dan kepala (tidak
        // jelas), dan yang pasti tanpa label Intake/FY 2025.
        let dispatch = &t[t.find("| Dispatch |").unwrap()..];
        assert!(
            !dispatch.contains("Intake") && !dispatch.contains("FY 2025"),
            "{t}"
        );
        assert!(
            !t.contains("FY 2026 › FY 2025") && !t.contains("Intake › Dispatch"),
            "{t}"
        );
    }

    #[test]
    fn new_section_with_an_unproven_header_drops_the_old_header() {
        let t = text(
            "Prod",
            &[
                &["FY 2026", ""],
                &["Month", "Qty"],
                &["Apr", "10"],
                &["FY 2025", ""],
                &["Month", "Status"],
                &["Apr", "Closed"],
            ],
        );
        assert!(
            t.ends_with("## Prod › FY 2025\n| Month | Status |\n| Apr | Closed |\n\n"),
            "{t}"
        );
    }

    #[test]
    fn period_label_followed_directly_by_data() {
        let t = text(
            "Prod",
            &[
                &["FY 2026", "", ""],
                &["Month", "Prod", "Note"],
                &["Apr", "10", "1"],
                &["FY 2025", "", ""],
                &["Apr", "9", "1"],
            ],
        );
        assert!(
            t.ends_with("## Prod › FY 2025\n| Month | Prod | Note |\n| --- | --- | --- |\n| Apr | 9 | 1 |\n\n"),
            "{t}"
        );
    }

    #[test]
    fn next_period_after_a_blank_row_reuses_the_header() {
        let t = text(
            "Prod",
            &[
                &["FY 2026", "", ""],
                &["Month", "Qty", "Note"],
                &["Apr", "10", "1"],
                &["", "", ""],
                &["FY 2025", "", ""],
                &["Apr", "9", "1"],
            ],
        );
        assert!(
            t.ends_with("## Prod › FY 2025\n| Month | Qty | Note |\n| --- | --- | --- |\n| Apr | 9 | 1 |\n\n"),
            "{t}"
        );
    }

    #[test]
    fn standalone_labels_accumulate_and_replace_periods() {
        let t = text(
            "Prod",
            &[
                &["Intake", ""],
                &["", ""],
                &["FY 2026", ""],
                &["", ""],
                &["Month", "Prod"],
                &["Apr", "10"],
                &["", ""],
                &["FY 2025", ""],
                &["", ""],
                &["Month", "Prod"],
                &["Apr", "9"],
            ],
        );
        assert!(
            t.contains("## Prod › Intake › FY 2026\n| Month | Prod |"),
            "{t}"
        );
        assert!(
            t.contains("## Prod › Intake › FY 2025\n| Month | Prod |"),
            "{t}"
        );
        assert!(!t.contains("FY 2025 › Intake"), "{t}");
    }

    #[test]
    fn data_after_a_blank_row_keeps_labels_and_header() {
        let t = text(
            "Prod",
            &[
                &["Intake", "", ""],
                &["Month", "Qty", "Note"],
                &["Apr", "10", "1"],
                &["", "", ""],
                &["May", "20", "1"],
            ],
        );
        assert!(
            t.ends_with("## Prod › Intake\n| Month | Qty | Note |\n| --- | --- | --- |\n| May | 20 | 1 |\n\n"),
            "{t}"
        );
    }

    #[test]
    fn merged_header_levels_are_combined() {
        let t = text(
            "Prod",
            &[
                &["Month", "Production", "", "Working hour", ""],
                &["", "Actual", "Plan", "Actual", "Plan"],
                &["Apr", "10", "12", "500", "520"],
            ],
        );
        assert!(
            t.contains(
                "| Month | Production Actual | Production Plan | Working hour Actual | Working hour Plan |\n| --- |"
            ),
            "{t}"
        );
        let t = text(
            "Prod",
            &[
                &["Item", "Production", "", "", ""],
                &["", "2025", "", "2026", ""],
                &["", "Actual", "Plan", "Actual", "Plan"],
                &["Widget", "1", "2", "3", "4"],
            ],
        );
        assert!(
            t.contains(
                "| Item | Production 2025 Actual | Production 2025 Plan | Production 2026 Actual | Production 2026 Plan |"
            ),
            "{t}"
        );
        assert!(t.contains("| Widget | 1 | 2 | 3 | 4 |"), "{t}");
    }

    #[test]
    fn long_text_is_never_dropped() {
        let note = "Catatan panjang ".repeat(20);
        let t = text("Catatan", &[&[note.as_str()]]);
        assert!(t.contains(note.trim()), "{t}");

        let warning = "Periksa tekanan sebelum mulai. STOP MACHINE IF PRESSURE EXCEEDS LIMIT";
        let t = text(
            "SOP",
            &[&[warning, ""], &["Step", "Action"], &["1", "Buka katup"]],
        );
        assert!(t.contains(warning), "{t}");
        let heading = t.lines().find(|l| l.starts_with("## SOP ›")).unwrap();
        assert!(heading.chars().count() < 80, "{heading}");

        // Label pendek yang tidak muat di judul tetap ditulis di isi.
        let labels: Vec<String> = (1..=4)
            .map(|i| format!("Lini operasional nomor {i} bagian pemotongan produk contoh ukuran"))
            .collect();
        let rows: Vec<Vec<&str>> = labels
            .iter()
            .map(|l| vec![l.as_str(), ""])
            .chain([vec!["Month", "Prod"], vec!["Apr", "10"]])
            .collect();
        let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
        let t = text("Prod", &rows);
        for l in &labels {
            assert!(t.contains(l.as_str()), "{l}\n{t}");
        }
        let heading = t.lines().find(|l| l.starts_with("## Prod ›")).unwrap();
        assert!(heading.chars().count() <= 200, "{heading}");
    }

    #[test]
    fn long_rows_repeat_their_identity_and_keep_every_value() {
        let observation = "retak halus di ujung produk ".repeat(90);
        let description = format!(
            "produk contoh generik {} varian khusus Z9",
            "ukuran ".repeat(25).trim()
        );
        let t = text(
            "Review",
            &[
                &["Produk", "Observation", "Qty"],
                &[description.as_str(), observation.trim(), "3"],
                &["Review-8", "ok", "1"],
            ],
        );
        assert!(t.contains(&format!("- Produk: {description}\n")), "{t}");
        let id = clip(&format!("Produk: {description}"), MAX_ID_CHARS);
        assert!(t.contains(&format!("- {id} · Qty: 3\n")), "{t}");
        for line in t.lines().filter(|l| l.contains("Observation:")) {
            assert!(
                line.starts_with(&format!("- {id} · Observation: ")),
                "{line}"
            );
            assert!(line.chars().count() < 1000, "{}", line.len());
        }
        // Baris biasa sesudahnya tetap di bawah kepala tabel.
        assert!(
            t.contains(
                "| Produk | Observation | Qty |\n| --- | --- | --- |\n| Review-8 | ok | 1 |"
            ),
            "{t}"
        );
    }

    #[test]
    fn wide_tables_write_rows_as_key_values() {
        let header: Vec<String> = (0..40).map(|i| format!("Specification {i}")).collect();
        let values: Vec<String> = (0..40).map(|i| i.to_string()).collect();
        let cells = vec![header, values];
        let (t, _) = to_text(
            &[Sheet {
                name: "Lebar".into(),
                hidden: false,
                cells: &cells,
            }],
            usize::MAX,
        );
        assert!(
            t.contains("- Specification 0: 0; Specification 1: 1;"),
            "{t}"
        );
        assert!(!t.contains("| --- |"), "{t}");
    }

    #[test]
    fn output_is_limited_in_characters_and_reports_it() {
        let row = "表".repeat(300);
        let cells: Vec<Vec<String>> = (0..100).map(|_| vec![row.clone(), "1".into()]).collect();
        let sheet = |cells| Sheet {
            name: "Besar".into(),
            hidden: false,
            cells,
        };
        let (t, cut) = to_text(&[sheet(&cells)], 10_000);
        assert!(cut);
        let chars = t.chars().count();
        assert!((10_000..10_400).contains(&chars), "{chars}");
        let (_, cut) = to_text(&[sheet(&cells)], usize::MAX);
        assert!(!cut);
    }

    #[test]
    #[ignore = "tes performa: cargo test --release -- --ignored stay_fast"]
    fn many_spacer_columns_stay_fast() {
        // 300 baris × 2.000 kolom dengan kolom kosong berselang: tidak boleh kuadratik.
        let cells: Vec<Vec<String>> = (0..300)
            .map(|r| {
                (0..2_000)
                    .map(|c| {
                        if c % 2 == 0 {
                            format!("{r}-{c}")
                        } else {
                            String::new()
                        }
                    })
                    .collect()
            })
            .collect();
        let t0 = std::time::Instant::now();
        let (t, _) = to_text(
            &[Sheet {
                name: "Lebar".into(),
                hidden: false,
                cells: &cells,
            }],
            usize::MAX,
        );
        assert!(t.contains("299-1998"));
        assert!(t0.elapsed().as_secs() < 5, "{:?}", t0.elapsed());
    }

    #[test]
    fn hidden_sheets_delimiters_and_empty_sheets() {
        let delimiters = grid(&[&["Kode", "Isi"], &["A\\|B", "x|y"], &["C", "7"]]);
        let empty = grid(&[&["", ""]]);
        let (t, _) = to_text(
            &[
                Sheet {
                    name: "Kosong".into(),
                    hidden: false,
                    cells: &empty,
                },
                Sheet {
                    name: "Arsip".into(),
                    hidden: true,
                    cells: &delimiters,
                },
            ],
            usize::MAX,
        );
        assert_eq!(
            t,
            "## Arsip (tersembunyi)\n\n## Arsip (tersembunyi)\n| Kode | Isi |\n| A\\\\\\|B | x\\|y |\n| C | 7 |\n\n"
        );
    }

    #[test]
    fn no_cell_is_ever_dropped() {
        // Grid acak (deterministik) berisi label, kepala, angka, tahun, teks, dan baris
        // kosong: setiap sel yang terisi harus muncul di keluaran.
        let mut seed: u64 = 7;
        let mut rand = |n: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        let words = [
            "FY 2026",
            "FY 2025",
            "Intake",
            "Month",
            "Prod",
            "Apr",
            "Total",
            "Review-1",
            "N/A",
            "-",
            "17 Nov 2026",
            "Bolt",
        ];
        for _ in 0..3_000 {
            let (h, w) = (1 + rand(12) as usize, 1 + rand(7) as usize);
            let cells: Vec<Vec<String>> = (0..h)
                .map(|_| {
                    (0..w)
                        .map(|_| match rand(6) {
                            0 | 1 => String::new(),
                            2 => words[rand(words.len() as u64) as usize].to_string(),
                            3 => (2000 + rand(30)).to_string(),
                            4 => rand(100_000).to_string(),
                            _ => format!("teks {}", rand(50)),
                        })
                        .collect()
                })
                .collect();
            let (t, cut) = to_text(
                &[Sheet {
                    name: "Acak".into(),
                    hidden: false,
                    cells: &cells,
                }],
                usize::MAX,
            );
            assert!(!cut);
            for c in cells.iter().flatten().filter(|c| !c.is_empty()) {
                assert!(t.contains(c.as_str()), "{c:?} hilang\n{cells:?}\n{t}");
            }
        }
    }

    #[test]
    fn numbers_and_headers() {
        assert!(is_number("1,234.5") && is_number("12%") && is_number("-3"));
        assert!(!is_number("Apr") && !is_number("") && !is_number("-"));
        assert!(is_header_start(&["Month", "Prod (pcs)", ""]));
        assert!(!is_header_start(&["Item", "2025"]));
        assert!(is_header_start(&["Item", "Tahun", "2025"]));
        assert!(is_header_start(&["Item", "2025", "2026"]));
        assert!(!is_header_start(&["Apr", "37611"]));
        assert!(!is_header_start(&["Apr", "2025"]));
        assert!(is_subheader(
            &["Month", "Production", ""],
            &["", "Actual", "Plan"],
            None
        ));
        assert!(!is_subheader(
            &["Sample", "Result", "Reading"],
            &["Review-001", "Pass", ""],
            None
        ));
        assert!(!is_subheader(
            &["Sample", "", "Result", ""],
            &["Elbow", "Galvanized", "Pass", "Visual only"],
            None
        ));
        assert!(!is_subheader(
            &["Month", "Production", ""],
            &["Apr", "2025", "pcs"],
            None
        ));
        assert!(is_subheader(
            &["Item", "Production", "", "", ""],
            &["", "2025", "", "2026", ""],
            None
        ));
        assert!(!is_label(&["", "10"]) && !is_label(&["12"]) && is_label(&["FY 2026", ""]));
        assert!(same_kind("Review-001", "Review-002") && same_kind("Bolt M8", "Nut M10"));
        assert!(!same_kind("Item", "Module type 058") && !same_kind("2025", "10"));
        assert_eq!(shape("FY 2025"), shape("FY 2026"));
        assert_ne!(shape("Intake"), shape("Dispatch"));
    }
}
