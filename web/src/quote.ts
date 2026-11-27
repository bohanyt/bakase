/** Kutipan (Cite) yang ikut dikirim di pesan user. Modul ini tanpa React supaya bisa dites
 * dengan `node --test`. */

/** Sama dengan `MAX_MESSAGE_CHARS` di server/src/main.rs: dihitung per karakter setelah trim. */
export const MAX_MESSAGE_CHARS = 20_000;

export type QuoteDraft = { text: string; source?: string };
export type Quote = QuoteDraft & { id: string; comment: string };
export type ParsedQuote = { text: string; source?: string; comment?: string };

const SOURCE_PREFIX = "— ";
const COMMENT_PREFIX = "Komentar: ";
const QUOTES_START = ":::kutipan";
const QUOTES_END = ":::akhir-kutipan";

let seq = 0;

export function newQuote(draft: QuoteDraft): Quote {
  seq += 1;
  return { id: `kutipan-${seq}`, text: draft.text, source: draft.source, comment: "" };
}

/** Teks pilihan dari browser: tanpa spasi di ujung baris dan tanpa baris kosong beruntun. */
export function cleanQuoteText(text: string): string {
  return text
    .replace(/\r\n?/g, "\n")
    .replace(/ /g, " ")
    .split("\n")
    .map((line) => line.trimEnd())
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function oneLine(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

/** Satu kutipan sebagai blockquote markdown, lalu sumber dan komentar di baris sendiri. */
export function formatQuote(quote: ParsedQuote): string {
  const lines = cleanQuoteText(quote.text)
    .split("\n")
    .map((line) => (line ? `> ${line}` : ">"));
  const source = quote.source ? oneLine(quote.source) : "";
  const comment = quote.comment ? oneLine(quote.comment) : "";
  if (source) lines.push(SOURCE_PREFIX + source);
  if (comment) lines.push(COMMENT_PREFIX + comment);
  return lines.join("\n");
}

function startsWithFrame(text: string): boolean {
  return /^\\*:::kutipan(?:\r?\n|$)/.test(text);
}

/** Bagian kutipan dibatasi eksplisit; pertanyaan sesudahnya tidak pernah diparse lagi.
 * Awalan penanda yang diketik sendiri di-escape satu backslash, termasuk backslash aslinya. */
export function formatMessage(quotes: readonly ParsedQuote[], question: string): string {
  const parts = quotes.filter((q) => cleanQuoteText(q.text)).map(formatQuote);
  // Sama persis dengan `str::trim` di server (Unicode White_Space: U+0085 ikut, U+FEFF tidak),
  // supaya pesan setelah dimuat ulang sama dengan yang tampil saat dikirim.
  const rest = question.replace(/^\p{White_Space}+|\p{White_Space}+$/gu, "");
  if (!parts.length) return startsWithFrame(rest) ? `\\${rest}` : rest;
  const section = `${QUOTES_START}\n${parts.join("\n\n")}\n${QUOTES_END}`;
  return rest ? `${section}\n\n${rest}` : section;
}

function isQuoteLine(line: string): boolean {
  return line === ">" || line.startsWith("> ");
}

/** Hanya bingkai lengkap buatan `formatMessage` yang menjadi kutipan. */
export function parseMessage(message: string): { quotes: ParsedQuote[]; question: string } {
  const plain = { quotes: [], question: message };
  if (message.startsWith("\\") && startsWithFrame(message)) return { quotes: [], question: message.slice(1) };
  const lines = message.split("\n");
  if (lines[0] !== QUOTES_START) return plain;
  const quotes: ParsedQuote[] = [];
  let i = 1;
  while (i < lines.length && isQuoteLine(lines[i])) {
    const text: string[] = [];
    while (i < lines.length && isQuoteLine(lines[i])) text.push(lines[i++].slice(2));
    const quote: ParsedQuote = { text: text.join("\n") };
    if (i < lines.length && lines[i].startsWith(SOURCE_PREFIX)) quote.source = lines[i++].slice(SOURCE_PREFIX.length);
    if (i < lines.length && lines[i].startsWith(COMMENT_PREFIX)) quote.comment = lines[i++].slice(COMMENT_PREFIX.length);
    quotes.push(quote);
    if (lines[i] === QUOTES_END) {
      if (i === lines.length - 1) return { quotes, question: "" };
      if (lines[i + 1] === "") return { quotes, question: lines.slice(i + 2).join("\n") };
      return plain;
    }
    if (lines[i] === "") i++;
    else return plain;
  }
  return plain;
}

/** Panjang pesan seperti dihitung server (`chars().count()` setelah trim). */
export function messageLength(message: string): number {
  return [...message.trim()].length;
}

const number = new Intl.NumberFormat("id-ID");

/** Pesan error kalau pesan beserta kutipannya melewati batas server, atau null. */
export function lengthError(message: string, withQuotes: boolean): string | null {
  const length = messageLength(message);
  if (length <= MAX_MESSAGE_CHARS) return null;
  const what = withQuotes ? "Pesan dan kutipan" : "Pesan";
  const fix = withQuotes ? "Hapus sebagian kutipan atau persingkat pesannya." : "Persingkat pesannya.";
  return `${what} terlalu panjang: ${number.format(length)} dari maksimal ${number.format(MAX_MESSAGE_CHARS)} karakter. ${fix}`;
}

/** Pesan error kalau satu kutipan saja sudah tidak muat di satu pesan, atau null. */
export function quoteError(draft: QuoteDraft): string | null {
  if (!cleanQuoteText(draft.text)) return "Tidak ada teks yang bisa dikutip.";
  const length = messageLength(formatMessage([draft], ""));
  if (length <= MAX_MESSAGE_CHARS) return null;
  return `Kutipan terlalu panjang: ${number.format(length)} dari maksimal ${number.format(MAX_MESSAGE_CHARS)} karakter. Pilih bagian yang lebih kecil.`;
}

/** Kolom ke-0 adalah A, ke-25 Z, ke-26 AA. */
export function columnName(index: number): string {
  let n = Math.max(0, Math.floor(index)) + 1;
  let name = "";
  while (n > 0) {
    const rem = (n - 1) % 26;
    name = String.fromCharCode(65 + rem) + name;
    n = Math.floor((n - 1) / 26);
  }
  return name;
}

/** Nama sheet untuk rumus: diberi kutip tunggal kalau ada spasi, tanda baca, atau mirip alamat sel. */
export function sheetRef(name: string): string {
  const plain =
    /^[A-Za-z_][A-Za-z0-9_.]*$/.test(name) && !/^[A-Za-z]{1,3}[0-9]+$/.test(name) && !/^(R|C|R[0-9]*C[0-9]*)$/i.test(name);
  return plain ? name : `'${name.replace(/'/g, "''")}'`;
}

/** Sel di sheet: baris mulai dari 1 (baris judul kolom), kolom mulai dari 0. */
export type CellPos = { row: number; col: number };

export function rangeBounds(a: CellPos, b: CellPos) {
  return {
    top: Math.min(a.row, b.row),
    bottom: Math.max(a.row, b.row),
    left: Math.min(a.col, b.col),
    right: Math.max(a.col, b.col),
  };
}

/**
 * Langkah gulir per frame saat seret memilih sel mendekati atau melewati tepi area tabel
 * yang terlihat (`start` sampai `end`). Makin jauh lewat tepi, makin cepat. Zona di dalam
 * tepi perlu ada karena tepi panel sering menempel di tepi layar.
 */
export function edgeScrollStep(pos: number, start: number, end: number, zone = 24, max = 24): number {
  const edge = Math.max(4, Math.min(zone, Math.floor((end - start) / 4)));
  if (pos < start + edge) return -Math.min(max, Math.ceil((start + edge - pos) / 6));
  if (pos > end - edge) return Math.min(max, Math.ceil((pos - (end - edge)) / 6));
  return 0;
}

/** `Data!B2:D5`, atau `Data!B2` untuk satu sel. */
export function rangeA1(sheet: string | null, a: CellPos, b: CellPos): string {
  const { top, bottom, left, right } = rangeBounds(a, b);
  const start = `${columnName(left)}${top}`;
  const end = `${columnName(right)}${bottom}`;
  const prefix = sheet === null ? "" : `${sheetRef(sheet)}!`;
  return `${prefix}${start}${start === end ? "" : `:${end}`}`;
}

function tableCell(text: string): string {
  return oneLine(text).replace(/\|/g, "\\|");
}

/** Tabel markdown kecil dari isi sel yang sudah diformat. */
export function markdownTable(header: readonly string[], rows: readonly (readonly string[])[]): string {
  const line = (cells: readonly string[]) => `| ${cells.map(tableCell).join(" | ")} |`;
  return [line(header), line(header.map(() => "---")), ...rows.map(line)].join("\n");
}

function splitRow(line: string): string[] {
  const inner = line.trim().replace(/^\|/, "").replace(/\|$/, "");
  return inner.split(/(?<!\\)\|/).map((cell) => cell.trim().replace(/\\\|/g, "|"));
}

/** Isi tabel kalau kutipan berupa tabel markdown buatan `markdownTable`, atau null. */
export function parseTable(text: string): { header: string[]; rows: string[][] } | null {
  const lines = text.split("\n");
  if (lines.length < 2 || !lines.every((l) => l.trim().startsWith("|") && l.trim().endsWith("|"))) return null;
  if (!/^\|(\s*:?-{3,}:?\s*\|)+$/.test(lines[1].replace(/\s/g, ""))) return null;
  const cells = lines.map(splitRow);
  const header = cells[0];
  if (!cells.every((row) => row.length === header.length)) return null;
  return { header, rows: cells.slice(2) };
}
