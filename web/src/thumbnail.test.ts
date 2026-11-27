import assert from "node:assert/strict";
import { test } from "node:test";
import { thumbnailSource, type SourceRef } from "./messages.ts";

const source = (n: number, kind: string, page: number | null, image?: SourceRef["image"]): SourceRef => ({
  n, chunk_id: `c${n}`, doc_id: n, title: `Uji ${n}`, source: "Uji", kind, page, image,
});

test("thumbnail memakai halaman PDF pertama yang dikutip", () => {
  const image = { key: "k", place: "hal. 1", similar: false };
  const sources = [source(1, "xlsx", null), source(2, "pdf", 4, image), source(3, "pdf", null), source(4, "pdf", 7), source(5, "pdf", 1)];
  assert.equal(thumbnailSource(sources)?.n, 4);
  assert.equal(thumbnailSource([source(1, "pptx", 2)]), undefined);
  assert.equal(thumbnailSource(undefined), undefined);
});
