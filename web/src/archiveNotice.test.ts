import { readFileSync } from "node:fs";
import { strict as assert } from "node:assert";
import { test } from "node:test";

test("the notice and archive opt-in ship together", () => {
  const guest = readFileSync(new URL("./GuestAsk.tsx", import.meta.url), "utf8");
  const sidebar = readFileSync(new URL("./Sidebar.tsx", import.meta.url), "utf8");
  const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
  assert.ok(guest.includes("Tanpa login, Bakase tidak membuka dokumen kantor. Pertanyaan dan jawaban disimpan di arsip IT."));
  assert.ok(guest.includes('fetch("/api/guest/chat/archive-v1"'));
  assert.ok(sidebar.includes('Hapus percakapan "${c.title}" dari daftar? Salinannya tetap disimpan di arsip IT.'));
  assert.ok(app.includes('/api/conversations/${id}/archive-v1'));
});
