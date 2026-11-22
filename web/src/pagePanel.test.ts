import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

type Message = Record<string, unknown>;
type Sender = { tab: { id: number }; documentId: string };
type Injection = {
  target: { tabId: number; allFrames?: boolean; frameIds?: number[]; documentIds?: string[] };
  files?: string[];
  func?: (...args: unknown[]) => unknown;
  args?: unknown[];
};

const panelSource = readFileSync(new URL("../../extension/panel.js", import.meta.url), "utf8");
const readerSource = readFileSync(new URL("../../extension/src/snapshot.js", import.meta.url), "utf8")
  .replace(/^import .*;$/m, "").replaceAll("export function", "function").replaceAll("export async function", "async function");
const hub = "http://127.0.0.1:8100";
const topAddress = "https://contoh.test/daftar?contoh=atas#sementara";
const frameAddress = "https://contoh.test/grid?contoh=bawah#sementara";

function event<Args extends unknown[]>() {
  const listeners = new Set<(...args: Args) => void>();
  return {
    addListener: (listener: (...args: Args) => void) => listeners.add(listener),
    removeListener: (listener: (...args: Args) => void) => listeners.delete(listener),
    emit: (...args: Args) => { for (const listener of listeners) listener(...args); },
    size: () => listeners.size,
  };
}

function gate() {
  let release!: () => void;
  const promise = new Promise<void>((resolve) => { release = resolve; });
  return { promise, release };
}

function fixture(frameOriginOffset = 0) {
  const onActivated = event<[{ windowId: number; tabId: number }]>();
  const onUpdated = event<[number, { url?: string; status?: string }]>();
  const onRemoved = event<[number]>();
  const onReplaced = event<[number, number]>();
  const onMessage = event<[Message, Sender, (response: boolean) => void]>();
  const fromChat = event<[{ origin: string; source: unknown; data: Message }]>();
  const toChat = event<[Message]>();
  const parent = { postMessage: (message: Message) => toChat.emit(message) };
  const tabId = 7;
  let activeId = tabId;
  let beforeFiles: (() => void) | undefined;
  let beforeAction: (() => Promise<void>) | undefined;
  let beforePermitReply: (() => Promise<void>) | undefined;
  let actions = 0;

  function frame(frameId: number, documentId: string, href: string) {
    const location = { href };
    const navigation = new EventTarget();
    const window = new EventTarget();
    const scroller = {
      scrollTop: 0, scrollLeft: 0, clientHeight: 300, scrollHeight: 3000,
      scrollBy({ top }: { top: number }) { this.scrollTop += top; },
      isConnected: true, getClientRects: () => [{}], closest: () => null, getAttribute: () => null,
      getRootNode: () => null, matches: () => false,
    };
    const context = createContext({
      location, navigation, window,
      performance: {
        timeOrigin: performance.timeOrigin + frameOriginOffset,
        now: () => performance.now() + 20_000,
      },
      document: { visibilityState: "visible", scrollingElement: scroller },
      getComputedStyle: () => ({ visibility: "visible", overflowY: "auto" }),
      innerHeight: 600, scrollX: 0, scrollY: 0,
      chrome: { runtime: { sendMessage: async (message: Message) => {
        let response = false;
        onMessage.emit(message, { tab: { id: tabId }, documentId }, (value) => { response = value; });
        if (response && message.type === "app:izin-aksi") await beforePermitReply?.();
        return response;
      } } },
      scroller,
    });
    runInContext(readerSource, context);
    runInContext(`
      globalThis.AppSnapshot = { act, watchFrame, unwatchFrame, readPage(prefix, base) {
        const ref = prefix + "e" + (base + 1);
        globalThis.__appRefs = new Map([[ref, { node: new WeakRef(scroller) }]]);
        const text = "Grid [" + ref + "] " + "baris uji ".repeat(30);
        return { title: "Grid", url: location.href, text, stats: {}, refEnds: [[ref, text.length]] };
      } };
    `, context);
    return { frameId, documentId, location, navigation, window, scroller, context };
  }
  const child = frame(2, "iframe-a", frameAddress);
  let main = frame(0, "utama-a", topAddress);
  const context = createContext({
    document: { getElementById: () => ({ contentWindow: parent }) },
    window: { addEventListener: (_: string, listener: Parameters<typeof fromChat.addListener>[0]) => fromChat.addListener(listener) },
    URL, TextEncoder, TextDecoder, performance,
    crypto: { randomUUID: () => "snapshot-a" },
    setTimeout: (callback: () => void) => queueMicrotask(callback),
    chrome: {
      tabs: {
        query: async () => [{ id: activeId, windowId: 1, url: main.location.href }],
        captureVisibleTab: async () => "gambar-uji",
        onActivated, onUpdated, onRemoved, onReplaced,
      },
      runtime: { onMessage },
      scripting: { executeScript: async ({ target, files, func, args = [] }: Injection) => {
        if (files) beforeFiles?.();
        const frames = [main, child];
        const selected = target.allFrames ? frames : target.documentIds
          ? target.documentIds.map((id) => frames.find((doc) => doc.documentId === id))
          : frames.filter((doc) => target.frameIds?.includes(doc.frameId));
        if (selected.some((doc) => !doc)) throw new Error("Dokumen sudah berganti");
        if ((args[0] as Message)?.action) {
          actions += 1;
          await beforeAction?.();
        }
        return Promise.all(selected.map(async (doc) => {
          assert.ok(doc);
          doc.context.args = args;
          const result: unknown = func ? await runInContext(`(${func.toString()})(...args)`, doc.context) : null;
          return { documentId: doc.documentId, frameId: doc.frameId, result };
        }));
      } },
    },
  });
  runInContext(panelSource, context);

  function send(data: Message) {
    fromChat.emit({ origin: hub, source: parent, data });
  }
  function next(type: string, id?: string) {
    return new Promise<Message>((resolve) => {
      function receive(message: Message) {
        if (message.type !== type || (id && message.id !== id)) return;
        toChat.removeListener(receive);
        resolve(message);
      }
      toChat.addListener(receive);
    });
  }
  async function read() {
    const answer = next("app:halaman", "baca");
    send({ type: "app:baca-halaman", id: "baca" });
    return answer;
  }
  function scroll(remaining_ms = 8000) {
    const answer = next("app:hasil-aksi", "gulir");
    send({ type: "app:aksi-halaman", pageId: "snapshot-a", id: "gulir", action: "gulir",
      direction: "down", ref: "f2e1", remaining_ms });
    return answer;
  }
  return { child, send, next, read, scroll, actions: () => actions, watchers: onActivated.size,
    replaceBeforeRead() { beforeFiles = () => { main = frame(0, "utama-b", topAddress); }; },
    activateOtherTab() { activeId = 8; onActivated.emit({ windowId: 1, tabId: activeId }); },
    queueAction(holdPermit = false) {
      const queued = gate();
      const resume = gate();
      const pause = () => { queued.release(); return resume.promise; };
      if (holdPermit) beforePermitReply = pause;
      else beforeAction = pause;
      return { queued: queued.promise, resume: resume.release };
    },
  };
}

test("Baca halaman tidak membaca dokumen pengganti pada URL yang sama", async () => {
  const panel = fixture();
  panel.replaceBeforeRead();
  const result = await panel.read();
  assert.equal(result.page, undefined);
  assert.equal(typeof result.error, "string");
});

test("event tab dan URL iframe melepas snapshot aktif", async () => {
  const tab = fixture();
  await tab.read();
  const tabChanged = tab.next("app:halaman-berubah");
  tab.activateOtherTab();
  assert.equal((await tabChanged).pageId, "snapshot-a");
  assert.equal(tab.watchers(), 0);

  const panel = fixture();
  const read = await panel.read();
  assert.equal((read.page as Message).url, "https://contoh.test/daftar");
  const scrolled = await panel.scroll();
  assert.equal(scrolled.status, "scrolled");
  assert.match(String(scrolled.text), /^Alamat: https:\/\/contoh.test\/grid\nDibaca:/);
  assert.doesNotMatch(String(scrolled.text), /contoh=bawah|sementara/);
  const changed = panel.next("app:halaman-berubah");
  panel.child.location.href = "https://contoh.test/grid?contoh=baru";
  panel.child.window.dispatchEvent(new Event("pagehide"));
  panel.child.navigation.dispatchEvent(new Event("currententrychange"));
  assert.equal(panel.watchers(), 1);
  const trusted = new Event("currententrychange");
  Object.defineProperty(trusted, "isTrusted", { value: true });
  panel.child.navigation.dispatchEvent(trusted);
  assert.equal((await changed).pageId, "snapshot-a");
  assert.equal(panel.watchers(), 0);
});

test("verifikasi sebelum kirim mendeteksi URL iframe yang berubah sebelum event masuk", async () => {
  const panel = fixture();
  await panel.read();
  assert.equal((await panel.scroll()).status, "scrolled");
  panel.child.location.href = "https://contoh.test/grid?contoh=baru";
  const checked = panel.next("app:verifikasi-hasil");
  panel.send({ type: "app:verifikasi-halaman", pageId: "snapshot-a", id: "verifikasi" });
  assert.equal((await checked).valid, false);
  assert.equal(panel.watchers(), 0);
  assert.equal(runInContext("globalThis.__appFrameWatch", panel.child.context), null);
});

test("aksi memakai sisa waktu lokal saat timeOrigin frame berbeda satu jam", async () => {
  for (const offset of [-3_600_000, 3_600_000]) {
    const panel = fixture(offset);
    await panel.read();
    assert.equal((await panel.scroll()).status, "scrolled");
    assert.ok(panel.child.scroller.scrollTop > 0);

    const expired = fixture(offset);
    await expired.read();
    assert.equal((await expired.scroll(0)).status, "timeout");
    assert.equal(expired.actions(), 0);
    const result = await runInContext("act({remaining_ms: 0})", expired.child.context);
    assert.equal(result.status, "timeout");
    assert.equal(expired.child.scroller.scrollTop, 0);
  }
});

test("pembatalan injeksi tidak menggeser grid", async () => {
  for (const holdPermit of [false, true]) {
    const panel = fixture();
    await panel.read();
    const injection = panel.queueAction(holdPermit);
    const result = panel.scroll();
    await injection.queued;
    panel.send({ type: "app:batal-aksi", id: "gulir", pageId: "snapshot-a" });
    injection.resume();
    assert.equal((await result).status, "unavailable");
    assert.equal(panel.child.scroller.scrollTop, 0);
    assert.equal(panel.watchers(), 0);
  }
});

test("snapshot kedua menyamarkan isian ketika node label yang sama berubah menjadi PIN", () => {
  const label = { textContent: "Kode contoh", getAttribute: () => null, querySelectorAll: () => [], childNodes: [] };
  const input = {
    nodeType: 1, tagName: "INPUT", type: "text", labels: [label], value: "nilai-sintetis",
    getBoundingClientRect: () => ({ width: 100, height: 20 }),
    getAttribute: () => null, closest: () => null, getRootNode: () => null,
  };
  const context = createContext({
    document: {
      title: "Uji redaksi", body: { childNodes: [input] },
      querySelectorAll: (selector: string) => selector === "*" || selector.startsWith("input,") ? [input] : [],
    },
    Node: { ELEMENT_NODE: 1, TEXT_NODE: 3 },
    location: { href: topAddress },
    getComputedStyle: () => ({ visibility: "visible", display: "block", opacity: "1" }),
    computeAccessibleName: () => label.textContent, computeAccessibleDescription: () => "",
    getRole: () => "textbox", isDisabled: () => false,
  });
  runInContext(readerSource, context);
  const first = runInContext("readPage().text", context) as string;
  assert.match(first, /nilai-sintetis/);
  label.textContent = "PIN";
  const second = runInContext("readPage('', 100, true).text", context) as string;
  assert.match(second, /rahasia, tidak dibaca/);
  assert.doesNotMatch(second, /nilai-sintetis/);
});
