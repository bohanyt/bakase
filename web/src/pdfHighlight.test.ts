import assert from "node:assert/strict";
import { test } from "node:test";
import { findPdfHighlight } from "./pdfHighlight.ts";

test("matches across PDF text runs despite spaces, accents, ligatures, and line breaks", () => {
  assert.deepEqual(findPdfHighlight(["🙂 Lot A – ", "kuali-", "tas oﬃce.", "Lanjut"], "LOT A: kualitas oﬃcé"), {
    start: { item: 0, offset: 3 },
    end: { item: 2, offset: 8 },
  });
});

test("does not mark absent text, partial words, blanks, or scans", () => {
  assert.equal(findPdfHighlight(["Kelas utama"], "utama lain"), null);
  assert.equal(findPdfHighlight(["Kelas utama"], "utama a"), null);
  assert.equal(findPdfHighlight(["Kelas utama"], "tama"), null);
  assert.equal(findPdfHighlight(["Kelas utama"], "— \n"), null);
  assert.equal(findPdfHighlight([], "Kelas utama"), null);
});

test("keeps the signs and separators that distinguish measurements", () => {
  for (const [text, wrong] of [
    ["Tebal 1,2 mm", "Tebal 12 mm"],
    ["Tebal 1.2 mm", "Tebal 12 mm"],
    ["Massa 1.200 kg", "Massa 1200 kg"],
    ["Massa 1 200 kg", "Massa 1200 kg"],
    ["Suhu −4 derajat", "Suhu 4 derajat"],
    ["Kadar 8%", "Kadar 8"],
    ["Toleransi ±5 mm", "Toleransi 5 mm"],
    ["Tebal 1/2 mm", "Tebal 12 mm"],
    ["1,2 mm", "2 mm"],
    ["1 200 kg", "200 kg"],
  ]) {
    assert.equal(findPdfHighlight([text], wrong), null, `${text} / ${wrong}`);
    assert.equal(findPdfHighlight([wrong], text), null, `${wrong} / ${text}`);
  }
  assert.ok(findPdfHighlight(["Suhu ", "−", " 4."], "Suhu -4"));
  assert.ok(findPdfHighlight(["Tebal 1", " , ", "2 mm"], "Tebal 1,2 mm"));
  assert.ok(findPdfHighlight(["Toleransi ± 5 %"], "Toleransi ±5%"));
});

test("matches Unicode fractions and fraction slashes without joining their digits", () => {
  for (const [unicode, ascii] of [["½", "1/2"], ["⅝", "5/8"], ["1⁄2", "1/2"]]) {
    assert.deepEqual(findPdfHighlight([`${unicode} mm`], `${ascii} mm`)?.end, { item: 0, offset: unicode.length + 3 });
    assert.ok(findPdfHighlight([`${ascii} mm`], `${unicode} mm`));
    assert.equal(findPdfHighlight([`${unicode} mm`], `${ascii.replace("/", "")} mm`), null);
    assert.equal(findPdfHighlight([`${ascii.replace("/", "")} mm`], `${unicode} mm`), null);
  }
});

test("matches hyphen and minus variants in both directions", () => {
  for (const dash of "‐‑‒–—―−﹣－") {
    assert.ok(findPdfHighlight([`ISO${dash}9001`], "ISO-9001"));
    assert.ok(findPdfHighlight(["ISO-9001"], `ISO${dash}9001`));
  }
});

test("preserves mixed numbers and distinguishes dash punctuation from minus", () => {
  for (const [text, expected] of [["1½ mm", "1 1/2 mm"], ["-4 mm", "−4 mm"], ["Tebal — 4 mm", "Tebal 4 mm"]]) {
    assert.ok(findPdfHighlight([text], expected));
    assert.ok(findPdfHighlight([expected], text));
  }
  for (const [text, wrong] of [["1½ mm", "11/2 mm"], ["Tebal — 4 mm", "Tebal −4 mm"], ["Tebal– 4 mm", "Tebal −4 mm"]]) {
    assert.equal(findPdfHighlight([text], wrong), null);
    assert.equal(findPdfHighlight([wrong], text), null);
  }
});
