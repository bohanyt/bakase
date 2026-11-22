//! Cek workbook BIFF sebelum Calamine membuat rentang nilai dan rumus.
use std::io::{Cursor, Read};

use super::{
    MAX_SHEET_BOX, MAX_WORKBOOK_BOX, MAX_WORKBOOK_SHEETS, MAX_WORKBOOK_TEXT_BYTES, excel_limit,
    unreadable,
};

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

// DIMENSIONS dipakai untuk reserve oleh Calamine. Posisi nyata juga dicek karena
// DIMENSIONS bisa hilang atau lebih kecil dari kotak sel yang benar-benar dibuat.
pub(super) fn check(bytes: &[u8]) -> Result<bool, String> {
    let raw = raw_workbook(bytes)?;
    let mut ole = cfb::CompoundFile::open(Cursor::new(bytes)).map_err(|_| invalid())?;
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
    ole.open_stream(path)
        .map_err(|_| invalid())?
        .take(bytes.len() as u64 + 1)
        .read_to_end(&mut workbook)
        .map_err(|_| invalid())?;
    if workbook != raw {
        return Err(invalid());
    }

    let mut stream = workbook.as_slice();
    if !matches!(record(&mut stream)?.0, 0x0809 | 0x0409 | 0x0209 | 0x0009) {
        return Err(invalid());
    }
    let mut offsets = Vec::new();
    let mut strings = Vec::new();
    let mut utf16 = true;
    loop {
        match record(&mut stream)? {
            (0x0085, data) => {
                offsets.push(u32_at(data, 0)? as usize);
                if offsets.len() > MAX_WORKBOOK_SHEETS {
                    return Err(excel_limit());
                }
            }
            (0x0042, data) => utf16 = u16_at(data, 0)? == 1200,
            (0x00FC, data) => strings = shared_string_sizes(data, &mut stream, utf16)?,
            (0x000A, _) => break,
            _ => {}
        }
    }
    if offsets.is_empty() {
        return Err(invalid());
    }
    let globals_end = workbook.len() - stream.len();
    offsets.sort_unstable();
    let mut total = 0;
    let mut text_bytes = 0;
    let mut unsupported = false;
    for (i, &offset) in offsets.iter().enumerate() {
        let end = offsets.get(i + 1).copied().unwrap_or(workbook.len());
        if offset < globals_end || offset >= end {
            return Err(invalid());
        }
        let mut sheet = workbook.get(offset..end).ok_or_else(invalid)?;
        let mut declared = None;
        let (mut first_row, mut last_row, mut first_col, mut last_col) = (u32::MAX, 0, u32::MAX, 0);
        let mut actual = 0;
        let mut formula_pos = (0, 0);
        loop {
            match record(&mut sheet)? {
                (0x000A, _) => break,
                (0x04BC, _) => unsupported = true, // SHRFMLA
                (0x0200, data) => {
                    if declared.replace(dimensions(data)?).is_some() {
                        return Err(invalid());
                    }
                }
                (
                    kind @ (0x0203 | 0x0204 | 0x00D6 | 0x0205 | 0x027E | 0x00FD | 0x00BD | 0x0006
                    | 0x0207),
                    data,
                ) => {
                    let (row, col) = if kind == 0x0207 {
                        formula_pos
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
                    text_bytes += match kind {
                        0x00FD => strings.get(u32_at(data, 6)? as usize).copied().unwrap_or(0),
                        0x0204 | 0x00D6 | 0x0207 => data.len() as u64 * 3,
                        _ => 16 * u64::from(end_col - col + 1),
                    };
                    if text_bytes > MAX_WORKBOOK_TEXT_BYTES {
                        return Err(excel_limit());
                    }
                    first_row = first_row.min(row);
                    last_row = last_row.max(row);
                    first_col = first_col.min(col);
                    last_col = last_col.max(end_col);
                    actual =
                        u64::from(last_row - first_row + 1) * u64::from(last_col - first_col + 1);
                    if kind == 0x0006 {
                        formula_pos = (row, col);
                        let count = u16_at(data, 20)? as usize;
                        if count > data.len() - 22 {
                            return Err(invalid());
                        }
                        unsupported |= count == 0 || data.get(22) == Some(&1);
                    }
                }
                _ => {}
            }
            let area = actual.max(declared.unwrap_or(0));
            if area > MAX_SHEET_BOX || total + area > MAX_WORKBOOK_BOX {
                return Err(excel_limit());
            }
        }
        total += actual.max(declared.unwrap_or(0));
    }
    Ok(unsupported)
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
mod tests {
    use super::*;

    fn number(row: u16, col: u16) -> Vec<u8> {
        let mut payload = row.to_le_bytes().to_vec();
        payload.extend(col.to_le_bytes());
        payload.extend([0, 0]);
        payload.extend(1.0_f64.to_le_bytes());
        payload
    }

    fn book(sheets: usize, globals: &[(u16, &[u8])], cells: &[(u16, &[u8])]) -> Vec<u8> {
        let mut book = Vec::new();
        add_record(&mut book, 0x0809, &[0, 6, 5, 0]);
        for &(kind, payload) in globals {
            add_record(&mut book, kind, payload);
        }
        let bounds_start = book.len();
        for i in 0..sheets {
            let mut bound = vec![0; 6];
            bound.extend([2, 0, b'S', b'A' + i as u8]);
            add_record(&mut book, 0x0085, &bound);
        }
        add_record(&mut book, 0x000A, &[]);
        for i in 0..sheets {
            let offset = (book.len() as u32).to_le_bytes();
            let at = bounds_start + i * 14 + 4;
            book[at..at + 4].copy_from_slice(&offset);
            add_record(&mut book, 0x0809, &[0, 6, 0x10, 0]);
            for &(kind, payload) in cells {
                add_record(&mut book, kind, payload);
            }
            add_record(&mut book, 0x000A, &[]);
        }
        book
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
    fn refuses_more_than_fifty_sheets() {
        let bytes = compound(&[("/Book", &book(51, &[], &[]))]);
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
