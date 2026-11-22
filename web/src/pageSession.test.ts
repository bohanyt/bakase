import assert from "node:assert/strict";
import { test } from "node:test";
import type { ActionReply, PageAction } from "./pageAction.ts";
import { createPageSession } from "./pageSession.ts";

const origin = "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const attachments = new Set(["lampiran"]);
const action = { id: "b".repeat(32), remaining_ms: 7500, action: "gulir", ref: null, direction: "down" } satisfies PageAction;

function setup() {
  const session = createPageSession();
  session.remember("halaman-A", origin);
  session.bind("halaman-A", "lampiran");
  const page = session.live(attachments);
  assert.ok(page);
  return { session, page };
}

test("abort sebelum atau selama aksi melewati POST", async () => {
  const { session, page } = setup();
  const controller = new AbortController();
  const alreadyAborted = AbortSignal.abort();
  await session.answer(page, action, alreadyAborted, async () => assert.fail("aksi dimulai"), async () => assert.fail("POST terkirim"));
  await session.answer(page, action, controller.signal, async () => {
    controller.abort();
    return { status: "scrolled", text: "baris 21", moved: true };
  }, async () => assert.fail("POST terkirim"));
  assert.equal(session.live(attachments), null);
});

test("kegagalan melepas halaman aktif, ref lama saja tidak", async () => {
  for (const status of ["failed", "timeout", "unavailable", "stale_ref"] as const) {
    const { session, page } = setup();
    const posted: ActionReply[] = [];
    await session.answer(page, action, new AbortController().signal, async () => ({ status }), async (_, reply) => {
      posted.push(reply);
    });
    assert.deepEqual(posted, [{ status }]);
    assert.equal(session.live(attachments)?.id ?? null, status === "stale_ref" ? page.id : null);
  }
  const { session, page } = setup();
  await assert.rejects(session.answer(page, action, new AbortController().signal, async () => (
    { status: "scrolled", text: "baris 21", moved: true }
  ), async () => { throw new Error("Koneksi putus"); }), /Koneksi putus/);
  assert.equal(session.live(attachments), null);
});

test("perubahan halaman hanya berlaku untuk origin dan halaman aktif yang cocok", async () => {
  const { session, page } = setup();
  const change = { type: "app:halaman-berubah", pageId: page.id };
  session.changed(origin, { ...change, pageId: "halaman-B" });
  session.changed("https://contoh.test", change);
  assert.equal(session.live(attachments)?.id, page.id);
  const posted: ActionReply[] = [];
  await session.answer(page, action, new AbortController().signal, async () => {
    session.changed(origin, change);
    return { status: "scrolled", text: "baris 21", moved: true };
  }, async (_, reply) => { posted.push(reply); });
  assert.equal(session.live(attachments), null);
  assert.deepEqual(posted, [{ status: "unavailable" }]);
});

test("hasil gulir hanya dibawa sekali ke pesan berikutnya", async () => {
  const { session, page } = setup();
  assert.equal(page.refresh, undefined);
  await session.answer(page, action, new AbortController().signal, async () => (
    { status: "scrolled", text: "baris 21", moved: true }
  ), async () => {});
  const followUp = session.live(attachments);
  assert.equal(followUp?.refresh?.text, "baris 21");
  session.consumed(page);
  assert.equal(session.live(attachments)?.refresh?.text, "baris 21");
  assert.ok(followUp);
  session.consumed(followUp);
  assert.equal(session.live(attachments)?.refresh, undefined);
});
