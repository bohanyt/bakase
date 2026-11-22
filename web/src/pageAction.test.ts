import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { limitPageText, pageActionTiming, readActionReply, readPageAction } from "./pageAction.ts";

const id = "a".repeat(32);
const remaining_ms = 7500;

test("kontrak JSON aksi dan balasan sama dengan server", () => {
  const fixture = JSON.parse(readFileSync(new URL("../../server/tests/fixtures/page-action.json", import.meta.url), "utf8"));
  for (const action of fixture.commands) assert.deepEqual(readPageAction(action), action);
  for (const reply of fixture.replies) {
    assert.deepEqual(readActionReply({ type: "app:hasil-aksi", id, ...reply }, id), reply);
  }
});

test("hanya aksi dan ref halaman yang sesuai bentuk pesan", () => {
  const highlight = { id, remaining_ms, action: "sorot", ref: "f14e1000001" };
  const scroll = { id, remaining_ms, action: "gulir", ref: null, direction: "down" };
  assert.deepEqual(readPageAction(highlight), highlight);
  assert.deepEqual(readPageAction(scroll), scroll);
  assert.equal(readPageAction({ ...scroll, direction: "to" }), null);
  assert.equal(readPageAction({ ...scroll, ref: "e0", direction: "up" }), null);
  assert.equal(readPageAction({ ...highlight, ref: "input" }), null);
  assert.equal(readPageAction({ ...highlight, action: "klik" }), null);
  assert.equal(readPageAction({ ...highlight, id: "lain" }), null);
  for (const remaining of [undefined, "7500", Infinity, -1]) {
    assert.equal(readPageAction({ ...highlight, remaining_ms: remaining }), null);
  }
});

test("sisa waktu aksi sama saat jam laptop berbeda 15 detik dari server", (t) => {
  const serverTime = 1795222800000;
  const clock = { timeOrigin: serverTime - 200, now: () => 200 };
  for (const skew of [-15000, 15000]) {
    const date = t.mock.method(Date, "now", () => serverTime + skew);
    assert.deepEqual(pageActionTiming(7500, clock), { timeout: 7500, deadline: 7700 });
    assert.equal(pageActionTiming(12000, clock).timeout, 8000);
    assert.equal(pageActionTiming(0, clock).timeout, 0);
    date.mock.restore();
  }
});

test("balasan hanya untuk ID aksi yang sama dan isi gulir dibatasi dalam byte", () => {
  const long = "baris 1\n" + "日本語".repeat(4_000);
  const reply = { id, type: "app:hasil-aksi", status: "scrolled", moved: false, text: long };
  assert.equal(readActionReply(reply, "lain"), null);
  assert.deepEqual(readActionReply({ ...reply, moved: "ya" }, id), { status: "failed" });
  assert.deepEqual(readActionReply(reply, id), { status: "scrolled", moved: false, text: limitPageText(long) });
  assert.match(limitPageText(long), /^baris 1\n\[Terpotong/);
  const single = limitPageText("😀".repeat(7_000));
  assert.ok(Buffer.byteLength(single) <= 24_000);
  assert.equal(limitPageText("teks utuh"), "teks utuh");
});
