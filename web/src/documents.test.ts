import assert from "node:assert/strict";
import { test } from "node:test";
import { sourcePreviewPage, sourcePreviewText, unsearchableSummary } from "./documents.ts";

test("sitasi memakai halaman hasil lookup terbaru, lalu nomor yang tersimpan", () => {
  assert.equal(sourcePreviewPage(8, 5), 5);
  assert.equal(sourcePreviewPage(8), 8);
  assert.equal(sourcePreviewPage(8, null), 8);
  assert.equal(sourcePreviewPage(null, 5), 5);
  assert.equal(sourcePreviewPage(null, null), null);
});

test("preview hanya menyembunyikan nama sheet dari teks spreadsheet sumber", () => {
  const input = "## Cek Visual\n\n## Cek Visual › Pemeriksaan › Agustus\n| Lot | Kondisi |\n| --- | --- |\n| 01 | Baik |\n\n\\## Isi sel";
  const expected = input.replace("## Cek Visual\n", "\n").replace("## Cek Visual › ", "## ");
  assert.equal(sourcePreviewText(input, "xlsx", "ok", 1), expected);
  assert.equal(sourcePreviewText(input, "xlsx", "ok"), expected);
  assert.equal(sourcePreviewText(input, "xlsx", "ok", 2), input);
  assert.equal(sourcePreviewText(input, "xlsx", "archive"), input);
  assert.equal(sourcePreviewText(input, "docx", "ok"), input);
});

test("ringkasan menggabungkan status menunggu dan semua status yang tidak bisa dibaca", () => {
  const counts = { ok: 40, vision: 12, scan: 160, gambar: 3, gagal: 1, tidak_didukung: 2, terlalu_besar: 1, kosong: 2 };
  assert.equal(unsearchableSummary(counts), "169 file belum bisa dicari Bakase: 163 menunggu dibaca, 6 tidak bisa dibaca.");
});

test("ringkasan melewati kelompok kosong dan hanya tampil jika ada file yang belum bisa dicari", () => {
  assert.equal(unsearchableSummary({ ok: 4, vision: 1 }), null);
  assert.equal(unsearchableSummary({ gambar: 2 }), "2 file belum bisa dicari Bakase: 2 menunggu dibaca.");
  assert.equal(unsearchableSummary({ kosong: 1 }), "1 file belum bisa dicari Bakase: 1 tidak bisa dibaca.");
});
