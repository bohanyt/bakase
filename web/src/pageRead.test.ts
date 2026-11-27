import assert from "node:assert/strict";
import { test } from "node:test";
import { allowedOrigin, fileTitle, pageFiles, readProgress, readReply } from "./pageRead.ts";

const ID = "ponmlkjihgfedcbaponmlkjihgfedcba";

test("hanya extension yang terdaftar yang dilayani", () => {
  assert.ok(allowedOrigin(`chrome-extension://${ID}`, [ID]));
  assert.ok(allowedOrigin(`extension://${ID}`, [ID]));
  assert.ok(!allowedOrigin(`chrome-extension://${ID}`, []));
  assert.ok(!allowedOrigin(`chrome-extension://${"a".repeat(32)}`, [ID]));
  assert.ok(!allowedOrigin("http://other.example.test", [ID]));
  assert.ok(!allowedOrigin(`chrome-extension://${ID}/panel.html`, [ID]));
  assert.ok(!allowedOrigin(`moz-extension://${ID}`, [ID]));
});

test("balasan extension diperiksa bentuknya", () => {
  assert.equal(readReply(null, "1"), null);
  assert.equal(readReply({ type: "lain", id: "1" }, "1"), null);
  assert.equal(readReply({ type: "app:halaman", id: "2", page: { text: "x" } }, "1"), null);
  assert.deepEqual(readReply({ type: "app:halaman", id: "1", error: "Tidak bisa" }, "1"), { error: "Tidak bisa" });
  assert.deepEqual(readReply({ type: "app:halaman", id: "1", page: {} }, "1"), { error: "Balasan extension tidak terbaca." });
  const ok = readReply(
    { type: "app:halaman", id: "1", page: { title: "Daftar permintaan", url: "https://demo.example.test/daftar", text: "isi", image: "data:image/png;base64,AA==" } },
    "1",
  );
  // Hanya JPEG dari captureVisibleTab yang diterima sebagai screenshot.
  assert.deepEqual(ok, { page: { title: "Daftar permintaan", url: "https://demo.example.test/daftar", text: "isi", image: null } });
  const snapshotId = "a".repeat(8) + "-bbbb-cccc-dddd-" + "e".repeat(12);
  const fromPanel = (id: string, ambilTabel?: unknown) =>
    readReply({ type: "app:halaman", id: "1", page: { text: "isi", snapshotId: id, ambilTabel } }, "1");
  assert.deepEqual(fromPanel(snapshotId), { page: { title: "", url: "", text: "isi", image: null, snapshotId } });
  assert.deepEqual(fromPanel(snapshotId, true), { page: { title: "", url: "", text: "isi", image: null, snapshotId, ambilTabel: true } });
  assert.deepEqual(fromPanel("lain", true), { page: { title: "", url: "", text: "isi", image: null } });
});

test("kabar progres hanya untuk permintaan yang sama", () => {
  assert.equal(readProgress({ type: "app:progres", id: "1", text: " Membaca  tabel 1:\n120 baris… " }, "1"), "Membaca tabel 1: 120 baris…");
  assert.equal(readProgress({ type: "app:progres", id: "2", text: "x" }, "1"), null);
  assert.equal(readProgress({ type: "app:progres", id: "1", text: 5 }, "1"), null);
  assert.equal(readProgress({ type: "app:halaman", id: "1", text: "x" }, "1"), null);
  assert.equal(readProgress({ type: "app:progres", id: "1", text: "a".repeat(200) }, "1")?.length, 80);
});

test("judul jadi nama file yang aman", () => {
  assert.equal(fileTitle('PO: 00123 / "Draft"?'), "PO 00123 Draft");
  assert.equal(fileTitle("  \n\t "), "halaman");
  assert.equal(Array.from(fileTitle("ä".repeat(80))).length, 60);
});

test("halaman jadi lampiran teks dan screenshot", async () => {
  const now = new Date(2026, 10, 20, 9, 5);
  const jpeg = `data:image/jpeg;base64,${btoa("\xff\xd8\xff\xe0")}`;
  const [md, jpg, ...rest] = pageFiles({ title: "Daftar permintaan", url: "https://demo.example.test/daftar", text: "## Tabel\n| a |", image: jpeg }, now);
  assert.equal(rest.length, 0);
  assert.equal(md.name, "Halaman - Daftar permintaan.md");
  assert.equal(
    await md.text(),
    "# Daftar permintaan\n\nAlamat: https://demo.example.test/daftar\nDibaca: 2026-11-20 09:05\n\n## Tabel\n| a |\n",
  );
  assert.equal(jpg.name, "Layar - Daftar permintaan.jpg");
  assert.deepEqual(Array.from(new Uint8Array(await jpg.arrayBuffer())), [0xff, 0xd8, 0xff, 0xe0]);
  // Tanpa screenshot, hanya teksnya.
  assert.equal(pageFiles({ title: "", url: "", text: "isi", image: null }, now).length, 1);
});

test("lampiran dari panel lama hanya menulis origin dan path HTTP(S)", async () => {
  const now = new Date(2026, 10, 20);
  for (const [url, expected] of [
    ["https://penguji:sintetis@contoh.test:8443/po?contoh=nilai#fragmen", "https://contoh.test:8443/po"],
    ["http://contoh.test/daftar", "http://contoh.test/daftar"],
    ["file:///contoh.html", ""],
    ["alamat rusak", ""],
  ]) {
    const reply = readReply({ type: "app:halaman", id: "1", page: { text: "isi", url } }, "1");
    assert.ok(reply && "page" in reply);
    const [file] = pageFiles(reply.page, now);
    assert.equal((await file.text()).split("\n")[2], `Alamat: ${expected}`);
  }
});

test("file Excel asli dari Excel Online jadi satu lampiran, bukan teks layar", async () => {
  const bytes = new Uint8Array([0x50, 0x4b, 3, 4, 9]).buffer;
  const reply = readReply({ type: "app:halaman", id: "a", page: { title: "Delivery.xlsx", url: "https://contoh.example.com/x", text: "", image: null,
    file: { name: "Delivery.xlsx", bytes } } }, "a");
  assert.ok(reply && "page" in reply && reply.page.file);
  const files = pageFiles(reply.page, new Date(2026, 10, 26, 9, 5));
  assert.deepEqual(files.map((f) => f.name), ["Delivery.xlsx"]);
  assert.equal(files[0].size, 5);
  // Nama bukan Excel, isi kosong, atau terlalu besar tidak diterima sebagai file.
  for (const file of [{ name: "a.exe", bytes }, { name: "a.xlsx", bytes: new ArrayBuffer(0) },
    { name: "a.xlsx", bytes: new TextEncoder().encode("<html>").buffer }, { name: "a.xls", bytes },
    { name: "a.xlsx", bytes: new ArrayBuffer(25 * 1024 * 1024 + 1) }, { name: "a.xlsx", bytes: "bukan" }]) {
    const r = readReply({ type: "app:halaman", id: "a", page: { title: "x", url: "", text: "", image: null, file } }, "a");
    assert.ok(r && "page" in r && !r.page.file);
  }
});


test("fokus user dibersihkan dan ikut di lampiran teks halaman", async () => {
  const page = { title: "Laporan", url: "https://contoh.test/laporan", text: "- text: \"isi\"", image: null };
  const reply = readReply(
    { type: "app:halaman", id: "1", page: { ...page, focus: "tab aktif \"Ringkasan\";\n teks dipilih \"Total\"\u0007" } },
    "1",
  );
  assert.ok(reply && "page" in reply);
  assert.equal(reply.page.focus, "tab aktif \"Ringkasan\"; teks dipilih \"Total\"");
  const blank = readReply({ type: "app:halaman", id: "1", page: { ...page, focus: "  " } }, "1");
  assert.ok(blank && "page" in blank && blank.page.focus === undefined);
  const long = readReply({ type: "app:halaman", id: "1", page: { ...page, focus: "x".repeat(900) } }, "1");
  assert.ok(long && "page" in long);
  assert.equal(long.page.focus?.length, 500);

  const [md] = pageFiles(reply.page, new Date(2026, 10, 26, 17, 0));
  assert.match(await md.text(), /\nFokus user: tab aktif "Ringkasan"; teks dipilih "Total"\n/);
  const [plain] = pageFiles({ ...page }, new Date(2026, 10, 26, 17, 0));
  assert.doesNotMatch(await plain.text(), /Fokus user/);
});
