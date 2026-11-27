// Bagian murni jembatan extension (tanpa window), supaya bisa dites di Node.
// Lihat pageBridge.ts untuk pengiriman pesannya.

/** Halaman yang dibaca extension (lihat extension/panel.js). */
export type PageRead = {
  title: string;
  url: string;
  text: string;
  /** Screenshot bagian yang terlihat, data URL JPEG, kalau berhasil diambil. */
  image: string | null;
  /** Id yang menandai bahwa panel bisa menjalankan aksi pada snapshot ini. */
  snapshotId?: string;
  ambilTabel?: true;
  /** File asli dari Excel Online. Kalau ada, file ini yang dilampirkan, bukan teks layar. */
  file?: { name: string; bytes: ArrayBuffer };
  /** Bagian yang sedang difokuskan user: sheet dan sel aktif, tab aktif, teks yang dipilih. */
  focus?: string;
};

/** Panjang catatan fokus paling banyak; sama dengan batas di server. */
export const MAX_FOCUS = 500;

/** Catatan fokus satu baris tanpa karakter kontrol, atau undefined. */
export function cleanFocus(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  // Surrogate tunggal (emoji yang terpotong) tidak bisa dikirim di header, jadi dibuang.
  const text = Array.from(value.replace(/[\uD800-\uDFFF]/gu, "").replace(/\p{Cc}+/gu, " ").replace(/\s+/g, " ").trim())
    .slice(0, MAX_FOCUS).join("");
  return text || undefined;
}

/** Sama dengan batas lampiran Office di server. */
const MAX_FILE = 25 * 1024 * 1024;

function excelFile(value: unknown): PageRead["file"] | undefined {
  if (!value || typeof value !== "object") return undefined;
  const { name, bytes } = value as Record<string, unknown>;
  if (typeof name !== "string" || !(bytes instanceof ArrayBuffer)) return undefined;
  if (!bytes.byteLength || bytes.byteLength > MAX_FILE || !/\.(xlsx|xlsm)$/i.test(name)) return undefined;
  // File xlsx/xlsm berupa ZIP. Isi lain tidak menggantikan teks layar.
  const head = new Uint8Array(bytes, 0, Math.min(2, bytes.byteLength));
  if (head[0] !== 0x50 || head[1] !== 0x4b) return undefined;
  const clean = name.replace(/[\p{Cc}\\/:*?"<>|]+/gu, " ").replace(/\s+/g, " ").trim();
  return clean ? { name: Array.from(clean).slice(-120).join(""), bytes } : undefined;
}

const JPEG_PREFIX = "data:image/jpeg;base64,";

/** Origin `chrome-extension://<id>` atau `extension://<id>` milik extension yang terdaftar. */
export function allowedOrigin(origin: string, extensionIds: readonly string[]): boolean {
  const match = /^(?:chrome-)?extension:\/\/([a-p]{32})$/.exec(origin);
  return match !== null && extensionIds.includes(match[1]);
}

/** Balasan extension untuk permintaan `id`, sudah diperiksa bentuknya. */
export function readReply(data: unknown, id: string): { page: PageRead } | { error: string } | null {
  if (!data || typeof data !== "object") return null;
  const msg = data as Record<string, unknown>;
  if (msg.type !== "app:halaman" || msg.id !== id) return null;
  if (typeof msg.error === "string") return { error: msg.error.slice(0, 300) };
  const page = msg.page as Record<string, unknown> | undefined;
  if (!page || typeof page.text !== "string") return { error: "Balasan extension tidak terbaca." };
  const file = excelFile(page.file);
  const focus = cleanFocus(page.focus);
  const snapshotId = typeof page.snapshotId === "string" &&
    /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/.test(page.snapshotId) ? page.snapshotId : undefined;
  return {
    page: {
      title: typeof page.title === "string" ? page.title : "",
      url: typeof page.url === "string" ? page.url : "",
      text: page.text,
      image: typeof page.image === "string" && page.image.startsWith(JPEG_PREFIX) ? page.image : null,
      ...(snapshotId ? { snapshotId } : {}),
      ...(snapshotId && page.ambilTabel === true ? { ambilTabel: true } : {}),
      ...(file ? { file } : {}),
      ...(focus ? { focus } : {}),
    },
  };
}

/** Kabar progres extension untuk permintaan `id` ("Membaca tabel 1: 120 baris…"), atau null. */
export function readProgress(data: unknown, id: string): string | null {
  if (!data || typeof data !== "object") return null;
  const msg = data as Record<string, unknown>;
  if (msg.type !== "app:progres" || msg.id !== id || typeof msg.text !== "string") return null;
  return msg.text.replace(/\s+/g, " ").trim().slice(0, 80) || null;
}

/** Judul halaman yang aman dipakai sebagai nama file. */
export function fileTitle(title: string): string {
  const clean = title
    .replace(/[\p{Cc}\\/:*?"<>|]+/gu, " ")
    .replace(/\s+/g, " ")
    .trim();
  const short = Array.from(clean).slice(0, 60).join("").trim();
  return short || "halaman";
}

function stamp(now: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())} ${pad(now.getHours())}:${pad(now.getMinutes())}`;
}

function displayUrl(value: string): string {
  try {
    const address = new URL(value);
    return /^https?:$/.test(address.protocol) ? address.origin + address.pathname : "";
  } catch {
    return "";
  }
}

/** Lampiran dari halaman yang dibaca: teksnya (.md) dan screenshot (.jpg) kalau ada. File Excel
 * asli dari Excel Online menggantikan teks layar, karena layar hanya memuat sel yang tampil. */
export function pageFiles(page: PageRead, now: Date): File[] {
  if (page.file) {
    const type = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
    return [new File([page.file.bytes], page.file.name, { type })];
  }
  const title = fileTitle(page.title);
  const head = [`# ${page.title.trim() || title}`, "", `Alamat: ${displayUrl(page.url)}`, `Dibaca: ${stamp(now)}`];
  if (page.focus) head.push(`Fokus user: ${page.focus}`);
  head.push("");
  const files = [new File([`${head.join("\n")}\n${page.text}\n`], `Halaman - ${title}.md`, { type: "text/markdown" })];
  if (page.image) {
    const bytes = Uint8Array.from(atob(page.image.slice(JPEG_PREFIX.length)), (c) => c.charCodeAt(0));
    files.push(new File([bytes], `Layar - ${title}.jpg`, { type: "image/jpeg" }));
  }
  return files;
}
