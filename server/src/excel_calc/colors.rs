use super::{TableSheet, Workbook, sql_error};
use crate::files::{column_letter, parse_cell};
use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{io::BufRead, num::NonZeroU32, ops::Range};

const MAX_COLORS: usize = 50_000;
const UNREADABLE: &str =
    "Format warna Excel ini tidak bisa dibaca. Simpan ulang workbook di Excel, lalu coba lagi.";

pub(super) fn populate(
    db: &mut Connection,
    book: &mut Workbook<'_>,
    sheets: &[TableSheet<'_>],
    multi: bool,
    output: &mut Value,
) -> Result<(), String> {
    let Workbook::Xlsx { package, parts, .. } = book else {
        return Err("Warna sel .xls belum didukung. Simpan salinan sebagai .xlsx untuk menghitung warnanya.".into());
    };
    let mut palette = Palette::default();
    if let Some(theme) = &parts.theme {
        palette.read_theme(package, theme)?;
    }
    let styles = parts
        .styles
        .as_deref()
        .map(|path| palette.read_styles(package, path))
        .transpose()?
        .unwrap_or_default();
    let mut conditional = false;
    let mut truncated = false;
    let mut remaining = MAX_COLORS;
    let tx = db.transaction().map_err(sql_error)?;
    {
        let sql = if multi {
            "INSERT INTO warna_sel (baris, kolom, warna, nama, sheet) VALUES (?1, ?2, ?3, ?4, ?5)"
        } else {
            "INSERT INTO warna_sel (baris, kolom, warna, nama) VALUES (?1, ?2, ?3, ?4)"
        };
        let mut insert = tx.prepare(sql).map_err(sql_error)?;
        for sheet in sheets {
            let path = parts.sheets.get(sheet.name).ok_or(UNREADABLE)?;
            let input = package.member(path)?;
            let colors = scan(input, sheet.rows.clone(), sheet.columns, &styles)?;
            conditional |= colors.conditional;
            // Baris yang disimpan mengikuti urutan sheet, baris, lalu kolom Excel.
            let cols = (0..sheet.columns).map(column_letter).collect::<Vec<_>>();
            for (row, col, rgb) in colors.fills(&styles) {
                if remaining == 0 {
                    truncated = true;
                    break;
                }
                remaining -= 1;
                let excel_row = (row + 1) as i64;
                let column = &cols[col];
                let hex = rgb.hex();
                let name = rgb.name();
                if multi {
                    insert.execute(params![excel_row, column, hex, name, sheet.name])
                } else {
                    insert.execute(params![excel_row, column, hex, name])
                }
                .map_err(sql_error)?;
            }
        }
    }
    tx.commit().map_err(sql_error)?;
    if conditional {
        output["catatan_warna"] = json!(
            "Pilihan sheet memuat pemformatan bersyarat. Tabel warna hanya memakai isi sel yang tersimpan dalam format dasar."
        );
    }
    if truncated {
        output["catatan_batas_warna"] = json!(format!(
            "Tabel warna terpotong pada {MAX_COLORS} sel. Pilih lebih sedikit sheet atau pecah file untuk menghitung semua warna."
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rgb(u32);

impl Rgb {
    fn parse(hex: &str) -> Option<Self> {
        let rgb = match hex.len() {
            6 => hex,
            8 => hex.get(2..)?,
            _ => return None,
        };
        rgb.bytes()
            .all(|b| b.is_ascii_hexdigit())
            .then(|| Self(u32::from_str_radix(rgb, 16).unwrap()))
    }

    fn hex(self) -> String {
        format!("#{:06X}", self.0)
    }

    fn channels(self) -> [f64; 3] {
        [16, 8, 0].map(|shift| f64::from((self.0 >> shift) & 255) / 255.0)
    }

    fn tint(self, tint: f64) -> Self {
        if tint == 0.0 {
            return self;
        }
        let values = self.channels();
        let min = values.into_iter().fold(1.0, f64::min);
        let max = values.into_iter().fold(0.0, f64::max);
        let lightness = (min + max) / 2.0;
        // Tint mengubah lightness HSL. Skala chroma menjaga hue dan saturasi.
        let next = if tint < 0.0 {
            lightness * (1.0 + tint)
        } else {
            lightness + (1.0 - lightness) * tint
        };
        let chroma = 1.0 - (2.0 * lightness - 1.0).abs();
        let scale = if chroma == 0.0 {
            0.0
        } else {
            (1.0 - (2.0 * next - 1.0).abs()) / chroma
        };
        Self(values.into_iter().fold(0, |rgb, channel| {
            let value = (next + (channel - lightness) * scale).clamp(0.0, 1.0);
            (rgb << 8) | (value * 255.0).round() as u32
        }))
    }

    fn name(self) -> &'static str {
        let [r, g, b] = self.channels();
        let max = r.max(g).max(b);
        let delta = max - r.min(g).min(b);
        if max <= 0.10 {
            return "hitam";
        }
        if delta <= 0.08 {
            return if max >= 0.95 { "putih" } else { "abu-abu" };
        }
        let hue = 60.0
            * if max == r {
                ((g - b) / delta).rem_euclid(6.0)
            } else if max == g {
                (b - r) / delta + 2.0
            } else {
                (r - g) / delta + 4.0
            };
        match hue as u32 {
            0..=14 | 345.. if max >= 0.75 && delta / max <= 0.55 => "merah muda",
            0..=14 | 345.. if max < 0.70 && delta / max < 0.85 => "cokelat",
            0..=14 | 345.. => "merah",
            15..=44 if max < 0.70 => "cokelat",
            15..=44 => "oranye",
            45..=69 => "kuning",
            70..=164 => "hijau",
            165..=194 => "toska",
            195..=255 => "biru",
            256..=305 => "ungu",
            _ => "merah muda",
        }
    }
}

const THEME_NAMES: [&str; 12] = [
    "lt1", "dk1", "lt2", "dk2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6",
    "hlink", "folHlink",
];
const OFFICE_THEME: [u32; 12] = [
    0xFFFFFF, 0x000000, 0xEEECE1, 0x1F497D, 0x4F81BD, 0xC0504D, 0x9BBB59, 0x8064A2, 0x4BACC6,
    0xF79646, 0x0000FF, 0x800080,
];
// Palet standar ECMA-376 18.8.27, ditambah warna foreground dan background sistem.
const INDEXED: [u32; 66] = [
    0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x000000,
    0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x800000, 0x008000,
    0x000080, 0x808000, 0x800080, 0x008080, 0xC0C0C0, 0x808080, 0x9999FF, 0x993366, 0xFFFFCC,
    0xCCFFFF, 0x660066, 0xFF8080, 0x0066CC, 0xCCCCFF, 0x000080, 0xFF00FF, 0xFFFF00, 0x00FFFF,
    0x800080, 0x800000, 0x008080, 0x0000FF, 0x00CCFF, 0xCCFFFF, 0xCCFFCC, 0xFFFF99, 0x99CCFF,
    0xFF99CC, 0xCC99FF, 0xFFCC99, 0x3366FF, 0x33CCCC, 0x99CC00, 0xFFCC00, 0xFF9900, 0xFF6600,
    0x666699, 0x969696, 0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366, 0x333399,
    0x333333, 0x000000, 0xFFFFFF,
];

struct Palette {
    theme: [Rgb; 12],
    indexed: [Rgb; 66],
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            theme: OFFICE_THEME.map(Rgb),
            indexed: INDEXED.map(Rgb),
        }
    }
}

enum Color {
    Rgb(Rgb),
    Theme(usize),
    Indexed(usize),
}

fn xml_color(attrs: &[docx_rs::OwnedAttribute]) -> Option<(Color, f64)> {
    use super::super::xml_attr;
    let color = if let Some(hex) = xml_attr(attrs, "rgb") {
        Color::Rgb(Rgb::parse(hex)?)
    } else if let Some(index) = xml_attr(attrs, "theme") {
        Color::Theme(index.parse().ok()?)
    } else {
        Color::Indexed(xml_attr(attrs, "indexed")?.parse().ok()?)
    };
    let tint = xml_attr(attrs, "tint")
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|tint| (-1.0..=1.0).contains(tint))
        .unwrap_or(0.0);
    Some((color, tint))
}

impl Palette {
    fn resolve(&self, spec: (Color, f64)) -> Option<Rgb> {
        let (color, tint) = spec;
        let rgb = match color {
            Color::Rgb(rgb) => rgb,
            Color::Theme(index) => *self.theme.get(index)?,
            Color::Indexed(index) => *self.indexed.get(index)?,
        };
        Some(rgb.tint(tint))
    }

    fn read_theme(
        &mut self,
        package: &mut super::super::XlsxPackage<'_>,
        path: &str,
    ) -> Result<(), String> {
        use super::super::xml_attr;
        use docx_rs::XmlEvent;
        let mut in_scheme = false;
        let mut slot = None;
        package.visit(path, |event| match event {
            XmlEvent::StartElement {
                name, attributes, ..
            } => {
                let local = name.local_name.as_str();
                match local {
                    "clrScheme" => in_scheme = true,
                    "srgbClr" | "sysClr" if in_scheme => {
                        if let Some(index) = slot.take()
                            && let Some(rgb) = xml_attr(
                                &attributes,
                                if local == "sysClr" { "lastClr" } else { "val" },
                            )
                            .and_then(Rgb::parse)
                        {
                            self.theme[index] = rgb;
                        }
                    }
                    _ if in_scheme => slot = THEME_NAMES.iter().position(|name| *name == local),
                    _ => {}
                }
            }
            XmlEvent::EndElement { name } if name.local_name == "clrScheme" => in_scheme = false,
            _ => {}
        })
    }

    fn read_styles(
        &mut self,
        package: &mut super::super::XlsxPackage<'_>,
        path: &str,
    ) -> Result<Vec<Option<Rgb>>, String> {
        use super::super::xml_attr;
        use docx_rs::XmlEvent;
        // `indexedColors` biasanya dipasang setelah `fills`, jadi selesaikan warna setelah XML dibaca.
        let mut fills = Vec::new();
        let mut parent_xfs = Vec::new();
        let mut cell_xfs = Vec::new();
        let mut section = String::new();
        let mut foreground = None;
        let mut background = None;
        let mut color_kind = None;
        let mut index = 0;
        package.visit(path, |event| match event {
            XmlEvent::StartElement {
                name, attributes, ..
            } => {
                let attrs = &attributes;
                let local = name.local_name.as_str();
                let number = |key| xml_attr(attrs, key).and_then(|s| s.parse::<usize>().ok());
                match (section.as_str(), local) {
                    ("", "fills" | "cellXfs" | "cellStyleXfs" | "indexedColors") => {
                        section = name.local_name
                    }
                    ("fills", "patternFill")
                        if xml_attr(attrs, "patternType").is_some_and(|p| p != "none") =>
                    {
                        color_kind = Some("pattern")
                    }
                    ("fills", "gradientFill") => color_kind = Some("gradient"),
                    ("fills", "fgColor") if color_kind == Some("pattern") => {
                        foreground = xml_color(attrs)
                    }
                    ("fills", "bgColor") if color_kind == Some("pattern") => {
                        background = xml_color(attrs)
                    }
                    ("fills", "color")
                        if color_kind == Some("gradient") && foreground.is_none() =>
                    {
                        foreground = xml_color(attrs)
                    }
                    ("cellStyleXfs", "xf") => parent_xfs.push(number("fillId")),
                    ("cellXfs", "xf") => {
                        let apply = !matches!(xml_attr(attrs, "applyFill"), Some("0" | "false"));
                        cell_xfs
                            .push((number("xfId"), if apply { number("fillId") } else { None }));
                    }
                    ("indexedColors", "rgbColor") => {
                        if let Some(entry) = self.indexed.get_mut(index)
                            && let Some(rgb) = xml_attr(attrs, "rgb").and_then(Rgb::parse)
                        {
                            *entry = rgb;
                        }
                        index += 1;
                    }
                    _ => {}
                }
            }
            XmlEvent::EndElement { name } if name.local_name == section => section.clear(),
            XmlEvent::EndElement { name } if section == "fills" && name.local_name == "fill" => {
                fills.push((foreground.take(), background.take()));
                color_kind = None;
            }
            _ => {}
        })?;
        let fills = fills
            .into_iter()
            .map(|(fg, bg)| {
                fg.and_then(|spec| self.resolve(spec))
                    .or_else(|| bg.and_then(|spec| self.resolve(spec)))
            })
            .collect::<Vec<_>>();
        Ok(cell_xfs
            .into_iter()
            .map(|(parent, fill)| {
                let fill = fill.or_else(|| {
                    parent.and_then(|index| parent_xfs.get(index).copied().flatten())
                })?;
                fills.get(fill).copied().flatten()
            })
            .collect())
    }
}

type StyleId = Option<NonZeroU32>;

struct SheetColors {
    rows: Range<usize>,
    row_styles: Vec<StyleId>,
    col_styles: Vec<StyleId>,
    cell_styles: Vec<StyleId>,
    conditional: bool,
}

impl SheetColors {
    fn fills<'a>(
        &'a self,
        styles: &'a [Option<Rgb>],
    ) -> impl Iterator<Item = (usize, usize, Rgb)> + 'a {
        let width = self.col_styles.len();
        self.cell_styles
            .iter()
            .enumerate()
            .filter_map(move |(offset, &cell)| {
                let (row, col) = (offset / width, offset % width);
                let style = cell.or(self.row_styles[row]).or(self.col_styles[col]);
                let index = style.map_or(0, |id| (id.get() - 1) as usize);
                Some((
                    self.rows.start + row,
                    col,
                    styles.get(index).copied().flatten()?,
                ))
            })
    }
}

fn attr_number(element: &BytesStart<'_>, key: &str) -> Option<usize> {
    element.try_get_attribute(key).ok()??.value.parse().ok()
}

fn style_id(element: &BytesStart<'_>, key: &str) -> StyleId {
    u32::try_from(attr_number(element, key)?)
        .ok()?
        .checked_add(1)
        .and_then(NonZeroU32::new)
}

fn scan(
    input: impl BufRead,
    rows: Range<usize>,
    width: usize,
    styles: &[Option<Rgb>],
) -> Result<SheetColors, String> {
    let limit = rows.len();
    let enabled = styles.iter().any(Option::is_some) && limit != 0 && width != 0;
    let mut sheet = SheetColors {
        rows,
        row_styles: vec![None; if enabled { limit } else { 0 }],
        col_styles: vec![None; if enabled { width } else { 0 }],
        cell_styles: vec![None; if enabled { limit * width } else { 0 }],
        conditional: false,
    };
    let mut reader = Reader::from_reader(input);
    let mut buffer = Vec::new();
    let mut sheet_data = false;
    let (mut row, mut col) = (0_usize, 0_usize);
    loop {
        buffer.clear();
        match reader
            .read_event_into(&mut buffer)
            .map_err(|_| UNREADABLE)?
        {
            Event::Start(element) | Event::Empty(element) => match element.local_name().as_ref() {
                "conditionalFormatting" => sheet.conditional = true,
                "sheetData" => sheet_data = true,
                "col" if enabled && !sheet_data => {
                    if let (Some(first), Some(last), Some(style)) = (
                        attr_number(&element, "min"),
                        attr_number(&element, "max"),
                        style_id(&element, "style"),
                    ) && let Some(columns) = sheet
                        .col_styles
                        .get_mut(first.saturating_sub(1)..last.min(width))
                    {
                        columns.fill(Some(style));
                    }
                }
                "row" if enabled && sheet_data => {
                    row = attr_number(&element, "r").unwrap_or_else(|| row.saturating_add(1));
                    col = 0;
                    if row > 0 && sheet.rows.contains(&(row - 1)) {
                        sheet.row_styles[row - 1 - sheet.rows.start] = style_id(&element, "s");
                    }
                }
                "c" if enabled && sheet_data => {
                    if let Some((c, r)) = element
                        .try_get_attribute("r")
                        .ok()
                        .flatten()
                        .and_then(|a| parse_cell(&a.value))
                    {
                        (col, row) = (c, r);
                    }
                    if col < width
                        && row > 0
                        && sheet.rows.contains(&(row - 1))
                        && let Some(style) = style_id(&element, "s")
                    {
                        sheet.cell_styles[(row - 1 - sheet.rows.start) * width + col] = Some(style);
                    }
                    col = col.saturating_add(1);
                }
                _ => {}
            },
            Event::End(element) if element.local_name().as_ref() == "sheetData" => {
                sheet_data = false
            }
            Event::Eof => return Ok(sheet),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachments::excel_calc::{Options, calculate};
    use std::io::{Cursor, Write};

    const STYLES: &str = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
      <fills count="6">
        <fill><patternFill patternType="none"><fgColor rgb="FF0000FF"/></patternFill></fill>
        <fill><patternFill patternType="gray125"/></fill>
        <fill><patternFill patternType="solid"><fgColor theme="4" tint="0.5"/></patternFill></fill>
        <fill><patternFill patternType="solid"><fgColor indexed="0"/></patternFill></fill>
        <fill><patternFill patternType="solid"><fgColor indexed="13"/></patternFill></fill>
        <fill><gradientFill><stop position="0"><color rgb="FFFFAA00"/></stop><stop position="1"><color rgb="FFFFFFFF"/></stop></gradientFill></fill>
      </fills>
      <cellStyleXfs count="2"><xf fillId="0"/><xf fillId="3"/></cellStyleXfs>
      <cellXfs count="6">
        <xf numFmtId="0" fillId="0" xfId="0"/>
        <xf numFmtId="0" fillId="2" xfId="0" applyFill="1"/>
        <xf numFmtId="0" fillId="0" xfId="1" applyFill="0"/>
        <xf numFmtId="0" fillId="1" xfId="0"/>
        <xf numFmtId="0" fillId="4" xfId="0"/>
        <xf numFmtId="0" fillId="5" xfId="0"/>
      </cellXfs>
      <colors><indexedColors><rgbColor rgb="FF00AA00"/></indexedColors></colors>
    </styleSheet>"#;
    const SHEET: &str = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
      <dimension ref="A1:F3"/>
      <sheetData>
        <row r="1"><c r="A1" s="4"><v>99</v></c></row>
        <row r="2"><c r="A2" s="1"><v>7</v></c><c r="B2" s="2"><v>8</v></c><c r="C2" s="3"><v>9</v></c><c r="D2"><v>10</v></c><c r="E2" s="4"><v>11</v></c><c r="F2" s="4"/></row>
        <row r="3"><c r="A3" s="5"><v>12</v></c></row>
      </sheetData>
      <mergeCells count="1"><mergeCell ref="E2:F2"/></mergeCells>
      <conditionalFormatting sqref="D2"><cfRule type="expression" dxfId="0" priority="1"><formula>TRUE()</formula></cfRule></conditionalFormatting>
    </worksheet>"#;
    const THEME: &str = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
      <a:themeElements><a:clrScheme name="Uji"><a:accent1><a:srgbClr val="204060"/></a:accent1></a:clrScheme></a:themeElements>
    </a:theme>"#;

    fn fixture(theme: &str) -> Vec<u8> {
        let mut book = rust_xlsxwriter::Workbook::new();
        book.add_worksheet().set_name("Data").unwrap();
        book.add_worksheet().set_name("Kosong").unwrap();
        let mut original =
            zip::ZipArchive::new(Cursor::new(book.save_to_buffer().unwrap())).unwrap();
        let mut patched = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for index in 0..original.len() {
            let part = original.by_index(index).unwrap();
            match part.name() {
                "xl/styles.xml" | "xl/worksheets/sheet1.xml" | "xl/theme/theme1.xml" => {
                    let xml = match part.name() {
                        "xl/styles.xml" => STYLES,
                        "xl/worksheets/sheet1.xml" => SHEET,
                        _ => theme,
                    };
                    patched
                        .start_file(part.name(), zip::write::SimpleFileOptions::default())
                        .unwrap();
                    patched.write_all(xml.as_bytes()).unwrap();
                }
                _ => patched.raw_copy_file(part).unwrap(),
            }
        }
        patched.finish().unwrap().into_inner()
    }

    fn query(kind: &str, bytes: &[u8], sheet: Option<&str>, sql: &str) -> Result<Value, String> {
        calculate(
            kind,
            bytes,
            Options {
                file: "Workbook uji".into(),
                sheet: sheet.map(str::to_owned),
                sheets: None,
                sql: Some(sql.to_owned()),
                budget: 40_000,
                pending: false,
                header_row: 1,
                simpan: None,
            },
            false,
        )
        .map(|result| serde_json::from_str(&result).unwrap())
    }

    #[test]
    fn joins_base_fills_to_excel_coordinates_and_reports_conditional_formats() {
        let bytes = fixture(THEME);
        let sql = "SELECT warna.baris, kolom, warna, nama, CASE kolom WHEN 'A' THEN A WHEN 'B' THEN B WHEN 'E' THEN E WHEN 'F' THEN F END FROM warna JOIN data USING (baris) ORDER BY baris, kolom";
        let expected = json!([
            [2, "A", "#70A0CF", "biru", 7],
            [2, "B", "#00AA00", "hijau", 8],
            [2, "E", "#FFFF00", "kuning", 11],
            [2, "F", "#FFFF00", "kuning", null],
            [3, "A", "#FFAA00", "oranye", 12]
        ]);
        for kind in ["xlsx", "xlsm"] {
            let output = query(kind, &bytes, None, sql).unwrap();
            assert_eq!(output["hasil"], expected);
            assert!(
                output["catatan_warna"]
                    .as_str()
                    .unwrap()
                    .contains("bersyarat")
            );
            assert!(output["catatan_batas_warna"].is_null());
        }
        let output = query(
            "xlsx",
            &bytes,
            Some("*"),
            "SELECT sheet, count(*) FROM warna GROUP BY sheet",
        )
        .unwrap();
        assert_eq!(output["hasil"], json!([["Data", 5]]));
        let empty = query("xlsx", &bytes, Some("Kosong"), "SELECT count(*) FROM warna").unwrap();
        assert_eq!(empty["hasil"], json!([[0]]));
        assert!(empty["catatan_warna"].is_null());
    }

    #[test]
    fn loads_colors_only_for_real_table_reads_and_keeps_sql_read_only() {
        let broken_theme = fixture("<rusak></salah>");
        for sql in [
            "SELECT 'warna' FROM data LIMIT 1",
            "WITH warna AS (SELECT 1) SELECT count(*) FROM warna",
            "SELECT SUM(A) FROM data /* warna */",
        ] {
            let output = query("xlsx", &broken_theme, None, sql).unwrap();
            assert!(output["catatan_warna"].is_null());
        }
        for sql in [
            "SELECT count(*) FROM WARNA",
            "WITH palet AS (SELECT * FROM main.warna) SELECT count(*) FROM palet",
        ] {
            assert!(query("xlsx", &broken_theme, None, sql).is_err(), "{sql}");
        }
        for sql in [
            "DELETE FROM warna RETURNING *",
            "WITH palet AS (SELECT * FROM warna) UPDATE warna SET nama = 'uji' RETURNING *",
            "SELECT * FROM warna; DROP TABLE warna",
            "SELECT count(*) FROM WARNA_SEL",
            "WITH warna AS (SELECT count(*) FROM warna_sel) SELECT * FROM warna",
        ] {
            assert!(query("xlsx", &fixture(THEME), None, sql).is_err());
        }
        let old = crate::attachments::xls_fixture(2, 1);
        assert_eq!(
            query("xls", &old, None, "SELECT SUM(A) FROM data").unwrap()["hasil"],
            json!([[5]])
        );
        assert!(
            query("xls", &old, None, "SELECT count(*) FROM warna")
                .unwrap_err()
                .contains(".xls belum didukung")
        );
    }

    #[test]
    fn clips_color_rows_and_keeps_the_excel_coordinates() {
        let mut book = rust_xlsxwriter::Workbook::new();
        let gray = rust_xlsxwriter::Format::new().set_background_color("#BFBFBF");
        let sheet = book.add_worksheet();
        sheet.set_column_format(0, &gray).unwrap();
        sheet.write((MAX_COLORS + 1) as u32, 0, 7).unwrap();
        let bytes = book.save_to_buffer().unwrap();
        let output = query(
            "xlsx",
            &bytes,
            None,
            "SELECT count(*), min(baris), max(baris), min(nama) FROM warna",
        )
        .unwrap();
        assert_eq!(
            output["hasil"],
            json!([[MAX_COLORS, 2, MAX_COLORS + 1, "abu-abu"]])
        );
        assert!(output["catatan_batas_warna"].is_string());
    }

    #[test]
    fn names_common_colors_and_tints_neutral_and_saturated_fills() {
        for (color, expected) in [
            (0x000000, "hitam"),
            (0xFFFFFF, "putih"),
            (0xBFBFBF, "abu-abu"),
            (0xFF0000, "merah"),
            (0x008000, "hijau"),
            (0x0000FF, "biru"),
            (0xFFA500, "oranye"),
            (0x800080, "ungu"),
            (0xFFC0CB, "merah muda"),
            (0xA52A2A, "cokelat"),
            (0x8B4513, "cokelat"),
            (0x008080, "toska"),
        ] {
            assert_eq!(Rgb(color).name(), expected, "#{color:06X}");
        }
        assert_eq!(Rgb(0xFFFFFF).tint(-0.5), Rgb(0x808080));
        assert_eq!(Rgb(0x0000FF).tint(0.5), Rgb(0x8080FF));
        assert_eq!(Rgb(0x0000FF).tint(-0.5), Rgb(0x000080));
    }
}
