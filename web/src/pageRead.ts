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
};

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
  const snapshotId = typeof page.snapshotId === "string" &&
    /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/.test(page.snapshotId) ? page.snapshotId : undefined;
  return {
    page: {
      title: typeof page.title === "string" ? page.title : "",
      url: typeof page.url === "string" ? page.url : "",
      text: page.text,
      image: typeof page.image === "string" && page.image.startsWith(JPEG_PREFIX) ? page.image : null,
      ...(snapshotId ? { snapshotId } : {}),
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

/** Lampiran dari halaman yang dibaca: teksnya (.md) dan screenshot (.jpg) kalau ada. */
export function pageFiles(page: PageRead, now: Date): File[] {
  const title = fileTitle(page.title);
  const head = [`# ${page.title.trim() || title}`, "", `Alamat: ${displayUrl(page.url)}`, `Dibaca: ${stamp(now)}`, ""];
  const files = [new File([`${head.join("\n")}\n${page.text}\n`], `Halaman - ${title}.md`, { type: "text/markdown" })];
  if (page.image) {
    const bytes = Uint8Array.from(atob(page.image.slice(JPEG_PREFIX.length)), (c) => c.charCodeAt(0));
    files.push(new File([bytes], `Layar - ${title}.jpg`, { type: "image/jpeg" }));
  }
  return files;
}
