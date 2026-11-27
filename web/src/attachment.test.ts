import assert from "node:assert/strict";
import { test } from "node:test";
import { ACCEPT, checkFile, extensionOf, pastedName, totalError } from "./attachment.ts";

const MB = 1024 * 1024;

test("jenis dan ukuran mengikuti batas server", () => {
  assert.equal(checkFile("Layar.PNG", 2 * MB), null);
  assert.equal(checkFile("Rekap.xlsx", 25 * MB), null);
  assert.equal(checkFile("Rekap.xlsm", 25 * MB), null);
  assert.equal(checkFile("Rekap.xls", 25 * MB), null);
  assert.match(checkFile("Rekap.xls", 25 * MB + 1) ?? "", /25 MB/);
  assert.ok(ACCEPT.split(",").includes(".xls"));
  assert.ok(ACCEPT.split(",").includes(".xlsm"));
  assert.match(checkFile("program.exe", 10) ?? "", /belum bisa dilampirkan/);
  assert.match(checkFile("gambar.svg", 10) ?? "", /belum bisa dilampirkan/);
  assert.match(checkFile("tanpa-ekstensi", 10) ?? "", /belum bisa dilampirkan/);
  assert.match(checkFile("foto.jpg", 10 * MB + 1) ?? "", /10 MB/);
  assert.match(checkFile("SOP.pdf", 20 * MB + 1) ?? "", /20 MB/);
  assert.match(checkFile("data.csv", 25 * MB + 1) ?? "", /25 MB/);
  assert.match(checkFile("kosong.txt", 0) ?? "", /kosong/);
  assert.equal(extensionOf("Foto.Mesin.JPEG"), "jpeg");
  assert.ok(ACCEPT.includes(".pptx") && ACCEPT.includes("image/png"));
});

test("gambar dan PDF dalam satu pesan dibatasi totalnya", () => {
  const nine = { size: 9 * MB };
  assert.equal(totalError([{ name: "a.png", ...nine }, { name: "b.pdf", ...nine }]), null);
  assert.match(totalError([{ name: "a.png", ...nine }, { name: "b.pdf", ...nine }, { name: "c.jpg", ...nine }]) ?? "", /25 MB/);
  // Dokumen Office dibaca jadi teks di server, jadi tidak dihitung.
  assert.equal(totalError([{ name: "a.xlsx", size: 24 * MB }, { name: "b.png", size: 9 * MB }]), null);
});

test("tangkapan layar dari clipboard diberi nama yang jelas", () => {
  const now = new Date(2026, 10, 17, 9, 5, 3);
  assert.equal(pastedName("image.png", "image/png", now), "Tangkapan layar 2026-11-17 09.05.03.png");
  assert.equal(pastedName("", "image/jpeg", now), "Tangkapan layar 2026-11-17 09.05.03.jpg");
  assert.equal(pastedName("Foto mesin.png", "image/png", now), "Foto mesin.png");
  assert.equal(pastedName("Laporan.pdf", "application/pdf", now), "Laporan.pdf");
});
