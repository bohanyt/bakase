/// <reference lib="dom" />

import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { workingMascotState } from "./mascotState.ts";
import { reloadThreadMessages } from "./messages.ts";

// pageBridge mendengarkan pesan dari extension saat modul dimuat.
Object.defineProperty(globalThis, "window", { value: new EventTarget(), configurable: true });
const { createChatAdapter } = await import("./chatAdapter.ts");

function event(name: string, payload: unknown) {
  return `event: ${name}\ndata: ${JSON.stringify(payload)}\n\n`;
}

function chatRunner(t: TestContext, abort = new AbortController()) {
  const onDone = t.mock.fn();
  const adapter = createChatAdapter({
    getConversationId: () => null,
    onConversation: () => {},
    onQuestion: () => {},
    onSummary: () => {},
    onDone,
  });
  const run = () => adapter.run({
    messages: [],
    abortSignal: abort.signal,
    runConfig: {},
    context: {},
    unstable_getMessage: () => assert.fail("Adapter tidak meminta pesan aktif"),
  });
  return { run, onDone };
}

function runChat(t: TestContext, events: string, abort = new AbortController()) {
  t.mock.method(globalThis, "fetch", async () => new Response(events));
  const { run, onDone } = chatRunner(t, abort);
  return { stream: run(), onDone };
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

test("kiriman ulang memakai ID yang sama, kiriman berikutnya menyebut yang lama", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const bodies: { turn_id: string; previous_turn_id?: string }[] = [];
  const { run } = chatRunner(t);
  t.mock.method(globalThis, "fetch", async (_: string, init: RequestInit) => {
    bodies.push(JSON.parse(String(init.body)));
    if (bodies.length === 1) {
      queueMicrotask(() => setImmediate(() => t.mock.timers.tick(1_000)));
      throw new TypeError("Failed to fetch");
    }
    return new Response(event("done", { duration_ms: 1 }));
  });
  for await (const _ of run()) { /* habiskan */ }
  for await (const _ of run()) { /* habiskan */ }
  assert.match(bodies[0].turn_id, /^[0-9a-f]{32}$/);
  assert.equal(bodies[1].turn_id, bodies[0].turn_id);
  assert.equal(bodies[1].previous_turn_id, undefined);
  assert.notEqual(bodies[2].turn_id, bodies[0].turn_id);
  assert.equal(bodies[2].previous_turn_id, bodies[0].turn_id);
});

test("jaringan yang putus dijelaskan dengan kalimat biasa", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { stream } = runChat(t, "");
  let calls = 0;
  t.mock.method(globalThis, "fetch", async () => {
    calls += 1;
    if (calls === 1) setImmediate(() => t.mock.timers.tick(1_000));
    throw new TypeError("Failed to fetch");
  });
  await assert.rejects(stream.next(), { message: "Bakase tidak bisa dihubungi. Cek jaringan, lalu kirim ulang." });

  const cut = runChat(t, "");
  let sent = false;
  t.mock.method(globalThis, "fetch", async () => new Response(new ReadableStream({
    pull(controller) {
      if (sent) controller.error(new TypeError("network error"));
      else controller.enqueue(new TextEncoder().encode(event("delta", { text: "Contoh" })));
      sent = true;
    },
  })));
  assert.equal((await cut.stream.next()).done, false);
  await assert.rejects(cut.stream.next(), { message: /^Jawaban terputus di tengah jalan/ });
  assert.equal(cut.onDone.mock.callCount(), 1);
});

test("sambungan yang diam dianggap putus", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { stream, onDone } = runChat(t, "");
  t.mock.method(globalThis, "fetch", async () => new Response(new ReadableStream({
    start(controller) {
      controller.enqueue(new TextEncoder().encode(event("delta", { text: "Contoh" })));
    },
  })));
  assert.equal((await stream.next()).done, false);
  const next = assert.rejects(stream.next(), { message: /^Jawaban terputus di tengah jalan/ });
  await Promise.resolve();
  t.mock.timers.tick(60_000);
  await next;
  assert.equal(onDone.mock.callCount(), 1);
});

test("muat ulang mengganti jawaban lokal yang gagal dengan isi server", () => {
  const message = (id: string, role: "user" | "assistant") => ({ id, role, metadata: { custom: {} } }) as never;
  const stored = (id: number, role: "user" | "assistant", partial?: boolean) => ({
    id, role, content: `isi ${id}`, created_at: 0, duration_ms: null, feedback: null, files: [], reasoning: null, steps: [], partial,
  });
  const local = [message("tanya", "user"), message("gagal", "assistant")];
  const questions = new Map([["tanya", 1]]);
  assert.equal(reloadThreadMessages([stored(1, "user")], local, questions), null);
  const merged = reloadThreadMessages([stored(1, "user"), stored(2, "assistant", true)], local, questions);
  assert.deepEqual(merged?.map((m) => m.id), ["tanya", "2"]);
  assert.deepEqual(merged?.map((m) => (m.metadata?.custom as { partial?: boolean } | undefined)?.partial), [undefined, true]);
});

test("stream yang dibatalkan user tidak gagal walau tanpa done", async (t) => {
  const abort = new AbortController();
  const { stream, onDone } = runChat(t, event("delta", { text: "Contoh jawaban" }), abort);
  assert.equal((await stream.next()).done, false);
  abort.abort();
  assert.equal((await stream.next()).done, true);
  assert.equal(onDone.mock.callCount(), 1);
});
