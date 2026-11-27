use crate::ingest::{MAX_FILE_BYTES, SCAN_CHARS_PER_PAGE};
use anyhow::{Result, anyhow};
use image::{DynamicImage, ImageFormat, ImageReader, Limits, RgbImage};
use std::{
    fs::File,
    io::{Cursor, Read, Seek},
    path::Path,
};
use xberg_native_pdf::{
    PdfDocument, PdfFilter, PdfImageHandle,
    rendering::{RenderOptions, render_page_fit},
};
mod office_place;
pub(super) use office_place::place as office_place;

pub(super) const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SIDE: u32 = 8192;
const MAX_PIXELS: u64 = 32 * 1024 * 1024;
const MIN_SIDE: u32 = 64;
const MIN_AREA: u64 = 128 * 128;

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(super) enum Kind {
    Direct,
    Unsupported,
    Unreadable,
    Docx,
    Pptx,
    Xlsx,
    Pdf,
}

impl Kind {
    pub(super) fn for_path(path: &Path) -> Result<Option<Self>> {
        let ext = extension(path);
        Ok(match ext.as_str() {
            "pdf" => Some(Self::Pdf),
            "docx" => Some(Self::Docx),
            "pptx" => Some(Self::Pptx),
            "xlsx" => Some(Self::Xlsx),
            _ if unsupported(&ext) => Some(Self::Unsupported),
            _ if ImageFormat::from_extension(&ext).is_some() => Some(Self::Direct),
            "txt" | "md" | "csv" | "tsv" | "xls" | "doc" | "ppt" | "xlsm" | "odt" | "ods"
            | "odp" | "rtf" => None,
            _ => {
                let mut head = [0; 16];
                let length = File::open(path).and_then(|file| file.take(16).read(&mut head))?;
                image::guess_format(&head[..length])
                    .is_ok()
                    .then_some(Self::Direct)
            }
        })
    }

    pub(super) fn group(self) -> usize {
        match self {
            Self::Direct | Self::Unsupported | Self::Unreadable => 0,
            Self::Docx => 1,
            Self::Pptx => 2,
            Self::Xlsx => 3,
            Self::Pdf => 4,
        }
    }
}

#[derive(Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(super) struct Counts {
    pub(super) taken: u64,
    pub(super) small: u64,
    pub(super) duplicate: u64,
    pub(super) failed: u64,
    pub(super) pdf_skipped: u64,
    pub(super) pdf_parse_failed: u64,
    pub(super) pdf_text: u64,
    pub(super) rendered: u64,
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn small(width: u32, height: u32) -> bool {
    width.min(height) < MIN_SIDE || u64::from(width) * u64::from(height) < MIN_AREA
}

pub(crate) fn allowed_size(width: u32, height: u32) -> bool {
    width.min(height) > 0
        && width.max(height) <= MAX_SIDE
        && u64::from(width) * u64::from(height) <= MAX_PIXELS
}

fn unsupported(ext: &str) -> bool {
    matches!(ext, "emf" | "wmf" | "svg" | "heic" | "heif" | "jp2" | "qoi")
}

pub(super) fn user_image(bytes: Vec<u8>) -> Option<RgbImage> {
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return None;
    }
    let (width, height) = ImageReader::new(Cursor::new(bytes.as_slice()))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    if !allowed_size(width, height) {
        return None;
    }
    let decoded = decode(bytes, Path::new(""))?;
    let (w, h) = (decoded.width(), decoded.height());
    if !allowed_size(w, h) {
        return None;
    }
    let image = decoded.into_rgb8();
    // Bingkai untuk gambar dari clipboard yang lebih tipis dari batas rasio EG2.
    let short = w.max(h).div_ceil(279);
    if w.min(h) >= short {
        return Some(image);
    }
    let mut padded = RgbImage::new(w.max(short), h.max(short));
    image::imageops::replace(
        &mut padded,
        &image,
        i64::from((short.saturating_sub(w)) / 2),
        i64::from((short.saturating_sub(h)) / 2),
    );
    Some(padded)
}

pub(super) fn at(path: &Path, location: &str) -> Option<RgbImage> {
    let file = open(path).ok()?;
    let pixels = match Kind::for_path(path).ok()?? {
        Kind::Direct if location.is_empty() => decode(bounded(file, MAX_IMAGE_BYTES).ok()??, path)?,
        Kind::Docx | Kind::Pptx | Kind::Xlsx => {
            let mut zip = zip::ZipArchive::new(file).ok()?;
            let entry = zip.by_name(location).ok()?;
            if entry.size() > MAX_IMAGE_BYTES || unsupported(&extension(Path::new(location))) {
                return None;
            }
            decode(bounded(entry, MAX_IMAGE_BYTES).ok()??, Path::new(location))?
        }
        Kind::Pdf => {
            let mut fields = location.strip_prefix("halaman:")?.split(':');
            let page = fields.next()?.parse::<usize>().ok()?.checked_sub(1)?;
            let doc = PdfDocument::from_bytes(bounded(file, MAX_FILE_BYTES).ok()??).ok()?;
            match (fields.next(), fields.next(), fields.next()) {
                (None, None, None) => {
                    if !doc
                        .page_image_handles(page)
                        .ok()?
                        .iter()
                        .all(safe_pdf_image)
                    {
                        return None;
                    }
                    render(&doc, page)?
                }
                (Some("gambar"), Some(position), None) => {
                    let position = position.parse::<usize>().ok()?.checked_sub(1)?;
                    let handles = doc.page_image_handles(page).ok()?;
                    let found = handles
                        .iter()
                        .find(|handle| handle.paint_order == position)?;
                    if !safe_pdf_image(found) {
                        return None;
                    }
                    found.decode().ok()?.to_dynamic_image().ok()?
                }
                _ => return None,
            }
        }
        _ => return None,
    };
    (allowed_size(pixels.width(), pixels.height()) && !small(pixels.width(), pixels.height()))
        .then(|| pixels.into_rgb8())
}

fn decode(bytes: Vec<u8>, path: &Path) -> Option<DynamicImage> {
    let ext = extension(path);
    if unsupported(&ext) {
        return None;
    }
    let hint = ImageFormat::from_extension(ext);
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    if reader.format().is_none() {
        reader.set_format(hint?);
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().ok()
}

fn open(path: &Path) -> Result<File> {
    File::open(path)
        .map_err(|_| anyhow!("File sumber gagal dibuka. Jalankan lagi setelah sumber tersedia."))
}

fn bounded(mut stream: impl Read, limit: u64) -> std::io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    stream.by_ref().take(limit + 1).read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 <= limit).then_some(bytes))
}

pub(super) type Accept<'a> = dyn FnMut(String, RgbImage) -> Result<bool> + Send + 'a;

fn accept(
    location: String,
    image: Option<DynamicImage>,
    counts: &mut Counts,
    emit: &mut Accept<'_>,
) -> Result<bool> {
    let Some(image) = image else {
        counts.failed += 1;
        return Ok(false);
    };
    if small(image.width(), image.height()) {
        counts.small += 1;
        return Ok(false);
    }
    if !allowed_size(image.width(), image.height()) {
        counts.failed += 1;
        return Ok(false);
    }
    counts.taken += 1;
    counts.duplicate += u64::from(emit(location, image.into_rgb8())?);
    Ok(true)
}

pub(super) fn visit(path: &Path, kind: Kind, emit: &mut Accept<'_>) -> Result<Counts> {
    let mut counts = Counts::default();
    if matches!(kind, Kind::Unreadable) {
        return Err(anyhow!("Jenis file sumber gagal diperiksa."));
    }
    if matches!(kind, Kind::Unsupported) {
        counts.failed = 1;
        return Ok(counts);
    }
    let file = open(path)?;
    match kind {
        Kind::Direct => {
            let bytes =
                bounded(file, MAX_IMAGE_BYTES).map_err(|_| anyhow!("File gambar gagal dibaca."))?;
            accept(
                String::new(),
                bytes.and_then(|bytes| decode(bytes, path)),
                &mut counts,
                emit,
            )?;
        }
        Kind::Docx => office(file, "word/media/", &mut counts, emit)?,
        Kind::Pptx => office(file, "ppt/media/", &mut counts, emit)?,
        Kind::Xlsx => office(file, "xl/media/", &mut counts, emit)?,
        Kind::Pdf => pdf(file, &mut counts, emit)?,
        Kind::Unreadable | Kind::Unsupported => unreachable!(),
    }
    Ok(counts)
}

fn office(
    file: impl Read + Seek,
    prefix: &str,
    counts: &mut Counts,
    emit: &mut Accept<'_>,
) -> Result<()> {
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return Err(anyhow!("Arsip media Office gagal dibuka."));
    };
    for number in 0..archive.len() {
        let Some(name) = archive.name_for_index(number) else {
            continue;
        };
        if !name.starts_with(prefix) || name.ends_with('/') {
            continue;
        }
        let name = name.to_string();
        let image = archive
            .by_index(number)
            .ok()
            .filter(|entry| {
                entry.size() <= MAX_IMAGE_BYTES && !unsupported(&extension(Path::new(&name)))
            })
            .and_then(|entry| bounded(entry, MAX_IMAGE_BYTES).ok().flatten())
            .and_then(|bytes| decode(bytes, Path::new(&name)));
        accept(name, image, counts, emit)?;
    }
    Ok(())
}

fn render(doc: &PdfDocument, page: usize) -> Option<DynamicImage> {
    let mut options = RenderOptions::default().as_raw();
    options.render_annotations = false;
    let page = render_page_fit(doc, page, 1200, 1200, &options).ok()?;
    image::RgbaImage::from_raw(page.width, page.height, page.data).map(DynamicImage::ImageRgba8)
}

fn pdf(file: impl Read, counts: &mut Counts, emit: &mut Accept<'_>) -> Result<()> {
    let bytes = bounded(file, MAX_FILE_BYTES).map_err(|_| anyhow!("PDF sumber gagal dibaca."))?;
    let Some((document, pages)) = bytes.and_then(|bytes| {
        let document = PdfDocument::from_bytes(bytes).ok()?;
        let pages = document.page_count().ok()?;
        Some((document, pages))
    }) else {
        counts.pdf_parse_failed += 1;
        return Ok(());
    };
    for page in 0..pages {
        let Ok(text) = document.extract_text(page) else {
            counts.pdf_skipped += 1;
            continue;
        };
        if text
            .chars()
            .filter(|character| character.is_alphanumeric())
            .take(SCAN_CHARS_PER_PAGE)
            .count()
            >= SCAN_CHARS_PER_PAGE
        {
            counts.pdf_text += 1;
            continue;
        }
        let Ok(handles) = document.page_image_handles(page) else {
            counts.pdf_skipped += 1;
            continue;
        };
        let mut found = false;
        let mut oversized = false;
        for handle in handles {
            if !safe_pdf_image(&handle) {
                oversized = true;
                counts.failed += 1;
                continue;
            }
            if small(handle.width, handle.height) {
                counts.small += 1;
                continue;
            }
            let image = handle
                .decode()
                .ok()
                .and_then(|image| image.to_dynamic_image().ok());
            let location = format!("halaman:{}:gambar:{}", page + 1, handle.paint_order + 1);
            found |= accept(location, image, counts, emit)?;
        }
        if !found {
            if oversized {
                counts.pdf_skipped += 1;
                continue;
            }
            let image = render(&document, page);
            if accept(format!("halaman:{}", page + 1), image, counts, emit)? {
                counts.rendered += 1;
            } else {
                counts.pdf_skipped += 1;
            }
        }
    }
    Ok(())
}

pub(crate) fn safe_pdf_image(handle: &PdfImageHandle<'_>) -> bool {
    if !allowed_size(handle.width, handle.height)
        || handle.byte_size_compressed > MAX_IMAGE_BYTES
        || handle.filter_chain.iter().any(|filter| match filter {
            PdfFilter::JPXDecode => true,
            PdfFilter::Other(name) => {
                name.eq_ignore_ascii_case("JPX") || name.eq_ignore_ascii_case("JPXDecode")
            }
            _ => false,
        })
    {
        return false;
    }
    if handle.filter_chain.contains(&PdfFilter::DCTDecode) {
        // Header JPEG dapat berbeda dari dimensi yang ditulis di kamus PDF.
        return handle
            .raw_compressed_bytes()
            .ok()
            .and_then(|bytes| jpeg_stream(&handle.filter_chain, bytes))
            .is_some_and(|jpeg| {
                ImageReader::with_format(Cursor::new(jpeg), ImageFormat::Jpeg)
                    .into_dimensions()
                    .is_ok_and(|(width, height)| allowed_size(width, height))
            });
    }
    true
}

/// Byte JPEG, juga di balik Flate seperti yang ditulis sebagian scanner. Rantai lain tidak dibaca.
fn jpeg_stream(chain: &[PdfFilter], bytes: Vec<u8>) -> Option<Vec<u8>> {
    match chain {
        [PdfFilter::DCTDecode] => Some(bytes),
        [PdfFilter::FlateDecode, PdfFilter::DCTDecode] => {
            let mut jpeg = Vec::new();
            flate2::read::ZlibDecoder::new(bytes.as_slice())
                .take(MAX_IMAGE_BYTES + 1)
                .read_to_end(&mut jpeg)
                .ok()?;
            (jpeg.len() as u64 <= MAX_IMAGE_BYTES).then_some(jpeg)
        }
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        let image = RgbImage::from_pixel(width, height, image::Rgb([20, 40, 60]));
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn office_reads_media_keeps_location_and_counts_skips() {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in [
            ("xl/media/foto.png", png(160, 128)),
            ("xl/media/ikon.png", png(32, 64)),
            ("xl/media/gambar.emf", vec![0; 20]),
            ("docProps/thumbnail.png", png(160, 128)),
        ] {
            archive
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(&bytes).unwrap();
        }
        let mut archive = archive.finish().unwrap();
        archive.set_position(0);
        let mut counts = Counts::default();
        let mut locations = Vec::new();
        office(archive, "xl/media/", &mut counts, &mut |location, image| {
            assert_eq!(image.dimensions(), (160, 128));
            locations.push(location);
            Ok(false)
        })
        .unwrap();
        assert_eq!(locations, ["xl/media/foto.png"]);
        assert_eq!((counts.taken, counts.small, counts.failed), (1, 1, 1));
    }

    #[test]
    fn kind_checks_extensions_and_sniffs_unknown_raster() {
        let dir = super::super::tests::Temp::new();
        let check = |name, matches: fn(Option<Kind>) -> bool| {
            assert!(matches(Kind::for_path(&dir.0.join(name)).unwrap()));
        };
        check("x.PDF", |kind| matches!(kind, Some(Kind::Pdf)));
        check("x.xlsx", |kind| matches!(kind, Some(Kind::Xlsx)));
        check("x.svg", |kind| matches!(kind, Some(Kind::Unsupported)));
        check("x.doc", |kind| kind.is_none());
        std::fs::write(dir.0.join("x.unknown"), png(128, 128)).unwrap();
        check("x.unknown", |kind| matches!(kind, Some(Kind::Direct)));
    }

    #[test]
    fn office_skips_entry_larger_than_byte_limit() {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        archive
            .start_file(
                "ppt/media/gambar.png",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
        archive.write_all(&png(128, 128)).unwrap();
        let padding = vec![0; 1024 * 1024];
        for _ in 0..32 {
            archive.write_all(&padding).unwrap();
        }
        let archive = archive.finish().unwrap();
        let mut counts = Counts::default();
        office(
            Cursor::new(archive.into_inner()),
            "ppt/media/",
            &mut counts,
            &mut |_, _| panic!("entri besar tidak boleh dibaca"),
        )
        .unwrap();
        assert_eq!((counts.taken, counts.failed), (0, 1));
    }

    #[test]
    fn oversized_pdf_dictionary_and_jpeg_header_both_prevent_render() {
        let mut jpeg = cmyk_jpeg();
        let sof = jpeg
            .windows(2)
            .position(|marker| marker == [0xff, 0xc0])
            .unwrap();
        jpeg[sof + 5..sof + 9].copy_from_slice(&[0x23, 0x28, 0x23, 0x28]);
        assert_eq!(jpeg[sof + 9], 4);
        assert_eq!(
            ImageReader::with_format(Cursor::new(&jpeg), ImageFormat::Jpeg)
                .into_dimensions()
                .unwrap(),
            (9000, 9000)
        );
        assert!(decode(jpeg.clone(), Path::new("uji.jpg")).is_none());
        assert!(user_image(jpeg.clone()).is_none());
        for (width, height) in [(20_000, 28_000), (128, 128)] {
            let mut counts = Counts::default();
            pdf(
                Cursor::new(pdf_with_image(width, height, "DCTDecode", &jpeg)),
                &mut counts,
                &mut |_, _| panic!("halaman besar tidak boleh dirender"),
            )
            .unwrap();
            assert_eq!(
                (counts.failed, counts.pdf_skipped, counts.rendered),
                (1, 1, 0)
            );
        }
    }

    fn deflate(bytes: &[u8]) -> Vec<u8> {
        let mut out = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        out.write_all(bytes).unwrap();
        out.finish().unwrap()
    }

    #[test]
    fn jpeg_behind_flate_is_checked_by_its_own_header() {
        const CHAIN: &str = "[/FlateDecode /DCTDecode]";
        let safe = |pdf: Vec<u8>| {
            let doc = PdfDocument::from_bytes(pdf).unwrap();
            let handles = doc.page_image_handles(0).unwrap();
            assert_eq!(handles.len(), 1);
            safe_pdf_image(&handles[0])
        };
        let mut jpeg = cmyk_jpeg();
        assert!(safe(pdf_with_image(128, 128, CHAIN, &deflate(&jpeg))));
        // Dimensi asli di header tetap menentukan, walau kamus PDF menulis ukuran kecil.
        let sof = jpeg
            .windows(2)
            .position(|marker| marker == [0xff, 0xc0])
            .unwrap();
        jpeg[sof + 5..sof + 9].copy_from_slice(&[0x23, 0x28, 0x23, 0x28]);
        assert!(!safe(pdf_with_image(128, 128, CHAIN, &deflate(&jpeg))));
        // Data yang mengembang melewati batas, bukan Flate, dan rantai lain ditolak.
        let bomb = vec![0; MAX_IMAGE_BYTES as usize + 1];
        assert!(!safe(pdf_with_image(128, 128, CHAIN, &deflate(&bomb))));
        assert!(!safe(pdf_with_image(128, 128, CHAIN, &cmyk_jpeg())));
        let chain = [PdfFilter::DCTDecode, PdfFilter::FlateDecode];
        assert!(jpeg_stream(&chain, cmyk_jpeg()).is_none());
    }

    #[test]
    fn pdf_extracts_cmyk_jpeg_but_never_decodes_or_renders_jpx() {
        let jpeg = cmyk_jpeg();
        let mut counts = Counts::default();
        pdf(
            Cursor::new(pdf_with_image(128, 128, "DCTDecode", &jpeg)),
            &mut counts,
            &mut |location, image| {
                assert_eq!(location, "halaman:1:gambar:1");
                assert_eq!(image.dimensions(), (128, 128));
                Ok(false)
            },
        )
        .unwrap();
        assert_eq!((counts.taken, counts.rendered), (1, 0));
        for filter in ["JPXDecode", "JPX", "jpxdecode"] {
            let mut counts = Counts::default();
            pdf(
                Cursor::new(pdf_with_image(128, 128, filter, &[])),
                &mut counts,
                &mut |_, _| panic!("halaman JPX tidak boleh dirender"),
            )
            .unwrap();
            assert_eq!(
                (counts.failed, counts.pdf_skipped, counts.rendered),
                (1, 1, 0)
            );
        }
    }

    fn cmyk_jpeg() -> Vec<u8> {
        let mut jpeg = vec![0xff, 0xd8];
        let mut segment = |marker, body: &[u8]| {
            jpeg.extend([0xff, marker]);
            jpeg.extend(((body.len() + 2) as u16).to_be_bytes());
            jpeg.extend(body);
        };
        segment(0xee, b"Adobe\x00\x64\x00\x00\x00\x00\x00");
        let mut quantization = [1; 65];
        quantization[0] = 0;
        segment(0xdb, &quantization);
        segment(
            0xc0,
            &[
                8, 0, 128, 0, 128, 4, b'C', 0x11, 0, b'M', 0x11, 0, b'Y', 0x11, 0, b'K', 0x11, 0,
            ],
        );
        // Satu kode DC nol dan satu kode akhir blok, keduanya satu bit nol.
        let mut table = [0; 18];
        table[1] = 1;
        segment(0xc4, &table);
        table[0] = 0x10;
        segment(0xc4, &table);
        segment(0xda, &[4, b'C', 0, b'M', 0, b'Y', 0, b'K', 0, 0, 63, 0]);
        jpeg.extend([0; 256]);
        jpeg.extend([0xff, 0xd9]);
        jpeg
    }

    pub(crate) fn pdf_with_image(width: u32, height: u32, filter: &str, jpeg: &[u8]) -> Vec<u8> {
        let content = "q 480 0 0 480 0 0 cm /Foto Do Q";
        let filter = if filter.starts_with('[') {
            filter.to_owned()
        } else {
            format!("/{filter}")
        };
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /XObject << /Foto 5 0 R >> >> >>".to_vec(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()).into_bytes(),
            [format!("<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceCMYK /BitsPerComponent 8 /Filter {filter} /Length {} >>\nstream\n", jpeg.len()).as_bytes(), jpeg, b"\nendstream"].concat(),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.into_iter().enumerate() {
            offsets.push(pdf.len());
            writeln!(&mut pdf, "{} 0 obj", index + 1).unwrap();
            pdf.extend(object);
            pdf.extend(b"\nendobj\n");
        }
        let xref = pdf.len();
        write!(&mut pdf, "xref\n0 6\n0000000000 65535 f \n").unwrap();
        for offset in offsets {
            writeln!(&mut pdf, "{offset:010} 00000 n ").unwrap();
        }
        write!(
            &mut pdf,
            "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF"
        )
        .unwrap();
        pdf
    }

    #[test]
    fn pdf_renders_only_pages_below_text_threshold() {
        let bytes = crate::ingest::minimal_pdf(&[
            "Satu dua tiga empat lima enam tujuh delapan sembilan.",
            "Garis",
        ]);
        let mut counts = Counts::default();
        let mut locations = Vec::new();
        pdf(Cursor::new(bytes), &mut counts, &mut |location, image| {
            assert!(image.width().max(image.height()) <= 1200);
            locations.push(location);
            Ok(false)
        })
        .unwrap();
        assert_eq!(locations, ["halaman:2"]);
        assert_eq!((counts.taken, counts.rendered, counts.pdf_text), (1, 1, 1));
    }
}
