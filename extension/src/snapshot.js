// Snapshot halaman untuk Bakase: struktur accessibility dari bagian yang dirender, seperti
// yang dibaca pembaca layar. Peran dan nama elemen dihitung dengan dom-accessibility-api (standar
// accname W3C), jadi cara bacanya sama untuk semua web app.
//
// Pembacaan tidak mengubah halaman. Ref (e1, e2, ...; frame lain f<id>e1, ...) disimpan di
// dunia terpisah milik extension. Ambil tabel mengembalikan posisi gulir setelah membaca grid.
//
// Nilai isian rahasia tidak pernah ikut terbaca. Isian rahasia dan semua induknya ditandai dulu;
// nama, keterangan, dan teks yang sumbernya menyentuh isian rahasia dihitung ulang dari teks
// halaman tanpa isian itu, bukan dengan mencocokkan string.
//
// Dibundel jadi extension/lib/snapshot.js dengan `npm run build:extension` di web/.
import { computeAccessibleDescription, computeAccessibleName, getRole, isDisabled } from "dom-accessibility-api";
import { busy, clearPointer, point } from "./pointer.js";

const NOTE = "\n[Terpotong: halaman lebih panjang dari batas baca.]";
const MAX_SNAPSHOT = 60000;
const MAX_ROWS = 200;
const MAX_NAME = 200;
const MAX_TEXT = 2000;

const FIELDS = new Set(["textbox", "searchbox", "combobox", "spinbutton", "slider", "checkbox", "radio", "switch", "listbox"]);
const ACTIONS = new Set(["button", "link", "menuitem", "menuitemcheckbox", "menuitemradio", "tab", "option", "treeitem"]);
const TABLES = new Set(["table", "grid", "treegrid"]);
const TABLE_SELECTOR = 'table, [role="table"], [role="grid"], [role="treegrid"]';
const ROW_SELECTOR = '[role="row"], tr';
const CELL_SELECTOR = '[role="gridcell"], [role="cell"], [role="columnheader"], [role="rowheader"], td, th';
// Pesan satu baris; dialog peringatan (alertdialog) ditelusuri seperti wadah karena berisi isian
// dan tombol.
const MESSAGES = new Set(["alert", "status"]);
// Wadah yang ditulis (kalau bernama, atau selalu untuk yang penting), lalu isinya di bawahnya.
const ALWAYS = new Set(["dialog", "alertdialog", "tablist", "toolbar", "menu", "menubar"]);
const IF_NAMED = new Set(["navigation", "form", "search", "region", "group", "radiogroup", "tabpanel", "tree", "complementary"]);
const CHECKABLE = new Set(["checkbox", "radio", "switch", "menuitemcheckbox", "menuitemradio"]);
// Peran yang namanya diambil dari isinya.
const NAME_FROM_CONTENT = new Set([...ACTIONS, ...CHECKABLE, "heading", "cell", "gridcell", "columnheader", "rowheader", "row", "tooltip"]);
const SKIP_TAGS = new Set(["SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE", "HEAD", "META", "LINK", "OBJECT", "EMBED"]);
const CONTROL_TAGS = new Set(["INPUT", "SELECT", "TEXTAREA"]);
const NOT_FIELD_INPUTS = new Set(["hidden", "submit", "reset", "button", "image"]);
const EDITABLE = '[contenteditable]:not([contenteditable="false"])';
const FIELD_SELECTOR = `input, select, textarea, ${EDITABLE}, [role="textbox"], [role="searchbox"], [role="combobox"], [role="spinbutton"], [role="slider"], [role="listbox"]`;
// Elemen yang membuat sebuah blok perlu ditelusuri, bukan dibaca sebagai satu teks.
const MEANINGFUL = `a[href], button, input, select, textarea, [role], h1, h2, h3, h4, h5, h6, table, img[alt], details, summary, iframe, ${EDITABLE}, [aria-expanded]`;
const SECRET = /(^|[^a-z])(pin|otp|cvv|cvc|passcode|password|sandi)([^a-z]|$)/i;
const SECRET_TYPES = new Set(["hidden", "password", "file"]);

const clean = (s, max = MAX_NAME) => {
  const t = (s || "").replace(/\s+/g, " ").trim();
  return t.length > max ? `${t.slice(0, max)}…` : t;
};
const quote = (s) => `"${s.replace(/"/g, '\\"')}"`;

/** Induk yang benar-benar merender elemen: slot tempat ia diproyeksikan, induk biasa, atau host
 * shadow root. */
const parentOf = (el) => el.assignedSlot || el.parentElement || el.getRootNode()?.host || null;

function scrollBox(el) {
  return el.clientHeight > 0 && el.scrollHeight > el.clientHeight &&
    /^(auto|scroll|overlay)$/.test(getComputedStyle(el).overflowY);
}

function scrollerOf(el) {
  // Ada grid yang membungkus area gulir di dalamnya, bukan menjadi area gulir itu sendiri.
  if (el?.matches(TABLE_SELECTOR) && !scrollBox(el)) {
    for (const row of el.querySelectorAll(ROW_SELECTOR)) {
      for (let parent = parentOf(row); parent && parent !== el; parent = parentOf(parent)) {
        if (scrollBox(parent)) return parent;
      }
    }
  }
  for (let parent = el; parent; parent = parentOf(parent)) {
    if (scrollBox(parent)) return parent;
  }
  return document.scrollingElement;
}

function reveal(el) {
  el.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "instant" });
}

export async function act({ id, pageId, action, ref, direction, url, remaining_ms }) {
  const receivedAt = performance.now();
  const table = action === "ambil_tabel";
  if (!Number.isFinite(remaining_ms) || remaining_ms <= 0 || remaining_ms > (table ? 62000 : 8000)) {
    return { status: "timeout" };
  }
  const localDeadline = receivedAt + remaining_ms;
  // Izin dicek saat skrip mulai berjalan karena injeksi bisa tertahan di antrean.
  const check = (type, details) => chrome.runtime.sendMessage({ type, id, pageId, ...details }).catch(() => false);
  if (!await check("app:izin-aksi") || !await check("app:cek-aksi") ||
      performance.now() >= localDeadline ||
      location.href !== url || document.visibilityState !== "visible") {
    return { status: "unavailable" };
  }
  const entry = ref && globalThis.__appRefs?.get(ref);
  const el = entry?.node.deref();
  if (ref && (!el?.isConnected || !el.getClientRects().length || hiddenUp(el) ||
      el.closest(ROW_SELECTOR)?.getAttribute("aria-rowindex") !== entry.rowIndex ||
      getComputedStyle(el).visibility !== "visible")) {
    return { status: "stale_ref" };
  }
  if (table) {
    if (!el?.matches(TABLE_SELECTOR)) return { status: "stale_ref" };
    const watching = globalThis.__appFrameWatch?.pageId === pageId;
    if (!watching) watchFrame(pageId, url);
    const done = busy("Bakase sedang mengambil tabel");
    try {
      return await collectTable(el, url, Math.min(localDeadline - 750, performance.now() + 60000),
        (rows) => check("app:lanjut-tabel", { rows }));
    } finally {
      done();
      if (!watching) unwatchFrame(pageId);
    }
  }
  if (action === "sorot") {
    reveal(el);
    point(el);
    return { status: "highlighted" };
  }
  const scroller = scrollerOf(el);
  if (!scroller) return { status: "failed" };
  const top = scroller.scrollTop;
  const left = scroller.scrollLeft;
  const pageTop = scrollY;
  const pageLeft = scrollX;
  const scope = el?.closest(TABLE_SELECTOR) || scroller;
  globalThis.__appScope = new WeakRef(scope);
  const viewport = scroller === document.scrollingElement ? innerHeight : scroller.clientHeight;
  if (direction === "to") reveal(el);
  else scroller.scrollBy({ top: Math.max(1, Math.floor(viewport * 0.85)) * (direction === "up" ? -1 : 1), behavior: "instant" });
  return { status: "scrolled", moved: scroller.scrollTop !== top || scroller.scrollLeft !== left ||
    scrollY !== pageTop || scrollX !== pageLeft };
}

export function watchFrame(pageId, url) {
  globalThis.__appFrameWatch?.stop();
  const changes = globalThis.navigation;
  function stop() {
    changes?.removeEventListener("currententrychange", changed);
    window.removeEventListener("pagehide", gone);
    globalThis.__appFrameWatch = null;
  }
  function gone(event) {
    if (!event.isTrusted) return;
    stop();
    void chrome.runtime.sendMessage({ type: "app:frame-berubah", pageId }).catch(() => {});
  }
  function changed(event) {
    if (location.href !== url) gone(event);
  }
  globalThis.__appFrameWatch = { pageId, stop };
  changes?.addEventListener("currententrychange", changed);
  window.addEventListener("pagehide", gone);
}

export function unwatchFrame(pageId) {
  if (globalThis.__appFrameWatch?.pageId === pageId) globalThis.__appFrameWatch.stop();
}

/** Elemen yang dirender. `display: contents` tidak punya kotak sendiri tapi isinya tetap tampil,
 * kecuali isinya disembunyikan dengan content-visibility; isi <details> yang tertutup tidak punya
 * kotak dan tidak tampil. */
function rendered(el) {
  const s = getComputedStyle(el);
  if (s.display === "none") return false;
  if (s.display === "contents") return s.contentVisibility !== "hidden";
  return el.checkVisibility ? el.checkVisibility() : true;
}

/** Ada induk (juga di luar shadow root) yang aria-hidden atau inert. */
function hiddenUp(el) {
  for (let e = el; e; e = parentOf(e)) if (e.getAttribute("aria-hidden") === "true" || e.inert) return true;
  return false;
}

/** Anak yang benar-benar dirender: isi shadow root, dan isi slot yang diproyeksikan. */
function childrenOf(node) {
  if (node.shadowRoot) return node.shadowRoot.childNodes;
  if (node.tagName === "SLOT") {
    const assigned = node.assignedNodes({ flatten: true });
    return assigned.length ? assigned : node.childNodes;
  }
  return node.childNodes;
}

const editableRoot = (el) => el.isContentEditable && !parentOf(el)?.isContentEditable;

/** Peran elemen. Input tanpa peran ARIA (password, tanggal, dan sejenisnya) dan area teks yang
 * bisa diedit tetap sebuah isian, juga kalau diberi peran lain (misalnya heading). */
function roleOf(el) {
  const role = getRole(el);
  if (editableRoot(el) && !FIELDS.has(role)) return "textbox";
  if (role) return role;
  if (el.tagName === "INPUT" && !NOT_FIELD_INPUTS.has((el.type || "").toLowerCase())) return "textbox";
  return null;
}

/** Elemen yang dirujuk lewat atribut id (aria-labelledby, aria-describedby). */
function referenced(el, attr) {
  const root = el.getRootNode();
  return (el.getAttribute(attr) || "")
    .split(/\s+/)
    .filter(Boolean)
    .map((id) => root.getElementById?.(id) ?? document.getElementById(id))
    .filter(Boolean);
}

/** Petunjuk nama isian untuk deteksi rahasia, tanpa menghitung nama accessible (yang bisa
 * memuat nilai isian lain). */
/** Teks label beserta alt, aria-label, dan title di dalamnya (label bisa berupa gambar "PIN"). */
function labelHint(node, labelHints) {
  if (!labelHints.has(node)) {
    const parts = [node.textContent, node.getAttribute?.("alt"), node.getAttribute?.("aria-label"), node.getAttribute?.("title")];
    for (const e of node.querySelectorAll?.("[alt], [aria-label], [title]") ?? []) {
      parts.push(e.getAttribute("alt"), e.getAttribute("aria-label"), e.getAttribute("title"));
    }
    labelHints.set(node, parts.filter(Boolean).join(" "));
  }
  return labelHints.get(node);
}
function hintOf(el, labelHints) {
  const parts = [el.getAttribute("aria-label"), el.getAttribute("name"), el.id, el.getAttribute("placeholder"), el.getAttribute("autocomplete"), el.getAttribute("title")];
  for (const r of referenced(el, "aria-labelledby")) parts.push(labelHint(r, labelHints));
  if (el.labels) for (const l of el.labels) parts.push(labelHint(l, labelHints));
  return parts.filter(Boolean).join(" ");
}

/** Isian rahasia: password, isian tersembunyi, isian yang disamarkan atau tak terlihat (misalnya
 * input PIN di balik titik-titik), dan isian yang namanya rahasia. Cek yang murah dulu. */
function isSecret(el, labelHints) {
  if (SECRET_TYPES.has((el.type || "").toLowerCase())) return true;
  if (/one-time-code|password|cc-number|cc-csc/i.test(el.getAttribute("autocomplete") || "")) return true;
  if (SECRET.test(hintOf(el, labelHints))) return true;
  const r = el.getBoundingClientRect();
  if (r.width <= 2 || r.height <= 2) return true;
  const s = getComputedStyle(el);
  if (s.webkitTextSecurity && s.webkitTextSecurity !== "none") return true;
  for (let e = el; e && e !== document.documentElement; e = parentOf(e)) {
    if (Number(getComputedStyle(e).opacity) < 0.1) return true;
  }
  return false;
}

function states(el, role) {
  const s = [];
  const a = (n) => el.getAttribute(n);
  if (el.required || a("aria-required") === "true") s.push("wajib");
  if (a("aria-invalid") && a("aria-invalid") !== "false") s.push("error");
  if (isDisabled(el) || a("aria-disabled") === "true") s.push("nonaktif");
  if (el.readOnly || a("aria-readonly") === "true") s.push("hanya-baca");
  if (a("aria-expanded") === "true") s.push("terbuka");
  if (a("aria-expanded") === "false") s.push("tertutup");
  if (CHECKABLE.has(role)) {
    const c = a("aria-checked") ?? (el.indeterminate ? "mixed" : String(!!el.checked));
    s.push(c === "true" ? "dicentang" : c === "mixed" ? "sebagian" : "tidak dicentang");
  }
  if (a("aria-selected") === "true" || (role === "option" && el.selected)) s.push("dipilih");
  if (a("aria-pressed") === "true") s.push("ditekan");
  if (a("aria-current") && a("aria-current") !== "false") s.push("saat ini");
  return s;
}

function pageContent(scope = document) {
  const labelHints = new WeakMap();
  // Semua shadow host sekali saja: dipakai untuk mencari elemen di dalam shadow root dan untuk
  // tahu blok mana yang berisi shadow root (querySelector biasa tidak menembusnya).
  const roots = [scope];
  const shadowUp = new Set();
  for (let i = 0; i < roots.length; i++) {
    for (const el of roots[i].querySelectorAll("*")) {
      if (!el.shadowRoot) continue;
      roots.push(el.shadowRoot);
      for (let e = el; e && !shadowUp.has(e); e = parentOf(e)) shadowUp.add(e);
    }
  }
  const deepAll = (selector) => {
    const found = [];
    for (const root of roots) for (const el of root.querySelectorAll(selector)) found.push(el);
    return found;
  };

  // Isian rahasia dan semua induknya.
  const secrets = new Set();
  const secretUp = new Set();
  const checked = new Set();
  const fields = deepAll(FIELD_SELECTOR);
  for (let el = scope; el.matches; el = parentOf(el) || document) {
    if (el.matches(FIELD_SELECTOR)) fields.push(el);
  }
  for (const el of fields) {
    checked.add(el);
    if (CHECKABLE.has(roleOf(el)) || !isSecret(el, labelHints)) continue;
    secrets.add(el);
    for (let e = parentOf(el); e && !secretUp.has(e); e = parentOf(e)) secretUp.add(e);
  }
  /** Node ada di dalam isian rahasia (misalnya anak sebuah editor PIN). */
  const insideMemo = new Map();
  const insideSecret = (node) => {
    if (!secrets.size) return false;
    const path = [];
    let found = false;
    for (let e = node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement; e; e = parentOf(e)) {
      if (insideMemo.has(e)) {
        found = insideMemo.get(e);
        break;
      }
      path.push(e);
      if (secrets.has(e)) {
        found = true;
        break;
      }
    }
    // Hasil untuk elemen yang dilewati dicatat, jadi tiap induk cukup dicek sekali.
    for (const e of path) insideMemo.set(e, found);
    return found;
  };
  /** Node adalah, berisi, atau berada di dalam isian rahasia. */
  const touchesSecret = (node) => secrets.has(node) || secretUp.has(node) || insideSecret(node);
  const secretField = (el) => (checked.has(el) ? secrets.has(el) : isSecret(el, labelHints));

  /** Teks yang tampil di dalam node, tanpa isian (rahasia atau bukan), tanpa yang tersembunyi. */
  const safeText = (node, max = MAX_TEXT, inCell = false) => {
    if (insideSecret(node)) return "";
    let out = "";
    const visit = (n) => {
      if (out.length > max) return;
      if (n.nodeType === Node.TEXT_NODE) {
        const holder = n.parentElement || n.parentNode?.host;
        if (!holder || getComputedStyle(holder).visibility === "visible") {
          out += inCell ? n.textContent : ` ${n.textContent}`;
        }
        return;
      }
      if (n.nodeType === Node.ELEMENT_NODE) {
        if (secrets.has(n) || CONTROL_TAGS.has(n.tagName) || SKIP_TAGS.has(n.tagName)) return;
        if (n.getAttribute("aria-hidden") === "true" || n.inert || !rendered(n)) return;
        if (inCell && n.tagName === "IMG" && getComputedStyle(n).visibility === "visible") {
          out += n.alt;
        }
      }
      for (const c of childrenOf(n)) visit(c);
    };
    visit(node);
    return inCell ? out : clean(out, max);
  };
  /** Teks tampil sebuah blok: innerText kalau aman, selain itu dihitung tanpa isian rahasia dan
   * tanpa bagian aria-hidden. */
  const blockText = (el, max = MAX_TEXT) =>
    touchesSecret(el) || el.querySelector('[aria-hidden="true"]') || shadowUp.has(el) ? safeText(el, max) : clean(el.innerText, max);

  const nameOf = (el, role) => {
    const labelledBy = referenced(el, "aria-labelledby");
    const labels = el.labels ? Array.from(el.labels) : [];
    // aria-owns juga ikut dihitung pustaka saat menyusun nama dari isi.
    const owned = referenced(el, "aria-owns");
    if (![el, ...labelledBy, ...labels, ...owned].some(touchesSecret)) return clean(computeAccessibleName(el));
    // Sumber nama memuat isian rahasia: hitung ulang dari teks tanpa isian.
    if (el.getAttribute("aria-label")) return clean(el.getAttribute("aria-label"));
    if (labelledBy.length) return clean(labelledBy.map((r) => safeText(r)).join(" "), MAX_NAME);
    if (labels.length) return clean(labels.map((l) => safeText(l)).join(" "), MAX_NAME);
    const legend = el.tagName === "FIELDSET" ? el.querySelector(":scope > legend") : null;
    if (legend) return safeText(legend, MAX_NAME);
    if (NAME_FROM_CONTENT.has(role)) return clean([el, ...owned].map((n) => safeText(n, MAX_NAME)).join(" "), MAX_NAME);
    if (TABLES.has(role)) {
      const caption = el.querySelector("caption");
      return caption ? safeText(caption, MAX_NAME) : "";
    }
    return clean(el.getAttribute("title"));
  };
  const descOf = (el) => {
    const targets = referenced(el, "aria-describedby");
    if (targets.some(touchesSecret)) return clean(targets.map((t) => safeText(t)).join(" "), MAX_NAME);
    return clean(computeAccessibleDescription(el));
  };

  /** Nilai isian sebagai teks, `null` kalau berupa centang, atau `undefined` kalau rahasia. */
  const fieldValue = (el, role, full = false) => {
    if (CHECKABLE.has(role)) return null;
    if (insideSecret(el) || secretField(el)) return undefined;
    if (el.tagName === "SELECT") return Array.from(el.selectedOptions, (o) => full ? o.text : clean(o.text)).join(", ");
    if (role === "listbox") {
      // Opsi bisa diproyeksikan lewat slot, jadi dicari di pohon yang dirender.
      const picked = [];
      const visit = (n) => {
        if (n.nodeType !== Node.ELEMENT_NODE && n.nodeType !== Node.DOCUMENT_FRAGMENT_NODE) return;
        if (n.getAttribute?.("aria-selected") === "true") {
          picked.push(full ? safeText(n, CELL_BYTES, true) : safeText(n, MAX_NAME));
        }
        else for (const c of childrenOf(n)) visit(c);
      };
      for (const c of childrenOf(el)) visit(c);
      return picked.join(", ");
    }
    if (typeof el.value === "string" && el.tagName !== "BUTTON") return el.value;
    if (el.isContentEditable) return el.innerText;
    return el.getAttribute("aria-valuetext") ?? el.getAttribute("aria-valuenow") ??
      (full ? safeText(el, CELL_BYTES, true) : safeText(el));
  };

  const cellText = (cell, max = MAX_NAME, full = false) => {
    if (insideSecret(cell)) return "(rahasia)";
    const controls = 'input, select, textarea, img[alt], [role="checkbox"], [role="radio"], [role="switch"]';
    const needsWalk = touchesSecret(cell) || shadowUp.has(cell) ||
      cell.querySelector(`[aria-hidden="true"], [inert], ${controls}`);
    const text = needsWalk ? safeText(cell, max, true) : cell.innerText;
    let value = text?.trim() || "";
    if (!value) {
      const input = Array.from(cell.querySelectorAll(`${FIELD_SELECTOR}, ${controls}`))
        .find((i) => i.tagName !== "IMG" && shown(i));
      if (input) {
        const role = roleOf(input);
        const v = fieldValue(input, role, full);
        value = v === undefined ? "(rahasia)" : v === null ? states(input, role).join(" ") : v;
      }
    }
    return full ? value : clean(value, max);
  };
  return { deepAll, shadowUp, touchesSecret, blockText, nameOf, descOf, fieldValue, cellText };
}

function shown(el) {
  return rendered(el) && !hiddenUp(el) && getComputedStyle(el).visibility === "visible";
}

function tableRows(el) {
  const head = [];
  const body = [];
  for (const r of el.querySelectorAll(ROW_SELECTOR)) {
    if (r.closest(TABLE_SELECTOR) !== el || !shown(r)) continue;
    const cells = Array.from(r.querySelectorAll(CELL_SELECTOR))
      .filter((c) => c.closest(ROW_SELECTOR) === r && shown(c));
    const isHead = r.closest("thead") !== null || (cells.length > 0 && cells.every((c) =>
      c.getAttribute("role") === "columnheader" ||
      (c.tagName === "TH" && !c.getAttribute("role") && c.getAttribute("scope") !== "row")));
    (isHead ? head : body).push({ r, cells });
  }
  return { head, body };
}

const TABLE_ROWS = 5000;
const TABLE_COLUMNS = 100;
const TABLE_BYTES = 2_000_000;
const CELL_BYTES = 8000;
const END_WAIT = 850;
/** Grid yang memuat data dari server saat digulir roda (app) butuh waktu lebih lama. */
const WHEEL_WAIT = 2500;
const utf8 = new TextEncoder();

function positiveIndex(el, attr) {
  const value = Number(el.getAttribute(attr));
  return Number.isSafeInteger(value) && value > 0 ? value : null;
}

function clipBytes(text, max) {
  const bytes = utf8.encode(text.slice(0, max));
  if (bytes.length <= max) return text.slice(0, max).toWellFormed();
  let end = max;
  while ((bytes[end] & 0xc0) === 0x80) end -= 1;
  return new TextDecoder().decode(bytes.subarray(0, end));
}

// Nilai sel gabungan tetap di sel asal. Slot lanjutan dicatat agar sel berikutnya
// tidak bergeser ke kiri. Untuk judul, nilai gabungan berlaku di semua kolomnya.
function columnSlots(row, cellText, carried, header, issues) {
  const slots = new Map();
  for (const [i, { value, remaining }] of carried) {
    slots.set(i, header ? value : "");
    if (remaining === 1) carried.delete(i);
    else carried.get(i).remaining -= 1;
  }
  let next = positiveIndex(row.r, "aria-colindex") || 1;
  for (const cell of row.cells) {
    while (slots.has(next)) next += 1;
    const start = positiveIndex(cell, "aria-colindex") ?? next;
    const colSpan = positiveIndex(cell, "aria-colspan") || cell.colSpan || 1;
    const rowSpan = positiveIndex(cell, "aria-rowspan") || cell.rowSpan || 1;
    if (start + colSpan - 1 > TABLE_COLUMNS) issues.add("column_limit");
    if (start > TABLE_COLUMNS) continue;
    const full = cellText(cell, CELL_BYTES, true);
    const value = clipBytes(full, CELL_BYTES);
    if (value !== full) issues.add("cell_limit");
    for (let i = start; i < start + colSpan && i <= TABLE_COLUMNS; i += 1) {
      if (slots.has(i)) {
        issues.add("missing_columns");
        continue;
      }
      slots.set(i, i === start || header ? value : "");
      if (rowSpan > 1) carried.set(i, { value, remaining: rowSpan - 1 });
    }
    next = start + colSpan;
  }
  return slots;
}

function waitForGrid(grid, move, deadline, maxWait) {
  const wait = Math.min(maxWait, deadline - performance.now());
  if (wait <= 0) return Promise.resolve();
  return new Promise((resolve) => {
    let raf;
    let version = 0;
    let previous = -1;
    let quietFrames = 0;
    const observer = new MutationObserver(() => { version += 1; });
    const stop = () => {
      observer.disconnect();
      cancelAnimationFrame(raf);
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", stop);
      resolve();
    };
    const frame = () => {
      quietFrames = version === previous ? quietFrames + 1 : 0;
      previous = version;
      if (!grid.isConnected || (version > 0 && quietFrames >= 2)) stop();
      else raf = requestAnimationFrame(frame);
    };
    const timer = setTimeout(stop, wait);
    observer.observe(grid, { subtree: true, childList: true, characterData: true, attributes: true });
    document.addEventListener("visibilitychange", stop, { once: true });
    move?.();
    raf = requestAnimationFrame(frame);
  });
}

/** Area gulir samping milik grid: elemen di antara baris dan grid (atau induknya) yang isinya
 * lebih lebar dari tampilannya. */
function sideScroller(grid, row) {
  for (let el = row; el; el = parentOf(el)) {
    if (el.clientWidth > 0 && el.scrollWidth > el.clientWidth + 1 &&
        /^(auto|scroll|overlay)$/.test(getComputedStyle(el).overflowX)) return el;
    if (el !== grid && el.contains(grid) && scrollBox(el)) return null;
  }
  return null;
}

function typicalHeight(body) {
  const heights = body.filter(({ cells }) => cells.length).map(({ r }) => r.getBoundingClientRect().height);
  return heights.sort((x, y) => x - y)[heights.length >> 1] ?? 0;
}

/** Tabel HTML yang hanya merender sebagian barisnya: ada ruang kosong di atas baris pertama atau di
 * bawah baris terakhir (baris atau padding pengganti). */
function hasSpacer(grid, body) {
  const boxes = body.filter(({ cells }) => cells.length).map(({ r }) => r.getBoundingClientRect());
  if (boxes.length < 3) return false;
  const typical = typicalHeight(body);
  const rows = boxes.filter((b) => b.height <= typical * 3);
  const box = grid.getBoundingClientRect();
  const headBottom = grid.tHead ? grid.tHead.getBoundingClientRect().bottom : box.top;
  const footTop = grid.tFoot?.rows.length ? grid.tFoot.getBoundingClientRect().top : box.bottom;
  const slack = typical * 2 + (grid.caption?.getBoundingClientRect().height ?? 0);
  return rows[0].top - headBottom > slack || footTop - rows.at(-1).bottom > slack;
}

async function collectTable(grid, url, deadline, allowed) {
  const issues = new Set();
  const rows = new Map();
  const headers = [];
  const title = clipBytes(document.title.trim() || location.hostname, 400);
  const initial = tableRows(grid);
  const rowCount = () => positiveIndex(grid, "aria-rowcount");
  const colCount = () => positiveIndex(grid, "aria-colcount");
  let expected = rowCount();
  const declaredColumns = colCount();
  const virtual = grid.tagName !== "TABLE" || ["grid", "treegrid"].includes(grid.getAttribute("role")) ||
    (grid.hasAttribute("aria-rowcount") && expected !== initial.body.length + initial.head.length) ||
    initial.body.some(({ r }) => getComputedStyle(r).position === "absolute") || hasSpacer(grid, initial.body);
  const scroller = virtual ? scrollerOf(grid) : null;
  const top = scroller?.scrollTop;
  const left = scroller?.scrollLeft;
  const scrollTo = (y) => scroller.scrollTo({ top: y, left: scroller.scrollLeft, behavior: "instant" });
  const bottom = () => scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - 1;
  const running = async () => location.href === url && document.visibilityState === "visible" &&
    grid.isConnected && await allowed(rows.size);
  const needed = Math.min(declaredColumns ?? 0, TABLE_COLUMNS);
  const missesColumns = () => Array.from(rows.values()).some(({ slots }) => slots.size < needed);
  let width = needed;
  let bytes = 0;
  let firstData = initial.head.length + 1; // aria-rowcount juga menghitung baris judul.
  let indexed = true;
  let indexedColumns = true;
  let pending = false; // Ada baris yang semua selnya kosong, biasanya belum dimuat.
  let stopped = false;
  let slowRenders = 0;
  // Grid roda merender baris pengganti kosong di luar layar; yang ditunggu hanya baris terlihat.
  let wheelMode = false;
  /** Pita layar tempat baris benar-benar tampil: grid, layar, dan wadah baris yang memotong isinya. */
  const band = () => {
    const box = grid.getBoundingClientRect();
    let top = Math.max(0, box.top);
    let end = Math.min(innerHeight, box.bottom);
    const row = tableRows(grid).body.find(({ r }) => positiveIndex(r, "aria-rowindex"))?.r;
    for (let el = row && parentOf(row); el && el !== grid; el = parentOf(el)) {
      if (getComputedStyle(el).overflowY === "visible") continue;
      const clip = el.getBoundingClientRect();
      top = Math.max(top, clip.top);
      end = Math.min(end, clip.bottom);
    }
    return { top, end };
  };
  const inView = (el, view = band()) => {
    const box = el.getBoundingClientRect();
    const middle = (box.top + box.bottom) / 2;
    return box.height > 0 && middle > view.top && middle < view.end;
  };
  const shownIndexes = () => {
    const view = band();
    return tableRows(grid).body.filter(({ r }) => inView(r, view)).map(({ r }) => positiveIndex(r, "aria-rowindex")).filter(Boolean);
  };
  if (declaredColumns > TABLE_COLUMNS) issues.add("column_limit");

  function sample({ head, body }) {
    const count = rowCount();
    // Jumlah boleh bertambah (grid yang memuat data saat digulir). Baris yang bergeser terlihat
    // dari baris tumpang tindih yang isinya tidak lagi sama.
    if (expected !== null && count > expected) expected = count;
    else if (count !== expected) issues.add("changed");
    if (colCount() !== declaredColumns) issues.add("changed");
    for (const [i, { r }] of head.entries()) {
      firstData = Math.max(firstData, (positiveIndex(r, "aria-rowindex") ?? i + 1) + 1);
    }
    const { cellText } = pageContent(grid);
    const loose = Array.from(grid.querySelectorAll('[role="columnheader"]'))
      .filter((c) => c.closest(TABLE_SELECTOR) === grid && !c.closest(ROW_SELECTOR) && shown(c));
    const currentHeaders = [];
    const headSpans = new Map();
    for (const h of loose.length ? [...head, { r: grid, cells: loose }] : head) {
      // Tanpa indeks kolom, judul yang terlihat setelah digulir ke samping tidak diketahui letaknya.
      if (sideLeft !== null && h.cells.some((c) => !positiveIndex(c, "aria-colindex"))) continue;
      const slots = columnSlots(h, cellText, headSpans, true, issues);
      width = Math.max(width, ...slots.keys());
      for (const [i, value] of slots) {
        const parts = currentHeaders[i - 1] ??= [];
        if (value && !parts.includes(value)) parts.push(value);
      }
    }
    for (let i = 0; i < currentHeaders.length; i += 1) {
      const value = currentHeaders[i]?.join(" / ");
      if (value === undefined) continue;
      const clipped = clipBytes(value, CELL_BYTES);
      if (clipped !== value) issues.add("cell_limit");
      if (headers[i] !== undefined && headers[i] !== clipped) issues.add("changed");
      headers[i] ??= clipped;
    }

    pending = false;
    const rowSpans = new Map();
    const positions = new Map();
    const portTop = !scroller || scroller === document.scrollingElement ? 0 : scroller.getBoundingClientRect().top;
    const typical = virtual ? typicalHeight(body) : 0;
    const view = wheelMode ? band() : null;
    let previousIndex = null;
    for (const [i, row] of body.entries()) {
      if (performance.now() >= deadline) {
        issues.add("time_limit");
        stopped = true;
        break;
      }
      if (!row.cells.length && !rowSpans.size) continue;
      const index = positiveIndex(row.r, "aria-rowindex") ||
        (row.cells[0] && positiveIndex(row.cells[0], "aria-rowindex")) || null;
      if (!index) indexed = false;
      if (row.cells.some((cell) => !positiveIndex(cell, "aria-colindex"))) indexedColumns = false;
      if (index !== null && previousIndex !== null && index !== previousIndex + 1) rowSpans.clear();
      previousIndex = index;
      const slots = columnSlots(row, cellText, rowSpans, false, issues);
      width = Math.max(width, ...slots.keys());
      const data = JSON.stringify([...slots]);
      const empty = virtual && Array.from(slots.values()).every((value) => value === "");
      // Baris pengganti yang hanya memberi ruang gulir, bukan data.
      if (empty && row.r.getBoundingClientRect().height > typical * 2.5) continue;
      const position = scroller ? Math.round((row.r.getBoundingClientRect().top - portTop + scroller.scrollTop) * 2) : i;
      const ordinal = positions.get(position) ?? 0;
      positions.set(position, ordinal + 1);
      // Posisi dalam area gulir tetap sama saat node grid didaur ulang. Ordinal menjaga
      // baris kembar, termasuk saat dua baris berada pada koordinat yang sama. Baris yang
      // menempel di layar (misalnya total) tidak punya posisi tetap, jadi dikenali dari isinya.
      const sticky = scroller && index === null && /sticky|fixed/.test(getComputedStyle(row.r).position);
      const key = virtual && index !== null ? `i${index}` : sticky ? `s${data}` : `p${position}:${ordinal}`;
      const stored = rows.get(key);
      const awaited = !wheelMode || inView(row.r, view);
      if (stored) {
        // Satu baris bisa dipecah ke beberapa wadah (kolom beku) atau muncul lagi saat digulir ke
        // samping, jadi selnya digabung per kolom. Baris pengganti yang kosong tidak menimpa isi.
        for (const [column, value] of empty && wheelMode ? [] : slots) {
          const old = stored.slots.get(column);
          if (old === value) continue;
          if (old !== undefined && !stored.empty) issues.add("changed");
          stored.slots.set(column, value);
          bytes += utf8.encode(value).length + 3;
        }
        if (!empty) stored.empty = false;
        else if (stored.empty && stored.waited !== true && awaited) pending = true;
        if (bytes > TABLE_BYTES) issues.add("size_limit");
      } else {
        const rowBytes = utf8.encode(data).length + 1;
        if (rows.size === TABLE_ROWS) issues.add("row_limit");
        else if (bytes + rowBytes > TABLE_BYTES) issues.add("size_limit");
        else {
          rows.set(key, { index, position, slots, empty });
          if (empty && awaited) pending = true;
          bytes += rowBytes;
        }
      }
      if (issues.has("row_limit") || issues.has("size_limit")) {
        stopped = true;
        break;
      }
    }
  }

  /** Grid kadang baru merender setelah beberapa frame (mesin sibuk). Ditunggu sampai baris yang
   * dirender menutup tampilan, supaya tidak ada baris yang terlewat. */
  async function settleView() {
    const until = Math.min(deadline, performance.now() + 2000);
    const covered = () => {
      const start = scroller === document.scrollingElement ? 0 : scroller.getBoundingClientRect().top;
      const end = Math.min(start + scroller.clientHeight, start + scroller.scrollHeight - scroller.scrollTop);
      const { head, body } = tableRows(grid);
      // Judul yang ikut di dalam area gulir menutupi bagian atasnya.
      const under = Math.max(start, ...head.map(({ r }) => r.getBoundingClientRect())
        .filter((box) => box.top <= start + 4 && box.bottom < end).map((box) => box.bottom));
      // Baris pengganti ruang dan baris yang menempel selalu terlihat, jadi tidak dihitung.
      const typical = typicalHeight(body);
      const boxes = body.filter(({ r, cells }) => cells.length && !/sticky|fixed/.test(getComputedStyle(r).position))
        .map(({ r }) => r.getBoundingClientRect())
        .filter((box) => box.height <= typical * 2.5 && box.bottom > under && box.top < end);
      return boxes.some((box) => box.top <= under + Math.max(4, box.height / 2)) &&
        boxes.some((box) => box.bottom >= end - Math.max(4, box.height / 2));
    };
    while (slowRenders < 3 && !covered()) {
      if (performance.now() >= until) {
        slowRenders += 1;
        return;
      }
      await waitForGrid(grid, null, deadline, Math.max(1, until - performance.now()));
    }
    if (slowRenders < 3) slowRenders = 0;
  }

  /** Satu lintasan dari atas ke bawah. `settle` menunggu baris yang baru dimuat di ujung bawah. */
  async function sweep(settle) {
    if (scroller.scrollTop !== 0) await waitForGrid(grid, () => scrollTo(0), deadline, 180);
    let endedAt = null;
    await settleView();
    while (await running()) {
      const before = rows.size;
      sample(tableRows(grid));
      for (const until = performance.now() + END_WAIT; pending && !stopped && performance.now() < until;) {
        await waitForGrid(grid, null, deadline, Math.max(1, until - performance.now()));
        if (!await running()) return;
        sample(tableRows(grid));
      }
      // Baris yang tetap kosong tidak ditunggu lagi di langkah berikutnya.
      if (pending) for (const row of rows.values()) if (row.empty) row.waited = true;
      if (stopped) return;
      if (performance.now() >= deadline) {
        issues.add("time_limit");
        stopped = true;
        return;
      }
      if (!bottom()) {
        endedAt = null;
        const from = scroller.scrollTop;
        const step = Math.max(1, Math.floor(scroller.clientHeight * 0.85));
        await waitForGrid(grid, () => scrollTo(from + step), deadline, 180);
        if (scroller.scrollTop <= from) {
          issues.add("missing_rows");
          return;
        }
        await settleView();
        continue;
      }
      if (!settle) return;
      const now = performance.now();
      if (endedAt === null || rows.size !== before) endedAt = now;
      const waited = now - endedAt;
      if (waited >= END_WAIT && (waited >= END_WAIT * 4 || grid.getAttribute("aria-busy") !== "true")) return;
      await waitForGrid(grid, null, deadline, Math.max(1, END_WAIT - waited));
    }
  }

  /** Tabel biasa di dalam kotak gulir bisa menambah baris saat mentok di bawah. */
  async function growStatic(box) {
    const start = box.scrollTop;
    try {
      let count = tableRows(grid).body.length;
      while (await running()) {
        const until = Math.min(deadline, performance.now() + END_WAIT);
        box.scrollTo({ top: box.scrollHeight, left: box.scrollLeft, behavior: "instant" });
        while (performance.now() < until && tableRows(grid).body.length === count) {
          await waitForGrid(grid, null, deadline, Math.max(1, until - performance.now()));
        }
        const now = tableRows(grid).body.length;
        if (now === count) return;
        // Jumlah seluruh baris tidak tertulis di halaman, jadi kelengkapannya tidak pasti.
        issues.add("unknown_rows");
        count = now;
        if (count > TABLE_ROWS) return;
        if (performance.now() >= deadline) {
          issues.add("time_limit");
          return;
        }
      }
    } finally {
      if (box.isConnected) box.scrollTo({ top: start, left: box.scrollLeft, behavior: "instant" });
    }
  }

  const rowIndexes = () => tableRows(grid).body.map(({ r }) => positiveIndex(r, "aria-rowindex")).filter(Boolean);
  const colIndexes = () => tableRows(grid).body.flatMap(({ cells }) => cells.map((c) => positiveIndex(c, "aria-colindex")))
    .filter(Boolean);
  // Grid menyimpan baris cadangan di luar layar, jadi kemajuan diukur dari baris pertama yang terlihat.
  const firstShown = () => Math.min(...shownIndexes());
  const wanted = () => expected - firstData + 1;
  const lastShown = () => Math.max(0, ...shownIndexes());
  const complete = () => rows.size >= wanted() && !Array.from(rows.values()).some((row) => row.empty) &&
    !(needed && missesColumns());
  /** Roda mouse tiruan di baris grid. Tidak menggerakkan kursor, klik, atau memindah fokus. */
  function wheel(dx, dy) {
    const row = tableRows(grid).body.find(({ r }) => r.getBoundingClientRect().height > 0)?.r ?? grid;
    row.dispatchEvent(new WheelEvent("wheel", { deltaX: dx, deltaY: dy, deltaMode: 0, bubbles: true, cancelable: true }));
  }
  const wheelStep = () => {
    const view = band();
    return Math.max(60, Math.floor((view.end - view.top) * 0.85));
  };
  const wheelSide = () => Math.max(120, Math.floor(grid.getBoundingClientRect().width * 0.85));
  /** Kembali ke baris pertama semula, dengan langkah sebesar tinggi baris. */
  async function wheelTo(target) {
    for (let i = 0; i < 6 && Number.isFinite(target) && grid.isConnected && location.href === url; i += 1) {
      const now = firstShown();
      if (!Number.isFinite(now) || now === target) return;
      const height = typicalHeight(tableRows(grid).body) || 24;
      await waitForGrid(grid, () => wheel(0, (target - now) * height), deadline + 3000, 600);
      if (firstShown() === now) return;
    }
  }
  /** Gulir roda sampai ukuran yang dipantau tidak berubah lagi, misalnya baris paling atas. */
  async function wheelUntil(dx, dy, measure, done, extra = 0) {
    for (let i = 0; i < 300 && !done() && grid.isConnected && location.href === url &&
        performance.now() < deadline + extra; i += 1) {
      const before = measure();
      await waitForGrid(grid, () => wheel(dx, dy), deadline + extra, 600);
      if (measure() === before) return;
    }
  }
  /** Grid tanpa area gulir asli (misalnya app) hanya bergerak lewat roda. Baris dikumpulkan per
   * nomor baris dari atas sampai jumlahnya sesuai aria-rowcount. */
  async function wheelSweep() {
    const step = wheelStep();
    await wheelUntil(0, -step * 3, firstShown, () => firstShown() <= firstData);
    let stuck = 0;
    while (await running() && !stopped) {
      sample(tableRows(grid));
      for (const until = performance.now() + WHEEL_WAIT; pending && !stopped && performance.now() < until;) {
        await waitForGrid(grid, null, deadline, Math.max(1, until - performance.now()));
        if (!await running()) return;
        sample(tableRows(grid));
      }
      if (pending) {
        // Hanya baris yang sudah terlihat yang dianggap selesai ditunggu.
        const seen = new Set(shownIndexes());
        for (const row of rows.values()) if (row.empty && seen.has(row.index)) row.waited = true;
      }
      if (stopped || complete() || lastShown() >= expected) return;
      if (performance.now() >= deadline) {
        issues.add("time_limit");
        stopped = true;
        return;
      }
      const before = firstShown();
      const moved = () => Number.isFinite(before) && firstShown() > before;
      await waitForGrid(grid, () => wheel(0, step), deadline, WHEEL_WAIT);
      // Data dari server bisa datang setelah baris pengganti dirender; ditunggu sekali lagi.
      if (!moved()) await waitForGrid(grid, null, deadline, WHEEL_WAIT);
      if (!moved()) {
        stuck += 1;
        if (stuck >= 3) return;
      } else stuck = 0;
    }
  }

  let side = null;
  let sideLeft = null;
  let wheeled = null;
  try {
    if (!scroller) {
      const box = grid.hasAttribute("aria-rowcount") ? null : scrollerOf(grid);
      if (box && box !== document.scrollingElement && box.scrollHeight > box.clientHeight + 1) await growStatic(box);
      sample(tableRows(grid));
    } else {
      await sweep(true);
      // Baris yang terlewat saat grid lambat merender dicoba sekali lagi.
      if (indexed && expected !== null && rows.size < expected - firstData + 1 && !stopped) await sweep(true);
      // Area gulirnya di luar grid dan tidak memuat baris baru: grid ini digulir dengan roda.
      if (indexed && expected !== null && !complete() && !stopped && !grid.contains(scroller) &&
          rowIndexes().length && await running()) {
        wheeled = { top: firstShown(), left: 0, count: rowCount() };
        wheelMode = true;
        // Baris di luar layar belum pernah sempat diisi grid; tunggu lagi saat terlihat.
        for (const row of rows.values()) row.waited = false;
        await wheelSweep();
        // Kolom di kanan yang baru dirender saat digulir ke samping.
        for (let k = 0; k < 30 && indexedColumns && missesColumns() && !stopped && await running(); k += 1) {
          const before = Math.max(0, ...colIndexes());
          await waitForGrid(grid, () => wheel(wheelSide(), 0), deadline, WHEEL_WAIT);
          if (Math.max(0, ...colIndexes()) <= before) break;
          wheeled.left += 1;
          await wheelSweep();
        }
      }
      for (const { r } of tableRows(grid).body) if ((side = sideScroller(grid, r))) break;
      // Tanpa jumlah kolom, kolom di luar layar bisa saja belum dirender.
      if (side && declaredColumns === null) issues.add("missing_columns");
      if (side && indexed && indexedColumns && missesColumns()) {
        sideLeft = side.scrollLeft;
        const step = Math.max(1, Math.floor(side.clientWidth * 0.85));
        const last = side.scrollWidth - side.clientWidth;
        for (let x = 0; !stopped && missesColumns(); x = Math.min(x + step, last)) {
          await waitForGrid(grid, () => side.scrollTo({ left: x, behavior: "instant" }), deadline, 180);
          await sweep(false);
          if (x >= last || !await running()) break;
        }
      }
    }
    if (!await running()) return { status: "unavailable" };
  } finally {
    // Tampilan grid roda dikembalikan ke posisi semula sebisanya.
    // Grid yang jumlah barisnya berkurang (misalnya filter diubah) tidak digulir ke posisi lama.
    // Jumlah yang bertambah wajar: grid memuat data saat digulir.
    if (wheeled && grid.isConnected && location.href === url && rowCount() >= wheeled.count) {
      if (wheeled.left) await wheelUntil(-wheelSide(), 0, () => Math.min(...colIndexes()), () => false, 3000);
      await wheelTo(wheeled.top);
    }
    if (side?.isConnected && sideLeft !== null) side.scrollTo({ left: sideLeft, behavior: "instant" });
    if (scroller?.isConnected && (scroller.scrollTop !== top || scroller.scrollLeft !== left)) {
      scroller.scrollTo({ top, left, behavior: "instant" });
    }
  }

  const ordered = Array.from(rows.values());
  if (virtual && indexed) ordered.sort((a, b) => a.index - b.index);
  else if (virtual) ordered.sort((a, b) => a.position - b.position);
  if (virtual && (!indexed || expected === null)) issues.add("unknown_rows");
  if ((expected !== null && (ordered.length !== expected - firstData + 1 ||
      (indexed && ordered.some((row, i) => row.index !== firstData + i)))) ||
      ordered.some((row) => row.empty) || grid.getAttribute("aria-busy") === "true") issues.add("missing_rows");
  // Kolom tanpa judul yang selalu kosong (misalnya kolom pilih di app) tidak membawa data dan sering
  // tidak dihitung di aria-colcount, jadi dibuang dari hasil.
  const blank = new Set();
  for (let i = 1; i <= width; i += 1) {
    if (!headers[i - 1]?.trim() && ordered.length && ordered.every(({ slots }) => !slots.get(i))) blank.add(i);
  }
  if (declaredColumns !== null && (missesColumns() || width - blank.size > declaredColumns)) issues.add("missing_columns");
  if (virtual && declaredColumns === null && ordered.some(({ slots }) => slots.size !== width)) {
    issues.add("missing_columns");
  }
  if (!width) {
    width = 1;
    if (virtual) issues.add("missing_columns");
  }
  const kept = Array.from({ length: width }, (_, i) => i + 1).filter((i) => !blank.has(i));
  if (!kept.length) kept.push(1);
  const columns = kept.map((i) => headers[i - 1] ?? `Kolom ${i}`);
  const table = { title, columns, rows: [], issues: [] };
  // Reservasi untuk enum alasan dan tanda baca saat baris dan kolom dipadatkan.
  bytes = utf8.encode(JSON.stringify(table)).length + 256;
  for (const { slots } of ordered) {
    const row = kept.map((i) => slots.get(i) ?? "");
    bytes += utf8.encode(JSON.stringify(row)).length + 1;
    if (bytes > TABLE_BYTES) {
      issues.add("size_limit");
      break;
    }
    table.rows.push(row);
  }
  table.issues = Array.from(issues);
  return { status: "table", table };
}

/** Kotak nama Excel Online (alamat sel terpilih) di frame ini, kalau ada. */
const NAME_BOX = 'input#NameBox, input[id*="NameBox" i], input[aria-label="Name Box" i], input[aria-label*="Kotak Nama" i], input[title="Name Box" i]';

/** Bagian halaman yang sedang difokuskan user di frame ini: sheet dan sel aktif (Excel Online),
 * tab atau posisi yang aktif, isian yang sedang diisi, dan teks yang dipilih. Dicatat bersama
 * hasil Baca halaman supaya Bakase tahu bagian mana yang dimaksud user. Nilai isian rahasia
 * tidak pernah diambil. */
export function readFocus() {
  const items = [];
  const labelHints = new WeakMap();
  const name = (el, max = 80) => clean(el.getAttribute("aria-label") || el.innerText || el.textContent || "", max);
  const found = document.querySelector(NAME_BOX);
  const nameBox = found && !isSecret(found, labelHints) ? found : null;
  const tabs = Array.from(document.querySelectorAll('[role="tab"][aria-selected="true"]')).filter(rendered);
  if (nameBox) {
    // Tab ribbon ("Home", "Insert") juga terpilih. Tab sheet ada di daftar tab sheet, atau kalau
    // daftarnya tidak dikenali, tab terpilih terakhir di halaman (bilah sheet ada di bawah grid).
    const sheetList = /sheet|lembar/i;
    const listName = (tab) => {
      const list = tab.closest('[role="tablist"]');
      if (!list) return "";
      const ids = (list.getAttribute("aria-labelledby") || "").split(/\s+/).filter(Boolean);
      return list.getAttribute("aria-label") || ids.map((id) => document.getElementById(id)?.textContent || "").join(" ");
    };
    // Kalau daftar tab sheet tidak dikenali: tab terpilih yang paling bawah di layar.
    const lowest = () => tabs.reduce((low, tab) => (!low || tab.getBoundingClientRect().top > low.getBoundingClientRect().top ? tab : low), null);
    const sheetTab = tabs.find((tab) => sheetList.test(listName(tab))) || lowest();
    const sheet = sheetTab ? name(sheetTab) : "";
    if (sheet) items.push(`sheet aktif ${quote(sheet)}`);
    const cell = (nameBox.value || "").trim();
    if (/^[\p{L}\p{N}_.$:!' ]{1,60}$/u.test(cell)) items.push(`sel dipilih ${cell}`);
  } else {
    for (const tab of tabs.slice(0, 3)) {
      const text = name(tab);
      if (text) items.push(`tab aktif ${quote(text)}`);
    }
  }
  const current = Array.from(document.querySelectorAll('[aria-current]:not([aria-current="false"])')).filter(rendered);
  for (const el of current.slice(0, 2)) {
    const text = name(el);
    if (text) items.push(`posisi aktif ${quote(text)}`);
  }
  const active = document.activeElement;
  const secret = active ? isSecret(active, labelHints) : false;
  if (active && active !== document.body && active !== document.documentElement && active !== nameBox && rendered(active)) {
    const role = roleOf(active);
    const label = clean(active.labels?.[0]?.innerText || active.getAttribute("aria-label") || active.getAttribute("placeholder") || active.getAttribute("title") || "", 80);
    const field = FIELDS.has(role) || CONTROL_TAGS.has(active.tagName) || active.isContentEditable;
    if (field && !secret && !NOT_FIELD_INPUTS.has((active.type || "").toLowerCase())) {
      const value = clean(active.isContentEditable ? active.innerText : active.value || "", 100);
      items.push(`isian aktif${label ? ` ${quote(label)}` : ""}${value ? ` berisi ${quote(value)}` : ""}`);
    } else if (!field && role && label) {
      items.push(`elemen aktif ${role} ${quote(label)}`);
    }
  }
  // Teks pilihan dibuang kalau menyentuh isian rahasia (misalnya pilihan yang melewati kolom PIN).
  // Isian di shadow root juga dicek, dan teks pilihan yang memuat nilai isian rahasia ikut dibuang
  // (pilihan bisa melewati shadow root tanpa terdeteksi containsNode).
  const selection = globalThis.getSelection?.();
  const raw = String(selection ?? "");
  const touchesSecret = raw.trim() !== "" && pageContent().deepAll(FIELD_SELECTOR).some((el) => {
    if (!isSecret(el, labelHints)) return false;
    if (selection.rangeCount > 0 && selection.containsNode(el, true)) return true;
    const value = el.isContentEditable ? el.innerText : el.value;
    return typeof value === "string" && value.trim() !== "" && raw.includes(value.trim());
  });
  const selected = secret || touchesSecret ? "" : clean(raw, 300);
  if (selected) items.push(`teks dipilih ${quote(selected)}`);
  // Potongan teks bisa memisahkan pasangan surrogate (emoji); sisanya tidak bisa dikirim.
  return items.map((item) => item.replace(/[\uD800-\uDFFF]/gu, ""));
}

/** Fokus hanya tambahan: kalau gagal dibaca, snapshot tetap jalan tanpa fokus. */
function safeFocus() {
  try {
    return readFocus();
  } catch {
    return [];
  }
}

export function readPage(prefix = "", base = 0, afterScroll = false) {
  clearPointer();
  const refs = new Map();
  globalThis.__appRefs = refs;
  const refEnds = [];
  let seq = 0;
  let pendingRef;
  const ref = (el) => {
    seq += 1;
    const id = `${prefix}e${base + seq}`;
    pendingRef = id;
    refs.set(id, { node: new WeakRef(el), rowIndex: el.closest(ROW_SELECTOR)?.getAttribute("aria-rowindex") });
    return `[${id}]`;
  };
  const { deepAll, shadowUp, touchesSecret, blockText, nameOf, descOf, fieldValue, cellText } = pageContent();

  const lines = [];
  let used = 0;
  let cut = false;
  const emit = (depth, text) => {
    if (cut) return false;
    const line = `${"  ".repeat(depth)}- ${text}`;
    const id = pendingRef;
    pendingRef = undefined;
    if (used + line.length + 1 > MAX_SNAPSHOT - NOTE.length) {
      if (id) refs.delete(id);
      cut = true;
      return false;
    }
    if (id) refEnds.push([id, used + line.length]);
    lines.push(line);
    used += line.length + 1;
    return true;
  };
  const stats = { fields: 0, actions: 0, tables: 0, rows: 0, messages: 0 };

  // Teks label dan keterangan (misalnya pesan error) yang sudah ikut di baris sebuah isian yang
  // tampil tidak ditulis lagi sebagai teks.
  const labelIds = new Set();
  for (const el of deepAll("[aria-labelledby], [aria-describedby]")) {
    if (!FIELDS.has(roleOf(el)) || !rendered(el)) continue;
    const ids = `${el.getAttribute("aria-labelledby") || ""} ${el.getAttribute("aria-describedby") || ""}`;
    for (const id of ids.split(/\s+/)) if (id) labelIds.add(id);
  }

  // Pesan (error, peringatan, konfirmasi) didahulukan supaya tidak terpotong.
  const shownMessages = new Set();
  for (const el of deepAll('[role="alert"], [role="status"], [aria-live="assertive"]')) {
    if (!rendered(el) || hiddenUp(el)) continue;
    const text = blockText(el);
    if (!text) continue;
    if (emit(0, `${getRole(el) || "pesan"}: ${quote(text)}`)) stats.messages += 1;
    shownMessages.add(el);
  }

  function table(el, role, depth) {
    stats.tables += 1;
    const name = nameOf(el, role);
    const { head, body } = tableRows(el);
    // aria-rowindex dan aria-rowcount ikut menghitung baris judul.
    const rowIndex = (r) => Number(r.getAttribute("aria-rowindex")) || 0;
    let headMax = head.length;
    for (const h of head) headMax = Math.max(headMax, rowIndex(h.r));
    let first = Infinity;
    let last = 0;
    for (const b of body) {
      const n = rowIndex(b.r);
      if (n > 0) {
        first = Math.min(first, n);
        last = Math.max(last, n);
      }
    }
    const total = Number(el.getAttribute("aria-rowcount")) - head.length;
    let range = `${body.length} baris`;
    if (last > 0 && total > 0) range = `baris ${first - headMax}–${last - headMax} dari ${total} terlihat`;
    else if (total > body.length) range = `${body.length} dari ${total} baris terlihat`;
    if (!emit(depth, `${role}${name ? ` ${quote(name)}` : ""} ${ref(el)} ${range}`)) return;
    for (const h of head) emit(depth + 1, `kolom: ${h.cells.map((c) => blockText(c, 60) || "-").join(" | ")}`);
    for (const { r, cells } of body.slice(0, MAX_ROWS)) {
      const n = rowIndex(r) - headMax;
      const st = states(r, "row");
      const label = `baris${n > 0 ? ` ${n}` : ""} ${ref(r)}${st.length ? ` ${st.join(" ")}` : ""}`;
      if (!emit(depth + 1, `${label}: ${cells.map((cell) => cellText(cell)).join(" | ")}`)) return;
      stats.rows += 1;
    }
    if (body.length > MAX_ROWS) emit(depth + 1, `… ${body.length - MAX_ROWS} baris lagi tidak ditampilkan`);
  }

  function element(el, depth) {
    if (cut || SKIP_TAGS.has(el.tagName) || shownMessages.has(el)) return;
    if (el.getAttribute("aria-hidden") === "true" || el.inert) return;
    if (!rendered(el)) return;
    if (el.tagName === "IFRAME") {
      // Isi frame dibaca terpisah oleh panel (semua frame).
      emit(depth, `frame ${quote(clean(el.title || el.name || "tanpa judul"))}`);
      return;
    }
    if (el.tagName === "LABEL" && el.control) {
      // Labelnya sudah jadi nama isian: cukup tulis isiannya kalau ada di dalam label.
      if (el.contains(el.control)) element(el.control, depth);
      return;
    }
    const role = roleOf(el);
    if (el.id && labelIds.has(el.id) && !role && !el.querySelector(MEANINGFUL) && !shadowUp.has(el)) return;
    const visible = getComputedStyle(el).visibility !== "hidden";

    if (visible && el.tagName === "DETAILS" && !el.open) {
      const summary = el.querySelector(":scope > summary");
      emit(depth, `bagian ${quote(summary ? blockText(summary, MAX_NAME) || "detail" : "detail")} ${ref(el)} tertutup`);
      return;
    }
    if (visible && FIELDS.has(role)) {
      const name = nameOf(el, role);
      const value = fieldValue(el, role);
      const st = states(el, role);
      if (value === "") st.push("kosong");
      let text = `${role}${name ? ` ${quote(name)}` : ""} ${ref(el)}`;
      if (st.length) text += ` ${st.join(" ")}`;
      if (value === undefined) text += " = (rahasia, tidak dibaca)";
      else if (value) text += ` = ${quote(clean(value))}`;
      const desc = descOf(el);
      if (desc && desc !== name) text += ` — ${quote(desc)}`;
      if (emit(depth, text)) stats.fields += 1;
      return;
    }
    if (visible && role && ACTIONS.has(role)) {
      const name = nameOf(el, role) || blockText(el, MAX_NAME);
      if (!name) return;
      const st = states(el, role);
      if (emit(depth, `${role} ${quote(name)} ${ref(el)}${st.length ? ` ${st.join(" ")}` : ""}`)) stats.actions += 1;
      return;
    }
    if (visible && role === "heading") {
      const level = el.getAttribute("aria-level") || el.tagName.match(/^H(\d)$/)?.[1];
      emit(depth, `heading ${quote(nameOf(el, role))}${level ? ` [level ${level}]` : ""}`);
      return;
    }
    if (visible && role === "img") {
      const name = nameOf(el, role);
      if (name) emit(depth, `img ${quote(name)}`);
      return;
    }
    if (role && TABLES.has(role)) {
      table(el, role, depth);
      return;
    }
    if (role && MESSAGES.has(role)) {
      const text = blockText(el);
      if (text) emit(depth, `${role}: ${quote(text)}`);
      return;
    }

    let inner = depth;
    if (visible && scrollBox(el)) {
      const name = role ? nameOf(el, role) : "";
      emit(depth, `wadah gulir${name ? ` ${quote(name)}` : ""} ${ref(el)}`);
      inner = depth + 1;
    } else if (visible && role && (ALWAYS.has(role) || IF_NAMED.has(role))) {
      const name = nameOf(el, role);
      if (name || ALWAYS.has(role)) {
        emit(depth, `${role}${name ? ` ${quote(name)}` : ""}`);
        inner = depth + 1;
      }
    }
    // Blok tanpa elemen penting di dalamnya dibaca sebagai satu teks. Blok yang panjang (misalnya
    // banyak paragraf) dipecah per anak supaya tidak terpotong.
    if (!el.shadowRoot && el.tagName !== "SLOT" && !shadowUp.has(el) && !el.querySelector(MEANINGFUL)) {
      const raw = visible ? blockText(el, MAX_TEXT + 1) : "";
      if (raw.length <= MAX_TEXT || !el.children.length) {
        const text = clean(raw, MAX_TEXT);
        if (text) emit(inner, `text: ${quote(text)}`);
        return;
      }
    }
    walk(el, inner);
  }

  function walk(node, depth) {
    for (const child of childrenOf(node)) {
      if (cut) return;
      if (child.nodeType === Node.ELEMENT_NODE) element(child, depth);
      else if (child.nodeType === Node.TEXT_NODE) {
        // Induk teks di shadow root adalah host-nya (`node`).
        const holder = child.parentElement || node;
        const text = clean(child.textContent, MAX_TEXT);
        if (text && holder.nodeType === Node.ELEMENT_NODE && getComputedStyle(holder).visibility !== "hidden") {
          emit(depth, `text: ${quote(text)}`);
        }
      }
    }
  }

  const scope = afterScroll ? globalThis.__appScope?.deref() : null;
  if (scope?.isConnected && scope !== document.scrollingElement) element(scope, 0);
  else if (document.body) walk(document.body, 0);
  const text = lines.join("\n") + (cut ? NOTE : "");
  return {
    title: clean(document.title) || location.hostname,
    url: location.href,
    text,
    refEnds,
    focus: safeFocus(),
    stats: { ...stats, refs: seq, chars: text.length, truncated: cut },
  };
}
