// Pertanyaan lanjutan yang diketik tamu sebelum masuk. Hanya di memori halaman ini: hilang
// kalau halaman dimuat ulang, kedaluwarsa setelah 10 menit, dan dihapus saat dipakai, supaya
// draft tamu yang ditinggal di komputer bersama tidak terbawa ke login berikutnya.
const MAX_AGE_MS = 10 * 60 * 1000;

let pending: { text: string; at: number } | null = null;

export function saveFollowUp(text: string) {
  const trimmed = text.trim();
  pending = trimmed ? { text: trimmed, at: Date.now() } : null;
}

export function takeFollowUp(): string | null {
  const found = pending;
  pending = null;
  return found && Date.now() - found.at <= MAX_AGE_MS ? found.text : null;
}
