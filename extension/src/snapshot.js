// Snapshot halaman untuk Bakase: struktur accessibility dari bagian yang dirender, seperti
// yang dibaca pembaca layar. Peran dan nama elemen dihitung dengan dom-accessibility-api (standar
// accname W3C), jadi cara bacanya sama untuk semua web app.
//
// Pembacaan tidak mengubah halaman. Ref (e1, e2, ...; frame lain f<id>e1, ...) disimpan di
// dunia terpisah milik extension. Hanya aksi gulir dan sorot yang dapat menggeser tampilan.
//
// Nilai isian rahasia tidak pernah ikut terbaca. Isian rahasia dan semua induknya ditandai dulu;
// nama, keterangan, dan teks yang sumbernya menyentuh isian rahasia dihitung ulang dari teks
// halaman tanpa isian itu, bukan dengan mencocokkan string.
//
// Dibundel jadi extension/lib/snapshot.js dengan `npm run build:extension` di web/.
import { computeAccessibleDescription, computeAccessibleName, getRole, isDisabled } from "dom-accessibility-api";

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
  if (!Number.isFinite(remaining_ms) || remaining_ms <= 0 || remaining_ms > 8000) {
    return { status: "timeout" };
  }
  const localDeadline = receivedAt + remaining_ms;
  // Izin dicek saat skrip mulai berjalan karena injeksi bisa tertahan di antrean.
  const check = (type) => chrome.runtime.sendMessage({ type, id, pageId }).catch(() => false);
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
  if (action === "sorot") {
    reveal(el);
    globalThis.__appOutline?.cancel();
    // Web Animations menggambar outline tanpa menyisipkan DOM, memindah fokus, atau klik.
    const marker = { outline: "3px solid #0078d4", outlineOffset: "-3px" };
    const outline = el.animate([marker, marker], { duration: 2500, fill: "none" });
    globalThis.__appOutline = outline;
    outline.onfinish = () => {
      outline.cancel();
      if (globalThis.__appOutline === outline) globalThis.__appOutline = null;
    };
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

export function readPage(prefix = "", base = 0, afterScroll = false) {
  const labelHints = new WeakMap();
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

  // Semua shadow host sekali saja: dipakai untuk mencari elemen di dalam shadow root dan untuk
  // tahu blok mana yang berisi shadow root (querySelector biasa tidak menembusnya).
  const roots = [document];
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
  for (const el of deepAll(FIELD_SELECTOR)) {
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
  const safeText = (node, max = MAX_TEXT) => {
    if (insideSecret(node)) return "";
    let out = "";
    const visit = (n) => {
      if (out.length > max) return;
      if (n.nodeType === Node.TEXT_NODE) {
        const holder = n.parentElement || n.parentNode?.host;
        if (!holder || getComputedStyle(holder).visibility !== "hidden") out += ` ${n.textContent}`;
        return;
      }
      if (n.nodeType === Node.ELEMENT_NODE) {
        if (secrets.has(n) || CONTROL_TAGS.has(n.tagName) || SKIP_TAGS.has(n.tagName)) return;
        if (n.getAttribute("aria-hidden") === "true" || n.inert || !rendered(n)) return;
      }
      for (const c of childrenOf(n)) visit(c);
    };
    visit(node);
    return clean(out, max);
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
  const fieldValue = (el, role) => {
    if (CHECKABLE.has(role)) return null;
    if (secretField(el)) return undefined;
    if (el.tagName === "SELECT") return Array.from(el.selectedOptions, (o) => clean(o.text)).join(", ");
    if (role === "listbox") {
      // Opsi bisa diproyeksikan lewat slot, jadi dicari di pohon yang dirender.
      const picked = [];
      const visit = (n) => {
        if (n.nodeType !== Node.ELEMENT_NODE && n.nodeType !== Node.DOCUMENT_FRAGMENT_NODE) return;
        if (n.getAttribute?.("aria-selected") === "true") picked.push(safeText(n, MAX_NAME));
        else for (const c of childrenOf(n)) visit(c);
      };
      for (const c of childrenOf(el)) visit(c);
      return picked.join(", ");
    }
    if (typeof el.value === "string" && el.tagName !== "BUTTON") return el.value;
    if (el.isContentEditable) return el.innerText;
    return el.getAttribute("aria-valuetext") ?? el.getAttribute("aria-valuenow") ?? safeText(el);
  };

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

  const cellText = (cell) => {
    const text = blockText(cell, MAX_NAME);
    if (text) return text;
    const input = Array.from(cell.querySelectorAll(FIELD_SELECTOR)).find((i) => rendered(i) && !i.closest('[aria-hidden="true"]'));
    if (!input) return "";
    const role = roleOf(input);
    const v = fieldValue(input, role);
    if (v === undefined) return "(rahasia)";
    return v === null ? states(input, role).join(" ") : clean(v);
  };

  function table(el, role, depth) {
    stats.tables += 1;
    const name = nameOf(el, role);
    const owner = (r) => r.closest('[role="grid"], [role="treegrid"], [role="table"], table');
    const cellsOf = (r) =>
      Array.from(r.querySelectorAll('[role="gridcell"], [role="cell"], [role="columnheader"], [role="rowheader"], td, th')).filter(
        (c) => c.closest('[role="row"], tr') === r && rendered(c) && !c.closest('[aria-hidden="true"]'),
      );
    const columnHeader = (c) =>
      c.getAttribute("role") === "columnheader" || (c.tagName === "TH" && !c.getAttribute("role") && c.getAttribute("scope") !== "row");
    const head = [];
    const body = [];
    for (const r of el.querySelectorAll('[role="row"], tr')) {
      if (owner(r) !== el || !rendered(r) || r.closest('[aria-hidden="true"]')) continue;
      const cells = cellsOf(r);
      const isHead = r.closest("thead") !== null || (cells.length > 0 && cells.every(columnHeader));
      (isHead ? head : body).push({ r, cells });
    }
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
      if (!emit(depth + 1, `${label}: ${cells.map(cellText).join(" | ")}`)) return;
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
    stats: { ...stats, refs: seq, chars: text.length, truncated: cut },
  };
}
