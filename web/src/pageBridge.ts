// Jembatan ke extension Bakase. Di panel samping Edge, Bakase tampil di iframe milik extension
// (`?panel=1`). Bakase meminta isi tab yang sedang dibuka lewat postMessage, extension
// membacanya, lalu isinya masuk ke kotak tanya sebagai lampiran biasa.
//
// Pesan hanya dikirim ke, dan diterima dari, origin extension yang terdaftar di server
// (`[auth] extension_ids`). Halaman lain tidak bisa memasang Bakase di iframe (CSP
// frame-ancestors), jadi tidak ada pihak lain yang bisa ikut menjawab.
import { allowedOrigin, readProgress, readReply, type PageRead } from "./pageRead.ts";
import { pageActionTiming, readActionReply, readTableProgress, type ActionReply, type PageAction } from "./pageAction.ts";
import { createPageSession, pageChanged, type LivePage } from "./pageSession.ts";

const TIMEOUT_MS = 120_000;

const session = createPageSession();

window.addEventListener("message", (event) => {
  if (event.source === window.parent) session.changed(event.origin, event.data);
});

export const bindPageAttachment = session.bind;
export const livePage = session.live;
export const consumePageRefresh = session.consumed;

export function releasePage(page: LivePage) {
  session.forget(page.id);
  window.parent.postMessage({ type: "app:lepas-halaman", pageId: page.id }, page.origin);
}

/** Bakase dibuka di panel extension (iframe dengan `?panel=1`). */
export function inPanel(): boolean {
  if (window.parent === window) return false;
  return new URLSearchParams(window.location.search).get("panel") === "1";
}

/** Origin extension yang memasang Bakase di iframe ini, kalau terdaftar. Selain itu null,
 * dan tombol baca halaman tidak muncul. */
export function panelOrigin(extensionIds: readonly string[]): string | null {
  if (!inPanel()) return null;
  // Edge mengisi ancestorOrigins. Browser yang tidak punya tidak didukung.
  const origin = window.location.ancestorOrigins?.[0];
  return origin && allowedOrigin(origin, extensionIds) ? origin : null;
}

function newId(): string {
  return typeof crypto.randomUUID === "function" ? crypto.randomUUID() : `${Date.now()}-${Math.random()}`;
}

export function verifyPageRefresh(page: LivePage, signal: AbortSignal): Promise<boolean> {
  if (signal.aborted) return Promise.resolve(false);
  const id = newId();
  return new Promise((resolve) => {
    function finish(valid: boolean) {
      clearTimeout(timer);
      signal.removeEventListener("abort", cancel);
      window.removeEventListener("message", onMessage);
      if (!valid) releasePage(page);
      resolve(valid);
    }
    const cancel = () => finish(false);
    function onMessage(event: MessageEvent) {
      if (event.source !== window.parent || event.origin !== page.origin) return;
      if (pageChanged(event.data, page.id)) finish(false);
      else if (event.data?.type === "app:verifikasi-hasil" && event.data.id === id) {
        finish(event.data.valid === true);
      }
    }
    const timer = setTimeout(cancel, 1500);
    signal.addEventListener("abort", cancel, { once: true });
    window.addEventListener("message", onMessage);
    window.parent.postMessage({ type: "app:verifikasi-halaman", id, pageId: page.id }, page.origin);
  });
}

/** Meminta extension membaca tab yang sedang dibuka. `onProgress` menerima kabar progresnya. */
export function requestPage(
  origin: string,
  onProgress: (text: string) => void = () => {},
  timeoutMs = TIMEOUT_MS,
): Promise<PageRead> {
  const id = newId();
  session.forget();
  return new Promise((resolve, reject) => {
    const done = () => {
      window.removeEventListener("message", onMessage);
      clearTimeout(timer);
    };
    const onMessage = (event: MessageEvent) => {
      if (event.source !== window.parent || event.origin !== origin) return;
      const progress = readProgress(event.data, id);
      if (progress) {
        onProgress(progress);
        return;
      }
      const reply = readReply(event.data, id);
      if (!reply) return;
      done();
      if ("error" in reply) reject(new Error(reply.error));
      else {
        const { page } = reply;
        if (page.snapshotId) session.remember(page.snapshotId, origin, page.ambilTabel);
        resolve(page);
      }
    };
    const timer = setTimeout(() => {
      done();
      reject(new Error("Halaman terlalu lama dibaca. Coba lagi, atau tutup dulu bagian yang sangat panjang."));
    }, timeoutMs);
    window.addEventListener("message", onMessage);
    window.parent.postMessage({ type: "app:baca-halaman", id }, origin);
  });
}

function askPanel(page: LivePage, action: PageAction, signal: AbortSignal): Promise<ActionReply> {
  const { id } = action;
  if (action.action === "ambil_tabel" && !page.ambilTabel) return Promise.resolve({ status: "unavailable" });
  const { timeout, deadline } = pageActionTiming(action);
  const remaining_ms = Math.min(timeout, deadline - performance.now());
  if (remaining_ms <= 0) {
    releasePage(page);
    return Promise.resolve({ status: "timeout" });
  }
  return new Promise((resolve) => {
    function finish(reply: ActionReply) {
      clearTimeout(timer);
      signal.removeEventListener("abort", cancel);
      window.removeEventListener("message", onMessage);
      resolve(reply);
    }
    function stop(status: "timeout" | "unavailable") {
      window.parent.postMessage({ type: "app:batal-aksi", id, pageId: page.id }, page.origin);
      finish({ status });
    }
    function cancel() {
      stop("unavailable");
    }
    function onMessage(event: MessageEvent) {
      if (event.source !== window.parent || event.origin !== page.origin) return;
      if (pageChanged(event.data, page.id)) {
        finish({ status: "unavailable" });
        return;
      }
      if (action.action === "ambil_tabel") {
        const progress = readTableProgress(event.data, id);
        if (progress !== null) {
          void postAction(id, { progress }, signal).catch(() => {});
          return;
        }
      }
      const reply = readActionReply(event.data, id);
      if (reply) finish(reply.status === "table" && action.action !== "ambil_tabel" ? { status: "failed" } : reply);
    }
    const timer = setTimeout(() => stop("timeout"), remaining_ms);
    signal.addEventListener("abort", cancel, { once: true });
    window.addEventListener("message", onMessage);
    window.parent.postMessage({ type: "app:aksi-halaman", pageId: page.id, ...action, remaining_ms }, page.origin);
  });
}

async function postAction(id: string, data: { reply: ActionReply } | { progress: number }, signal: AbortSignal) {
  const res = await fetch("/api/chat/page-action", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ id, ...data }),
    signal,
  });
  if (!res.ok) throw new Error("Hasil aksi halaman tidak diterima server.");
}

export function answerPageAction(page: LivePage, action: PageAction, signal: AbortSignal) {
  return session.answer(page, action, signal, askPanel, (id, reply, signal) => postAction(id, { reply }, signal)).catch((err: unknown) => {
    releasePage(page);
    throw err;
  });
}
