const DAY = 24 * 60 * 60 * 1000;
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "Mei", "Jun", "Jul", "Agu", "Sep", "Okt", "Nov", "Des"];
const DAYS = ["Minggu", "Senin", "Selasa", "Rabu", "Kamis", "Jumat", "Sabtu"];

function startOfDay(d: Date) {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** Kelompok riwayat seperti ChatGPT: Hari ini, Kemarin, 7 hari terakhir, dst. */
export function groupLabel(unixSeconds: number, now = new Date()): string {
  const diff = startOfDay(now) - startOfDay(new Date(unixSeconds * 1000));
  if (diff <= 0) return "Hari ini";
  if (diff <= DAY) return "Kemarin";
  if (diff <= 7 * DAY) return "7 hari terakhir";
  if (diff <= 30 * DAY) return "30 hari terakhir";
  return "Lebih lama";
}

/** "Hari ini", "Kemarin", atau "14 Nov" (ditambah tahun kalau beda tahun). */
export function shortDate(unixSeconds: number, now = new Date()): string {
  const label = groupLabel(unixSeconds, now);
  if (label === "Hari ini" || label === "Kemarin") return label;
  const d = new Date(unixSeconds * 1000);
  const base = `${d.getDate()} ${MONTHS[d.getMonth()]}`;
  return d.getFullYear() === now.getFullYear() ? base : `${base} ${d.getFullYear()}`;
}

/** "16 Nov 2026" */
export function longDate(d: Date): string {
  return `${d.getDate()} ${MONTHS[d.getMonth()]} ${d.getFullYear()}`;
}

/** "Senin, 16 Nov 2026 · 13.21" */
export function fullDateTime(unixSeconds: number): string {
  const d = new Date(unixSeconds * 1000);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `${DAYS[d.getDay()]}, ${longDate(d)} · ${hh}.${mm}`;
}

/** Tanggal kalender `YYYY-MM-DD` sebagai tanggal lokal, atau null kalau tidak valid. */
export function calendarDate(value: string | null): Date | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value ?? "");
  if (!match) return null;
  const d = new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3]));
  return d.getDate() === Number(match[3]) ? d : null;
}
