import { strict as assert } from "node:assert";
import { test } from "node:test";
import type { SourceRef } from "./messages.ts";
import { scanPageLinks } from "./scanPageLinks.ts";

const source: SourceRef = {
  n: 1, chunk_id: "1-0-abc", doc_id: 1, title: "Contoh", source: "Uji", kind: "pdf", page: null,
};

test("catatan hanya untuk tulisan tangan di halaman yang dikutip atau scan ARCHIVE yang belum dicek", () => {
  assert.deepEqual(scanPageLinks([
    source,
    { ...source, scan: true },
    { ...source, scan: true, handwriting: [] },
    { ...source, scan: true, page: 1, handwriting: [2] },
  ]), []);
  assert.deepEqual(scanPageLinks([{ ...source, scan: true, handwriting: [2, 4] }]).map((link) => link.page), [2]);
  assert.deepEqual(scanPageLinks([{ ...source, scan: true, page: 4, handwriting: [2, 4] }]).map((link) => link.page), [4]);
  assert.deepEqual(scanPageLinks([{ ...source, scan: true, handwriting: "belum_dicek" }]).map((link) => link.page), [1]);
});

test("tombol dibatasi tiga dokumen dan mengutamakan halaman tulisan tangan yang diketahui", () => {
  const refs = [1, 2, 3, 4].map((doc_id): SourceRef => ({
    ...source, doc_id, scan: true, handwriting: "belum_dicek",
  }));
  refs.push({ ...refs[0], page: 3, handwriting: [3] });
  assert.deepEqual(
    scanPageLinks(refs).map(({ source, page, unchecked }) => [source.doc_id, page, unchecked]),
    [[1, 3, false], [2, 1, true], [3, 1, true]],
  );
});
