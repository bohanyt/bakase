use super::*;
use calamine::{Reader as _, Xlsx, open_workbook_from_rs};
use quick_xml::{Reader, XmlVersion, events::Event, name::QName};
use std::{
    collections::{BTreeMap, HashSet},
    io::{Cursor, Read},
};
use zip::ZipArchive;

type Package = ZipArchive<Cursor<Vec<u8>>>;
type Attributes = BTreeMap<String, String>;

fn xml_parts(xml: &str, tag: &str) -> Vec<Attributes> {
    let mut reader = Reader::from_str(xml);
    let mut parts = Vec::new();
    loop {
        match reader.read_event().unwrap() {
            Event::Start(node) | Event::Empty(node) if node.name() == QName(tag) => {
                parts.push(
                    node.attributes()
                        .map(|attr| {
                            let attr = attr.unwrap();
                            (
                                attr.key.as_ref().to_owned(),
                                attr.normalized_value(XmlVersion::Implicit1_0)
                                    .unwrap()
                                    .into_owned(),
                            )
                        })
                        .collect(),
                );
            }
            Event::Eof => return parts,
            _ => {}
        }
    }
}

fn element_text(xml: &str, tag: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    let mut result = Vec::new();
    let name = QName(tag);
    loop {
        match reader.read_event().unwrap() {
            Event::Start(node) if node.name() == name => {
                let text = reader.read_text(name).unwrap();
                result.push(
                    quick_xml::escape::unescape(text.as_ref())
                        .unwrap()
                        .into_owned(),
                );
            }
            Event::Eof => return result,
            _ => {}
        }
    }
}

fn part(package: &mut Package, path: &str) -> Vec<u8> {
    let mut data = Vec::new();
    package
        .by_name(path)
        .unwrap()
        .read_to_end(&mut data)
        .unwrap();
    data
}

fn xml(package: &mut Package, path: &str) -> String {
    String::from_utf8(part(package, path)).unwrap()
}

fn chart(kind: &str) -> Value {
    json!({
        "type": "chart", "title": "Hasil <sintetis> & contoh", "chart_type": kind, "unit": "kg",
        "categories": ["1", "B & C", "Feb"], "series": [{"name": "=Contoh", "values": [1.5, 0.0, 3.25]}]
    })
}

fn bodies<'a>(
    xml: &'a str,
    open: &'static str,
    close: &'static str,
) -> impl Iterator<Item = &'a str> {
    xml.split(open)
        .skip(1)
        .map(move |part| part.split_once(close).unwrap().0)
}

fn cache_values(cache: &str, count: usize) -> Vec<String> {
    let [points] = &xml_parts(cache, "c:ptCount")[..] else {
        panic!()
    };
    assert_eq!(points["val"].parse::<usize>().unwrap(), count);
    let indexes: Vec<_> = xml_parts(cache, "c:pt")
        .iter()
        .map(|point| point["idx"].parse::<usize>().unwrap())
        .collect();
    assert_eq!(indexes, (0..count).collect::<Vec<_>>());
    let values = element_text(cache, "c:v");
    assert_eq!(values.len(), count);
    values
}

#[test]
fn native_charts_link_to_workbooks_with_the_same_values() {
    let mut bar = chart("bar");
    bar["categories"][1] = json!("Kode _x000D_ & C");
    bar["series"][0]["name"] = json!("=Uji _x000D_x000A_");
    bar["series"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"Kedua","values":[-2.5,1,9]}));
    let spec: PptxSpec = serde_json::from_value(json!({
        "filename": "Contoh",
        "slides": [
            {"type":"title","title":"Contoh & uji","subtitle":null},
            bar, chart("line"),
            {"type":"table","title":"Tabel","columns":["A","B"],"rows":[[true,null],["<B>",1.2],[9007199254740993u64,0.0004]]},
            chart("pie")
        ]
    }))
    .unwrap();
    validate(&spec).unwrap();
    let mut package = ZipArchive::new(Cursor::new(build(&spec).unwrap())).unwrap();
    let types = xml(&mut package, "[Content_Types].xml");
    let defaults = xml_parts(&types, "Default");
    let overrides = xml_parts(&types, "Override");
    assert!(defaults.iter().any(
        |entry| entry["Extension"] == "xlsx" && entry["ContentType"] == crate::files::XLSX_MIME
    ));
    for entry in &overrides {
        assert!(
            package
                .by_name(entry["PartName"].trim_start_matches('/'))
                .is_ok()
        );
    }
    let paths: Vec<_> = package.file_names().map(str::to_owned).collect();
    for path in paths {
        if let Some((folder, _)) = path.rsplit_once("_rels/") {
            let links = xml_parts(&xml(&mut package, &path), "Relationship");
            let mut ids = HashSet::new();
            for link in &links {
                assert!(ids.insert(&link["Id"]));
                assert!(!link.contains_key("TargetMode"));
                let joined = format!("{folder}{}", link["Target"]);
                let mut target = Vec::new();
                for step in joined.split('/') {
                    if step == ".." {
                        target.pop().unwrap();
                    } else {
                        target.push(step);
                    }
                }
                assert!(
                    package.by_name(&target.join("/")).is_ok(),
                    "{path} → {joined}"
                );
            }
        } else if path.ends_with(".xml") && path != "[Content_Types].xml" {
            assert!(
                overrides
                    .iter()
                    .any(|entry| entry["PartName"] == format!("/{path}"))
            );
            xml_parts(&xml(&mut package, &path), "");
        }
    }
    let presentation = xml(&mut package, "ppt/presentation.xml");
    let slides = xml_parts(&presentation, "p:sldId");
    assert_eq!(slides.len(), spec.slides.len());
    let links = xml_parts(
        &xml(&mut package, "ppt/_rels/presentation.xml.rels"),
        "Relationship",
    );
    assert!(slides.iter().enumerate().all(|(i, slide)| {
        links.iter().any(|link| {
            link["Id"] == slide["r:id"]
                && link["Type"].ends_with("/slide")
                && link["Target"] == format!("slides/slide{}.xml", i + 1)
        })
    }));
    let cover = xml(&mut package, "ppt/slides/slide1.xml");
    assert_eq!(element_text(&cover, "a:t"), ["Contoh & uji"]);
    let table = xml(&mut package, "ppt/slides/slide4.xml");
    assert_eq!(xml_parts(&table, "a:tbl").len(), 1);
    assert_eq!(
        element_text(&table, "a:t"),
        [
            "Tabel",
            "A",
            "B",
            "TRUE",
            "",
            "<B>",
            "1.2",
            "9007199254740993",
            "0.0004"
        ]
    );

    for (n, kind) in [(2, "bar"), (3, "line"), (5, "pie")] {
        let Slide::Chart {
            categories, series, ..
        } = &spec.slides[n - 1]
        else {
            panic!()
        };
        let slide = xml(&mut package, &format!("ppt/slides/slide{n}.xml"));
        let [chart] = &xml_parts(&slide, "c:chart")[..] else {
            panic!()
        };
        assert_eq!(
            element_text(&slide, "a:t"),
            ["Hasil <sintetis> & contoh", "Satuan: kg"]
        );
        let links = xml_parts(
            &xml(&mut package, &format!("ppt/slides/_rels/slide{n}.xml.rels")),
            "Relationship",
        );
        let link = links
            .iter()
            .find(|link| link["Id"] == chart["r:id"])
            .unwrap();
        assert!(link["Type"].ends_with("/chart"));
        let path = format!("ppt/{}", link["Target"].strip_prefix("../").unwrap());
        assert!(
            overrides
                .iter()
                .any(|entry| entry["PartName"] == format!("/{path}")
                    && entry["ContentType"]
                        == "application/vnd.openxmlformats-officedocument.drawingml.chart+xml")
        );
        let chart = xml(&mut package, &path);
        assert_eq!(xml_parts(&chart, &format!("c:{kind}Chart")).len(), 1);
        let [external_data] = &xml_parts(&chart, "c:externalData")[..] else {
            panic!()
        };
        let [link] = &xml_parts(
            &xml(&mut package, &format!("ppt/charts/_rels/chart{n}.xml.rels")),
            "Relationship",
        )[..] else {
            panic!()
        };
        assert_eq!(link["Id"], external_data["r:id"]);
        assert!(link["Type"].ends_with("/package"));
        let workbook = part(
            &mut package,
            &format!("ppt/{}", link["Target"].strip_prefix("../").unwrap()),
        );
        let mut workbook: Xlsx<_> = open_workbook_from_rs(Cursor::new(workbook)).unwrap();
        let data = workbook.worksheet_range("Data").unwrap();
        assert_eq!(data.height(), categories.len() + 1);
        assert_eq!(data.width(), series.len() + 1);
        for (row, category) in categories.iter().enumerate() {
            assert_eq!(data[(row + 1, 0)].to_string(), *category);
        }
        let chart_series: Vec<_> = bodies(&chart, "<c:ser>", "</c:ser>").collect();
        assert_eq!(chart_series.len(), series.len());
        for (col, (xml, series)) in chart_series.into_iter().zip(series).enumerate() {
            let column = char::from(b'B' + col as u8);
            let last = data.height();
            assert_eq!(
                element_text(xml, "c:f"),
                [
                    format!("Data!${column}$1"),
                    format!("Data!$A$2:$A${last}"),
                    format!("Data!${column}$2:${column}${last}"),
                ]
            );
            let string_caches: Vec<_> = bodies(xml, "<c:strCache>", "</c:strCache>").collect();
            let [name, labels] = &string_caches[..] else {
                panic!()
            };
            let name = cache_values(name, 1);
            let labels = cache_values(labels, last - 1);
            assert_eq!(data[(0, col + 1)].to_string(), series.name);
            if kind == "bar" {
                assert_eq!(labels[1], "Kode _x005F_x000D_ & C");
                if col == 0 {
                    assert_eq!(name[0], "=Uji _x005F_x000D_x005F_x000A_");
                }
            }
            assert_eq!(
                name[0].replace("_x005F_", "_"),
                data[(0, col + 1)].to_string()
            );
            for (row, label) in labels.iter().enumerate() {
                assert_eq!(
                    label.replace("_x005F_", "_"),
                    data[(row + 1, 0)].to_string()
                );
            }
            let numeric_caches: Vec<_> = bodies(xml, "<c:numCache>", "</c:numCache>").collect();
            let [values] = &numeric_caches[..] else {
                panic!()
            };
            let values: Vec<f64> = cache_values(values, last - 1)
                .iter()
                .map(|value| value.parse().unwrap())
                .collect();
            assert_eq!(values, series.values);
            for (row, value) in values.into_iter().enumerate() {
                assert_eq!(
                    data[(row + 1, col + 1)].to_string().parse::<f64>().unwrap(),
                    value
                );
            }
        }
    }
}

#[test]
fn rejects_chart_values_outside_excel_range() {
    for value in [1e-310, -1e-310, 1e308, -1e308] {
        let mut slide = chart("bar");
        slide["series"][0]["values"] = json!([value, 0, 3]);
        let spec = serde_json::from_value(json!({"filename":"Uji","slides":[slide]})).unwrap();
        assert!(
            validate(&spec)
                .unwrap_err()
                .contains("di luar rentang Excel")
        );
    }
    let mut pie = chart("pie");
    pie["categories"] = json!(["Terukur"]);
    pie["series"][0]["values"] = json!([1e-310]);
    let spec = serde_json::from_value(json!({"filename":"Uji","slides":[pie]})).unwrap();
    assert!(
        validate(&spec)
            .unwrap_err()
            .contains("di luar rentang Excel")
    );
    let mut bar = chart("bar");
    bar["series"][0]["values"] = json!([f64::MIN_POSITIVE, -EXCEL_MAX_NUMBER, 0]);
    let spec = serde_json::from_value(json!({"filename":"Uji","slides":[bar]})).unwrap();
    validate(&spec).unwrap();
}

#[test]
fn fits_table_rows_inside_frame_or_asks_to_split_the_table() {
    let table = |rows| {
        serde_json::from_value::<PptxSpec>(json!({
            "filename":"Uji", "slides":[{"type":"table","title":"Tabel","columns":["A","B","C","D","E","F"],"rows":rows}]
        })).unwrap()
    };
    let dense = table(vec![vec!["WWWW".repeat(8); 6]; 8]);
    assert!(
        validate(&dense)
            .unwrap_err()
            .contains("Kurangi kolom atau baris")
    );

    let mut rows = vec![vec!["Siap".to_string(); 6]; 8];
    rows[0][0] = "Pemeriksaan perlu diulang".into();
    rows[1][0] = "12345678901234".into();
    let wrapped = table(rows);
    validate(&wrapped).unwrap();
    let mut package = ZipArchive::new(Cursor::new(build(&wrapped).unwrap())).unwrap();
    let slide = xml(&mut package, "ppt/slides/slide1.xml");
    let heights: Vec<u32> = xml_parts(&slide, "a:tr")
        .iter()
        .map(|row| row["h"].parse().unwrap())
        .collect();
    assert_eq!(heights.len(), 9);
    assert!(
        heights[1] > heights[2],
        "baris yang membungkus teks perlu lebih tinggi"
    );
    let frame_height: u32 = xml_parts(&slide, "a:ext").last().unwrap()["cy"]
        .parse()
        .unwrap();
    assert_eq!(heights.iter().sum::<u32>(), frame_height);
    assert!(frame_height <= 4_620_000);
    let cells = element_text(&slide, "a:t");
    assert!(cells.iter().any(|text| text == "Pemeriksaan perlu diulang"));
    assert!(cells.iter().any(|text| text == "12345678901234"));
}

#[test]
fn reads_complete_slides_within_the_page_budget() {
    let slides = json!([
        {"type":"title","title":"Uji","subtitle":"Demo"},
        {"type":"bullets","title":"Poin","items":["Satu"]},
        {"type":"table","title":"Tabel","columns":["A"],"rows":[[0.0004]]},
        chart("line"),
    ]);
    let spec: PptxSpec = serde_json::from_value(json!({"filename":"Uji","slides":slides})).unwrap();
    let budget = json!({"filename":"Uji","total_slides":4,"from_slide":1,"slides":[slides[0]],"next_from_slide":null}).to_string().len() + 1;
    let page = read_page(&spec, 1, budget).unwrap();
    assert!(page.len() <= budget);
    let page: Value = serde_json::from_str(&page).unwrap();
    assert_eq!(page["slides"], json!([slides[0]]));
    let next = page["next_from_slide"].as_u64().unwrap() as usize;
    assert_eq!(next, 2);
    let page: Value = serde_json::from_str(&read_page(&spec, next, 4000).unwrap()).unwrap();
    assert_eq!(
        page["slides"].as_array().unwrap(),
        &slides.as_array().unwrap()[1..]
    );
    assert_eq!(page["next_from_slide"], Value::Null);
    assert!(read_page(&spec, 1, 30).is_err());
    assert!(read_page(&spec, 0, 4000).is_err());
    assert!(read_page(&spec, 5, 4000).is_err());
}

#[test]
fn rejects_invalid_slide_content() {
    let mut mismatch = chart("line");
    mismatch["series"][0]["values"] = json!([1, 2]);
    let mut negative_pie = chart("pie");
    negative_pie["series"][0]["values"] = json!([1, 0, -2]);
    for (slide, message) in [
        (mismatch, "satu angka"),
        (negative_pie, "nonnegatif"),
        (
            json!({"type":"table","title":"Tabel","columns":["A","B"],"rows":[[1]]}),
            "jumlah kolom",
        ),
        (
            json!({"type":"bullets","title":"Teks\0","items":["A"]}),
            "karakter kontrol",
        ),
    ] {
        let spec = serde_json::from_value(json!({"filename":"Contoh","slides":[slide]})).unwrap();
        assert!(validate(&spec).unwrap_err().contains(message));
    }
    assert!(
        serde_json::from_value::<Slide>(json!({"type":"title","title":"X","rows":[]})).is_err()
    );
}
