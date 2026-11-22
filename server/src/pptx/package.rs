use super::{Cell, ChartKind, PptxSpec, Series, Slide};
use std::io::{Cursor, Read, Write};

use anyhow::{Context, Result};
use rust_xlsxwriter::{
    Chart, ChartAxisLabelPosition, ChartFont, ChartFormat, ChartLegendPosition, ChartLine,
    ChartMarker, ChartMarkerType, ChartPoint, ChartSolidFill, ChartType, Workbook,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const COLORS: [&str; 6] = ["C8102E", "2A78D6", "1BAF7A", "EDA100", "6250D6", "008300"];
const FONT: &str = "Segoe UI";
const TABLE_STYLE: &str = "{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}";
const TABLE_WIDTH: u32 = 10_590_000;
const TABLE_HEIGHT: u32 = 4_620_000;
const CELL_PAD_X: u32 = 150_000;
const CELL_PAD_Y: u32 = 60_000;
const GROUP: &str =
    r#"<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>"#;

pub fn build(spec: &PptxSpec) -> Result<Vec<u8>> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let mut types = String::new();
    let mut slide_ids = String::new();
    let mut slide_targets = Vec::new();
    for (index, slide) in spec.slides.iter().enumerate() {
        let n = index + 1;
        let chart_target = format!("../charts/chart{n}.xml");
        let mut links = vec![("slideLayout", "../slideLayouts/slideLayout1.xml")];
        if let Slide::Chart {
            chart_type,
            categories,
            series,
            ..
        } = slide
        {
            let (chart, workbook) = chart_parts(*chart_type, categories, series)?;
            let chart_path = format!("ppt/charts/chart{n}.xml");
            part(&mut zip, &chart_path, chart)?;
            part(&mut zip, &format!("ppt/embeddings/data{n}.xlsx"), workbook)?;
            part(
                &mut zip,
                &format!("ppt/charts/_rels/chart{n}.xml.rels"),
                rels(&[("package", &format!("../embeddings/data{n}.xlsx"))]),
            )?;
            types.push_str(&content_type(
                &format!("/{chart_path}"),
                "application/vnd.openxmlformats-officedocument.drawingml.chart+xml",
            ));
            links.push(("chart", chart_target.as_str()));
        }
        let shapes = slide_shapes(slide)?;
        part(
            &mut zip,
            &format!("ppt/slides/slide{n}.xml"),
            presentation_xml(
                "sld",
                &format!(
                    r#"<p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree>{GROUP}{shapes}</p:spTree></p:cSld>{COLOR_OVERRIDE}"#
                ),
            ),
        )?;
        part(
            &mut zip,
            &format!("ppt/slides/_rels/slide{n}.xml.rels"),
            rels(&links),
        )?;
        types.push_str(&content_type(
            &format!("/ppt/slides/slide{n}.xml"),
            "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
        ));
        slide_ids.push_str(&format!(
            r#"<p:sldId id="{}" r:id="rId{}"/>"#,
            255 + n,
            n + 1
        ));
        slide_targets.push(format!("slides/slide{n}.xml"));
    }
    let mut links = vec![("slideMaster", "slideMasters/slideMaster1.xml")];
    links.extend(
        slide_targets
            .iter()
            .map(|target| ("slide", target.as_str())),
    );
    links.push(("tableStyles", "tableStyles.xml"));
    part(&mut zip, "ppt/_rels/presentation.xml.rels", rels(&links))?;
    part(
        &mut zip,
        "_rels/.rels",
        rels(&[("officeDocument", "ppt/presentation.xml")]),
    )?;
    part(
        &mut zip,
        "ppt/presentation.xml",
        presentation_xml(
            "presentation",
            &format!(
                r#"<p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst>{slide_ids}</p:sldIdLst><p:sldSz cx="12192000" cy="6858000" type="screen16x9"/><p:notesSz cx="6858000" cy="9144000"/>"#
            ),
        ),
    )?;
    part(
        &mut zip,
        "ppt/slideMasters/slideMaster1.xml",
        presentation_xml(
            "sldMaster",
            &format!(
                r#"<p:cSld><p:spTree>{GROUP}</p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/><p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst><p:txStyles/>"#
            ),
        ),
    )?;
    part(
        &mut zip,
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        rels(&[
            ("slideLayout", "../slideLayouts/slideLayout1.xml"),
            ("theme", "../theme/theme1.xml"),
        ]),
    )?;
    part(
        &mut zip,
        "ppt/slideLayouts/slideLayout1.xml",
        presentation_xml(
            "sldLayout",
            &format!(
                r#"<p:cSld name="Kosong"><p:spTree>{GROUP}</p:spTree></p:cSld>{COLOR_OVERRIDE}"#
            ),
        ),
    )?;
    part(
        &mut zip,
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        rels(&[("slideMaster", "../slideMasters/slideMaster1.xml")]),
    )?;
    part(&mut zip, "ppt/theme/theme1.xml", theme())?;
    part(
        &mut zip,
        "ppt/tableStyles.xml",
        format!(r#"<a:tblStyleLst xmlns:a="{A}" def="{TABLE_STYLE}"/>"#),
    )?;
    part(
        &mut zip,
        "[Content_Types].xml",
        format!(
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Default Extension="xlsx" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"/>
<Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
<Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/>
<Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/>
<Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>
<Override PartName="/ppt/tableStyles.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.tableStyles+xml"/>
{types}</Types>"#
        ),
    )?;
    Ok(zip.finish()?.into_inner())
}

fn part(zip: &mut ZipWriter<Cursor<Vec<u8>>>, name: &str, data: impl AsRef<[u8]>) -> Result<()> {
    zip.start_file(
        name,
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
    )?;
    zip.write_all(data.as_ref())?;
    Ok(())
}

fn rels(links: &[(&str, &str)]) -> String {
    let mut xml = String::from(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for (index, (kind, target)) in links.iter().enumerate() {
        xml.push_str(&format!(
            r#"<Relationship Id="rId{}" Type="{R}/{kind}" Target="{target}"/>"#,
            index + 1
        ));
    }
    xml.push_str("</Relationships>");
    xml
}

fn content_type(path: &str, mime: &str) -> String {
    format!(r#"<Override PartName="{path}" ContentType="{mime}"/>"#)
}

const COLOR_OVERRIDE: &str = "<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>";
type Rect = (u32, u32, u32, u32);

fn presentation_xml(root: &str, body: &str) -> String {
    format!(
        r#"<p:{root} xmlns:p="{P}" xmlns:a="{A}" xmlns:r="{R}" xmlns:c="{C}">{body}</p:{root}>"#
    )
}

fn transform(prefix: &str, (x, y, w, h): Rect) -> String {
    format!(r#"<{prefix}:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{w}" cy="{h}"/></{prefix}:xfrm>"#)
}

fn paragraph(text: &str, size: u32, color: &str, bold: bool, bullet: bool) -> String {
    let text = quick_xml::escape::escape(text);
    let props = if bullet {
        r#"<a:pPr marL="350000" indent="-260000"><a:spcAft><a:spcPts val="1800"/></a:spcAft><a:buClr><a:srgbClr val="C8102E"/></a:buClr><a:buFont typeface="Segoe UI"/><a:buChar char="•"/></a:pPr>"#
    } else {
        "<a:pPr><a:buNone/></a:pPr>"
    };
    let bold = u8::from(bold);
    format!(
        r#"<a:p>{props}<a:r><a:rPr lang="id-ID" sz="{size}" b="{bold}"><a:solidFill><a:srgbClr val="{color}"/></a:solidFill><a:latin typeface="{FONT}"/></a:rPr><a:t>{text}</a:t></a:r></a:p>"#
    )
}

fn text_box(
    id: u32,
    rect: Rect,
    size: u32,
    color: &str,
    bold: bool,
    lines: &[&str],
    bullet: bool,
) -> String {
    let body: String = lines
        .iter()
        .map(|line| paragraph(line, size, color, bold, bullet))
        .collect();
    let xfrm = transform("a", rect);
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="Teks {id}"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr>{xfrm}<a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr lIns="0" tIns="0" rIns="0" bIns="0"><a:normAutofit/></a:bodyPr><a:lstStyle/>{body}</p:txBody></p:sp>"#
    )
}

fn slide_shapes(slide: &Slide) -> Result<String> {
    let title_slide = matches!(slide, Slide::Title { .. });
    let title_rect = if title_slide {
        (800_000, 2_170_000, 10_590_000, 1_200_000)
    } else {
        (800_000, 530_000, 10_590_000, 960_000)
    };
    let mut shapes = text_box(
        2,
        title_rect,
        if title_slide { 4400 } else { 3200 },
        "0D0D0D",
        true,
        &[slide.title()],
        false,
    );
    let body = match slide {
        Slide::Title {
            subtitle: Some(text),
            ..
        } => text_box(
            3,
            (800_000, 3_480_000, 10_590_000, 1_100_000),
            2200,
            "5D5D5D",
            false,
            &[text],
            false,
        ),
        Slide::Title { subtitle: None, .. } => String::new(),
        Slide::Bullets { items, .. } => text_box(
            3,
            (840_000, 1_720_000, 10_000_000, 4_580_000),
            2400,
            "0D0D0D",
            false,
            &items.iter().map(String::as_str).collect::<Vec<_>>(),
            true,
        ),
        Slide::Table { columns, rows, .. } => table(columns, rows)?,
        Slide::Chart { unit, .. } => format!(
            "{}{}",
            text_box(
                4,
                (800_000, 1_510_000, 10_590_000, 290_000),
                1500,
                "5D5D5D",
                false,
                &[&format!("Satuan: {unit}")],
                false
            ),
            frame(
                C,
                r#"<c:chart r:id="rId2"/>"#,
                (800_000, 1_860_000, 10_590_000, 4_480_000)
            ),
        ),
    };
    shapes.push_str(&body);
    Ok(shapes)
}

fn frame(uri: &str, body: &str, rect: Rect) -> String {
    let xfrm = transform("p", rect);
    format!(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="3" name="Data"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>{xfrm}<a:graphic><a:graphicData uri="{uri}">{body}</a:graphicData></a:graphic></p:graphicFrame>"#
    )
}

struct TableLayout {
    col_width: u32,
    size: u32,
    row_heights: Vec<u32>,
    cells: Vec<Vec<String>>,
}

pub(super) fn table_fits(columns: &[String], rows: &[Vec<Cell>]) -> bool {
    table_layout(columns, rows).is_some()
}

fn table_layout(columns: &[String], rows: &[Vec<Cell>]) -> Option<TableLayout> {
    let col_width = TABLE_WIDTH.checked_div(columns.len() as u32)?;
    let cells: Vec<Vec<String>> = std::iter::once(columns.to_vec())
        .chain(
            rows.iter()
                .map(|row| row.iter().map(ToString::to_string).collect()),
        )
        .collect();
    let available = col_width.saturating_sub(2 * CELL_PAD_X);
    for size in [1600, 1500, 1400, 1300, 1200, 1100, 1000] {
        let line_height = size * 127 * 13 / 10;
        let row_heights: Option<Vec<_>> = cells
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let lines = row.iter().try_fold(1, |max, cell| {
                    Some(max.max(table_cell_lines(cell, available, size, index == 0)?))
                })?;
                Some(lines * line_height + 2 * CELL_PAD_Y)
            })
            .collect();
        if let Some(mut row_heights) = row_heights {
            let total: u32 = row_heights.iter().sum();
            if total <= TABLE_HEIGHT {
                let extra = (TABLE_HEIGHT - total) / row_heights.len() as u32;
                for height in &mut row_heights {
                    *height += extra.min(780_000u32.saturating_sub(*height));
                }
                return Some(TableLayout {
                    col_width,
                    size,
                    row_heights,
                    cells,
                });
            }
        }
    }
    None
}

fn table_cell_lines(text: &str, available: u32, size: u32, bold: bool) -> Option<u32> {
    let available = available * 100 / 115;
    let available = if bold {
        available * 100 / 108
    } else {
        available
    };
    let limit = available * 100 / (size * 127);
    let mut lines = 1;
    let mut line_width = 0;
    let mut word = 0;
    let mut gap = 0;
    let mut place_word = |word: &mut u32, gap: &mut u32| {
        if *word == 0 {
            return Some(());
        }
        if *word > limit {
            return None;
        }
        if line_width != 0 && line_width + *gap + *word > limit {
            lines += 1;
            line_width = 0;
        }
        if line_width != 0 {
            line_width += *gap;
        }
        line_width += std::mem::take(word);
        *gap = 0;
        Some(())
    };
    for grapheme in text.graphemes(true) {
        let ch = grapheme.chars().next().unwrap();
        let wide = grapheme.width() >= 2;
        let width = table_grapheme_width(ch, wide);
        if ch.is_whitespace() && !matches!(ch, '\u{00a0}' | '\u{2007}' | '\u{202f}') {
            place_word(&mut word, &mut gap)?;
            gap += width;
        } else {
            if wide {
                place_word(&mut word, &mut gap)?;
            }
            word += width;
            if wide || matches!(ch, '-' | '‐' | '–') {
                place_word(&mut word, &mut gap)?;
            }
        }
    }
    place_word(&mut word, &mut gap)?;
    Some(lines)
}

// Perkiraan dalam seperseratus em untuk Segoe UI dan font fallback.
fn table_grapheme_width(ch: char, wide: bool) -> u32 {
    match ch {
        '\u{2000}' | '\u{2002}' => 50,
        '\u{2001}' | '\u{2003}' | '\u{3000}' => 100,
        '\u{2004}' => 33,
        '\u{2005}' => 25,
        '\u{2006}' => 17,
        '\u{2007}' => 55,
        '\u{2008}' => 35,
        '\u{2009}' => 20,
        '\u{200a}' => 10,
        ch if ch.is_whitespace() => 30,
        _ if wide => 110,
        '‱' => 150,
        '‰' => 130,
        'W' | 'M' | '@' | '%' => 100,
        'w' | 'm' | '&' => 90,
        'i' | 'l' | 'I' | '.' | ',' | ':' | '-' | '‐' => 35,
        '–' => 60,
        '0'..='9' | 'a'..='z' => 55,
        ch if ch.is_uppercase() => 75,
        ch if ch.is_lowercase() || ch.is_ascii() => 60,
        _ => 105,
    }
}

fn table(columns: &[String], rows: &[Vec<Cell>]) -> Result<String> {
    let TableLayout {
        col_width,
        size,
        row_heights,
        cells,
    } = table_layout(columns, rows).context("Tabel tidak muat di slide")?;
    let grid = format!(r#"<a:gridCol w="{col_width}"/>"#).repeat(columns.len());
    let mut table = format!(
        r#"<a:tbl><a:tblPr firstRow="1"><a:tableStyleId>{TABLE_STYLE}</a:tableStyleId></a:tblPr><a:tblGrid>{grid}</a:tblGrid>"#,
    );
    let total_height = row_heights.iter().sum();
    for (index, (cells, row_height)) in cells.iter().zip(row_heights).enumerate() {
        table.push_str(&format!(r#"<a:tr h="{row_height}">"#));
        let (fill, color) = match (index, index % 2) {
            (0, _) => ("FBE5E8", COLORS[0]),
            (_, 0) => ("F9F9F9", "0D0D0D"),
            _ => ("FFFFFF", "0D0D0D"),
        };
        for text in cells {
            let p = paragraph(text, size, color, index == 0, false);
            table.push_str(&format!(r#"<a:tc><a:txBody><a:bodyPr wrap="square" lIns="0" rIns="0" tIns="0" bIns="0"/><a:lstStyle/>{p}</a:txBody><a:tcPr marL="{CELL_PAD_X}" marR="{CELL_PAD_X}" marT="{CELL_PAD_Y}" marB="{CELL_PAD_Y}" anchor="ctr"><a:solidFill><a:srgbClr val="{fill}"/></a:solidFill></a:tcPr></a:tc>"#));
        }
        table.push_str("</a:tr>");
    }
    table.push_str("</a:tbl>");
    Ok(frame(
        "http://schemas.openxmlformats.org/drawingml/2006/table",
        &table,
        (
            800_000,
            1_650_000,
            col_width * columns.len() as u32,
            total_height,
        ),
    ))
}

fn chart_parts(
    kind: ChartKind,
    categories: &[String],
    series: &[Series],
) -> Result<(String, Vec<u8>)> {
    let chart_type = match kind {
        ChartKind::Bar => ChartType::Bar,
        ChartKind::Line => ChartType::Line,
        ChartKind::Pie => ChartType::Pie,
    };
    let mut book = Workbook::new();
    let sheet = book.add_worksheet().set_name("Data")?;
    sheet.write_string(0, 0, "Kategori")?;
    for (row, category) in categories.iter().enumerate() {
        sheet.write_string(row as u32 + 1, 0, category)?;
    }
    let last_row = categories.len() as u32;
    let mut chart = Chart::new(chart_type);
    let mut font = ChartFont::new();
    font.set_name(FONT).set_size(14).set_color("#5D5D5D");
    chart
        .legend()
        .set_font(&font)
        .set_position(ChartLegendPosition::Bottom);
    for (index, input) in series.iter().enumerate() {
        let col = index as u16 + 1;
        let color = format!("#{}", COLORS[index]);
        sheet.write_string(0, col, &input.name)?;
        for (row, value) in input.values.iter().enumerate() {
            sheet.write_number(row as u32 + 1, col, *value)?;
        }
        let series = chart
            .add_series()
            .set_name(("Data", 0, col))
            .set_categories(("Data", 1, 0, last_row, 0))
            .set_values(("Data", 1, col, last_row, col));
        match kind {
            ChartKind::Bar => {
                series.set_format(ChartSolidFill::new().set_color(&*color));
            }
            ChartKind::Line => {
                series.set_format(ChartLine::new().set_color(&*color).set_width(2.5));
                series.set_marker(
                    ChartMarker::new()
                        .set_type(ChartMarkerType::Circle)
                        .set_size(7)
                        .set_format(ChartSolidFill::new().set_color(&*color)),
                );
            }
            ChartKind::Pie => {
                let points: Vec<_> = COLORS
                    .iter()
                    .cycle()
                    .take(categories.len())
                    .map(|color| {
                        ChartPoint::new().set_format(
                            ChartSolidFill::new().set_color(format!("#{color}").as_str()),
                        )
                    })
                    .collect();
                series.set_points(&points);
            }
        }
    }
    if kind == ChartKind::Pie {
        chart.legend().set_position(ChartLegendPosition::Right);
    } else {
        if series.len() == 1 {
            chart.legend().set_hidden();
        }
        let mut grid = ChartLine::new();
        grid.set_color("#E5E5E5").set_width(0.75);
        chart
            .x_axis()
            .set_font(&font)
            .set_label_position(ChartAxisLabelPosition::Low);
        chart
            .y_axis()
            .set_font(&font)
            .set_label_position(ChartAxisLabelPosition::Low);
        match kind {
            ChartKind::Bar => {
                chart.y_axis().set_reverse();
                chart.x_axis().set_major_gridlines_line(&grid);
            }
            ChartKind::Line => {
                chart.y_axis().set_major_gridlines_line(&grid);
            }
            ChartKind::Pie => unreachable!(),
        }
    }
    chart
        .chart_area()
        .set_format(ChartFormat::new().set_no_line());
    let data = book.save_to_buffer()?;
    book.worksheet_from_index(0)?
        .insert_chart(0, series.len() as u16 + 1, &chart)?;
    let mut source = ZipArchive::new(Cursor::new(book.save_to_buffer()?))?;
    let mut xml = String::new();
    source
        .by_name("xl/charts/chart1.xml")?
        .read_to_string(&mut xml)?;
    // Cache Excel dibuat dari sel yang sama. PowerPoint memerlukan relasi ke workbook.
    let (body, _) = xml
        .split_once("<c:printSettings>")
        .context("Bagian akhir chartSpace Excel berubah")?;
    let body = escape_chart_cache(body);
    // PowerPoint memerlukan status visibilitas sumbu secara eksplisit.
    let body = body.replace(
        "</c:scaling><c:axPos",
        r#"</c:scaling><c:delete val="0"/><c:axPos"#,
    );
    Ok((
        format!(
            r#"{body}<c:externalData xmlns:r="{R}" r:id="rId1"><c:autoUpdate val="0"/></c:externalData></c:chartSpace>"#
        ),
        data,
    ))
}

fn escape_chart_cache(xml: &str) -> String {
    // rust_xlsxwriter melindungi literal _xHHHH_ di sel, tetapi belum di cache grafik.
    let mut escaped = String::with_capacity(xml.len());
    for part in xml.split_inclusive("</c:strCache>") {
        let Some((prefix, cache)) = part.split_once("<c:strCache>") else {
            escaped.push_str(part);
            continue;
        };
        escaped.push_str(prefix);
        escaped.push_str("<c:strCache>");
        let mut start = 0;
        for (index, _) in cache.match_indices("_x") {
            if let Some([digits @ .., b'_']) = cache.as_bytes().get(index + 2..index + 7)
                && digits.iter().all(u8::is_ascii_hexdigit)
            {
                escaped.push_str(&cache[start..index]);
                escaped.push_str("_x005F");
                start = index;
            }
        }
        escaped.push_str(&cache[start..]);
    }
    escaped
}

fn theme() -> String {
    let mut palette = String::new();
    for (slot, rgb) in [
        ("dk1", "0D0D0D"),
        ("lt1", "FFFFFF"),
        ("dk2", "5D5D5D"),
        ("lt2", "F9F9F9"),
        ("accent1", "C8102E"),
        ("accent2", "2A78D6"),
        ("accent3", "1BAF7A"),
        ("accent4", "EDA100"),
        ("accent5", "6250D6"),
        ("accent6", "008300"),
        ("hlink", "2A78D6"),
        ("folHlink", "6250D6"),
    ] {
        palette.push_str(&format!(r#"<a:{slot}><a:srgbClr val="{rgb}"/></a:{slot}>"#));
    }
    let font = r#"<a:latin typeface="Segoe UI"/><a:ea typeface=""/><a:cs typeface=""/>"#;
    let fill = r#"<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>"#;
    let fills = fill.repeat(3);
    let lines: String = [6350, 12700, 19050]
        .map(|width| format!(r#"<a:ln w="{width}">{fill}</a:ln>"#))
        .concat();
    let effects = "<a:effectStyle><a:effectLst/></a:effectStyle>".repeat(3);
    format!(
        r#"<a:theme xmlns:a="{A}" name="Bakase"><a:themeElements><a:clrScheme name="Bakase">{palette}</a:clrScheme><a:fontScheme name="Segoe UI"><a:majorFont>{font}</a:majorFont><a:minorFont>{font}</a:minorFont></a:fontScheme><a:fmtScheme name="Bakase"><a:fillStyleLst>{fills}</a:fillStyleLst><a:lnStyleLst>{lines}</a:lnStyleLst><a:effectStyleLst>{effects}</a:effectStyleLst><a:bgFillStyleLst>{fills}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_table_wrap_for_digits_wide_symbols_and_graphemes() {
        let width = TABLE_WIDTH / 6 - 2 * CELL_PAD_X;
        assert_eq!(
            table_cell_lines("12345678901234", width, 1200, false),
            Some(1)
        );
        assert_eq!(table_cell_lines("‱‱ ‱‱", width, 1900, false), Some(2));
        assert_eq!(
            table_cell_lines("日本語の検査結果", width, 1900, false),
            Some(2)
        );
        assert_eq!(
            table_cell_lines(&"👩‍💻".repeat(8), width, 1900, false),
            Some(2)
        );
        assert_eq!(
            table_cell_lines("abcdef-abcdef", width, 1900, false),
            Some(2)
        );
        assert_eq!(table_grapheme_width('\u{3000}', true), 100);
        assert_eq!(
            table_grapheme_width('\u{00a0}', false),
            table_grapheme_width(' ', false)
        );
    }

    #[test]
    fn caps_table_font_and_leaves_space_for_bold_headers_and_em_spaces() {
        let columns = vec!["Gudang 0001".to_owned(); 6];
        let value = "12,5\u{2003}±\u{2003}0,05";
        let rows: Vec<_> = (0..8)
            .map(|_| (0..6).map(|_| Cell::Text(value.into())).collect())
            .collect();
        let layout = table_layout(&columns, &rows).unwrap();
        let available = layout.col_width - 2 * CELL_PAD_X;
        assert!(layout.size <= 1600);
        assert_eq!(layout.row_heights.len(), 9);
        assert!(layout.row_heights.iter().sum::<u32>() <= TABLE_HEIGHT);
        // Lebar Segoe UI pada 19 pt dibulatkan ke atas untuk isi dan header tebal.
        for points_at_19 in [121, 120] {
            let width = points_at_19 * 12_700 * layout.size / 1900;
            assert!(width * 115 / 100 <= available);
        }
        assert_eq!(table_cell_lines(value, available, 1600, false), Some(2));
        assert_eq!(
            table_cell_lines(&columns[0], available, 1600, false),
            Some(1)
        );
        assert_eq!(
            table_cell_lines(&columns[0], available, 1600, true),
            Some(2)
        );

        let columns = ["Bagian", "Kode", "Status", "Periode", "Unit", "Catatan"].map(str::to_owned);
        let rows: Vec<_> = (0..8)
            .map(|_| {
                Vec::from(
                    [
                        "Operasional",
                        "A-001",
                        "Selesai",
                        "Maret",
                        "batang",
                        "Sesuai",
                    ]
                    .map(|value| Cell::Text(value.into())),
                )
            })
            .collect();
        assert_eq!(table_layout(&columns, &rows).unwrap().size, 1600);
    }
}
