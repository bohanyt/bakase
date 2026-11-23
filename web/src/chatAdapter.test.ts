/// <reference lib="dom" />

import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { workingMascotState } from "./mascotState.ts";

// pageBridge mendengarkan pesan dari extension saat modul dimuat.
Object.defineProperty(globalThis, "window", { value: new EventTarget(), configurable: true });
const { createChatAdapter } = await import("./chatAdapter.ts");

function event(name: string, payload: unknown) {
  return `event: ${name}\ndata: ${JSON.stringify(payload)}\n\n`;
}

function runChat(t: TestContext, events: string, abort = new AbortController()) {
  const onDone = t.mock.fn();
  const adapter = createChatAdapter({
    getConversationId: () => null,
    onConversation: () => {},
    onQuestion: () => {},
    onSummary: () => {},
    onDone,
  });
  t.mock.method(globalThis, "fetch", async () => new Response(events));
  return {
    stream: adapter.run({
      messages: [],
      abortSignal: abort.signal,
      runConfig: {},
      context: {},
      unstable_getMessage: () => assert.fail("Adapter tidak meminta pesan aktif"),
    }),
    onDone,
  };
}

test("status stream mengikuti teks setelah tool terakhir", async (t) => {
  const { stream } = runChat(t, [
    event("delta", { text: "Saya cek dulu." }),
    event("step", { id: "1", done: false }),
    event("step", { id: "2", done: false }),
    event("step", { id: "1", done: true }),
    event("step", { id: "2", done: true }),
    event("reasoning", { text: "Contoh pemikiran" }),
    event("delta", { text: "\n\n" }),
    event("delta", { text: "Ini hasilnya." }),
    event("done", { duration_ms: 100 }),
  ].join(""));
  const states = [];
  for await (const snapshot of stream) {
    states.push(workingMascotState(snapshot.content, snapshot.metadata.custom));
  }
  assert.deepEqual(states, ["bicara", "focus", "focus", "focus", "mikir", "mikir", "mikir", "bicara", "bicara"]);
});

test("stream tanpa done atau error gagal setelah menampilkan teks parsial", async (t) => {
  const { stream, onDone } = runChat(t, event("delta", { text: "Contoh jawaban" }));
  const partial = await stream.next();
  assert.equal(partial.done, false);
  assert.deepEqual(partial.value.content, [{ type: "text", text: "Contoh jawaban" }]);
  await assert.rejects(stream.next(), { message: "Jawaban terputus di tengah jalan. Kirim ulang pertanyaannya. Kalau terus terjadi, hubungi IT." });
  assert.equal(onDone.mock.callCount(), 1);
});

test("stream tanpa done tetap memakai error yang dikirim server", async (t) => {
  const { stream } = runChat(t, event("error", { message: "Coba lagi sebentar lagi." }));
  await assert.rejects(stream.next(), { message: "Coba lagi sebentar lagi." });
});

test("stream yang dibatalkan user tidak gagal walau tanpa done", async (t) => {
  const abort = new AbortController();
  const { stream, onDone } = runChat(t, event("delta", { text: "Contoh jawaban" }), abort);
  assert.equal((await stream.next()).done, false);
  abort.abort();
  assert.equal((await stream.next()).done, true);
  assert.equal(onDone.mock.callCount(), 1);
});
