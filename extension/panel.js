// Panel samping: menampilkan Bakase di iframe dan menjembatani baca, sorot, dan gulir.
// Hanya pesan dari iframe Bakase (origin HUB) yang dilayani, dan balasan hanya dikirim ke
// origin itu. Halaman dibaca oleh lib/snapshot.js (dibundel dari src/snapshot.js).

// Set this to the origin of your server, without a trailing slash.
const HUB = "http://127.0.0.1:8100";
// Batas teks gabungan semua frame. Teks yang lebih panjang tetap bisa dibaca Bakase lewat
// read_attachment, tapi halaman sepanjang ini biasanya sudah tidak relevan.
const MAX_TEXT = 100000;
const MAX_ACTION_TEXT = 24000;
const REF = /^(?:f([1-9][0-9]{0,9}))?e[1-9][0-9]{0,15}$/;
const frame = document.getElementById("app");
frame.src = `${HUB}/?panel=1`;

async function activeTab() {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  return tab;
}

function displayUrl(url) {
  const address = new URL(url);
  return /^https?:$/.test(address.protocol) ? address.origin + address.pathname : "";
}

/** Mencatat setiap perpindahan tab di jendela itu dan setiap navigasi tab yang dibaca, sejak
 * halaman mulai dibaca. Pengecekan sesaat saja tidak cukup: user bisa pindah ke tab lain dan
 * kembali di antara dua pengecekan, sementara captureVisibleTab memotret tab apa pun yang
 * sedang tampil. */
function watchTab(tab, onMove) {
  let moved = false;
  const changed = () => {
    if (moved) return;
    moved = true;
    onMove();
  };
  const onActivated = (info) => {
    if (info.windowId === tab.windowId) changed();
  };
  const onUpdated = (tabId, change) => {
    if (tabId === tab.id && (change.url || change.status === "loading")) changed();
  };
  const onGone = (tabId) => {
    if (tabId === tab.id) changed();
  };
  const onReplaced = (_, removedTabId) => onGone(removedTabId);
  chrome.tabs.onActivated.addListener(onActivated);
  chrome.tabs.onUpdated.addListener(onUpdated);
  chrome.tabs.onRemoved.addListener(onGone);
  chrome.tabs.onReplaced.addListener(onReplaced);
  return {
    moved: () => moved,
    stop() {
      chrome.tabs.onActivated.removeListener(onActivated);
      chrome.tabs.onUpdated.removeListener(onUpdated);
      chrome.tabs.onRemoved.removeListener(onGone);
      chrome.tabs.onReplaced.removeListener(onReplaced);
    },
  };
}

/** Tab yang dibaca masih tab yang tampil di jendelanya, dengan dokumen yang sama. */
async function stillShowing(tab, documentId, url) {
  if (!documentId) return false;
  const [active] = await chrome.tabs.query({ active: true, windowId: tab.windowId });
  if (active?.id !== tab.id) return false;
  const [probe] = await chrome.scripting.executeScript({ target: { tabId: tab.id, frameIds: [0] }, func: () => location.href });
  return probe?.documentId === documentId && probe.result === url;
}

/** Screenshot hanya dipakai kalau sejak halaman mulai dibaca sampai sesudah dipotret tidak ada
 * perpindahan tab atau navigasi, dan tab serta dokumennya masih sama. Kalau ragu, dibuang:
 * teksnya tetap dikirim. */
async function captureSameTab(tab, { documentId, result }, watch) {
  if (watch.moved() || !(await stillShowing(tab, documentId, result.url))) return null;
  const image = await chrome.tabs.captureVisibleTab(tab.windowId, { format: "jpeg", quality: 70 });
  // Event perpindahan tab yang terjadi tepat saat memotret diberi waktu untuk sampai, lalu
  // dicek paling akhir (event juga bisa datang selagi pengecekan tab berjalan).
  await new Promise((r) => setTimeout(r, 200));
  const same = await stillShowing(tab, documentId, result.url);
  return same && !watch.moved() ? image : null;
}

const MERGE_NOTE = "\n\n[Terpotong: bagian halaman lain melewati batas baca.]";

/** Teks frame utama, lalu frame lain yang berisi (misalnya isi aplikasi bisnis yang ada
 * di iframe), paling banyak MAX_TEXT karakter termasuk catatannya, dipotong di akhir baris. */
function mergeFrames(main, extra) {
  let text = main.text;
  const refs = new Set(main.refEnds.map(([id]) => id));
  const addRefs = (part, end) => {
    for (const [id, offset] of part.refEnds) if (offset <= end) refs.add(id);
  };
  for (let i = 0; i < extra.length; i++) {
    const header = `\n\n## Bagian halaman: ${extra[i].title}\n`;
    const part = header + extra[i].text;
    const last = i === extra.length - 1;
    if (text.length + part.length + (last ? 0 : MERGE_NOTE.length) <= MAX_TEXT) {
      text += part;
      addRefs(extra[i], extra[i].text.length);
      continue;
    }
    const room = MAX_TEXT - text.length - MERGE_NOTE.length;
    if (room > 2000) {
      // Potong di akhir baris, kecuali barisnya sangat panjang (misalnya teks tanpa baris baru).
      const end = part.lastIndexOf("\n", room);
      const boundary = end > room / 2 ? end : room;
      text += part.slice(0, boundary);
      addRefs(extra[i], boundary - header.length);
    }
    text += MERGE_NOTE;
    break;
  }
  return { text, refs };
}

// Satu permintaan sekaligus. Watch terakhir dipertahankan sampai snapshot diganti.
let current = null;
let latest = null;
let nextBase = 0;
function refBase() {
  const base = nextBase;
  nextBase += 1000000;
  return base;
}

function unwatchSource(page) {
  if (!page.source) return;
  void chrome.scripting.executeScript({
    target: { tabId: page.tab.id, documentIds: [page.source.documentId] },
    func: (pageId) => globalThis.AppSnapshot?.unwatchFrame?.(pageId),
    args: [page.id],
  }).catch(() => {});
}

function forgetPage(page = latest) {
  if (!page || page !== latest) return;
  latest = null;
  page.watch.stop();
  unwatchSource(page);
  frame.contentWindow.postMessage({ type: "app:halaman-berubah", pageId: page.id }, HUB);
}
const sendProgress = (id, text) => frame.contentWindow.postMessage({ type: "app:progres", id, text }, HUB);

async function readFrame(tabId, { frameId, documentId, url }, base, pageId = null) {
  const [r] = await chrome.scripting.executeScript({
    target: { tabId, documentIds: [documentId] },
    func: (prefix, start, expectedUrl, sourcePageId) => {
      if (location.href !== expectedUrl) return null;
      if (sourcePageId) globalThis.AppSnapshot.watchFrame(sourcePageId, expectedUrl);
      return globalThis.AppSnapshot.readPage(prefix, start, !!sourcePageId);
    },
    args: [frameId === 0 ? "" : `f${frameId}`, base, url, pageId],
  });
  return { frameId, documentId: r?.documentId, result: r?.result };
}

/** Menjalankan pembaca di semua frame tab. Frame yang tidak bisa dibaca (misalnya penampil PDF)
 * membuat seluruh pemanggilan gagal, jadi kalau begitu cukup frame utama. */
async function snapshot(tab, watch) {
  const tabId = tab.id;
  const probe = (target) => chrome.scripting.executeScript({ target, func: () => location.href });
  const inspected = await probe({ tabId, allFrames: true }).catch(() => []);
  if (!inspected.some((r) => r.frameId === 0)) inspected.unshift(...await probe({ tabId, frameIds: [0] }));
  const frames = inspected.filter((r) => r.documentId && typeof r.result === "string")
    .map(({ frameId, documentId, result }) => ({ frameId, documentId, url: result }));
  const top = frames.find((r) => r.frameId === 0);
  if (!top || top.url !== tab.url || watch.moved()) return [];

  const inject = (documentIds) => chrome.scripting.executeScript({
    target: { tabId, documentIds }, files: ["lib/snapshot.js"],
  });
  const installed = await inject(frames.map((f) => f.documentId)).catch(() => inject([top.documentId]));
  const ready = new Set(installed.map((f) => f.documentId));
  const base = refBase();
  const settled = await Promise.allSettled(frames.filter((f) => ready.has(f.documentId)).map((f) => readFrame(tabId, f, base)));
  return settled.flatMap((s) => (s.status === "fulfilled" && s.value.result ? [s.value] : []));
}

async function readPage(msg) {
  const reply = (data) => frame.contentWindow.postMessage({ type: "app:halaman", id: msg.id, ...data }, HUB);
  // Satu pembacaan sekaligus (misalnya permintaan lama dari percakapan yang sudah ditinggal
  // masih berjalan).
  if (current) {
    reply({ error: "Halaman masih dibaca untuk permintaan sebelumnya. Coba lagi sebentar lagi." });
    return;
  }
  current = { id: msg.id };
  forgetPage();
  let watch = null;
  try {
    const tab = await activeTab();
    if (!tab?.id || !/^https?:/.test(tab.url || "")) {
      reply({ error: "Halaman ini tidak bisa dibaca. Buka halaman web (http/https) di tab aktif." });
      return;
    }
    const readingWatch = watchTab(tab, () => {
      if (latest?.watch === readingWatch) forgetPage();
    });
    watch = readingWatch;
    sendProgress(msg.id, "Membaca struktur halaman…");
    const results = await snapshot(tab, watch);
    const top = results.find((r) => r.frameId === 0);
    if (!top?.result) throw new Error("isi halaman utama tidak terbaca");
    // Frame lain yang berisi (misalnya isi aplikasi di iframe), juga frame kecil yang memuat
    // pesan error.
    const extra = results
      .filter((r) => r.frameId !== 0 && typeof r.result?.text === "string")
      .filter((r) => r.result.text.length > 200 || r.result.stats?.messages > 0)
      .map((r) => r.result)
      .sort((a, b) => b.text.length - a.text.length);
    let image = null;
    try {
      sendProgress(msg.id, "Memotret layar…");
      image = await captureSameTab(tab, top, watch);
    } catch {
      // Screenshot tambahan saja; teks tetap dikirim.
    }
    const main = top.result;
    const { text, refs } = mergeFrames(main, extra);
    const pageData = { title: main.title, url: displayUrl(main.url), text, stats: main.stats };
    const same = !watch.moved() && await stillShowing(tab, top.documentId, main.url).catch(() => false);
    if (!same || watch.moved()) {
      reply({ page: { ...pageData, image: null } });
      return;
    }
    const documents = new Map(results.map(({ frameId, documentId, result }) => [frameId, { frameId, documentId, url: result.url }]));
    const snapshotId = crypto.randomUUID();
    latest = { id: snapshotId, tab, watch, documents, refs, source: null };
    watch = null;
    reply({ page: { ...pageData, image, snapshotId } });
  } catch {
    reply({ error: "Halaman tidak bisa dibaca. Buka halaman web di tab aktif dan coba lagi." });
  } finally {
    watch?.stop();
    if (current?.id === msg.id) current = null;
  }
}

function actionSnapshot(result) {
  const header = `Alamat: ${displayUrl(result.url)}\nDibaca: ${new Date().toISOString()}\n\n`;
  const text = header + result.text;
  const bytes = new TextEncoder().encode(text);
  if (bytes.length <= MAX_ACTION_TEXT) return { text, refEnds: result.refEnds };
  const note = "\n[Terpotong: snapshot melewati batas baca.]";
  const head = new TextDecoder().decode(bytes.subarray(0, MAX_ACTION_TEXT - note.length - 3));
  const end = head.lastIndexOf("\n");
  if (end < 0) return { text: head + note, refEnds: [] };
  return { text: head.slice(0, end) + note, refEnds: result.refEnds.filter(([, offset]) => header.length + offset <= end) };
}

async function actOnPage(msg) {
  const receivedAt = performance.now();
  const reply = (result) => frame.contentWindow.postMessage({ type: "app:hasil-aksi", id: msg.id, ...result }, HUB);
  const page = latest;
  if (page?.watch.moved()) forgetPage(page);
  if (current || !page || page !== latest || msg.pageId !== page.id) {
    reply({ status: "unavailable" });
    return;
  }
  const { ref, action, direction, remaining_ms } = msg;
  const match = typeof ref === "string" && REF.exec(ref);
  if ((ref !== null && !match) || (action === "sorot" && !match) ||
      (action === "gulir" && !["up", "down", "to"].includes(direction)) ||
      (direction === "to" && !match) || !["sorot", "gulir"].includes(action) ||
      !Number.isFinite(remaining_ms) || remaining_ms < 0 || remaining_ms > 8000) {
    reply({ status: "failed" });
    forgetPage(page);
    return;
  }
  if (remaining_ms === 0) {
    reply({ status: "timeout" });
    forgetPage(page);
    return;
  }
  const dest = page.documents.get(Number(match?.[1] || 0));
  if (!dest || (ref && !page.refs.has(ref))) {
    reply({ status: "stale_ref" });
    return;
  }
  const localDeadline = receivedAt + remaining_ms;
  const request = { id: msg.id, pageId: page.id, dest, localDeadline, cancelled: false, phase: "app:izin-aksi" };
  current = request;
  const active = async () => {
    if (request.cancelled || latest !== page || performance.now() >= localDeadline) return false;
    const top = page.documents.get(0);
    const same = await stillShowing(page.tab, top.documentId, top.url);
    return same && !request.cancelled && latest === page && performance.now() < localDeadline;
  };
  let keep = false;
  try {
    if (!await active()) {
      reply({ status: "unavailable" });
      return;
    }
    const remaining = Math.min(remaining_ms, localDeadline - performance.now());
    if (remaining <= 0) {
      reply({ status: "timeout" });
      return;
    }
    const [r] = await chrome.scripting.executeScript({
      target: { tabId: page.tab.id, documentIds: [dest.documentId] },
      func: (args) => globalThis.AppSnapshot?.act(args),
      args: [{ id: request.id, pageId: page.id, action, ref, direction, url: dest.url, remaining_ms: remaining }],
    });
    if (!await active()) {
      reply({ status: "unavailable" });
      return;
    }
    const result = r?.result;
    if (result?.status !== "scrolled") {
      keep = result?.status === "highlighted" || result?.status === "stale_ref";
      reply({ status: result?.status || "failed" });
      return;
    }
    // Beri grid waktu untuk merender baris yang baru masuk ke layar.
    await new Promise((resolve) => setTimeout(resolve, 250));
    if (!await active()) {
      reply({ status: "unavailable" });
      return;
    }
    if (page.source && page.source.documentId !== dest.documentId) unwatchSource(page);
    page.source = dest;
    const fresh = await readFrame(page.tab.id, dest, refBase(), page.id);
    const same = await active();
    if (!same || fresh.result?.url !== dest.url) {
      reply({ status: "unavailable" });
      return;
    }
    const { text, refEnds } = actionSnapshot(fresh.result);
    page.refs = new Set(refEnds.map(([id]) => id));
    keep = true;
    reply({ status: "scrolled", moved: result.moved, text });
  } catch {
    reply({ status: "unavailable" });
  } finally {
    if (!keep) {
      if (page === latest) forgetPage(page);
      else unwatchSource(page);
    }
    current = null;
  }
}

async function verifyPage(msg) {
  const page = latest;
  let valid = false;
  if (page?.id === msg.pageId && page.source) {
    const source = page.source;
    const top = page.documents.get(0);
    try {
      const [probe] = await chrome.scripting.executeScript({
        target: { tabId: page.tab.id, documentIds: [source.documentId] },
        func: () => location.href,
      });
      valid = probe?.result === source.url && await stillShowing(page.tab, top.documentId, top.url)
        && page === latest && source === page.source;
    } catch {
      // Dokumen bisa hilang saat diperiksa.
    }
    if (!valid) forgetPage(page);
  }
  frame.contentWindow.postMessage({ type: "app:verifikasi-hasil", id: msg.id, valid }, HUB);
}

chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  const page = latest;
  if (!page || msg?.pageId !== page.id || sender.tab?.id !== page.tab.id) return;
  if (msg.type === "app:frame-berubah" && sender.documentId === page.source?.documentId) {
    forgetPage(page);
    return;
  }
  if (!current?.phase || msg.type !== current.phase || msg.id !== current.id) return;
  const allowed = sender.documentId === current.dest.documentId && !current.cancelled
    && performance.now() < current.localDeadline;
  if (allowed) current.phase = msg.type === "app:izin-aksi" ? "app:cek-aksi" : null;
  sendResponse(allowed);
});

window.addEventListener("message", (event) => {
  if (event.origin !== HUB || event.source !== frame.contentWindow) return;
  const msg = event.data;
  if (!msg) return;
  if (msg.type === "app:lepas-halaman" && msg.pageId === latest?.id) {
    forgetPage();
    return;
  }
  if (typeof msg.id !== "string") return;
  if (msg.type === "app:baca-halaman") void readPage(msg);
  else if (msg.type === "app:aksi-halaman") void actOnPage(msg);
  else if (msg.type === "app:verifikasi-halaman") void verifyPage(msg);
  else if (msg.type === "app:batal-aksi" && current?.id === msg.id && current.pageId === msg.pageId) {
    current.cancelled = true;
    if (msg.pageId === latest?.id) forgetPage();
  }
});
