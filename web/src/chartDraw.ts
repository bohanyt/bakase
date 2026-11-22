// Menggambar blok ```chart dengan Chart.js. File ini baru diunduh browser saat grafik pertama
// muncul. Warna, huruf, dan ukuran diambil dari token tema, sama seperti grafik Pemakaian.
import {
  ArcElement,
  BarController,
  BarElement,
  CategoryScale,
  Chart,
  DoughnutController,
  LinearScale,
  LineController,
  LineElement,
  PointElement,
  Tooltip,
  type ChartConfiguration,
  type Plugin,
  type TooltipModel,
} from "chart.js";
import { formatTick, formatValue, type ChartSpec } from "./chartSpec";

Chart.register(BarController, BarElement, LineController, LineElement, PointElement, DoughnutController, ArcElement, CategoryScale, LinearScale, Tooltip);

type Tokens = {
  series: string[];
  fg: string;
  muted: string;
  faint: string;
  line: string;
  bg: string;
  font: string;
  base: number;
  small: number;
};

function tokens(): Tokens {
  const root = getComputedStyle(document.documentElement);
  const v = (name: string) => root.getPropertyValue(name).trim();
  return {
    series: [1, 2, 3, 4, 5, 6].map((i) => v(`--series-${i}`)),
    fg: v("--fg"),
    muted: v("--muted"),
    faint: v("--faint"),
    line: v("--line"),
    bg: v("--bg"),
    font: root.fontFamily,
    base: parseFloat(root.fontSize),
    // 0.75rem, ukuran label sumbu di grafik Pemakaian.
    small: parseFloat(root.fontSize) * 0.75,
  };
}

/** Tooltip HTML dengan gaya kartu yang sama seperti grafik Pemakaian. Teks dari model
 * dimasukkan lewat textContent. */
function htmlTooltip(tip: HTMLElement, spec: ChartSpec, colors: string[]) {
  return ({ chart, tooltip }: { chart: Chart; tooltip: TooltipModel<"bar" | "line" | "doughnut"> }) => {
    if (tooltip.opacity === 0 || !tooltip.dataPoints?.length) {
      tip.hidden = true;
      return;
    }
    const first = tooltip.dataPoints[0];
    tip.replaceChildren();
    const head = document.createElement("span");
    head.textContent = spec.labels[first.dataIndex];
    tip.append(head);
    const total = spec.type === "pie" ? spec.series[0].data.reduce<number>((a, b) => a + (b ?? 0), 0) : 0;
    for (const point of tooltip.dataPoints) {
      const series = spec.series[point.datasetIndex];
      const value = series.data[point.dataIndex];
      const row = document.createElement("div");
      const key = document.createElement("i");
      key.style.background = spec.type === "pie" ? colors[point.dataIndex] : colors[point.datasetIndex];
      const strong = document.createElement("strong");
      strong.textContent =
        formatValue(value, spec.unit) + (spec.type === "pie" && total ? ` · ${Math.round(((value ?? 0) / total) * 100)}%` : "");
      row.append(key, strong);
      if (spec.series.length > 1) {
        const name = document.createElement("span");
        name.textContent = series.name;
        row.append(name);
      }
      tip.append(row);
    }
    // Diukur dari kiri dulu, supaya posisi lama tidak mempersempit lebar tooltip.
    tip.style.left = "0px";
    tip.hidden = false;
    const width = chart.width;
    const left = Math.min(Math.max(tooltip.caretX - tip.offsetWidth / 2, 0), Math.max(width - tip.offsetWidth, 0));
    tip.style.left = `${left}px`;
    tip.style.top = `${Math.max(tooltip.caretY - tip.offsetHeight - 10, 0)}px`;
  };
}

/** Garis tipis vertikal di bulan yang sedang ditunjuk, untuk grafik garis. */
const crosshair = (color: string): Plugin<"line"> => ({
  id: "crosshair",
  afterDatasetsDraw(chart) {
    const active = chart.tooltip?.getActiveElements();
    if (!active?.length) return;
    const { ctx, chartArea } = chart;
    const x = active[0].element.x;
    ctx.save();
    ctx.strokeStyle = color;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(x, chartArea.top);
    ctx.lineTo(x, chartArea.bottom);
    ctx.stroke();
    ctx.restore();
  },
});

/** Nilai di atas batang, seperti label puncak di grafik Pemakaian. Hanya untuk satu seri
 * dengan sedikit batang supaya tidak ramai. */
const valueLabels = (spec: ChartSpec, t: Tokens): Plugin<"bar"> => ({
  id: "valueLabels",
  afterDatasetsDraw(chart) {
    const { ctx } = chart;
    ctx.save();
    ctx.fillStyle = t.muted;
    ctx.font = `${t.small}px ${t.font}`;
    ctx.textAlign = "center";
    chart.getDatasetMeta(0).data.forEach((bar, i) => {
      const v = spec.series[0].data[i];
      if (v !== null) ctx.fillText(formatValue(v, ""), bar.x, bar.y - 6);
    });
    ctx.restore();
  },
});

/** Batang satu kelompok berdempetan dengan celah tipis: lebar kelompok mengikuti jumlah seri
 * (sekitar 28 px per batang), dihitung ulang dari lebar plot setiap kali grafik diperbarui,
 * termasuk saat ukurannya berubah. */
const groupWidth = (spec: ChartSpec): Plugin<"bar"> => ({
  id: "groupWidth",
  beforeUpdate(chart) {
    const slot = (chart.chartArea?.width || chart.width) / spec.labels.length;
    const share = Math.min(0.8, (spec.series.length * 30) / slot);
    for (const dataset of chart.data.datasets) (dataset as { categoryPercentage?: number }).categoryPercentage = share;
  },
});

export type DrawnChart = { destroy(): void; png(): Promise<Blob | null> };

export function drawChart(canvas: HTMLCanvasElement, tip: HTMLElement, spec: ChartSpec): DrawnChart {
  const t = tokens();
  Chart.defaults.font.family = t.font;
  Chart.defaults.font.size = t.small;
  const colors = spec.type === "pie" ? spec.labels.map((_, i) => t.series[i % t.series.length]) : t.series;
  const tooltip = { enabled: false, external: htmlTooltip(tip, spec, colors) };
  const axis = {
    x: { grid: { display: false }, border: { color: t.muted }, ticks: { color: t.faint } },
    y: {
      beginAtZero: spec.type === "bar",
      grid: { color: t.line },
      border: { display: false },
      ticks: { color: t.faint, maxTicksLimit: 6, callback: (v: string | number) => formatTick(Number(v), spec.unit) },
    },
  };

  let config: ChartConfiguration<"bar" | "line" | "doughnut">;
  if (spec.type === "pie") {
    config = {
      type: "doughnut",
      data: {
        labels: spec.labels,
        datasets: [{ data: spec.series[0].data, backgroundColor: colors, borderColor: t.bg, borderWidth: 2, hoverOffset: 4 }],
      },
      options: { maintainAspectRatio: false, cutout: "58%", plugins: { tooltip } },
    } as ChartConfiguration<"doughnut">;
  } else if (spec.type === "line") {
    config = {
      type: "line",
      data: {
        labels: spec.labels,
        datasets: spec.series.map((s, i) => ({
          label: s.name,
          data: s.data,
          borderColor: colors[i],
          backgroundColor: colors[i],
          borderWidth: 2,
          pointRadius: spec.labels.length <= 12 ? 3 : 0,
          pointHoverRadius: 5,
          pointHoverBorderWidth: 2,
          pointHoverBorderColor: t.bg,
          tension: 0.25,
          spanGaps: false,
        })),
      },
      options: { maintainAspectRatio: false, interaction: { mode: "index", intersect: false }, plugins: { tooltip }, scales: axis },
      plugins: [crosshair(t.muted)],
    } as ChartConfiguration<"line">;
  } else {
    const single = spec.series.length === 1;
    config = {
      type: "bar",
      data: {
        labels: spec.labels,
        datasets: spec.series.map((s, i) => ({
          label: s.name,
          data: s.data,
          backgroundColor: colors[i],
          hoverBackgroundColor: colors[i],
          borderRadius: { topLeft: 4, topRight: 4 },
          borderSkipped: "start",
          ...(single ? { maxBarThickness: 48, categoryPercentage: 0.7, barPercentage: 0.9 } : { barPercentage: 0.92 }),
        })),
      },
      options: {
        maintainAspectRatio: false,
        layout: { padding: { top: single ? 18 : 4 } },
        interaction: { mode: single ? "nearest" : "index", intersect: single },
        plugins: { tooltip },
        scales: axis,
      },
      plugins: single ? (spec.labels.length <= 12 ? [valueLabels(spec, t)] : []) : [groupWidth(spec)],
    } as ChartConfiguration<"bar">;
  }

  const chart = new Chart(canvas, config);
  return {
    destroy: () => chart.destroy(),
    png: () => exportPng(canvas, spec, t, colors),
  };
}

const segmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });
const graphemes = (text: string) => Array.from(segmenter.segment(text), (s) => s.segment);

/** Teks dipecah per kata supaya muat `max` piksel; kata yang terlalu panjang dipotong. */
function wrap(ctx: CanvasRenderingContext2D, text: string, max: number): string[] {
  const lines: string[] = [];
  let line = "";
  for (const word of text.split(" ")) {
    const next = line ? `${line} ${word}` : word;
    if (ctx.measureText(next).width <= max) {
      line = next;
      continue;
    }
    if (line) lines.push(line);
    line = word;
    // Dipotong per grafem supaya emoji dan huruf beraksen tidak terbelah.
    let parts = graphemes(line);
    while (parts.length > 1 && ctx.measureText(line).width > max) {
      let cut = parts.length - 1;
      while (cut > 1 && ctx.measureText(parts.slice(0, cut).join("")).width > max) cut--;
      lines.push(parts.slice(0, cut).join(""));
      parts = parts.slice(cut);
      line = parts.join("");
    }
  }
  if (line) lines.push(line);
  return lines;
}

/** PNG yang bisa dibaca tanpa halaman: judul dengan satuan, legenda, lalu grafiknya, di atas
 * latar tema. Kanvas Chart.js sendiri transparan dan tidak memuat judul atau legenda. Judul
 * dan legenda yang panjang dibungkus ke baris berikutnya. */
function exportPng(canvas: HTMLCanvasElement, spec: ChartSpec, t: Tokens, colors: string[]): Promise<Blob | null> {
  const scale = canvas.clientWidth ? canvas.width / canvas.clientWidth : 1;
  const pad = 16 * scale;
  const titleSize = t.base * scale;
  const keySize = t.small * scale;
  const out = document.createElement("canvas");
  const ctx = out.getContext("2d");
  if (!ctx) return Promise.resolve(null);
  const inner = canvas.width;
  const title = spec.unit ? `${spec.title || "Grafik"} (${spec.unit})` : spec.title;
  const titleFont = `600 ${titleSize}px ${t.font}`;
  const keyFont = `${keySize}px ${t.font}`;
  const keys = spec.type === "pie" ? spec.labels : spec.series.length > 1 ? spec.series.map((s) => s.name) : [];

  ctx.font = titleFont;
  const titleLines = title ? wrap(ctx, title, inner) : [];
  const titleLineH = titleSize * 1.35;
  const titleH = titleLines.length ? titleLines.length * titleLineH + titleSize * 0.5 : 0;

  // Legenda: entri berderet dan pindah baris kalau tidak muat. Entri yang lebih lebar dari
  // grafik mendapat baris sendiri dan teksnya dibungkus.
  ctx.font = keyFont;
  const mark = keySize * 1.6;
  const gap = keySize * 1.2;
  type Key = { lines: string[]; color: string; x: number };
  const rows: Key[][] = [];
  let row: Key[] = [];
  let x = 0;
  keys.forEach((text, i) => {
    const color = colors[i % colors.length];
    const w = ctx.measureText(text).width;
    if (mark + w > inner) {
      if (row.length) rows.push(row);
      rows.push([{ lines: wrap(ctx, text, inner - mark), color, x: 0 }]);
      row = [];
      x = 0;
      return;
    }
    if (x + mark + w > inner && row.length) {
      rows.push(row);
      row = [];
      x = 0;
    }
    row.push({ lines: [text], color, x });
    x += mark + w + gap;
  });
  if (row.length) rows.push(row);
  const keyLineH = keySize * 1.5;
  const rowH = (r: Key[]) => Math.max(...r.map((k) => k.lines.length)) * keyLineH + keySize * 0.3;
  const keysH = rows.reduce((h, r) => h + rowH(r), 0);

  out.width = inner + pad * 2;
  out.height = pad + titleH + keysH + canvas.height + pad;
  ctx.fillStyle = t.bg;
  ctx.fillRect(0, 0, out.width, out.height);
  ctx.textBaseline = "middle";
  ctx.fillStyle = t.fg;
  ctx.font = titleFont;
  titleLines.forEach((line, i) => ctx.fillText(line, pad, pad + titleLineH * (i + 0.5)));
  ctx.font = keyFont;
  let y = pad + titleH;
  for (const r of rows) {
    for (const key of r) {
      const mid = y + keyLineH / 2;
      ctx.fillStyle = key.color;
      if (spec.type === "line") ctx.fillRect(pad + key.x, mid - scale, keySize * 1.2, 2 * scale);
      else ctx.fillRect(pad + key.x, mid - keySize * 0.4, keySize * 0.8, keySize * 0.8);
      ctx.fillStyle = t.muted;
      key.lines.forEach((line, i) => ctx.fillText(line, pad + key.x + mark, mid + keyLineH * i));
    }
    y += rowH(r);
  }
  ctx.drawImage(canvas, pad, pad + titleH + keysH);
  return new Promise((resolve) => out.toBlob(resolve, "image/png"));
}
