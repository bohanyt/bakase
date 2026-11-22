/** Aturan lampiran di kotak tanya. Sama dengan server/src/attachments.rs; server tetap yang
 * memutuskan (termasuk cek isi file). Modul ini tanpa React supaya bisa dites dengan
 * `node --test`. */

export const MAX_ATTACHMENTS = 5;

const MB = 1024 * 1024;
const MAX_IMAGE = 10 * MB;
const MAX_PDF = 20 * MB;
const MAX_DOC = 25 * MB;
/** Gambar dan PDF dikirim utuh ke model, jadi totalnya per pesan dibatasi. */
const MAX_SENT = 25 * MB;

type Group = "image" | "pdf" | "office" | "text";

const GROUPS: Record<string, Group> = {
  png: "image",
  jpg: "image",
  jpeg: "image",
  webp: "image",
  gif: "image",
  pdf: "pdf",
  docx: "office",
  xlsx: "office",
  xls: "office",
  pptx: "office",
  txt: "text",
  csv: "text",
  md: "text",
};

/** Untuk atribut `accept` pada pemilih file. */
export const ACCEPT = [...Object.keys(GROUPS).map((ext) => `.${ext}`), "image/png", "image/jpeg", "image/webp", "image/gif"].join(",");

export function extensionOf(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot < 0 ? "" : name.slice(dot + 1).trim().toLowerCase();
}

export function isImageKind(kind: string): boolean {
  return ["png", "jpg", "jpeg", "gif", "webp"].includes(kind);
}

/** Pesan error kalau file ini tidak bisa dilampirkan, atau null. */
export function checkFile(name: string, size: number): string | null {
  const ext = extensionOf(name);
  const group = GROUPS[ext];
  if (!group) {
    return "Jenis file ini belum bisa dilampirkan. Yang bisa: gambar (PNG, JPG, WebP, GIF), PDF, Word (.docx), Excel (.xlsx, .xls), PowerPoint (.pptx), dan teks (.txt, .csv, .md).";
  }
  if (size === 0) return "File ini kosong.";
  if (group === "image" && size > MAX_IMAGE) return "Gambar terlalu besar. Maksimal 10 MB per gambar.";
  if (group === "pdf" && size > MAX_PDF) return "PDF terlalu besar. Maksimal 20 MB per file.";
  if (size > MAX_DOC) return "File terlalu besar. Maksimal 25 MB per file.";
  return null;
}

/** Pesan error kalau gambar dan PDF di satu pesan melewati batas, atau null. */
export function totalError(files: readonly { name: string; size: number }[]): string | null {
  const sent = files
    .filter((f) => ["image", "pdf"].includes(GROUPS[extensionOf(f.name)] ?? ""))
    .reduce((sum, f) => sum + f.size, 0);
  return sent > MAX_SENT ? "Gambar dan PDF di satu pesan maksimal 25 MB. Kirim sebagian di pesan berikutnya." : null;
}

const IMAGE_EXT: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/webp": "webp",
  "image/gif": "gif",
};

function pad(n: number): string {
  return String(n).padStart(2, "0");
}

/** Nama untuk gambar yang ditempel dengan Ctrl+V. Tangkapan layar dari clipboard biasanya
 * bernama "image.png", jadi diberi nama yang lebih jelas. */
export function pastedName(name: string, type: string, now: Date): string {
  const ext = IMAGE_EXT[type];
  if (!ext || (name && !/^image\.\w+$/i.test(name))) return name || `Lampiran.${ext ?? "bin"}`;
  const date = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
  const time = `${pad(now.getHours())}.${pad(now.getMinutes())}.${pad(now.getSeconds())}`;
  return `Tangkapan layar ${date} ${time}.${ext}`;
}
