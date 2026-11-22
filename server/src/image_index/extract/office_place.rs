use quick_xml::{
    Reader, XmlVersion,
    events::{BytesStart, Event},
};
use std::{fs::File, io::Read, path::Path};
use zip::ZipArchive;

const XML_LIMIT: u64 = 2 * 1024 * 1024;
const ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;
const ARCHIVE_ENTRIES: usize = 4096;
const XML_READS: usize = 64;

pub(in crate::image_index) fn place(
    path: &Path,
    kind: &str,
    media: &str,
) -> Option<(String, Option<u32>)> {
    let book = match kind {
        "pptx" => "ppt/presentation.xml",
        "xlsx" => "xl/workbook.xml",
        _ => return None,
    };
    let file = File::open(path).ok()?;
    if file.metadata().ok()?.len() > ARCHIVE_BYTES {
        return None;
    }
    let mut zip = ZipArchive::new(file).ok()?;
    if zip.len() > ARCHIVE_ENTRIES {
        return None;
    }
    let mut budget = XML_READS;
    let index = xml(&mut zip, book, &mut budget)?;
    let units = if kind == "pptx" { "sldId" } else { "sheet" };
    let ids = elements(&index, units, |e| Some((attr(e, "r:id")?, attr(e, "name"))));
    let links = relationships(&mut zip, book, &mut budget);
    for (i, (id, name)) in ids.into_iter().enumerate() {
        if budget == 0 {
            break;
        }
        let Some((_, unit)) = links.iter().find(|(key, _)| *key == id) else {
            continue;
        };
        let links = relationships(&mut zip, unit, &mut budget);
        let found = links.iter().any(|(_, asset)| {
            asset == media
                || (kind == "xlsx"
                    && asset.starts_with("xl/drawings/")
                    && relationships(&mut zip, asset, &mut budget)
                        .iter()
                        .any(|(_, image)| image == media))
        });
        if found {
            return if kind == "pptx" {
                let slide = u32::try_from(i + 1).ok()?;
                Some((format!("Slide {slide}"), Some(slide)))
            } else {
                Some((
                    format!("Sheet {}", name.unwrap_or_else(|| (i + 1).to_string())),
                    None,
                ))
            };
        }
    }
    None
}

fn xml(zip: &mut ZipArchive<File>, name: &str, budget: &mut usize) -> Option<Vec<u8>> {
    *budget = budget.checked_sub(1)?;
    let entry = zip.by_name(name).ok()?;
    if entry.size() > XML_LIMIT {
        return None;
    }
    let mut data = Vec::new();
    entry.take(XML_LIMIT + 1).read_to_end(&mut data).ok()?;
    (data.len() as u64 <= XML_LIMIT).then_some(data)
}

fn attr(e: &BytesStart<'_>, key: &str) -> Option<String> {
    let value = e.try_get_attribute(key).ok()??;
    Some(
        value
            .normalized_value(XmlVersion::Implicit1_0)
            .ok()?
            .into_owned(),
    )
}

fn elements<T>(
    xml: &[u8],
    name: &str,
    mut read: impl FnMut(&BytesStart<'_>) -> Option<T>,
) -> Vec<T> {
    let mut reader = Reader::from_reader(xml);
    let mut out = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Empty(e) | Event::Start(e)) if e.local_name().as_ref() == name => {
                if let Some(value) = read(&e) {
                    out.push(value);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

fn relationships(
    zip: &mut ZipArchive<File>,
    owner: &str,
    budget: &mut usize,
) -> Vec<(String, String)> {
    let Some((dir, name)) = owner.rsplit_once('/') else {
        return Vec::new();
    };
    let Some(xml) = xml(zip, &format!("{dir}/_rels/{name}.rels"), budget) else {
        return Vec::new();
    };
    elements(&xml, "Relationship", |element| {
        if attr(element, "TargetMode").is_some_and(|mode| mode.eq_ignore_ascii_case("External")) {
            return None;
        }
        let target = attr(element, "Target")?;
        Some((attr(element, "Id")?, resolve(dir, &target)?))
    })
}

fn resolve(dir: &str, target: &str) -> Option<String> {
    let mut parts: Vec<_> = if target.starts_with('/') {
        Vec::new()
    } else {
        dir.split('/').collect()
    };
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}
