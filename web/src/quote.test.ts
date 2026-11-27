import assert from "node:assert/strict";
import { test } from "node:test";
import {
  MAX_MESSAGE_CHARS,
  cleanQuoteText,
  columnName,
  edgeScrollStep,
  formatMessage,
  lengthError,
  markdownTable,
  messageLength,
  parseMessage,
  parseTable,
  quoteError,
  rangeA1,
  sheetRef,
} from "./quote.ts";

test("kutipan ditaruh sebelum pertanyaan, dengan sumber dan komentar", () => {
  const message = formatMessage(
    [
      { text: "Paket A berisi 12 barang.\n\nPakai kemasan ukuran sedang.", source: "Jawaban Bakase", comment: "yang ini" },
      { text: "Periksa segel kemasan", source: "Panduan Pemeriksaan.pdf · hal. 3" },
    ],
    "  Kalau paket B berapa?  ",
  );
  assert.equal(
    message,
    [
      ":::kutipan",
      "> Paket A berisi 12 barang.",
      ">",
      "> Pakai kemasan ukuran sedang.",
      "— Jawaban Bakase",
      "Komentar: yang ini",
      "",
      "> Periksa segel kemasan",
      "— Panduan Pemeriksaan.pdf · hal. 3",
      ":::akhir-kutipan",
      "",
      "Kalau paket B berapa?",
    ].join("\n"),
  );
});

test("komentar dan sumber selalu satu baris, kutipan kosong dilewati", () => {
  const message = formatMessage(
    [
      { text: "a", source: "x\ny", comment: "satu\ndua  tiga" },
      { text: "  \n " },
    ],
    "",
  );
  assert.equal(message, ":::kutipan\n> a\n— x y\nKomentar: satu dua tiga\n:::akhir-kutipan");
});

test("pesan yang diformat bisa dibaca balik untuk ditampilkan", () => {
  const quotes = [
    { text: "baris 1\n\n> bukan sumber\n— juga bukan", source: "Rekap.xlsx · Data!B2:C3", comment: "cek" },
    { text: "tanpa sumber" },
  ];
  const parsed = parseMessage(formatMessage(quotes, "Pertanyaan\nbaris dua"));
  assert.deepEqual(parsed, {
    quotes: [
      { text: "baris 1\n\n> bukan sumber\n— juga bukan", source: "Rekap.xlsx · Data!B2:C3", comment: "cek" },
      { text: "tanpa sumber" },
    ],
    question: "Pertanyaan\nbaris dua",
  });
});

test("pesan biasa tidak dianggap kutipan", () => {
  assert.deepEqual(parseMessage("Halo\n> bukan di awal"), { quotes: [], question: "Halo\n> bukan di awal" });
  assert.deepEqual(parseMessage(">tanpa spasi"), { quotes: [], question: ">tanpa spasi" });
});

test("pertanyaan mirip kutipan tidak mendapat atribusi dari teksnya sendiri", () => {
  const quotes = [{ text: "Batas ukuran", source: "Jawaban Bakase", comment: "> cek — Review Komentar: batas" }];
  const question = "> 10 mm\n— Review\nKomentar: batas bawah";
  assert.deepEqual(parseMessage(formatMessage(quotes, question)), { quotes, question });
  assert.deepEqual(parseMessage(question), { quotes: [], question });
});

test("awalan metadata dan isi pertanyaan dipertahankan dengan atau tanpa chip", () => {
  for (const question of [">20 mm boleh?", "> 10 mm", "— Review\nbaris dua", "Komentar: batas bawah", "a\r\n\r\n> b  \r\nc"]) {
    for (const quotes of [[], [{ text: "kutipan" }]]) {
      assert.deepEqual(parseMessage(formatMessage(quotes, question)), { quotes, question });
    }
  }
  const plain = "  > 10 mm\n— Review\nKomentar: batas bawah  ";
  assert.deepEqual(parseMessage(plain), { quotes: [], question: plain });
});

test("penanda yang diketik sendiri dan backslash tetap teks biasa", () => {
  const frame = ":::kutipan\n> palsu\n— Review\n:::akhir-kutipan";
  for (const question of [":::kutipan", frame, `\\${frame}`, `\\\\${frame}`]) {
    assert.deepEqual(parseMessage(formatMessage([], question)), { quotes: [], question });
    const quotes = [{ text: "asli" }];
    assert.deepEqual(parseMessage(formatMessage(quotes, question)), { quotes, question });
  }
  const quotes = [{ text: frame, source: ":::akhir-kutipan", comment: ":::akhir-kutipan\n> palsu" }];
  assert.deepEqual(parseMessage(formatMessage(quotes, "")), {
    quotes: [{ ...quotes[0], comment: ":::akhir-kutipan > palsu" }], question: "",
  });
});

test("bingkai tidak lengkap atau rusak ditampilkan utuh", () => {
  for (const question of [
    ":::kutipan\n> tanpa akhir",
    ":::kutipan\n:::akhir-kutipan",
    ":::kutipan\n> isi\nbaris liar\n:::akhir-kutipan",
    ":::kutipan\n> isi\n:::akhir-kutipan\nlangsung teks",
  ]) assert.deepEqual(parseMessage(question), { quotes: [], question });
});

test("teks pilihan dirapikan", () => {
  assert.equal(cleanQuoteText("  a  \r\nb \n\n\n\nc \n"), "a\nb\n\nc");
});

test("panjang dihitung per karakter seperti server", () => {
  assert.equal(messageLength("  😀ab  "), 3);
  assert.equal(lengthError("x".repeat(MAX_MESSAGE_CHARS), true), null);
  assert.equal(
    lengthError("x".repeat(MAX_MESSAGE_CHARS + 1), true),
    "Pesan dan kutipan terlalu panjang: 20.001 dari maksimal 20.000 karakter. Hapus sebagian kutipan atau persingkat pesannya.",
  );
  assert.match(lengthError("x".repeat(MAX_MESSAGE_CHARS + 1), false) ?? "", /^Pesan terlalu panjang/);
});

test("satu kutipan yang terlalu panjang ditolak saat dikutip", () => {
  assert.equal(quoteError({ text: "pendek" }), null);
  assert.equal(quoteError({ text: " \n " }), "Tidak ada teks yang bisa dikutip.");
  assert.match(quoteError({ text: "x".repeat(MAX_MESSAGE_CHARS) }) ?? "", /^Kutipan terlalu panjang/);
  const overhead = messageLength(formatMessage([{ text: "x" }], "")) - 1;
  const draft = { text: "x".repeat(MAX_MESSAGE_CHARS - overhead) };
  assert.equal(quoteError(draft), null);
  assert.equal(messageLength(formatMessage([draft], "")), MAX_MESSAGE_CHARS);
  assert.notEqual(quoteError({ text: draft.text + "x" }), null);
  assert.notEqual(lengthError(formatMessage([draft], "a"), true), null);
  assert.notEqual(lengthError(formatMessage([], ":::kutipan\n" + "x".repeat(MAX_MESSAGE_CHARS - 11)), false), null);
});

test("nomor kolom jadi huruf Excel", () => {
  assert.deepEqual([0, 1, 25, 26, 27, 51, 52, 701, 702].map(columnName), [
    "A",
    "B",
    "Z",
    "AA",
    "AB",
    "AZ",
    "BA",
    "ZZ",
    "AAA",
  ]);
});

test("nama sheet diberi kutip kalau perlu", () => {
  assert.equal(sheetRef("Data"), "Data");
  assert.equal(sheetRef("Data_2024"), "Data_2024");
  assert.equal(sheetRef("Rekap Juni"), "'Rekap Juni'");
  assert.equal(sheetRef("O'Neil"), "'O''Neil'");
  assert.equal(sheetRef("A1"), "'A1'");
  assert.equal(sheetRef("R1C1"), "'R1C1'");
  assert.equal(sheetRef("2024"), "'2024'");
});

test("rentang jadi alamat A1, ke arah mana pun dipilih", () => {
  assert.equal(rangeA1("Data", { row: 2, col: 1 }, { row: 5, col: 3 }), "Data!B2:D5");
  assert.equal(rangeA1("Data", { row: 5, col: 3 }, { row: 2, col: 1 }), "Data!B2:D5");
  assert.equal(rangeA1("Data", { row: 7, col: 0 }, { row: 7, col: 0 }), "Data!A7");
  assert.equal(rangeA1("Rekap Juni", { row: 1, col: 26 }, { row: 3, col: 27 }), "'Rekap Juni'!AA1:AB3");
  assert.equal(rangeA1(null, { row: 2, col: 1 }, { row: 5, col: 3 }), "B2:D5");
});

test("tabel markdown kecil bisa dibaca balik", () => {
  const table = markdownTable(["Item", "Harga | Rp"], [["Widget\n2 inch", "Rp 1.000"], ["", "5"]]);
  assert.equal(table, "| Item | Harga \\| Rp |\n| --- | --- |\n| Widget 2 inch | Rp 1.000 |\n|  | 5 |");
  assert.deepEqual(parseTable(table), {
    header: ["Item", "Harga | Rp"],
    rows: [
      ["Widget 2 inch", "Rp 1.000"],
      ["", "5"],
    ],
  });
  assert.equal(parseTable("| bukan tabel |"), null);
  assert.equal(parseTable("teks biasa\n| --- |"), null);
});

test("tabel dengan lebar header, pemisah, atau baris berbeda tetap teks literal", () => {
  for (const text of [
    "| H |\n| --- |\n| a | hidden |",
    "| H | H2 |\n| --- | --- |\n| a |",
    "| H |\n| --- | --- |\n| a |",
    "| H | H2 |\n| --- |\n| a | b |",
  ]) {
    assert.equal(parseTable(text), null);
    assert.equal(parseMessage(formatMessage([{ text }], "")).quotes[0].text, text);
  }
});

test("HTML, gambar markdown, dan tautan dipertahankan sebagai teks", () => {
  const text = '<img src=x onerror="alert(1)">\n![gambar](https://example.invalid/a.png)\n[klik](javascript:alert(1))';
  const quote = { text, source: "<script>alert(1)</script>", comment: "[klik](https://example.invalid)" };
  assert.deepEqual(parseMessage(formatMessage([quote], text)), { quotes: [quote], question: text });
  assert.equal(parseTable(text), null);
  assert.deepEqual(parseTable(markdownTable(["HTML"], [[text]])), {
    header: ["HTML"], rows: [[text.replace(/\n/g, " ")]],
  });
});

test("U+0085 di awal dipangkas seperti di server, jadi penanda yang diketik tetap di-escape", () => {
  const typed = "\u0085:::kutipan\n> 10 mm\n— Review\n:::akhir-kutipan\n\nbenar?";
  const message = formatMessage([], typed);
  assert.ok(message.startsWith("\\:::kutipan"), message);
  assert.equal(parseMessage(message).quotes.length, 0);
});

test("U+FEFF tidak dipangkas, sama seperti str::trim di server", () => {
  const typed = "\uFEFF:::kutipan\n> 10 mm\n:::akhir-kutipan\n\nbenar?";
  const message = formatMessage([], typed);
  assert.ok(message.startsWith("\uFEFF:::kutipan"), message);
  assert.equal(parseMessage(message).quotes.length, 0);
});

test("edgeScrollStep: arah ke tepi terdekat, makin jauh lewat tepi makin cepat", () => {
  // Area sel terlihat 100..500 piksel.
  assert.equal(edgeScrollStep(300, 100, 500), 0);
  assert.equal(edgeScrollStep(476, 100, 500), 0);
  assert.ok(edgeScrollStep(490, 100, 500) > 0);
  assert.ok(edgeScrollStep(560, 100, 500) > edgeScrollStep(500, 100, 500));
  assert.equal(edgeScrollStep(5000, 100, 500), 24);
  assert.ok(edgeScrollStep(110, 100, 500) < 0);
  assert.equal(edgeScrollStep(-5000, 100, 500), -24);
  // Area sempit: zona tepi mengecil supaya bagian tengah tetap diam.
  assert.equal(edgeScrollStep(130, 100, 160), 0);
});
