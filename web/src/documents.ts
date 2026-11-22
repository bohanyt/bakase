import { isImageKind } from "./attachment.ts";

export function isSourceImage(kind: string) {
  return isImageKind(kind) || kind === "bmp";
}

export function sourcePreviewPage(stored: number | null, indexed?: number | null) {
  return indexed ?? stored;
}

/** Judul tingkat dua dari pembaca spreadsheet berawalan nama sheet, lalu label tabel. */
export function sourcePreviewText(text: string, kind: string, status: string, sheetCount?: number) {
  if (status !== "ok" || (sheetCount ?? 1) > 1 || !["xls", "xlsx", "xlsm", "ods"].includes(kind)) return text;
  return text.replace(/^##[ \t]+([^\r\n]*)/gm, (_heading, title: string) => {
    const separator = title.indexOf(" › ");
    return separator < 0 ? "" : `## ${title.slice(separator + 3)}`;
  });
}

/** Satu file di halaman Sumber Data. */
export type DocumentItem = {
  id: number;
  title: string;
  display_title: string;
  source: string;
  /** Folder di dalam sumber, kosong kalau di akar. */
  folder: string;
  kind: string;
  status: string;
  /** Milidetik sejak epoch. */
  modified: number;
  /** Tanggal dokumen `YYYY-MM-DD` dari judul, isi, atau metadata file, kalau diketahui. */
  doc_date: string | null;
  /** Asal tanggal dokumen: `judul`, `isi`, atau `metadata`. */
  doc_date_from: string | null;
  size: number;
  pages: number;
  sheet_count?: number;
  chunks: number;
  /** File asli bisa diunduh. */
  available: boolean;
};

export type DocumentList = {
  documents: DocumentItem[];
  total: number;
  offset: number;
  limit: number;
  sources: { name: string; total: number }[];
  statuses: Record<string, number>;
};

const WAITING = ["scan", "gambar"];
const UNREADABLE = ["gagal", "tidak_didukung", "terlalu_besar", "kosong"];
const WAITING_LABEL = "Menunggu dibaca";
const UNREADABLE_LABEL = "Tidak bisa dibaca";

export const UNSEARCHABLE_STATUSES = [...WAITING, ...UNREADABLE];

export function attentionStatus(status: string) {
  if (WAITING.includes(status)) return { label: WAITING_LABEL, tone: "wait" };
  if (UNREADABLE.includes(status)) return { label: UNREADABLE_LABEL, tone: "bad" };
  return undefined;
}

export function unsearchableSummary(statuses: DocumentList["statuses"]): string | null {
  const waiting = WAITING.reduce((total, status) => total + (statuses[status] ?? 0), 0);
  const unreadable = UNREADABLE.reduce((total, status) => total + (statuses[status] ?? 0), 0);
  if (waiting + unreadable === 0) return null;
  const counts = [
    waiting > 0 ? `${waiting} ${WAITING_LABEL.toLowerCase()}` : null,
    unreadable > 0 ? `${unreadable} ${UNREADABLE_LABEL.toLowerCase()}` : null,
  ].filter(Boolean);
  return `${waiting + unreadable} file belum bisa dicari Bakase: ${counts.join(", ")}.`;
}

/** Penjelasan untuk file yang isinya belum bisa ditampilkan. */
export function statusNote(status: string): string {
  switch (status) {
    case "scan":
    case "gambar":
      return "Ini foto atau hasil scan yang teksnya belum dibaca, jadi Bakase belum bisa memakai isinya. Teksnya ikut terbaca setelah admin menjalankan pembacaan scan.";
    case "gagal":
      return "File ini gagal dibaca saat diindeks. Mungkin filenya rusak atau dikunci password.";
    case "tidak_didukung":
      return "Format file ini belum didukung Bakase.";
    case "terlalu_besar":
      return "File ini terlalu besar untuk dibaca (lebih dari 150 MB).";
    case "kosong":
      return "File ini tidak berisi teks yang bisa dibaca.";
    default:
      return "Teks file ini belum tersedia.";
  }
}
