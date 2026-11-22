//! Tanggal dokumen untuk aturan "yang terbaru menang" dan filter tahun. Urutannya: tanggal di
//! judul, tanggal di isi (bukan spreadsheet, karena tanggal di sel adalah data), lalu metadata
//! file. Kalau semuanya tidak ada, ingest memakai tanggal upload ke ARCHIVE. Tanggal file diubah
//! tidak dipakai di sini: untuk file dari ARCHIVE itu tanggal file ditarik.

use regex::Regex;
use std::io::Read;
use std::sync::LazyLock;
use time::{Date, Month, OffsetDateTime, UtcOffset};

/// Asal tanggal, disimpan di `documents_v4.doc_date_from`.
pub const FROM_TITLE: &str = "judul";
pub const FROM_TEXT: &str = "isi";
pub const FROM_METADATA: &str = "metadata";
/// Tanggal upload ke ARCHIVE. Dokumennya bisa lebih tua dari tanggal ini.
pub const FROM_ARCHIVE: &str = "archive";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocDate {
    pub date: Date,
    pub from: &'static str,
}

/// Isi yang diperiksa untuk tanggal: awal dokumen, tempat kop dan tanggal form biasanya berada.
pub const TEXT_CHARS: usize = 20_000;

const MONTHS: &str = r"(jan\w*|feb\w*|mar\w*|apr\w*|mei|may|jun\w*|jul\w*|agu\w*|agt|aug\w*|sep\w*|okt\w*|oct\w*|nov\w*|des\w*|dec\w*)";

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("pola tanggal valid")
}

static DAY_MONTH_YEAR: LazyLock<Regex> = LazyLock::new(|| {
    regex(&format!(
        r"(?i)\b(\d{{1,2}})[\s\-./]+{MONTHS}[\s\-./,]+(20\d\d)\b"
    ))
});
static ISO: LazyLock<Regex> =
    LazyLock::new(|| regex(r"\b(20\d\d)[\-/.](\d{1,2})[\-/.](\d{1,2})\b"));
/// Di nama file, `_` juga pemisah, dan `\b` tidak berlaku setelah `_`.
static TITLE_ISO: LazyLock<Regex> = LazyLock::new(|| regex(r"(20\d\d)[\-_.](\d{2})[\-_.](\d{2})"));
static NUMERIC: LazyLock<Regex> =
    LazyLock::new(|| regex(r"\b(\d{1,2})[\-/.](\d{1,2})[\-/.](20\d\d|\d\d)\b"));
/// Kata sebelum tanggal yang menandai tanggal dokumen atau tanggal form diisi.
static LABEL: LazyLock<Regex> =
    LazyLock::new(|| regex(r"(?i)(tanggal|tgl|date|dated|issued|diterbitkan|日付|作成日)"));
/// Kata sebelum tanggal revisi atau tanggal berlaku template form, bukan tanggal isinya.
static REVISION: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"(?i)(rev|revisi|berlaku|effective|edisi|edition|terbit|issue\s*no|amd|amendment|改訂|制定)",
    )
});
/// Untuk SOP dan standar, tanggal revisi terbaru justru menandai versinya.
static STANDARD: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"(?i)\b(sop|prosedur|procedure|instruksi kerja|work instruction|standar|standard|spesifikasi|specification|pedoman|manual)\b",
    )
});

fn wib() -> UtcOffset {
    UtcOffset::from_hms(7, 0, 0).expect("offset WIB valid")
}

pub fn today() -> Date {
    OffsetDateTime::now_utc().to_offset(wib()).date()
}

pub(crate) fn date(year: i32, month: u8, day: u8) -> Option<Date> {
    Date::from_calendar_date(year, Month::try_from(month).ok()?, day).ok()
}

fn month(name: &str) -> Option<u8> {
    Some(match name.get(..3)?.to_ascii_lowercase().as_str() {
        "jan" => 1,
        "feb" => 2,
        "mar" => 3,
        "apr" => 4,
        "mei" | "may" => 5,
        "jun" => 6,
        "jul" => 7,
        "agu" | "agt" | "aug" => 8,
        "sep" => 9,
        "okt" | "oct" => 10,
        "nov" => 11,
        "des" | "dec" => 12,
        _ => return None,
    })
}

fn number<T: std::str::FromStr>(caps: &regex::Captures<'_>, i: usize) -> Option<T> {
    caps.get(i)?.as_str().parse().ok()
}

fn named(caps: &regex::Captures<'_>) -> Option<Date> {
    date(
        number(caps, 3)?,
        month(caps.get(2)?.as_str())?,
        number(caps, 1)?,
    )
}

fn iso(caps: &regex::Captures<'_>) -> Option<Date> {
    date(number(caps, 1)?, number(caps, 2)?, number(caps, 3)?)
}

/// Semua tanggal di teks beserta posisinya, urut dari depan.
fn dates(text: &str) -> Vec<(usize, Date)> {
    let mut out = Vec::new();
    for caps in DAY_MONTH_YEAR.captures_iter(text) {
        out.extend(named(&caps).map(|d| (caps.get(0).map_or(0, |m| m.start()), d)));
    }
    for caps in ISO.captures_iter(text) {
        out.extend(iso(&caps).map(|d| (caps.get(0).map_or(0, |m| m.start()), d)));
    }
    for caps in NUMERIC.captures_iter(text) {
        let (Some(mut day), Some(mut month), Some(year)) = (
            number::<u8>(&caps, 1),
            number::<u8>(&caps, 2),
            number::<i32>(&caps, 3),
        ) else {
            continue;
        };
        // Format Indonesia hari/bulan; kalau "bulan" lebih dari 12, berarti bulan/hari.
        if month > 12 && day <= 12 {
            std::mem::swap(&mut day, &mut month);
        }
        let year = if year < 100 { year + 2000 } else { year };
        out.extend(date(year, month, day).map(|d| (caps.get(0).map_or(0, |m| m.start()), d)));
    }
    out.sort_by_key(|(pos, _)| *pos);
    out
}

/// 40 karakter sebelum posisi `pos`.
fn before(text: &str, pos: usize) -> String {
    let head = text.get(..pos).unwrap_or_default();
    let mut chars: Vec<char> = head.chars().rev().take(40).collect();
    chars.reverse();
    chars.into_iter().collect()
}

fn in_range(d: Date, from_year: i32, today: Date) -> bool {
    d.year() >= from_year && d <= today
}

/// Tanggal di judul, misalnya "Laporan Cacat (13 Feb 2026)" atau "Rekap 2025-03-04".
pub fn from_title(title: &str, today: Date) -> Option<Date> {
    let found = DAY_MONTH_YEAR
        .captures(title)
        .and_then(|c| named(&c))
        .or_else(|| TITLE_ISO.captures(title).and_then(|c| iso(&c)))?;
    in_range(found, 2000, today).then_some(found)
}

/// Tanggal di isi: yang diawali Tanggal/Tgl/Date lebih dulu, lalu tanggal pertama. Tanggal
/// revisi dan berlaku template form dilewati, kecuali untuk SOP dan standar (`standard`),
/// yang memakai tanggal terbaru.
pub fn from_text(text: &str, today: Date, standard: bool) -> Option<Date> {
    let mut first = None;
    let mut latest = None;
    for (pos, d) in dates(text) {
        if !in_range(d, 2010, today) {
            continue;
        }
        if standard {
            latest = latest.max(Some(d));
            continue;
        }
        let context = before(text, pos);
        // Kata revisi di mana pun dalam 40 karakter sebelumnya membuat tanggal dilewati: di kop
        // form, "Rev", "Berlaku", dan "Tanggal" biasanya satu baris dan merujuk ke template.
        // Aturan "kata terdekat yang menentukan" lebih buruk pada PDF bertanggal judul (78%
        // lawan 98,5% cocok dalam 7 hari).
        if REVISION.is_match(&context) {
            continue;
        }
        if LABEL.is_match(&context) {
            return Some(d);
        }
        first.get_or_insert(d);
    }
    if standard { latest } else { first }
}

/// Tanggal dari metadata Xberg (ISO 8601, misalnya `2025-03-04T05:06:07Z`).
pub fn from_metadata(value: &str, today: Date) -> Option<Date> {
    let head = value.trim().get(..10)?;
    let mut parts = head.split('-');
    let found = date(
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    )?;
    in_range(found, 2000, today).then_some(found)
}

/// Tanggal terakhir disimpan dan dibuat dari stream SummaryInformation file Office lama
/// (.xls, .doc, .ppt). Xberg tidak membacanya untuk format ini.
pub fn legacy_office(bytes: &[u8], today: Date) -> (Option<Date>, Option<Date>) {
    let read = || -> Option<Vec<u8>> {
        let mut file = cfb::CompoundFile::open(std::io::Cursor::new(bytes)).ok()?;
        let stream = file.open_stream("\u{5}SummaryInformation").ok()?;
        let mut buf = Vec::new();
        stream.take(64 * 1024).read_to_end(&mut buf).ok()?;
        Some(buf)
    };
    let Some(buf) = read() else {
        return (None, None);
    };
    let pick = |pid| filetime(&buf, pid).filter(|d| in_range(*d, 2000, today));
    // PIDSI_LASTSAVE_DTM = 13, PIDSI_CREATE_DTM = 12.
    (pick(13), pick(12))
}

fn filetime(buf: &[u8], pid: u32) -> Option<Date> {
    let u32_at = |at: usize| {
        Some(u32::from_le_bytes(
            buf.get(at..at.checked_add(4)?)?.try_into().ok()?,
        ))
    };
    let section = u32_at(44)? as usize;
    let count = u32_at(section.checked_add(4)?)? as usize;
    for i in 0..count.min(256) {
        let entry = section.checked_add(8 + 8 * i)?;
        if u32_at(entry)? != pid {
            continue;
        }
        let at = section.checked_add(u32_at(entry + 4)? as usize)?;
        // VT_FILETIME: jumlah 100 ns sejak 1 Januari 1601.
        if u32_at(at)? != 64 {
            return None;
        }
        let ticks = u64::from_le_bytes(buf.get(at + 4..at + 12)?.try_into().ok()?);
        let secs = i64::try_from(ticks / 10_000_000).ok()? - 11_644_473_600;
        return Some(
            OffsetDateTime::from_unix_timestamp(secs)
                .ok()?
                .to_offset(wib())
                .date(),
        );
    }
    None
}

/// Tanggal dokumen menurut urutan di atas. `text` kosong untuk spreadsheet dan file tanpa isi.
pub fn pick(
    title: &str,
    text: Option<&str>,
    modified: Option<Date>,
    created: Option<Date>,
    today: Date,
) -> Option<DocDate> {
    let found = |date, from| Some(DocDate { date, from });
    if let Some(d) = from_title(title, today) {
        return found(d, FROM_TITLE);
    }
    if let Some(d) = text.and_then(|t| from_text(t, today, STANDARD.is_match(title))) {
        return found(d, FROM_TEXT);
    }
    found(modified.or(created)?, FROM_METADATA)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u8, day: u8) -> Date {
        date(y, m, day).unwrap()
    }

    const TODAY: (i32, u8, u8) = (2026, 11, 19);

    fn today() -> Date {
        d(TODAY.0, TODAY.1, TODAY.2)
    }

    #[test]
    fn titles_with_month_names_or_iso_dates() {
        let t = today();
        assert_eq!(
            from_title("Laporan Cacat Burr (13 Feb 2026)", t),
            Some(d(2026, 2, 13))
        );
        assert_eq!(
            from_title("Inspeksi Lokasi - 3 Januari 2026", t),
            Some(d(2026, 1, 3))
        );
        assert_eq!(
            from_title("Excel Import - 12 Desember 2025", t),
            Some(d(2025, 12, 12))
        );
        assert_eq!(from_title("Rekap_2025_03_04", t), Some(d(2025, 3, 4)));
        assert_eq!(from_title("Rencana 1 Jan 2027", t), None, "masa depan");
        assert_eq!(from_title("Spesifikasi CX300AL", t), None);
    }

    #[test]
    fn text_prefers_labeled_dates_and_skips_revisions() {
        let t = today();
        let form =
            "FORM-DEMO-001 Rev. 03 Berlaku 02-02-2024\nPemeriksaan Kualitas\nTanggal: 17/06/2025";
        assert_eq!(from_text(form, t, false), Some(d(2025, 6, 17)));
        // Tanpa label, tanggal pertama yang bukan revisi.
        let report = "Dokumen revisi 05/05/2021.\nRingkasan hasil pengujian tarik batch produk contoh: 7 Okt 2024 dan 8 Okt 2024.";
        assert_eq!(from_text(report, t, false), Some(d(2024, 10, 7)));
        // Bulan/hari ditukar kalau "bulan" lebih dari 12; tahun dua digit.
        assert_eq!(from_text("diukur 03/25/25", t, false), Some(d(2025, 3, 25)));
        assert_eq!(from_text("tanggal 31/12/2009 dan 1/1/2030", t, false), None);
    }

    #[test]
    fn standards_use_the_latest_revision() {
        let sop = "Rev 01 berlaku 2 Jan 2023\nRev 02 berlaku 5 Mei 2025\nDisusun 1 Feb 2022";
        assert_eq!(from_text(sop, today(), true), Some(d(2025, 5, 5)));
    }

    #[test]
    fn metadata_dates_and_bounds() {
        let t = today();
        assert_eq!(
            from_metadata("2025-03-04T05:06:07Z", t),
            Some(d(2025, 3, 4))
        );
        assert_eq!(from_metadata("2025-03-04", t), Some(d(2025, 3, 4)));
        assert_eq!(from_metadata("1998-01-01T00:00:00Z", t), None);
        assert_eq!(from_metadata("bukan tanggal", t), None);
    }

    #[test]
    fn pick_follows_title_text_then_metadata() {
        let t = today();
        let meta = Some(d(2024, 6, 1));
        let p = |title, text| pick(title, text, meta, None, t);
        assert_eq!(
            p("Laporan (13 Feb 2026)", Some("Tanggal: 1/1/2025"))
                .unwrap()
                .from,
            FROM_TITLE
        );
        let text = p("Laporan", Some("Tanggal: 1/2/2025")).unwrap();
        assert_eq!((text.date, text.from), (d(2025, 2, 1), FROM_TEXT));
        assert_eq!(p("Laporan", None).unwrap().from, FROM_METADATA);
        assert_eq!(
            pick("Laporan", None, None, Some(d(2023, 1, 1)), t)
                .unwrap()
                .date,
            d(2023, 1, 1)
        );
        assert_eq!(pick("Laporan", Some("tanpa tanggal"), None, None, t), None);
    }

    fn summary_information(saved: Date) -> Vec<u8> {
        // Property set minimal: header 48 byte, satu section berisi PIDSI_LASTSAVE_DTM.
        let unix = saved.midnight().assume_utc().unix_timestamp() + 5 * 3600;
        let ticks = ((unix + 11_644_473_600) as u64) * 10_000_000;
        let mut buf = vec![0u8; 48];
        buf[0..2].copy_from_slice(&0xFFFEu16.to_le_bytes());
        buf[24..28].copy_from_slice(&1u32.to_le_bytes());
        buf[44..48].copy_from_slice(&48u32.to_le_bytes());
        let mut section = Vec::new();
        section.extend_from_slice(&28u32.to_le_bytes());
        section.extend_from_slice(&1u32.to_le_bytes());
        section.extend_from_slice(&13u32.to_le_bytes());
        section.extend_from_slice(&16u32.to_le_bytes());
        section.extend_from_slice(&64u32.to_le_bytes());
        section.extend_from_slice(&ticks.to_le_bytes());
        buf.extend_from_slice(&section);
        buf
    }

    #[test]
    fn legacy_office_reads_the_last_saved_date() {
        let mut file = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new())).unwrap();
        {
            use std::io::Write;
            let mut stream = file.create_stream("\u{5}SummaryInformation").unwrap();
            stream
                .write_all(&summary_information(d(2024, 7, 9)))
                .unwrap();
        }
        file.flush().unwrap();
        let bytes = file.into_inner().into_inner();
        assert_eq!(legacy_office(&bytes, today()), (Some(d(2024, 7, 9)), None));
        assert_eq!(legacy_office(b"bukan file office", today()), (None, None));
    }
}
