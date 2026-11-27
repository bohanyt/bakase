import assert from "node:assert/strict";
import { test } from "node:test";
import type { MyFile } from "./api.ts";
import { fileDate, filesOf } from "./myFiles.ts";

const file = (id: string, upload: boolean): MyFile => ({
  id,
  name: `${id}.xlsx`,
  kind: "xlsx",
  size: 1,
  created_at: 0,
  conversation_id: "c",
  conversation_title: "Rekap",
  upload,
});

test("tab File Saya memisahkan buatan Bakase dan lampiran tanpa mengubah urutan", () => {
  const list = [file("a", false), file("b", true), file("c", false)];
  assert.deepEqual(filesOf(list, "hasil").map((f) => f.id), ["a", "c"]);
  assert.deepEqual(filesOf(list, "upload").map((f) => f.id), ["b"]);
});

test("tanggal file: hari ini dan kemarin dengan jam, selebihnya tanggal", () => {
  const now = new Date(2026, 10, 26, 15, 0);
  const at = (d: Date) => Math.floor(d.getTime() / 1000);
  assert.equal(fileDate(at(new Date(2026, 10, 26, 9, 5)), now), "hari ini 09.05");
  assert.equal(fileDate(at(new Date(2026, 10, 25, 16, 40)), now), "kemarin 16.40");
  assert.equal(fileDate(at(new Date(2026, 10, 20, 8, 0)), now), "20 Nov");
  assert.equal(fileDate(at(new Date(2025, 11, 3, 8, 0)), now), "3 Des 2025");
});
