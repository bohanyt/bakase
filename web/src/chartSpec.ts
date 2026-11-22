// Blok ```chart di jawaban Bakase: JSON data dari model, digambar web. Isinya data dari model,
// jadi dibaca ketat. Blok yang tidak lolos tampil sebagai kode biasa.

export type ChartType = "bar" | "line" | "pie";

export type ChartSeries = { name: string; data: (number | null)[] };

export type ChartSpec = {
  type: ChartType;
  title: string;
  /** Satuan nilai, misalnya "ton" atau "%". */
  unit: string;
  /** Nama sumbu kategori, misalnya "Bulan"; dipakai sebagai kepala kolom tabel. */
  x: string;
  labels: string[];
  series: ChartSeries[];
};

export const MAX_SERIES = 6;
export const MAX_LABELS = 60;
export const MAX_PIE_SLICES = 6;

const TYPES: readonly ChartType[] = ["bar", "line", "pie"];

function text(value: unknown, max: number): string | null {
  if (value === undefined || value === null) return "";
  if (typeof value !== "string" && typeof value !== "number") return null;
  return String(value).replace(/\s+/g, " ").trim().slice(0, max);
}

/** Spesifikasi grafik dari isi blok, atau `null` kalau bentuknya tidak dikenal. */
export function parseChartSpec(code: string): ChartSpec | null {
  let raw: unknown;
  try {
    raw = JSON.parse(code);
  } catch {
    return null;
  }
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return null;
  const obj = raw as Record<string, unknown>;
  const type = obj.type as ChartType;
  if (!TYPES.includes(type)) return null;
  const title = text(obj.title, 120);
  const unit = text(obj.unit, 20);
  const x = text(obj.x, 40);
  if (title === null || unit === null || x === null) return null;

  if (!Array.isArray(obj.labels) || obj.labels.length === 0 || obj.labels.length > MAX_LABELS) return null;
  const labels: string[] = [];
  for (const label of obj.labels) {
    const t = text(label, 80);
    if (!t) return null;
    labels.push(t);
  }

  if (!Array.isArray(obj.series) || obj.series.length === 0 || obj.series.length > MAX_SERIES) return null;
  const series: ChartSeries[] = [];
  for (const item of obj.series) {
    if (typeof item !== "object" || item === null) return null;
    const s = item as Record<string, unknown>;
    const name = text(s.name, 60);
    if (name === null || !Array.isArray(s.data) || s.data.length !== labels.length) return null;
    const data: (number | null)[] = [];
    for (const v of s.data) {
      if (v === null) data.push(null);
      else if (typeof v === "number" && Number.isFinite(v)) data.push(v);
      else return null;
    }
    if (data.every((v) => v === null)) return null;
    series.push({ name: name || title || "Nilai", data });
  }

  if (type === "pie") {
    const values = series[0].data;
    if (series.length !== 1 || labels.length > MAX_PIE_SLICES) return null;
    if (values.some((v) => v === null || v < 0) || !values.some((v) => (v ?? 0) > 0)) return null;
  }
  return { type, title, unit, x, labels, series };
}

const number = new Intl.NumberFormat("id-ID", { maximumFractionDigits: 2 });
const compact = new Intl.NumberFormat("id-ID", { notation: "compact", maximumFractionDigits: 1 });

/** Nilai lengkap dengan satuan: "1.234,5 ton", "12,5%". */
export function formatValue(value: number | null, unit: string): string {
  if (value === null) return "–";
  const n = number.format(value);
  if (!unit) return n;
  return unit === "%" ? `${n}%` : `${n} ${unit}`;
}

/** Label sumbu yang pendek: "1,2 rb", "3,4 jt". */
export function formatTick(value: number, unit: string): string {
  const n = Math.abs(value) >= 10_000 ? compact.format(value) : number.format(value);
  return unit === "%" ? `${n}%` : n;
}

/** Isi tabel yang sama dengan grafik, supaya angkanya bisa dibaca tanpa hover. */
export function tableRows(spec: ChartSpec): { head: string[]; rows: string[][] } {
  const head = [spec.x || (spec.type === "pie" ? "Bagian" : "Kategori"), ...spec.series.map((s) => s.name)];
  const rows = spec.labels.map((label, i) => [label, ...spec.series.map((s) => formatValue(s.data[i], spec.unit))]);
  return { head, rows };
}
