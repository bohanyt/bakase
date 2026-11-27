import type { MyFile } from "./api.ts";
import { shortDate } from "./time.ts";

export type FilesTab = "hasil" | "upload";

/** File di satu tab File Saya: buatan Bakase atau lampiran user. Urutan dari server tetap. */
export function filesOf(files: readonly MyFile[], tab: FilesTab): MyFile[] {
  return files.filter((f) => f.upload === (tab === "upload"));
}

/** "hari ini 09.12", "kemarin 16.40", atau "20 Nov" (ditambah tahun kalau beda tahun). */
export function fileDate(unixSeconds: number, now = new Date()): string {
  const day = shortDate(unixSeconds, now);
  if (day !== "Hari ini" && day !== "Kemarin") return day;
  const d = new Date(unixSeconds * 1000);
  const time = `${String(d.getHours()).padStart(2, "0")}.${String(d.getMinutes()).padStart(2, "0")}`;
  return `${day.toLowerCase()} ${time}`;
}
