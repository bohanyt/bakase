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
const manifest = JSON.parse(readFileSync(new URL("../../extension/manifest.json", import.meta.url), "utf8"));
// Penunjuk (pointer.js) menggambar di halaman; di sini tidak ada halamannya.
const readerSource = "function point() {} function clearPointer() {} function busy() { return () => {}; }" +
  readFileSync(new URL("../../extension/src/snapshot.js", import.meta.url), "utf8")
  .replace(/^import .*;$/gm, "").replaceAll("export function", "function").replaceAll("export async function", "async function");
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

type PageFetch = (url: string, options: RequestInit) => Promise<{ ok: boolean; headers: Headers;
  body: { getReader: () => { read: () => Promise<{ done: boolean; value?: Uint8Array }>; cancel: () => Promise<void> } } }>;

/** Badan respons tiruan yang dibaca bertahap, seperti `response.body.getReader()`. */
function body(...chunks: Uint8Array[]) {
  return { getReader: () => ({
    read: async () => (chunks.length ? { done: false, value: chunks.shift() } : { done: true }),
    cancel: async () => { chunks.length = 0; },
  }) };
}

function fixture(frameOriginOffset = 0, probe: () => { ok: boolean } | Promise<{ ok: boolean }> = () => ({ ok: true }), tabUrl?: string,
  pageFetch?: PageFetch) {
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
  let probes = 0;
  const connectionEvents = new Map<string, () => Promise<void>>();
  const listen = (name: string, listener: () => Promise<void>) => connectionEvents.set(name, listener);
  let navigation = gate();
  let src: string | undefined;
  const panelFrame = {
    contentWindow: parent, hidden: true,
    get src() { return src; },
    set src(url) { src = url; navigation.release(); },
    hasAttribute(name: string) { return name === "src" && src !== undefined; },
    removeAttribute(name: string) { if (name === "src") src = undefined; },
    addEventListener(name: string, listener: () => Promise<void>) {
      listen(name, async () => {
        connectionEvents.delete(name);
        await listener();
      });
    },
  };
  const offline = { hidden: true };
  const retry = { disabled: false, addEventListener: listen };
  const elements = { app: panelFrame, offline, retry };
  const timeoutSignal = new AbortController().signal;

  function frame(frameId: number, documentId: string, href: string) {
    const location = { href };
    const navigation = new EventTarget();
    const window = new EventTarget();
    const scroller = {
      scrollTop: 0, scrollLeft: 0, clientHeight: 300, scrollHeight: 3000,
      scrollBy({ top }: { top: number }) { this.scrollTop += top; },
      isConnected: true, getClientRects: () => [{}], closest: () => null, getAttribute: () => null,
      getRootNode: () => null, matches: (selector: string) => selector.startsWith("table,"),
    };
    const context = createContext({
      location, navigation, window,
      TextEncoder, btoa,
      ...(pageFetch ? { fetch: pageFetch } : {}),
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
    document: { getElementById: (id: keyof typeof elements) => elements[id], addEventListener: listen },
    window: { addEventListener: (name: string, listener: Parameters<typeof fromChat.addListener>[0] | (() => Promise<void>)) => {
      if (name === "message") fromChat.addListener(listener);
      else listen(name, listener as () => Promise<void>);
    } },
    AbortSignal: { timeout(ms: number) { assert.equal(ms, 6000); return timeoutSignal; } },
    fetch: (url: string, options: RequestInit) => {
      assert.equal(url, hub + "/api/health");
      assert.equal(options.cache, "no-store");
      assert.equal(options.signal, timeoutSignal);
      probes += 1;
      return probe();
    },
    URL, TextEncoder, TextDecoder, performance, atob,
    crypto: { randomUUID: () => "snapshot-a" },
    setTimeout: (callback: () => void) => queueMicrotask(callback),
    chrome: {
      tabs: {
        query: async () => [{ id: activeId, windowId: 1, url: tabUrl ?? main.location.href }],
        captureVisibleTab: async () => "gambar-uji",
        onActivated, onUpdated, onRemoved, onReplaced,
      },
      runtime: { onMessage, getManifest: () => manifest },
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
    connection: {
      frame: panelFrame, offline, retry, probes: () => probes,
      emit: (name: string) => connectionEvents.get(name)!(),
      navigating: () => navigation.promise,
      loaded() {
        navigation = gate();
        return connectionEvents.get("load")?.();
      },
      setTabUrl(url: string) { tabUrl = url; },
    },
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

test("percobaan ulang memakai URL pilot yang dibaca saat panel dibuka", async () => {
  let healthy = false;
  const panel = fixture(0, () => ({ ok: healthy }), hub + "/?microsoft=pilot").connection;
  const { frame, offline } = panel;
  await panel.emit("DOMContentLoaded");
  assert.equal(offline.hidden, false);
  assert.equal(frame.hidden, true);
  assert.equal(frame.src, undefined);

  panel.setTabUrl(hub);
  healthy = true;
  await panel.emit("click");
  await panel.loaded();
  assert.equal(offline.hidden, true);
  assert.equal(frame.hidden, false);
  assert.equal(frame.src, `${hub}/?panel=1&microsoft=pilot&v=${manifest.version}`);
});

test("navigasi awal yang gagal setelah health 200 bisa dicoba ulang", async () => {
  let healthy = true;
  const panel = fixture(0, () => ({ ok: healthy })).connection;
  const initial = panel.emit("DOMContentLoaded");
  await panel.navigating();
  assert.equal(panel.frame.hidden, false);
  assert.equal(panel.offline.hidden, true);
  assert.equal(panel.retry.disabled, false);
  await initial;
  await panel.emit("online");
  assert.equal(panel.probes(), 1);
  healthy = false;
  await panel.loaded();
  assert.equal(panel.frame.src, undefined);
  assert.equal(panel.frame.hidden, true);
  assert.equal(panel.offline.hidden, false);

  healthy = true;
  await panel.emit("click");
  await panel.loaded();
  assert.equal(panel.frame.hidden, false);
  assert.equal(panel.offline.hidden, true);
  assert.equal(panel.probes(), 4);
});

test("event online saat probe digabung menjadi satu percobaan susulan", async () => {
  const first = gate();
  let attempts = 0;
  const panel = fixture(0, async () => {
    if (++attempts > 1) return { ok: true };
    await first.promise;
    return { ok: false };
  }).connection;
  const initial = panel.emit("DOMContentLoaded");
  await panel.emit("online");
  await panel.emit("online");
  assert.equal(panel.probes(), 1);
  first.release();
  await panel.navigating();
  await panel.loaded();
  await initial;
  assert.equal(panel.probes(), 3);
  assert.equal(panel.frame.hidden, false);
  assert.equal(panel.offline.hidden, true);
});

test("timeout tampil sebagai offline dan event online tidak memuat ulang iframe yang sudah hidup", async () => {
  let healthy = false;
  const { frame, offline, emit, loaded, probes } = fixture(0, () => {
    if (!healthy) throw new DOMException("Waktu habis", "TimeoutError");
    return { ok: true };
  }).connection;
  await emit("DOMContentLoaded");
  assert.equal(offline.hidden, false);

  healthy = true;
  await emit("online");
  await loaded();
  assert.equal(frame.src, `${hub}/?panel=1&v=${manifest.version}`);
  assert.equal(offline.hidden, true);

  healthy = false;
  await emit("online");
  assert.equal(frame.src, `${hub}/?panel=1&v=${manifest.version}`);
  assert.equal(frame.hidden, false);
  assert.equal(offline.hidden, true);
  assert.equal(probes(), 3);
});

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

test("ambil tabel melaporkan progres dan berhenti jika tab berubah di tengah pembacaan", async () => {
  const panel = fixture();
  assert.equal(((await panel.read()).page as Message).ambilTabel, true);
  const paused = gate();
  panel.child.context.wait = paused.promise;
  runInContext(`
    collectTable = async (grid, url, deadline, allowed) => {
      if (!await allowed(18)) return { status: "unavailable" };
      await wait;
      if (!await allowed(30)) return { status: "unavailable" };
      return { status: "table", table: { title: "Uji", columns: ["A"], rows: [], issues: [] } };
    };
  `, panel.child.context);
  const progress = panel.next("app:progres", "tabel");
  const result = panel.next("app:hasil-aksi", "tabel");
  panel.send({ type: "app:aksi-halaman", pageId: "snapshot-a", id: "tabel",
    action: "ambil_tabel", ref: "f2e1", remaining_ms: 62000 });
  assert.equal((await progress).rows, 18);
  panel.activateOtherTab();
  paused.release();
  assert.equal((await result).status, "unavailable");
  assert.equal(panel.watchers(), 0);
});

test("snapshot kedua menyamarkan isian ketika node label yang sama berubah menjadi PIN", () => {
  const label = { textContent: "Kode contoh", getAttribute: () => null, querySelectorAll: () => [], childNodes: [] };
  const input = {
    nodeType: 1, tagName: "INPUT", type: "text", labels: [label], value: "nilai-sintetis",
    getBoundingClientRect: () => ({ width: 100, height: 20 }),
    getAttribute: () => null, closest: () => null, getRootNode: () => null,
  };
  const context = createContext({
    TextEncoder,
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

test("Baca halaman di Excel Online melampirkan file aslinya", async () => {
  const seen: string[] = [];
  const sheet = "https://appid-my.example.com/:x:/g/personal/uji/IQAabc?e=XyZ";
  const panel = fixture(0, undefined, sheet, async (url, options) => {
    seen.push(url);
    assert.equal(options.credentials, "include");
    return { ok: true, headers: new Headers({ "content-disposition": "attachment; filename*=UTF-8''Rekap%20uji.xlsx" }),
      body: body(new Uint8Array([0x50, 0x4b]), new Uint8Array([3, 4])) };
  });
  const page = (await panel.read()).page as { text: string; snapshotId?: string; file: { name: string; bytes: ArrayBuffer } };
  assert.deepEqual(seen, [`${sheet}&download=1`]);
  assert.equal(page.text, "");
  assert.equal(page.snapshotId, undefined);
  assert.equal(page.file.name, "Rekap uji.xlsx");
  assert.equal(page.file.bytes.byteLength, 4);
});

test("Excel Online yang tidak bisa diunduh tidak dikirim sebagai file", async () => {
  const sheet = "https://appid-my.example.com/:x:/g/personal/uji/IQAabc?e=XyZ";
  // Unduhan diblokir: shared storage mengembalikan halaman HTML, bukan ZIP.
  const panel = fixture(0, undefined, sheet, async () => ({ ok: true, headers: new Headers(),
    body: body(new TextEncoder().encode("<html>")) }));
  const result = await panel.read();
  assert.equal((result.page as { file?: unknown } | undefined)?.file, undefined);
  // Fetch yang gagal juga tidak mengirim file.
  const broken = fixture(0, undefined, sheet, async () => { throw new TypeError("Failed to fetch"); });
  assert.equal(((await broken.read()).page as { file?: unknown } | undefined)?.file, undefined);
});

test("Excel Online di atas 25 MB ditolak tanpa membaca semuanya", async () => {
  const sheet = "https://appid-my.example.com/:x:/g/personal/uji/IQAabc?e=XyZ";
  let read = 0;
  const big = new Uint8Array(1024 * 1024);
  big[0] = 0x50;
  big[1] = 0x4b;
  // Server tidak mengirim content-length: batasnya ditegakkan sambil membaca.
  const panel = fixture(0, undefined, sheet, async () => ({ ok: true, headers: new Headers(), body: { getReader: () => ({
    read: async () => { read += 1; return { done: false, value: big }; },
    cancel: async () => {},
  }) } }));
  const result = await panel.read();
  assert.match(String(result.error), /25 MB/);
  assert.equal(read, 26);
});

