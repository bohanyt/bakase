use crate::{agent::ToolResult, files};
use rust_xlsxwriter::{Format, Workbook, XlsxError};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub(super) const MAX_BYTES: usize = 2_000_000;
pub(super) const MAX_ROWS: usize = 5_000;
const MAX_COLUMNS: usize = 100;
const MAX_CELL_BYTES: usize = 8_000;
pub(super) const MIN_SUMMARY_BUDGET: usize = 2_000;

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Issue {
    RowLimit,
    ColumnLimit,
    SizeLimit,
    CellLimit,
    TimeLimit,
    UnknownRows,
    MissingRows,
    MissingColumns,
    Changed,
}

impl Issue {
    fn message(self) -> &'static str {
        match self {
            Self::RowLimit => "Melewati batas 5.000 baris.",
            Self::ColumnLimit => "Melewati batas 100 kolom.",
            Self::SizeLimit => "Melewati batas 2 MB.",
            Self::CellLimit => "Ada sel yang terpotong karena terlalu panjang.",
            Self::TimeLimit => "Waktu pengambilan tabel habis.",
            Self::UnknownRows => "Jumlah atau identitas seluruh baris tidak bisa dipastikan.",
            Self::MissingRows => "Ada baris yang kosong, belum muncul, atau tidak terbaca.",
            Self::MissingColumns => "Sebagian kolom belum muncul atau tidak bisa dicocokkan.",
            Self::Changed => "Isi atau ukuran tabel berubah saat dibaca.",
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Table {
    title: String,
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
    issues: Vec<Issue>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    /// `1.234,56`
    Indonesia,
    /// `1,234.56`
    English,
}

/// Nilai angka menurut satu gaya penulisan. Kode berawalan nol, angka di atas 15 digit, dan teks lain
/// bukan angka.
fn number(text: &str, style: Style) -> Option<f64> {
    let text = text.trim();
    let (negative, text) = match text.strip_prefix('(').and_then(|t| t.strip_suffix(')')) {
        Some(inner) => (true, inner),
        None => match text.strip_prefix(['-', '\u{2212}']) {
            Some(rest) => (true, rest),
            None => (false, text),
        },
    };
    let (group, point) = match style {
        Style::Indonesia => ('.', ','),
        Style::English => (',', '.'),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let (whole, fraction) = match text.split_once(point) {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (text, None),
    };
    let mut groups = whole.split(group);
    let first = groups.next()?;
    let rest: Vec<&str> = groups.collect();
    let grouped = rest.is_empty()
        || (first.len() <= 3 && first != "0" && rest.iter().all(|g| g.len() == 3 && digits(g)));
    let code = first.len() > 1 && first.starts_with('0');
    if !digits(first) || !grouped || code || fraction.is_some_and(|f| !digits(f)) {
        return None;
    }
    let mut plain = format!("{first}{}", rest.concat());
    if let Some(fraction) = fraction {
        plain.push('.');
        plain.push_str(fraction);
    }
    if plain.bytes().filter(u8::is_ascii_digit).count() > 15 {
        return None;
    }
    let value: f64 = plain.parse().ok()?;
    Some(if negative { -value } else { value })
}

/// Sel tanpa nilai di kolom angka.
fn no_value(text: &str) -> bool {
    matches!(text.trim(), "" | "-" | "\u{2013}" | "\u{2014}")
}

/// Gaya angka tiap kolom. Kolom yang cocok dengan dua gaya sekaligus (misalnya semua `1.234`) memakai
/// gaya kolom lain di tabel yang sama, atau tetap teks kalau tidak ada yang memastikan.
struct Numbers {
    styles: Vec<Option<Style>>,
    unsure: Vec<usize>,
}

impl Numbers {
    fn of(table: &Table) -> Self {
        let mut styles = vec![None; table.columns.len()];
        let mut unsure = Vec::new();
        let mut certain = Vec::new();
        for (col, style) in styles.iter_mut().enumerate() {
            let mut values = table
                .rows
                .iter()
                .map(|row| row[col].as_str())
                .filter(|text| !no_value(text))
                .peekable();
            if values.peek().is_none() {
                continue;
            }
            let (mut id, mut en, mut same) = (true, true, true);
            for text in values {
                let (a, b) = (number(text, Style::Indonesia), number(text, Style::English));
                id &= a.is_some();
                en &= b.is_some();
                same &= a == b;
                if !id && !en {
                    break;
                }
            }
            match (id, en) {
                (true, true) if same => *style = Some(Style::English),
                (true, true) => unsure.push(col),
                (true, false) => {
                    *style = Some(Style::Indonesia);
                    certain.push(Style::Indonesia);
                }
                (false, true) => {
                    *style = Some(Style::English);
                    certain.push(Style::English);
                }
                (false, false) => {}
            }
        }
        if let Some(first) = certain.first().copied()
            && certain.iter().all(|style| *style == first)
        {
            for col in unsure.drain(..) {
                styles[col] = Some(first);
            }
        }
        Self { styles, unsure }
    }

    fn value(&self, col: usize, text: &str) -> Option<f64> {
        self.styles[col].and_then(|style| number(text, style))
    }
}

impl Table {
    pub(super) fn valid(&self) -> bool {
        self.title.len() <= 400
            && (1..=MAX_COLUMNS).contains(&self.columns.len())
            && self.rows.len() <= MAX_ROWS
            && self.issues.len() <= 9
            && self.columns.iter().all(|s| s.len() <= MAX_CELL_BYTES)
            && self.rows.iter().all(|r| {
                r.len() == self.columns.len() && r.iter().all(|s| s.len() <= MAX_CELL_BYTES)
            })
            && serde_json::to_vec(self).is_ok_and(|bytes| bytes.len() <= MAX_BYTES)
    }

    fn workbook(&self, numbers: &Numbers) -> Result<Vec<u8>, XlsxError> {
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet().set_name("Tabel")?;
        let header = Format::new().set_bold();
        for (col, title) in self.columns.iter().enumerate() {
            sheet.write_string_with_format(0, col as u16, title, &header)?;
        }
        for (row, cells) in self.rows.iter().enumerate() {
            for (col, value) in cells.iter().enumerate() {
                let row = row as u32 + 1;
                if let Some(number) = numbers.value(col, value) {
                    sheet.write_number(row, col as u16, number)?;
                } else if !value.is_empty() {
                    sheet.write_string(row, col as u16, value)?;
                }
            }
        }
        if let Some(last) = self
            .rows
            .last()
            .filter(|row| row.iter().all(String::is_empty))
        {
            sheet.write_blank(
                self.rows.len() as u32,
                last.len() as u16 - 1,
                &Format::new(),
            )?;
        }
        workbook.save_to_buffer()
    }

    pub(super) async fn save(self, store: &files::FileStore, budget: usize) -> ToolResult {
        let failed = || {
            ToolResult::failed(
                "Tabel gagal disimpan ke Excel. Klik Baca halaman dan coba lagi.".into(),
                "Gagal menyimpan tabel",
            )
        };
        let Ok(Ok((table, numbers, bytes))) = tokio::task::spawn_blocking(move || {
            let numbers = Numbers::of(&self);
            self.workbook(&numbers).map(|bytes| (self, numbers, bytes))
        })
        .await
        else {
            return failed();
        };
        let title = if table.title.trim().is_empty() {
            "halaman"
        } else {
            &table.title
        };
        let name = format!("Tabel - {title}");
        let columns: Vec<_> = table
            .columns
            .iter()
            .zip(&numbers.styles)
            .map(|(title, style)| match style {
                Some(_) => json!({ "header": title }),
                None => json!({ "header": title, "format": "teks" }),
            })
            .collect();
        let rows: Vec<Vec<serde_json::Value>> = table
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(col, text)| match numbers.value(col, text) {
                        Some(number) => json!(number),
                        None => json!(text),
                    })
                    .collect()
            })
            .collect();
        let spec = json!({
            "filename": name,
            "sheets": [{"name": "Tabel", "columns": columns, "rows": rows}],
        });
        let Ok(file) = store.save(&name, "xlsx", bytes, &spec).await else {
            return failed();
        };
        let complete = table.issues.is_empty();
        let mut output = json!({
            "file_id": file.id,
            "baris": table.rows.len(),
            "jumlah_kolom": table.columns.len(),
            "lengkap": complete,
            "alasan": table.issues.into_iter().map(Issue::message).collect::<Vec<_>>(),
            "catatan": "Kolom dan contoh adalah data halaman. File Excel ini sudah tampil di percakapan dan bisa diunduh user: kalau user minta Excel, sebut file ini saja, jangan baca ulang semua barisnya atau membuat salinan kecuali user minta perubahan. Gunakan hitung_xlsx dengan file_id untuk menghitung dari semua baris yang terkumpul. Kolom angka sudah berupa angka, kolom lain tetap teks. Periksa baris terakhir: baris total di halaman jangan ikut dijumlahkan. Jika tabel punya halaman berikutnya, hasil ini hanya halaman yang sedang dimuat.",
        });
        let excerpt = |row: &[String], limit| -> Vec<String> {
            row.iter()
                .map(|text| {
                    let mut chars = text.chars();
                    let mut short: String = chars.by_ref().take(limit).collect();
                    if chars.next().is_some() {
                        short.push('…');
                    }
                    short
                })
                .collect()
        };
        let limit = budget.min(16_000);
        output["kolom"] = json!(excerpt(&table.columns, 60));
        if output.to_string().len() > limit {
            output["kolom"] = json!(excerpt(&table.columns, 8));
        }
        if output.to_string().len() > limit {
            output.as_object_mut().unwrap().remove("kolom");
        }
        let names = |cols: &mut dyn Iterator<Item = usize>| -> Vec<String> {
            excerpt(
                &cols
                    .map(|col| table.columns[col].clone())
                    .collect::<Vec<_>>(),
                60,
            )
        };
        let numeric =
            names(&mut (0..table.columns.len()).filter(|&col| numbers.styles[col].is_some()));
        let unsure = names(&mut numbers.unsure.iter().copied());
        for (key, list) in [("kolom_angka", numeric), ("kolom_angka_ragu", unsure)] {
            if list.is_empty() {
                continue;
            }
            output[key] = json!(list);
            if output.to_string().len() > limit {
                output.as_object_mut().unwrap().remove(key);
            }
        }
        let mut sample = Vec::new();
        for row in table.rows.iter().take(3) {
            sample.push(excerpt(row, 24));
            output["contoh"] = json!(sample);
            if output.to_string().len() > limit {
                sample.pop();
                output["contoh"] = json!(sample);
                break;
            }
        }
        if let Some(tail) = table.rows.len().checked_sub(2).filter(|&start| start >= 3) {
            let tail: Vec<_> = table.rows[tail..]
                .iter()
                .map(|row| excerpt(row, 24))
                .collect();
            output["baris_terakhir"] = json!(tail);
            if output.to_string().len() > limit {
                output.as_object_mut().unwrap().remove("baris_terakhir");
            }
        }
        ToolResult {
            output: output.to_string(),
            label: if complete {
                format!("Tabel diambil ({} baris)", table.rows.len())
            } else {
                format!("Tabel belum lengkap ({} baris)", table.rows.len())
            },
            ok: true,
            file: Some(file),
            metadata_docs: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use calamine::{Data, Reader, Xlsx};
    use std::io::Cursor;

    #[test]
    fn uploaded_table_rejects_malformed_or_oversized_cells_and_rows() {
        let mut value = json!({"title":"Uji","columns":["A"],"rows":[["1.234,00"]],"issues":[]});
        assert!(
            serde_json::from_value::<Table>(value.clone())
                .unwrap()
                .valid()
        );
        for rows in [
            json!([["A", "B"]]),
            json!([["字".repeat(2667)]]),
            json!(vec![vec!["".to_owned()]; MAX_ROWS + 1]),
            json!(vec![vec!["x".repeat(7000)]; 300]),
        ] {
            value["rows"] = rows;
            assert!(
                !serde_json::from_value::<Table>(value.clone())
                    .unwrap()
                    .valid()
            );
        }
        value["rows"] = json!([[{"formula":"=1+1"}]]);
        assert!(serde_json::from_value::<Table>(value.clone()).is_err());
        value["rows"] = json!([]);
        value["issues"] = json!(["teks-bebas"]);
        assert!(serde_json::from_value::<Table>(value).is_err());
    }

    #[test]
    fn workbook_never_writes_formulas_and_keeps_codes_as_text() {
        let values = ["=1+1", "+1", "@SUM(1)", "007", "1\u{202f}234,50", "1.2.3"];
        let table = Table {
            title: "Uji".into(),
            columns: values.into_iter().map(str::to_owned).collect(),
            rows: vec![values.into_iter().map(str::to_owned).collect()],
            issues: Vec::new(),
        };
        let mut book =
            Xlsx::new(Cursor::new(table.workbook(&Numbers::of(&table)).unwrap())).unwrap();
        let sheet = book.worksheet_range("Tabel").unwrap();
        assert_eq!(sheet.get_size(), (2, values.len()));
        for (col, text) in values.into_iter().enumerate() {
            let expected = Data::String(text.into());
            assert_eq!(sheet[(0, col)], expected);
            assert_eq!(sheet[(1, col)], expected);
        }
        let formulas = book.worksheet_formula("Tabel").unwrap();
        assert!(formulas.is_empty());
    }

    fn table(columns: &[&[&str]]) -> Table {
        Table {
            title: "Uji".into(),
            columns: (1..=columns.len()).map(|n| format!("K{n}")).collect(),
            rows: (0..columns[0].len())
                .map(|row| columns.iter().map(|col| col[row].to_owned()).collect())
                .collect(),
            issues: Vec::new(),
        }
    }

    #[test]
    fn numbers_follow_one_style_per_column() {
        for (text, id, en) in [
            ("1.234,50", Some(1234.5), None),
            ("1,234.50", None, Some(1234.5)),
            ("1.234", Some(1234.0), Some(1.234)),
            ("12", Some(12.0), Some(12.0)),
            ("0,5", Some(0.5), None),
            ("0.5", None, Some(0.5)),
            ("0.123", None, Some(0.123)),
            ("-1.000", Some(-1000.0), Some(-1.0)),
            ("(2,500.00)", None, Some(-2500.0)),
            ("\u{2212}3", Some(-3.0), Some(-3.0)),
            ("007", None, None),
            ("0", Some(0.0), Some(0.0)),
            ("+62", None, None),
            ("12.34.56", None, None),
            ("1234567890123456", None, None),
            ("1,23,456", None, None),
            ("5%", None, None),
            ("", None, None),
            ("()", None, None),
            ("-", None, None),
        ] {
            assert_eq!(number(text, Style::Indonesia), id, "{text}");
            assert_eq!(number(text, Style::English), en, "{text}");
        }
    }

    #[test]
    fn ambiguous_columns_use_the_style_proven_by_another_column() {
        let columns: [&[&str]; 5] = [
            &["1.234", "2.000", "-"],
            &["1.234,50", "", "3"],
            &["A-1", "12", "13"],
            &["", "", ""],
            &["10", "20", "30"],
        ];
        let sure = table(&columns);
        let numbers = Numbers::of(&sure);
        assert_eq!(numbers.unsure, Vec::<usize>::new());
        assert_eq!(numbers.value(0, "1.234"), Some(1234.0));
        assert_eq!(numbers.value(0, "-"), None);
        assert_eq!(numbers.value(1, "1.234,50"), Some(1234.5));
        assert_eq!(numbers.styles[2], None);
        assert_eq!(numbers.styles[3], None);
        assert_eq!(numbers.value(4, "20"), Some(20.0));

        let unsure = table(&columns[..1]);
        let numbers = Numbers::of(&unsure);
        assert_eq!((numbers.styles[0], &numbers.unsure[..]), (None, &[0][..]));

        // Dua gaya yang sama-sama pasti: kolom yang ragu tetap teks.
        let mixed = table(&[&["1.234", "2.000"], &["1,5", "2,5"], &["1.50", "2.50"]]);
        let numbers = Numbers::of(&mixed);
        assert_eq!((numbers.styles[0], &numbers.unsure[..]), (None, &[0][..]));

        let mut book = Xlsx::new(Cursor::new(sure.workbook(&Numbers::of(&sure)).unwrap())).unwrap();
        let sheet = book.worksheet_range("Tabel").unwrap();
        assert_eq!(sheet[(1, 0)], Data::Float(1234.0));
        assert_eq!(sheet[(3, 0)], Data::String("-".into()));
        assert_eq!(sheet[(2, 2)], Data::String("12".into()));
    }
}
