use super::unreadable;
use crate::files::parse_cell;
use quick_xml::{Reader, events::Event};
use std::{
    collections::{HashMap, HashSet},
    io::BufRead,
};

#[derive(Default)]
pub(super) struct SheetXml {
    pub row_count: usize,
    pub merged_columns: usize,
    pub merged: usize,
    pub large_integers: HashMap<(u32, u32), i64>,
    pub array_formulas: HashSet<usize>,
}

#[derive(Clone, Copy)]
pub(super) enum Scan {
    Values,
    Arrays,
}

pub(super) fn inspect(input: impl BufRead, scan: Scan) -> Result<SheetXml, String> {
    let mut reader = Reader::from_reader(input);
    reader.config_mut().expand_empty_elements = true;
    let error = |_| unreadable("xlsx");
    let mut buffer = Vec::new();
    let mut sheet = SheetXml::default();
    let mut in_rows = false;
    let mut in_cell = false;
    let mut cell_number = 0;
    let values = matches!(scan, Scan::Values);
    let (mut row, mut col) = (0_usize, 0_usize);
    let mut numeric = false;
    let mut in_value = false;
    // Sel tanpa nilai atau rumus hanya membawa format (misalnya satu kolom diformat sampai
    // baris 1.048.576). Calamine melewatinya, jadi sel itu juga tidak menambah jumlah baris.
    let mut has_content = false;
    loop {
        buffer.clear();
        match reader.read_event_into(&mut buffer).map_err(error)? {
            Event::Start(element) => match element.local_name().as_ref() {
                "sheetData" => in_rows = true,
                "row" if in_rows && values => {
                    row = element
                        .try_get_attribute("r")
                        .map_err(|_| unreadable("xlsx"))?
                        .and_then(|a| a.value.parse().ok())
                        .unwrap_or_else(|| row.saturating_add(1));
                    col = 0;
                }
                "c" if in_rows => {
                    in_cell = true;
                    cell_number += 1;
                    if !values {
                        continue;
                    }
                    if let Some((c, r)) = element
                        .try_get_attribute("r")
                        .map_err(|_| unreadable("xlsx"))?
                        .and_then(|a| parse_cell(&a.value))
                    {
                        (col, row) = (c, r);
                    }
                    has_content = false;
                    numeric = element
                        .try_get_attribute("t")
                        .map_err(|_| unreadable("xlsx"))?
                        .is_none_or(|a| a.value == "n");
                }
                "f" if !values && in_cell => {
                    if element
                        .try_get_attribute("t")
                        .map_err(|_| unreadable("xlsx"))?
                        .is_some_and(|a| a.value == "array")
                    {
                        sheet.array_formulas.insert(cell_number);
                    }
                }
                "v" | "is" | "f" if in_cell && values => {
                    has_content = true;
                    in_value = numeric && element.local_name().as_ref() == "v";
                }
                "mergeCell" if values => {
                    sheet.merged += 1;
                    if let Some((col, row)) = element
                        .try_get_attribute("ref")
                        .map_err(|_| unreadable("xlsx"))?
                        .and_then(|a| parse_cell(a.value.rsplit(':').next()?))
                    {
                        sheet.merged_columns = sheet.merged_columns.max(col + 1);
                        sheet.row_count = sheet.row_count.max(row);
                    }
                }
                _ => {}
            },
            Event::End(element) => match element.local_name().as_ref() {
                "sheetData" => in_rows = false,
                "c" if in_rows => {
                    if values && has_content {
                        sheet.row_count = sheet.row_count.max(row);
                    }
                    in_cell = false;
                    numeric = false;
                    col += 1;
                }
                "v" => in_value = false,
                _ => {}
            },
            Event::Text(value) if in_value && value.len() >= 16 && row > 0 => {
                if let Some(number) = exact_integer(value.as_ref())
                    && number.unsigned_abs() > (1_u64 << 53)
                {
                    sheet
                        .large_integers
                        .insert(((row - 1) as u32, col as u32), number);
                }
            }
            Event::Eof => return Ok(sheet),
            _ => {}
        }
    }
}

fn exact_integer(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    if let Ok(integer) = raw.parse() {
        return Some(integer);
    }
    let (mantissa, exponent) = match raw.split_once(['e', 'E']) {
        Some((m, e)) => (m, e.parse::<i32>().ok()?),
        None => (raw, 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let coefficient = format!("{whole}{fraction}").parse::<i128>().ok()?;
    let shift = exponent.checked_sub(i32::try_from(fraction.len()).ok()?)?;
    let factor = 10_i128.checked_pow(shift.unsigned_abs())?;
    let integer = if shift >= 0 {
        coefficient.checked_mul(factor)?
    } else {
        if coefficient % factor != 0 {
            return None;
        }
        coefficient / factor
    };
    i64::try_from(integer).ok()
}
