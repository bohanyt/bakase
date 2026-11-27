//! Cek workbook BIFF sebelum Calamine membuat rentang nilai dan rumus.
use std::{
    borrow::Cow,
    io::{Cursor, Read, Write},
};

use super::{
    ENCRYPTED_OFFICE, MAX_SHEET_BOX, MAX_SHEET_CELLS, MAX_WORKBOOK_BOX, MAX_WORKBOOK_SHEETS,
    MAX_WORKBOOK_TEXT_BYTES, excel_limit, unreadable, unsupported_excel,
};

const BOF: [u32; 4] = [0x0809, 0x0409, 0x0209, 0x0009];
const EOF: u32 = 0x000A;
const FILEPASS: u32 = 0x002F;
const MAX_DEPTH: usize = 16;

#[derive(Clone, Copy)]
pub(crate) enum Scope {
    Attachment,
    Source,
}

fn invalid() -> String {
    unreadable("xls")
}

fn u16_at(data: &[u8], at: usize) -> Result<u32, String> {
    let bytes = data.get(at..at + 2).ok_or_else(invalid)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]).into())
}

fn u32_at(data: &[u8], at: usize) -> Result<u32, String> {
    let bytes = data.get(at..at + 4).ok_or_else(invalid)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn record<'a>(stream: &mut &'a [u8]) -> Result<(u32, &'a [u8]), String> {
    let kind = u16_at(stream, 0)?;
    let end = 4 + u16_at(stream, 2)? as usize;
    let (head, tail) = stream.split_at_checked(end).ok_or_else(invalid)?;
    *stream = tail;
    Ok((kind, &head[4..]))
}

fn record_mut<'a>(stream: &mut &'a mut [u8]) -> Result<(&'a mut [u8], &'a [u8]), String> {
    let end = 4 + u16_at(stream, 2)? as usize;
    let (head, tail) = std::mem::take(stream)
        .split_at_mut_checked(end)
        .ok_or_else(invalid)?;
    let (kind, data) = head.split_at_mut(2);
    *stream = tail;
    Ok((kind, &data[2..]))
}

fn dimensions(data: &[u8]) -> Result<u64, String> {
    let (r1, r2, c1, c2) = match data.len() {
        14 => (
            u32_at(data, 0)?,
            u32_at(data, 4)?,
            u16_at(data, 8)?,
            u16_at(data, 10)?,
        ),
        10 => (
            u16_at(data, 0)?,
            u16_at(data, 2)?,
            u16_at(data, 4)?,
            u16_at(data, 6)?,
        ),
        _ => return Err(invalid()),
    };
    if r2 > 65_536 || c2 > 256 || r1 > r2 || c1 > c2 {
        return Err(invalid());
    }
    if r2 == 0 || c2 == 0 {
        return Ok(1); // Ukuran yang dipakai Calamine untuk DIMENSIONS kosong.
    }
    if r1 == r2 || c1 == c2 {
        return Err(invalid());
    }
    Ok(u64::from(r2 - r1) * u64::from(c2 - c1))
}

// Calamine memakai UTF_16LE.decode, yang mengikuti BOM bila ada.
fn decode_utf16le(data: &[u8]) -> String {
    if let Some(utf8) = data.strip_prefix(b"\xEF\xBB\xBF") {
        return String::from_utf8_lossy(utf8).into_owned();
    }
    let (data, decode) = if let Some(big_endian) = data.strip_prefix(b"\xFE\xFF") {
        (big_endian, u16::from_be_bytes as fn([u8; 2]) -> u16)
    } else {
        (
            data.strip_prefix(b"\xFF\xFE").unwrap_or(data),
            u16::from_le_bytes as fn([u8; 2]) -> u16,
        )
    };
    char::decode_utf16(data.chunks_exact(2).map(|b| decode([b[0], b[1]])))
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

// Ikuti FAT dan direktori datar yang dilihat Calamine, termasuk entri di luar tree.
fn raw_workbook(bytes: &[u8]) -> Result<Vec<u8>, String> {
    const END: u32 = 0xFFFF_FFFE;
    const DIF: u32 = 0xFFFF_FFFC;
    let sector_size = match u16_at(bytes, 30)? {
        9 => 512,
        12 => 4096,
        _ => return Err(invalid()),
    };
    let sector = |id: u32| {
        let start = (id as usize + 1)
            .checked_mul(sector_size)
            .ok_or_else(invalid)?;
        bytes.get(start..start + sector_size).ok_or_else(invalid)
    };
    let words = |data: &[u8], output: &mut Vec<u32>| {
        output.extend(
            data.chunks_exact(4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap())),
        );
    };
    let mut difat = Vec::new();
    words(bytes.get(76..512).ok_or_else(invalid)?, &mut difat);
    let mut id = u32_at(bytes, 68)?;
    let mut seen = std::collections::HashSet::new();
    while id < DIF {
        if !seen.insert(id) {
            return Err(invalid());
        }
        let data = sector(id)?;
        words(&data[..sector_size - 4], &mut difat);
        id = u32_at(data, sector_size - 4)?;
    }
    let mut fat = Vec::new();
    seen.clear();
    for id in difat.into_iter().filter(|&id| id < DIF) {
        if !seen.insert(id) {
            return Err(invalid());
        }
        words(sector(id)?, &mut fat);
    }
    fn chain<'a>(
        mut id: u32,
        fat: &[u32],
        sector: impl Fn(u32) -> Result<&'a [u8], String>,
        limit: usize,
    ) -> Result<Vec<u8>, String> {
        let mut output = Vec::new();
        let mut seen = std::collections::HashSet::new();
        while id != END && output.len() < limit {
            if !seen.insert(id) {
                return Err(invalid());
            }
            let data = sector(id)?;
            output.extend_from_slice(&data[..data.len().min(limit - output.len())]);
            if output.len() < limit {
                id = *fat.get(id as usize).ok_or_else(invalid)?;
            }
        }
        Ok(output)
    }
    let directory = chain(u32_at(bytes, 48)?, &fat, sector, usize::MAX)?;
    let entries = directory.chunks_exact(128);
    let root = entries.clone().next().ok_or_else(invalid)?;
    let stream = |entry: &[u8]| -> Result<(u32, usize), String> {
        let start = u32_at(entry, 116)?;
        let mut size = u64::from(u32_at(entry, 120)?);
        if sector_size == 4096 {
            size |= u64::from(u32_at(entry, 124)?) << 32;
        }
        if size > bytes.len() as u64 {
            return Err(invalid());
        }
        Ok((start, size as usize))
    };
    let mut candidates = [None; 4];
    let mut count = 0;
    for entry in entries {
        let name = decode_utf16le(&entry[..64]);
        let name = name.split('\0').next().unwrap_or_default();
        if let Some(priority) = WORKBOOK_NAMES.iter().position(|&n| n == name) {
            candidates[priority] = Some(entry);
            count += 1;
        }
    }
    if count == 0 {
        return Err(unsupported_excel());
    }
    if count != 1 {
        return Err(invalid());
    }
    let (start, size) = stream(
        candidates
            .into_iter()
            .flatten()
            .next()
            .ok_or_else(invalid)?,
    )?;
    let workbook = if size >= 4096 {
        chain(start, &fat, sector, size)?
    } else {
        let (root_start, root_size) = stream(root)?;
        let mini_stream = chain(root_start, &fat, sector, root_size)?;
        let mini_fat_size = (u32_at(bytes, 64)? as usize)
            .checked_mul(sector_size)
            .ok_or_else(invalid)?;
        if mini_fat_size > bytes.len() {
            return Err(invalid());
        }
        let mut mini_fat = Vec::new();
        words(
            &chain(u32_at(bytes, 60)?, &fat, sector, mini_fat_size)?,
            &mut mini_fat,
        );
        chain(
            start,
            &mini_fat,
            |id| {
                let at = id as usize * 64;
                mini_stream.get(at..at + 64).ok_or_else(invalid)
            },
            size,
        )?
    };
    if workbook.len() != size {
        return Err(invalid());
    }
    Ok(workbook)
}

const WORKBOOK_NAMES: [&str; 4] = ["Workbook", "Book", "WORKBOOK", "BOOK"];

// SST boleh melintasi CONTINUE. Byte pertama fragmen lanjutan karakter
// menentukan apakah karakter di fragmen itu memakai satu atau dua byte.
fn shared_string_sizes<'a>(
    mut data: &'a [u8],
    stream: &mut &'a [u8],
    utf16: bool,
) -> Result<Vec<u64>, String> {
    fn continuation<'a>(stream: &mut &'a [u8]) -> Result<&'a [u8], String> {
        if u16_at(stream, 0)? != 0x003C {
            return Err(invalid());
        }
        Ok(record(stream)?.1)
    }
    data = data.get(8..).ok_or_else(invalid)?;
    let mut sizes = Vec::new();
    loop {
        if data.is_empty() {
            if u16_at(stream, 0)? != 0x003C {
                break;
            }
            data = continuation(stream)?;
            if data.is_empty() {
                sizes.push(0);
                continue;
            }
        }
        let mut chars = u16_at(data, 0)? as usize;
        let flags = *data.get(2).ok_or_else(invalid)?;
        let mut wide = flags & 1 != 0;
        data = &data[3..];
        let mut skip = 0;
        if flags & 8 != 0 {
            skip += u16_at(data, 0)? as usize * 4;
            data = &data[2..];
        }
        if flags & 4 != 0 {
            skip += u32_at(data, 0)? as usize;
            data = &data[4..];
        }
        let mut size = 0;
        while chars > 0 {
            let width = if wide { 2 } else { 1 };
            let count = chars.min(data.len() / width);
            let (text, tail) = data.split_at(count * width);
            data = tail;
            if !utf16 {
                // Batas atas untuk decoding code page lama (dua byte input per karakter BIFF).
                size += count as u64 * 6;
            } else if wide {
                size += decode_utf16le(text).len() as u64;
            } else {
                size += text
                    .iter()
                    .map(|&b| if b < 128 { 1 } else { 2 })
                    .sum::<u64>();
            }
            chars -= count;
            if chars > 0 {
                let next = continuation(stream)?;
                wide = next.first().ok_or_else(invalid)? & 1 != 0;
                data = &next[1..];
            }
        }
        while skip > data.len() {
            skip -= data.len();
            data = continuation(stream)?;
        }
        data = &data[skip..];
        sizes.push(size);
    }
    Ok(sizes)
}

pub(super) fn encrypted_package(bytes: &[u8]) -> bool {
    cfb::CompoundFile::open(Cursor::new(bytes))
        .is_ok_and(|ole| ole.is_stream("/EncryptedPackage") && ole.is_stream("/EncryptionInfo"))
}

pub(super) struct WorkbookInfo {
    pub formula_warning: bool,
    pub sheets: Vec<SheetInfo>,
}

#[derive(Clone, Copy, Default)]
pub(super) struct SheetInfo {
    pub last_row: usize,
    pub recalculation: bool,
}

pub(super) fn prepare(
    mut bytes: Cow<'_, [u8]>,
    scope: Scope,
) -> Result<(Cow<'_, [u8]>, WorkbookInfo), String> {
    let (mut workbook, path) = workbook_stream(&bytes)?;
    let (changed, info) = prepare_workbook(&mut workbook, scope)?;
    if changed {
        let mut ole = cfb::CompoundFile::open(Cursor::new(bytes.to_mut().as_mut_slice()))
            .map_err(|_| invalid())?;
        let mut stream = ole.open_stream(path).map_err(|_| invalid())?;
        stream.write_all(&workbook).map_err(|_| invalid())?;
        stream.flush().map_err(|_| invalid())?;
    }
    Ok((bytes, info))
}

pub(crate) fn prepare_source(bytes: Vec<u8>) -> Option<Vec<u8>> {
    match prepare(Cow::Borrowed(&bytes), Scope::Source) {
        Ok((Cow::Owned(prepared), _)) => Some(prepared),
        Ok((Cow::Borrowed(_), _)) => Some(bytes),
        Err(_) if raw_workbook(&bytes).and_then(|book| chart_free(&book)) == Ok(true) => {
            Some(bytes)
        }
        Err(_) => None,
    }
}

fn chart_free(workbook: &[u8]) -> Result<bool, String> {
    let is_chart = |kind, data: &[u8]| -> Result<bool, String> {
        Ok(kind == FILEPASS || (BOF.contains(&kind) && u16_at(data, 2)? == 0x0020))
    };
    let mut stream = workbook;
    let mut offsets = Vec::new();
    loop {
        let (kind, data) = record(&mut stream)?;
        if is_chart(kind, data)? {
            return Ok(false);
        }
        match kind {
            0x0085 => {
                if data.get(5) == Some(&2) {
                    return Ok(false);
                }
                offsets.push(u32_at(data, 0)? as usize);
            }
            EOF => break,
            _ => {}
        }
    }
    let globals_end = workbook.len() - stream.len();
    offsets.sort_unstable();
    offsets.dedup();
    if offsets.first().is_none_or(|&offset| offset < globals_end) {
        return Ok(false);
    }
    for (i, &offset) in offsets.iter().enumerate() {
        let end = offsets.get(i + 1).copied().unwrap_or(workbook.len());
        let mut sheet = workbook.get(offset..end).ok_or_else(invalid)?;
        while sheet.len() >= 4 {
            let (kind, data) = record(&mut sheet)?;
            if is_chart(kind, data)? {
                return Ok(false);
            }
        }
        if sheet.iter().any(|&byte| byte != 0) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn workbook_stream(bytes: &[u8]) -> Result<(Vec<u8>, std::path::PathBuf), String> {
    let mut ole = cfb::CompoundFile::open(Cursor::new(bytes)).map_err(|_| invalid())?;
    if ole.is_stream("/EncryptedPackage") && ole.is_stream("/EncryptionInfo") {
        return Err(ENCRYPTED_OFFICE.into());
    }
    let raw = raw_workbook(bytes)?;
    // Calamine mencari nama stream secara datar. Tolak nama workbook yang ambigu
    // agar stream yang dicek sama dengan stream yang akan dibaca.
    let mut candidates = ole
        .walk()
        .filter(|entry| WORKBOOK_NAMES.contains(&entry.name()));
    let entry = candidates.next().ok_or_else(invalid)?;
    if !entry.is_stream()
        || entry.path().parent() != Some(std::path::Path::new("/"))
        || candidates.next().is_some()
    {
        return Err(invalid());
    }
    let path = entry.path().to_owned();
    let mut workbook = Vec::new();
    ole.open_stream(&path)
        .map_err(|_| invalid())?
        .take(bytes.len() as u64 + 1)
        .read_to_end(&mut workbook)
        .map_err(|_| invalid())?;
    if workbook != raw {
        return Err(invalid());
    }

    Ok((workbook, path))
}

fn prepare_workbook(workbook: &mut [u8], scope: Scope) -> Result<(bool, WorkbookInfo), String> {
    let limited = matches!(scope, Scope::Attachment);
    let mut stream = &*workbook;
    let (kind, data) = record(&mut stream)?;
    if !BOF.contains(&kind) || u16_at(data, 2)? != 0x0005 {
        return Err(unsupported_excel());
    }
    let mut offsets = Vec::new();
    let mut strings = Vec::new();
    let mut utf16 = true;
    loop {
        match record(&mut stream)? {
            (0x0085, data) => {
                offsets.push((u32_at(data, 0)? as usize, offsets.len()));
                if limited && offsets.len() > MAX_WORKBOOK_SHEETS {
                    return Err(excel_limit());
                }
            }
            (0x0042, data) => utf16 = u16_at(data, 0)? == 1200,
            (0x00FC, data) => strings = shared_string_sizes(data, &mut stream, utf16)?,
            (FILEPASS, _) => return Err(ENCRYPTED_OFFICE.into()),
            (EOF, []) => break,
            (kind, _) if kind == EOF || BOF.contains(&kind) => return Err(invalid()),
            _ => {}
        }
    }
    if offsets.is_empty() {
        return Err(unsupported_excel());
    }
    let globals_end = workbook.len() - stream.len();
    offsets.sort_unstable();
    if offsets[0].0 < globals_end
        || workbook
            .get(globals_end..offsets[0].0)
            .is_none_or(|gap| !empty_tail(gap, scope))
    {
        return Err(invalid());
    }
    let mut stats = WorkbookStats::default();
    let mut sheets = vec![SheetInfo::default(); offsets.len()];
    for (i, &(offset, index)) in offsets.iter().enumerate() {
        let end = offsets
            .get(i + 1)
            .map_or(workbook.len(), |&(offset, _)| offset);
        if offset >= end {
            return Err(invalid());
        }
        let stream = workbook.get_mut(offset..end).ok_or_else(invalid)?;
        sheets[index] = prepare_sheet(stream, &mut stats, &strings, scope)?;
    }
    Ok((
        stats.changed,
        WorkbookInfo {
            formula_warning: stats.unsupported,
            sheets,
        },
    ))
}

fn empty_tail(mut stream: &[u8], scope: Scope) -> bool {
    if matches!(scope, Scope::Source) {
        while let Some(rest) = stream.strip_prefix(&[0x0a, 0, 0, 0]) {
            stream = rest;
        }
    }
    stream.iter().all(|&byte| byte == 0)
}

#[derive(Default)]
struct WorkbookStats {
    area: u64,
    text_bytes: u64,
    changed: bool,
    unsupported: bool,
}

#[derive(Default)]
struct Substream {
    declared: Option<u64>,
    bounds: Option<(u32, u32, u32, u32)>,
    count: u64,
    formula_pos: (u32, u32),
}

impl Substream {
    fn area(&self) -> u64 {
        let actual = self.bounds.map_or(0, |(r1, r2, c1, c2)| {
            u64::from(r2 - r1 + 1) * u64::from(c2 - c1 + 1)
        });
        actual.max(self.declared.unwrap_or(0))
    }

    // DIMENSIONS mengatur reserve Calamine. Posisi sel juga dicek karena DIMENSIONS
    // boleh hilang atau lebih kecil dari kotak sel yang dibuat.
    fn read(
        &mut self,
        kind: u32,
        data: &[u8],
        strings: &[u64],
        stats: &mut WorkbookStats,
        worksheet: bool,
    ) -> Result<(), String> {
        match kind {
            FILEPASS => return Err(ENCRYPTED_OFFICE.into()),
            0x04BC if worksheet => stats.unsupported = true, // SHRFMLA
            0x0200 => {
                if self.declared.replace(dimensions(data)?).is_some() {
                    return Err(invalid());
                }
            }
            0x0203 | 0x0204 | 0x00D6 | 0x0205 | 0x027E | 0x00FD | 0x00BD | 0x0006 | 0x0207 => {
                let (row, col) = if kind == 0x0207 {
                    self.formula_pos
                } else {
                    (u16_at(data, 0)?, u16_at(data, 2)?)
                };
                let end_col = if kind == 0x00BD {
                    u16_at(data, data.len().checked_sub(2).ok_or_else(invalid)?)?
                } else {
                    col
                };
                if end_col < col || end_col > 255 {
                    return Err(invalid());
                }
                let cells = u64::from(end_col - col + 1);
                self.count += cells;
                stats.text_bytes += match kind {
                    0x00FD => strings.get(u32_at(data, 6)? as usize).copied().unwrap_or(0),
                    0x0204 | 0x00D6 | 0x0207 => data.len() as u64 * 3,
                    _ => 16 * cells,
                };
                let bounds = self.bounds.get_or_insert((row, row, col, end_col));
                bounds.0 = bounds.0.min(row);
                bounds.1 = bounds.1.max(row);
                bounds.2 = bounds.2.min(col);
                bounds.3 = bounds.3.max(end_col);
                if kind == 0x0006 {
                    self.formula_pos = (row, col);
                    let count = u16_at(data, 20)? as usize;
                    if count > data.len() - 22 {
                        return Err(invalid());
                    }
                    stats.unsupported |= worksheet && (count == 0 || data.get(22) == Some(&1));
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn prepare_sheet(
    mut stream: &mut [u8],
    stats: &mut WorkbookStats,
    strings: &[u64],
    scope: Scope,
) -> Result<SheetInfo, String> {
    let limited = matches!(scope, Scope::Attachment);
    let (kind, data) = record_mut(&mut stream)?;
    if !BOF.contains(&u16_at(kind, 0)?) {
        return Err(invalid());
    }
    let worksheet = match u16_at(data, 2)? {
        0x0010 => true,
        0x0020 | 0x0040 | 0x0006 => false, // chart, macro sheet, VB module
        _ => return Err(unsupported_excel()),
    };
    let mut opened = vec![Substream::default()];
    let mut info = SheetInfo::default();
    loop {
        let (id, data) = record_mut(&mut stream)?;
        let kind = u16_at(id, 0)?;
        if BOF.contains(&kind) {
            u16_at(data, 2)?;
            if opened.len() >= MAX_DEPTH {
                return Err(unsupported_excel());
            }
            opened.push(Substream::default());
        } else if kind == EOF {
            if !data.is_empty() {
                return Err(invalid());
            }
            stats.area += opened.pop().unwrap().area();
            if limited && stats.area > MAX_WORKBOOK_BOX {
                return Err(excel_limit());
            }
            if opened.is_empty() {
                return if empty_tail(stream, scope) {
                    Ok(info)
                } else {
                    Err(invalid())
                };
            }
            // EOF harus tetap dianggap bagian substream yang baru saja ditutup.
            hide_record(id, stats);
            continue;
        } else {
            let worksheet_cell = worksheet && opened.len() == 1;
            if worksheet_cell {
                match kind {
                    0x005E => info.recalculation = true, // Uncalced
                    0x0208 | 0x0201 | 0x00BE if data.len() >= 2 => {
                        info.last_row = info.last_row.max(u16_at(data, 0)? as usize + 1);
                    }
                    _ => {}
                }
            }
            let current = opened.last_mut().unwrap();
            current.read(kind, data, strings, stats, worksheet_cell)?;
            if limited
                && (current.area() > MAX_SHEET_BOX
                    || current.count > MAX_SHEET_CELLS
                    || stats.area + current.area() > MAX_WORKBOOK_BOX
                    || stats.text_bytes > MAX_WORKBOOK_TEXT_BYTES)
            {
                return Err(excel_limit());
            }
        }
        if !worksheet || opened.len() > 1 {
            hide_record(id, stats);
        }
    }
}

fn hide_record(id: &mut [u8], stats: &mut WorkbookStats) {
    // Calamine mengabaikan tipe ini. Panjang dan semua offset BOUNDSHEET tetap sama.
    id.copy_from_slice(&0xFFFF_u16.to_le_bytes());
    stats.changed = true;
}

#[cfg(test)]
fn check(bytes: &[u8]) -> Result<bool, String> {
    prepare(Cow::Borrowed(bytes), Scope::Attachment).map(|(_, info)| info.formula_warning)
}

#[cfg(test)]
fn add_record(book: &mut Vec<u8>, kind: u16, data: &[u8]) {
    book.extend(kind.to_le_bytes());
    book.extend((data.len() as u16).to_le_bytes());
    book.extend(data);
}

#[cfg(test)]
fn compound(streams: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;

    let mut ole =
        cfb::CompoundFile::create_with_version(cfb::Version::V3, Cursor::new(Vec::new())).unwrap();
    for &(name, book) in streams {
        ole.create_stream(name).unwrap().write_all(book).unwrap();
    }
    ole.into_inner().into_inner()
}

#[cfg(test)]
pub(crate) fn fixture(rows: u32, cols: u16) -> Vec<u8> {
    // BIFF8 sintetis: BOF global, BOUNDSHEET "Data", lalu satu sheet dengan
    // DIMENSIONS, angka 4 di A1, serta SUM(B2:B10) relatif (cache 5) di A2.
    let mut book = Vec::new();
    add_record(&mut book, 0x0809, &[0, 6, 5, 0]);
    let offset = (book.len() + 4 + 12 + 4) as u32;
    let mut bound = offset.to_le_bytes().to_vec();
    bound.extend([0, 0, 4, 0]);
    bound.extend(b"Data");
    add_record(&mut book, 0x0085, &bound);
    add_record(&mut book, 0x000A, &[]);
    add_record(&mut book, 0x0809, &[0, 6, 0x10, 0]);

    let mut dimensions = vec![0; 4];
    dimensions.extend(rows.to_le_bytes());
    dimensions.extend([0, 0]);
    dimensions.extend(cols.to_le_bytes());
    dimensions.extend([0, 0]);
    add_record(&mut book, 0x0200, &dimensions);

    let mut number = vec![0; 6];
    number.extend(4.0_f64.to_le_bytes());
    add_record(&mut book, 0x0203, &number);
    let mut formula = vec![1, 0, 0, 0, 0, 0];
    formula.extend(5.0_f64.to_le_bytes());
    formula.extend([0; 6]);
    formula.extend([13, 0, 0x25, 1, 0, 9, 0, 1, 0xC0, 1, 0xC0, 0x19, 0x10, 0, 0]);
    add_record(&mut book, 0x0006, &formula);
    add_record(&mut book, 0x000A, &[]);

    compound(&[("/Workbook", &book)])
}

#[cfg(test)]
pub(crate) use tests::rejected_chart_fixture;

#[cfg(test)]
mod tests {
    use super::*;

    fn number(row: u16, col: u16) -> Vec<u8> {
        number_at(row, col, 1.0)
    }

    fn number_at(row: u16, col: u16, value: f64) -> Vec<u8> {
        let mut payload = row.to_le_bytes().to_vec();
        payload.extend(col.to_le_bytes());
        payload.extend([0, 0]);
        payload.extend(value.to_le_bytes());
        payload
    }

    type Records<'a> = &'a [(u16, &'a [u8])];

    fn book(sheets: usize, globals: &[(u16, &[u8])], cells: &[(u16, &[u8])]) -> Vec<u8> {
        book_with_sheets(globals, &vec![(0x10, cells); sheets])
    }

    fn book_with_sheets(globals: Records<'_>, sheets: &[(u8, Records<'_>)]) -> Vec<u8> {
        let mut book = Vec::new();
        add_record(&mut book, 0x0809, &[0, 6, 5, 0]);
        for &(kind, payload) in globals {
            add_record(&mut book, kind, payload);
        }
        let bounds_start = book.len();
        for (i, &(kind, _)) in sheets.iter().enumerate() {
            let mut bound = vec![0; 6];
            if kind == 0x20 {
                bound[5] = 2; // BOUNDSHEET: chart sheet
            }
            bound.extend([2, 0, b'S' + (i / 26) as u8, b'A' + (i % 26) as u8]);
            add_record(&mut book, 0x0085, &bound);
        }
        add_record(&mut book, 0x000A, &[]);
        for (i, &(kind, records)) in sheets.iter().enumerate() {
            let offset = (book.len() as u32).to_le_bytes();
            let at = bounds_start + i * 14 + 4;
            book[at..at + 4].copy_from_slice(&offset);
            add_record(&mut book, 0x0809, &[0, 6, kind, 0]);
            for &(kind, payload) in records {
                add_record(&mut book, kind, payload);
            }
            add_record(&mut book, 0x000A, &[]);
        }
        book
    }

    fn chart_dimensions(rows: u32, cols: u16) -> [u8; 14] {
        let mut dimensions = [0; 14];
        dimensions[4..8].copy_from_slice(&rows.to_le_bytes());
        dimensions[10..12].copy_from_slice(&cols.to_le_bytes());
        dimensions
    }

    #[tokio::test]
    async fn xls_with_twenty_nine_sheets_has_a_full_overview_and_attachment_note() {
        let amount = number_at(4, 2, 7.0);
        let bytes = compound(&[("/Workbook", &book(29, &[], &[(0x0203, &amount)]))]);
        let spec = super::super::read_content("xls", "Banyak", &bytes)
            .await
            .unwrap();
        assert!(spec.get("sheets").is_none());
        let note = spec["sheet_note"].as_str().unwrap();
        assert!(note.contains("\"SA\"") && note.contains("\"TC\""));
        let options: super::super::excel_calc::Options = serde_json::from_value(serde_json::json!({
            "file": "Banyak.xls", "sheet": "", "sql": "", "header_row": 0, "pending": false, "budget": 8000
        })).unwrap();
        let output = super::super::excel_calc::calculate("xls", &bytes, options, false).unwrap();
        let summary: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(summary["sheets"].as_array().unwrap().len(), 29);
        assert_eq!(summary["sheets"][0], serde_json::json!(["SA", 5, 3]));
        assert_eq!(summary["sheets"][28], serde_json::json!(["TC", 5, 3]));
        let options: super::super::excel_calc::Options = serde_json::from_value(serde_json::json!({
            "file": "Banyak.xls", "sheet": "*", "sql": "SELECT COUNT(DISTINCT sheet), SUM(C) FROM data",
            "header_row": 0, "pending": false, "budget": 8000
        })).unwrap();
        let output = super::super::excel_calc::calculate("xls", &bytes, options, false).unwrap();
        let result: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(result["hasil"], serde_json::json!([[29, 203]]));
    }

    #[tokio::test]
    async fn ignores_two_nested_charts_and_a_standalone_chart_in_both_readers() {
        let dimensions = chart_dimensions(3, 2);
        let a1 = number_at(0, 0, 11.0);
        let b1 = number_at(0, 1, 12.0);
        let a2 = number_at(1, 0, 21.0);
        let b2 = number_at(1, 1, 22.0);
        let a3 = number_at(2, 0, 31.0);
        let b3 = number_at(2, 1, 32.0);
        let cache_a1 = number_at(0, 0, 77889.0);
        let cache_b2 = number_at(1, 1, 88990.0);
        let chart_bof = [0, 6, 0x20, 0];
        let worksheet: Records<'_> = &[
            (0x0200, &dimensions),
            (0x0203, &a1),
            (0x0203, &b1),
            (0x0809, &chart_bof),
            (0x0200, &dimensions),
            (0x0203, &cache_a1),
            (0x000A, &[]),
            (0x0203, &a2),
            (0x0203, &b2),
            (0x0809, &chart_bof),
            (0x0200, &dimensions),
            (0x0203, &cache_b2),
            (0x000A, &[]),
            (0x0203, &a3),
            (0x0203, &b3),
        ];
        let standalone: Records<'_> = &[(0x0200, &dimensions), (0x0203, &cache_a1)];
        let bytes = compound(&[(
            "/Workbook",
            &book_with_sheets(&[], &[(0x10, worksheet), (0x20, standalone)]),
        )]);
        let spec = super::super::read_content("xls", "Uji grafik", &bytes)
            .await
            .unwrap();
        assert_eq!(spec["sheets"][0]["columns"][0]["header"], "11");
        assert_eq!(spec["sheets"][0]["columns"][1]["header"], "12");
        assert_eq!(
            spec["sheets"][0]["rows"],
            serde_json::json!([[21, 22], [31, 32]])
        );
        assert_eq!(spec["sheets"].as_array().unwrap().len(), 1);
        let check_text = |text: &str| {
            for number in ["11", "12", "21", "22", "31", "32"] {
                assert!(text.contains(number), "{text}");
            }
            assert!(!text.contains("77889") && !text.contains("88990"), "{text}");
        };
        check_text(spec["text"].as_str().unwrap());
        let calculation = super::super::excel_calc::calculate(
            "xls",
            &bytes,
            super::super::excel_calc::Options {
                file: "Uji grafik.xls".into(),
                sheet: None,
                sheets: None,
                sql: Some("SELECT SUM(A), SUM(B) FROM data".into()),
                budget: 8_000,
                pending: false,
                header_row: 1,
                simpan: None,
            },
            false,
        )
        .unwrap();
        let calculation: serde_json::Value = serde_json::from_str(&calculation).unwrap();
        assert_eq!(calculation["hasil"], serde_json::json!([[52, 54]]));
        let source = crate::ingest::extract_with_xberg(prepare_source(bytes).unwrap(), "xls", true)
            .await
            .unwrap();
        check_text(
            &source
                .sections
                .into_iter()
                .map(|(_, t)| t)
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }

    #[test]
    fn rejects_unbalanced_substreams_and_checks_graph_dimensions() {
        let chart = (0x0809, &[0, 6, 0x20, 0][..]);
        let eof = (0x000A, &[][..]);
        for cells in [&[chart][..], &[eof][..], &[chart, eof, eof][..]] {
            let bytes = compound(&[("/Workbook", &book(1, &[], cells))]);
            assert_eq!(check(&bytes), Err(invalid()));
        }
        let large = chart_dimensions(10_001, 200);
        let bytes = compound(&[("/Workbook", &book(1, &[], &[chart, (0x0200, &large), eof]))]);
        assert_eq!(check(&bytes), Err(excel_limit()));
        let root = (0x0200, chart_dimensions(10_000, 200));
        let nested = (0x0200, chart_dimensions(12_000, 100));
        let cells = [
            (root.0, &root.1[..]),
            chart,
            (nested.0, &nested.1[..]),
            eof,
            chart,
            (nested.0, &nested.1[..]),
            eof,
        ];
        let bytes = compound(&[("/Workbook", &book(1, &[], &cells))]);
        assert_eq!(check(&bytes), Err(excel_limit()));
    }

    #[tokio::test]
    async fn source_keeps_reading_legacy_biff_accepted_by_xberg() {
        let mut legacy = book(
            1,
            &[(0x0042, &1252_u16.to_le_bytes())],
            &[(0x0203, &number_at(0, 0, 197.0))],
        );
        legacy[4..6].copy_from_slice(&0x0500_u16.to_le_bytes());
        let sheet = u32::from_le_bytes(legacy[18..22].try_into().unwrap()) as usize;
        legacy[sheet + 4..sheet + 6].copy_from_slice(&0x0500_u16.to_le_bytes());
        // Pembaca lama berhenti di EOF pertama dan menerima tambahan EOF di ujung stream.
        add_record(&mut legacy, 0x000A, &[]);
        let bytes = compound(&[("/Workbook", &legacy)]);
        assert_eq!(check(&bytes), Err(invalid()));
        let (prepared, _) = prepare(Cow::Borrowed(&bytes), Scope::Source).unwrap();
        let source = crate::ingest::extract_with_xberg(prepared.into_owned(), "xls", true)
            .await
            .unwrap();
        assert!(source.sections.iter().any(|(_, text)| text.contains("197")));
    }

    fn duplicate_dimensions_fixture(chart: bool) -> Vec<u8> {
        let dimensions = chart_dimensions(3, 2);
        let first = number_at(0, 0, 197.0);
        let second = number_at(1, 0, 198.0);
        let cache = number_at(0, 0, 9811.0);
        let mut records: Vec<(u16, &[u8])> = vec![
            (0x0200, &dimensions),
            (0x0203, &first),
            (0x0203, &second),
            (0x0200, &dimensions),
        ];
        if chart {
            records.extend([
                (0x0809, &[0, 6, 0x20, 0][..]),
                (0x0203, &cache),
                (0x000A, &[]),
            ]);
        }
        compound(&[("/Workbook", &book(1, &[], &records))])
    }

    pub(crate) fn rejected_chart_fixture() -> Vec<u8> {
        duplicate_dimensions_fixture(true)
    }

    #[tokio::test]
    async fn raw_fallback_requires_a_chart_free_workbook_even_after_early_validation_failure() {
        let with_chart = rejected_chart_fixture();
        assert!(prepare(Cow::Borrowed(&with_chart), Scope::Source).is_err());
        assert!(prepare_source(with_chart).is_none());

        let chart_free = duplicate_dimensions_fixture(false);
        assert!(prepare(Cow::Borrowed(&chart_free), Scope::Source).is_err());
        let allowed = prepare_source(chart_free.clone()).unwrap();
        assert_eq!(allowed, chart_free);
        let extracted = crate::ingest::extract_with_xberg(allowed, "xls", true)
            .await
            .unwrap();
        assert!(
            extracted
                .sections
                .iter()
                .any(|(_, text)| text.contains("198"))
        );
    }

    #[test]
    fn xls_counts_empty_formula_caches_only_on_the_uncalced_sheet() {
        let mut formula = vec![1, 0, 0, 0, 0, 0];
        formula.extend([3, 0, 0, 0, 0, 0, 0xff, 0xff]);
        formula.extend([0; 6]);
        formula.extend([3, 0, 0x1e, 1, 0]);
        let title = number_at(0, 0, 197.0);
        let bytes = compound(&[(
            "/Workbook",
            &book_with_sheets(
                &[],
                &[
                    (0x10, &[(0x0203, &title), (0x0006, &formula)]),
                    (
                        0x10,
                        &[(0x005E, &[0, 0]), (0x0203, &title), (0x0006, &formula)],
                    ),
                ],
            ),
        )]);
        let calculate = |sheet| {
            let options = super::super::excel_calc::Options {
                file: "Uji.xls".into(),
                sheet: Some(sheet),
                sheets: None,
                sql: Some("SELECT COUNT(A) FROM data".into()),
                budget: 8_000,
                pending: false,
                header_row: 1,
                simpan: None,
            };
            serde_json::from_str::<serde_json::Value>(
                &super::super::excel_calc::calculate("xls", &bytes, options, false).unwrap(),
            )
            .unwrap()
        };
        let clean = calculate("SA".into());
        assert_eq!(clean["hasil"], serde_json::json!([[0]]));
        assert!(clean["profil_kolom"].is_null() && clean["catatan_rumus"].is_null());
        let pending = calculate("SB".into());
        assert_eq!(pending["hasil"], serde_json::json!([[0]]));
        assert!(
            pending["catatan_rumus"]
                .as_str()
                .unwrap()
                .contains("hitung ulang")
        );
        assert_eq!(
            pending["profil_kolom"]["kolom"],
            serde_json::json!([["A", 0, 0, 0, 0, 0, 1, 0, 1]])
        );
    }

    #[test]
    fn xls_overview_and_sql_agree_on_format_only_sheets_and_trailing_rows() {
        use super::super::excel_calc::{Options, calculate};
        use serde_json::{Value, json};

        let blank = [4, 0, 7, 0, 1, 0];
        let mut row = [0; 16];
        row[0] = 7;
        let value = number_at(1, 0, 9.0);
        let workbook = book_with_sheets(
            &[],
            &[
                (0x10, &[(0x0201, &blank), (0x0208, &row)]),
                (0x10, &[(0x0203, &value), (0x0201, &blank), (0x0208, &row)]),
            ],
        );
        let bytes = compound(&[("/Workbook", &workbook)]);
        let run = |sheet, sql| {
            let options = Options {
                file: "Uji.xls".into(),
                sheet,
                sheets: None,
                sql,
                budget: 8_000,
                pending: false,
                header_row: 0,
                simpan: None,
            };
            serde_json::from_str::<Value>(&calculate("xls", &bytes, options, false).unwrap())
                .unwrap()
        };
        assert_eq!(
            run(None, None)["sheets"],
            json!([["SA", 0, 0], ["SB", 8, 1]])
        );
        let sql = "SELECT sheet, COUNT(*), MAX(baris), SUM(A) FROM data GROUP BY sheet";
        assert_eq!(
            run(Some("*".into()), Some(sql.into()))["hasil"],
            json!([["SB", 8, 8, 9]])
        );
    }

    #[tokio::test]
    async fn distinguishes_passwords_from_unsupported_or_damaged_excel() {
        let locked = compound(&[("/Workbook", &book(1, &[(0x002F, &[0, 0])], &[]))]);
        assert_eq!(check(&locked), Err(ENCRYPTED_OFFICE.into()));
        let package = compound(&[
            ("/EncryptedPackage", b"paket"),
            ("/EncryptionInfo", b"info"),
        ]);
        assert_eq!(check(&package), Err(ENCRYPTED_OFFICE.into()));
        let kind = super::super::sniff("xlsx", &package).unwrap();
        assert_eq!(
            super::super::read_content(kind, "Uji", &package).await,
            Err(ENCRYPTED_OFFICE.into())
        );
        let other = compound(&[("/Something", b"uji")]);
        assert_eq!(check(&other), Err(unsupported_excel()));
        assert!(!invalid().contains("password"));
    }

    #[test]
    fn checks_actual_coordinates_and_mulrk_end_columns() {
        let first = number(0, 0);
        let far = number(10_000, 255);
        let small_dimensions = [0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0];
        let bytes = compound(&[(
            "/Workbook",
            &book(
                1,
                &[],
                &[
                    (0x0200, &small_dimensions),
                    (0x0203, &first),
                    (0x0203, &far),
                ],
            ),
        )]);
        assert_eq!(check(&bytes), Err(excel_limit()));

        let mut mulrk = 10_000_u16.to_le_bytes().to_vec();
        mulrk.extend([0, 0]);
        mulrk.extend([0; 256 * 6]);
        mulrk.extend(255_u16.to_le_bytes());
        let bytes = compound(&[(
            "/Workbook",
            &book(1, &[], &[(0x0203, &first), (0x00BD, &mulrk)]),
        )]);
        assert_eq!(check(&bytes), Err(excel_limit()));
        mulrk[4 + 256 * 6..].copy_from_slice(&256_u16.to_le_bytes());
        let bytes = compound(&[("/Workbook", &book(1, &[], &[(0x00BD, &mulrk)]))]);
        assert_eq!(check(&bytes), Err(invalid()));
    }

    #[test]
    fn refuses_more_sheets_than_the_limit() {
        let bytes = compound(&[("/Book", &book(MAX_WORKBOOK_SHEETS + 1, &[], &[]))]);
        assert_eq!(check(&bytes), Err(excel_limit()));
    }

    #[test]
    fn refuses_ambiguous_raw_workbook_entry() {
        let hostile = book(
            1,
            &[],
            &[(0x0203, &number(0, 0)), (0x0203, &number(10_000, 255))],
        );
        let safe = book(1, &[], &[(0x0203, &number(0, 0))]);
        let mut bytes = compound(&[("/Wor", &hostile), ("/Workbook", &safe)]);
        assert_eq!(check(&bytes), Ok(false));
        // Nama tercatat tiga huruf bagi cfb, sementara Calamine membaca sampai NUL.
        let short: Vec<u8> = "Wor\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let at = (0..bytes.len() - 66)
            .step_by(128)
            .find(|&i| bytes[i..i + 8] == short && bytes[i + 64..i + 66] == [8, 0])
            .unwrap();
        let long: Vec<u8> = "Workbook"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        bytes[at..at + long.len()].copy_from_slice(&long);
        assert_eq!(check(&bytes), Err(invalid()));

        bytes.copy_within(at..at + long.len(), at + 2);
        bytes[at..at + 2].copy_from_slice(b"\xFF\xFE");
        assert_eq!(check(&bytes), Err(invalid()));
    }

    #[test]
    fn counts_expanded_shared_strings_across_continue() {
        let mut sst = vec![0; 8];
        sst[4] = 1;
        sst.extend(16_000_u16.to_le_bytes());
        sst.push(0);
        sst.extend([b'x'; 8_000]);
        // Ubah encoding dari compressed ke UTF-16 pada batas CONTINUE.
        let mut cont = vec![1];
        for _ in 0..4_000 {
            cont.extend(u16::to_le_bytes(0x3042));
        }
        let globals = [
            (0x00FC, sst.as_slice()),
            (0x003C, cont.as_slice()),
            (0x003C, cont.as_slice()),
        ];
        let labels: Vec<_> = (0..=1_638_u16)
            .map(|row| {
                let mut label = row.to_le_bytes().to_vec();
                label.extend([0; 8]);
                label
            })
            .collect();
        let cells: Vec<_> = labels
            .iter()
            .map(|label| (0x00FD, label.as_slice()))
            .collect();
        let under = compound(&[("/WORKBOOK", &book(1, &globals, &cells[..1]))]);
        assert_eq!(check(&under), Ok(false));
        let over = compound(&[("/WORKBOOK", &book(1, &globals, &cells))]);
        assert_eq!(check(&over), Err(excel_limit()));
    }

    #[tokio::test]
    async fn warns_about_legacy_shared_formulas() {
        let mut formula = vec![0; 6];
        formula.extend(5.0_f64.to_le_bytes());
        formula.extend([0; 6]);
        formula.extend([5, 0, 1, 0, 0, 0, 0]); // PtgExp
        let bytes = compound(&[("/BOOK", &book(1, &[], &[(0x0006, &formula)]))]);
        let spec = super::super::read_content("xls", "Shared", &bytes)
            .await
            .unwrap();
        assert_eq!(spec["peringatan_rumus"], true);
        assert_eq!(spec["teks_rumus"], "");
        let page = super::super::read_text_page("xls", &spec, 0, 8_000).unwrap();
        assert_eq!(page["peringatan"], super::super::XLS_SHARED_WARNING);
        assert_eq!(page["catatan_rumus"], super::super::XLS_VALUES_NOTE);
    }
}
