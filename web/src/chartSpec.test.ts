import { test } from "node:test";
import assert from "node:assert/strict";
import { formatTick, formatValue, parseChartSpec, tableRows } from "./chartSpec.ts";

const bar = {
  type: "bar",
  title: "Tiket selesai per tim",
  unit: "tiket",
  x: "Tim",
  labels: ["Tim A", "Tim B", "Tim C"],
  series: [{ name: "Selesai", data: [24, 37.5, null] }],
};

test("blok grafik yang benar dibaca lengkap", () => {
  const spec = parseChartSpec(JSON.stringify(bar));
  assert.ok(spec);
  assert.equal(spec.type, "bar");
  assert.deepEqual(spec.labels, ["Tim A", "Tim B", "Tim C"]);
  assert.deepEqual(spec.series[0].data, [24, 37.5, null]);
});

test("blok yang rusak atau tidak dikenal ditolak", () => {
  const bad: unknown[] = [
    "bukan json",
    "[1, 2]",
    { ...bar, type: "scatter" },
    { ...bar, labels: [] },
    { ...bar, labels: ["A", ""] },
    { ...bar, series: [{ name: "x", data: [1, 2] }] },
    { ...bar, series: [{ name: "x", data: [1, "2", 3] }] },
    { ...bar, series: [{ name: "x", data: [null, null, null] }] },
    { ...bar, series: Array.from({ length: 7 }, () => ({ name: "x", data: [1, 2, 3] })) },
    { ...bar, title: { teks: "x" } },
  ];
  for (const b of bad) assert.equal(parseChartSpec(typeof b === "string" ? b : JSON.stringify(b)), null, JSON.stringify(b));
  assert.equal(parseChartSpec('{"type":"bar","labels":["A"],"series":[{"name":"x","data":[1e999]}]}'), null);
});

test("pie hanya satu seri, paling banyak enam bagian, tanpa nilai negatif", () => {
  const pie = { type: "pie", title: "Porsi", labels: ["A", "B"], series: [{ name: "n", data: [3, 1] }] };
  assert.ok(parseChartSpec(JSON.stringify(pie)));
  assert.equal(parseChartSpec(JSON.stringify({ ...pie, series: [pie.series[0], pie.series[0]] })), null);
  assert.equal(parseChartSpec(JSON.stringify({ ...pie, series: [{ name: "n", data: [3, -1] }] })), null);
  const seven = { ...pie, labels: ["1", "2", "3", "4", "5", "6", "7"], series: [{ name: "n", data: [1, 1, 1, 1, 1, 1, 1] }] };
  assert.equal(parseChartSpec(JSON.stringify(seven)), null);
});

test("teks dirapikan dan dipotong, nama seri kosong memakai judul", () => {
  const spec = parseChartSpec(
    JSON.stringify({ type: "line", title: "  Reject\n2026 ", labels: [2025, 2026], series: [{ data: [1, 2] }] }),
  );
  assert.ok(spec);
  assert.equal(spec.title, "Reject 2026");
  assert.deepEqual(spec.labels, ["2025", "2026"]);
  assert.equal(spec.series[0].name, "Reject 2026");
  assert.equal(spec.unit, "");
});

test("angka memakai format Indonesia dan satuan", () => {
  assert.equal(formatValue(1234.5, "ton"), "1.234,5 ton");
  assert.equal(formatValue(12.5, "%"), "12,5%");
  assert.equal(formatValue(null, "ton"), "–");
  assert.equal(formatTick(1_200_000, ""), "1,2 jt");
  assert.equal(formatTick(450, "%"), "450%");
});

test("tabel berisi angka yang sama dengan grafik", () => {
  const spec = parseChartSpec(JSON.stringify(bar));
  assert.ok(spec);
  assert.deepEqual(tableRows(spec), {
    head: ["Tim", "Selesai"],
    rows: [
      ["Tim A", "24 tiket"],
      ["Tim B", "37,5 tiket"],
      ["Tim C", "–"],
    ],
  });
});
