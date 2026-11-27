// Penunjuk sorot: cincin merah yang berdenyut di elemen dan balon "Yang ini" di luarnya, supaya
// isinya tetap terbaca. Keduanya digambar di shadow root tertutup di atas halaman, jadi gaya halaman
// tidak diubah. Gayanya lewat CSSOM dan ikonnya lewat DOM SVG, jadi tidak terhalang CSP halaman.

const RED = "#365a77";
/** Tiga denyut, lalu diam 3 detik, berulang sampai disentuh (pengecualian AGENTS.md poin 4). */
const PULSE_MS = 600;
const PAUSE_MS = 3000;
const GAP = 8;
const EDGE = 8;
/** Jarak ujung panah dari sisi kiri balon. */
const ARROW = 24;

const SVG = "http://www.w3.org/2000/svg";
const NO_STROKE = { stroke: "none" };
const eye = (x) => [
  ["circle", { cx: x, cy: 116, r: 21, fill: "#fff" }],
  ["ellipse", { cx: x, cy: 116, rx: 10, ry: 13, fill: "#188f99", ...NO_STROKE }],
  ["ellipse", { cx: x + 1, cy: 117, rx: 5.5, ry: 8, fill: "#1b2433", ...NO_STROKE }],
  ["circle", { cx: x - 2, cy: 111, r: 3.2, fill: "#fff", ...NO_STROKE }],
];
/** Kepala Bakase dari web/public/brand/app-head.svg, tanpa tulisan kecil di helm. */
const HEAD = [
  ["path", { d: "M15 78H115V142Q115 160 65 160Q15 160 15 142Z", fill: "#dce4e9" }],
  ...eye(42),
  ...eye(88),
  ["path", { d: "M63 114Q65 108 67 114M21 113L14 110M109 113L116 110M57 146Q65 153 73 146", fill: "none" }],
  ["path", { d: "M10 75C11 43 29 25 57 23H73C101 25 119 43 120 75Z", fill: "#fff" }],
  ["path", { d: "M49 54L55 22Q56 17 61 17H69Q74 17 75 22L81 54", fill: "#eef1f4" }],
  ["path", { d: "M12 65L43 65V73H10ZM87 65H118L120 73H87Z", fill: "#365a77", ...NO_STROKE }],
  ["path", { d: "M9 77Q65 72 121 77L125 84Q125 89 119 89H11Q5 89 5 84Z", fill: "#fff" }],
];

function node(name, attributes) {
  const el = document.createElementNS(SVG, name);
  for (const [key, value] of Object.entries(attributes)) el.setAttribute(key, String(value));
  return el;
}

function head() {
  const svg = node("svg", { viewBox: "0 0 148 148", width: 25, height: 25, fill: "none" });
  const group = node("g", {
    transform: "translate(9 -14)", stroke: "#1b2433", "stroke-width": 3.2, "stroke-linecap": "round", "stroke-linejoin": "round",
  });
  group.append(...HEAD.map(([name, attributes]) => node(name, attributes)));
  svg.append(group);
  return svg;
}

const CSS = `
  :host { all: initial; position: fixed; inset: 0 auto auto 0; display: block; margin: 0; padding: 0; border: 0;
    background: none; overflow: visible; pointer-events: none; z-index: 2147483647; }
  .balon { position: fixed; display: flex; align-items: center; gap: 7px; padding: 5px 10px 5px 5px; border: 1px solid #d9d9d9;
    border-radius: 12px; background: #fff; color: #0d0d0d; box-shadow: 0 8px 28px #0003; font: 450 14px/1.4 "Segoe UI", sans-serif;
    white-space: nowrap; pointer-events: auto; cursor: pointer; }
  .balon::before { content: ""; position: absolute; top: -6px; left: ${ARROW - 5}px; width: 9px; height: 9px; transform: rotate(45deg);
    background: #fff; border: solid #d9d9d9; border-width: 1px 0 0 1px; }
  .balon.atas::before { top: auto; bottom: -6px; border-width: 0 1px 1px 0; }
  .cincin { position: fixed; box-sizing: border-box; border: 2px solid ${RED}; border-radius: 4px; pointer-events: none; }
  .cahaya { position: absolute; inset: -2px; border-radius: 6px; box-shadow: 0 0 18px 7px ${RED}cc; opacity: 0.6; }
`;

/** Kunci animasi satu putaran: tiga kali terang-redup, lalu diam sampai putaran berikutnya. */
export function pulseFrames(pulses = 3, pulseMs = PULSE_MS, pauseMs = PAUSE_MS) {
  const total = pulses * pulseMs + pauseMs;
  const frames = [{ opacity: 0, offset: 0 }];
  for (let i = 0; i < pulses; i++) {
    frames.push({ opacity: 1, offset: (i * pulseMs + pulseMs / 2) / total });
    frames.push({ opacity: 0, offset: ((i + 1) * pulseMs) / total });
  }
  frames.push({ opacity: 0, offset: 1 });
  return { frames, total };
}

const BUSY_CSS = `
  :host { all: initial; position: fixed; inset: 0; display: block; margin: 0; padding: 0; border: 0;
    background: none; overflow: visible; pointer-events: none; z-index: 2147483646; }
  .tepi { position: fixed; inset: 0; pointer-events: none;
    box-shadow: inset 0 0 0 2px ${RED}, inset 0 0 26px 6px ${RED}99, inset 0 0 80px 18px ${RED}33; }
  .label { position: fixed; left: 50%; bottom: 14px; transform: translateX(-50%); padding: 5px 12px; border-radius: 999px;
    background: #fff; color: #0d0d0d; border: 1px solid ${RED}66; box-shadow: 0 6px 20px #0003;
    font: 500 13px/1.4 "Segoe UI", sans-serif; white-space: nowrap; }
`;

/**
 * Tanda di tepi layar selama Bakase bekerja di halaman (misalnya mengambil tabel): gradasi merah
 * yang bernapas pelan, diam kalau sistem meminta animasi dikurangi. Mengembalikan fungsi penutup.
 */
export function busy(text) {
  const host = document.createElement("app-sibuk");
  host.setAttribute("aria-hidden", "true");
  const root = host.attachShadow({ mode: "closed" });
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(BUSY_CSS);
  root.adoptedStyleSheets = [sheet];
  const edge = document.createElement("div");
  edge.className = "tepi";
  const label = document.createElement("div");
  label.className = "label";
  label.textContent = text;
  root.append(edge, label);
  document.documentElement.append(host);
  try {
    host.popover = "manual";
    host.showPopover();
  } catch {
    /* Edge lama: cukup z-index */
  }
  const glow = matchMedia("(prefers-reduced-motion: reduce)").matches ? null
    : edge.animate([{ opacity: 0.55 }, { opacity: 1 }], { duration: 1400, iterations: Infinity, direction: "alternate" });
  return () => {
    glow?.cancel();
    host.remove();
  };
}

export function clearPointer() {
  globalThis.__appPointer?.();
}

/**
 * Menunjuk `el` sampai balon atau elemennya disentuh (klik atau ketuk), halaman dibaca lagi, atau
 * elemennya hilang. Saat kursor di atas balon atau elemen, denyutnya berhenti tetapi penunjuk tetap ada.
 */
export function point(el) {
  clearPointer();

  const host = document.createElement("app-penunjuk");
  host.setAttribute("aria-hidden", "true");
  const root = host.attachShadow({ mode: "closed" });
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(CSS);
  root.adoptedStyleSheets = [sheet];
  const balloon = document.createElement("div");
  balloon.className = "balon";
  balloon.title = "Klik untuk menyembunyikan penunjuk";
  balloon.append(head(), "Yang ini");
  const ring = document.createElement("div");
  ring.className = "cincin";
  const glow = document.createElement("div");
  glow.className = "cahaya";
  ring.append(glow);
  root.append(ring, balloon);
  document.documentElement.append(host);
  // Lapisan teratas, supaya tidak tertutup dialog halaman.
  try {
    host.popover = "manual";
    host.showPopover();
  } catch {
    /* Edge lama: cukup z-index */
  }

  let frame = 0;
  let visible = true;
  function place() {
    frame = 0;
    const box = el.getBoundingClientRect();
    if (!el.isConnected || (!box.width && !box.height)) return clear();
    const { offsetWidth: width, offsetHeight: height } = balloon;
    const above = box.bottom + GAP + height > innerHeight && box.top - GAP - height > 0;
    const left = Math.min(Math.max(EDGE, box.left - ARROW / 2), Math.max(EDGE, innerWidth - width - EDGE));
    balloon.classList.toggle("atas", above);
    balloon.style.visibility = ring.style.visibility = visible ? "visible" : "hidden";
    Object.assign(ring.style, { left: `${box.left}px`, top: `${box.top}px`, width: `${box.width}px`, height: `${box.height}px` });
    balloon.style.left = `${left}px`;
    balloon.style.top = `${above ? box.top - GAP - height : Math.min(box.bottom + GAP, innerHeight - height - EDGE)}px`;
  }
  const move = () => {
    frame ||= requestAnimationFrame(place);
  };
  // Juga menangkap elemen yang tergulir di balik wadahnya sendiri, atau dicopot dari halaman.
  const watcher = new IntersectionObserver((entries) => {
    visible = entries.at(-1).isIntersecting;
    move();
  });
  watcher.observe(el);
  // Grid virtual memakai ulang elemen baris untuk data lain; penunjuknya jangan ikut pindah.
  const row = el.closest("[role=row], tr");
  const seen = el.textContent;
  const seenRow = row?.getAttribute("aria-rowindex");
  const reuse = new MutationObserver(() => {
    if (el.textContent !== seen || row?.getAttribute("aria-rowindex") !== seenRow) clear();
  });
  // observe() kedua pada node yang sama mengganti opsi yang pertama.
  const rowChange = { attributeFilter: ["aria-rowindex"] };
  reuse.observe(el, { characterData: true, childList: true, subtree: true, ...(row === el && rowChange) });
  if (row && row !== el) reuse.observe(row, rowChange);
  // Kalau user mematikan animasi di sistem, cahayanya diam tanpa denyut.
  const still = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const { frames, total } = pulseFrames();
  const pulse = still ? null : glow.animate(frames, { duration: total, iterations: Infinity });
  const hold = () => pulse?.pause();
  const resume = () => pulse?.play();
  function clear() {
    cancelAnimationFrame(frame);
    watcher.disconnect();
    reuse.disconnect();
    pulse?.cancel();
    host.remove();
    el.removeEventListener("click", clear, true);
    el.removeEventListener("pointerenter", hold);
    el.removeEventListener("pointerleave", resume);
    window.removeEventListener("scroll", move, true);
    window.removeEventListener("resize", move);
    if (globalThis.__appPointer === clear) globalThis.__appPointer = null;
  }
  balloon.addEventListener("click", (event) => {
    event.stopPropagation();
    clear();
  });
  balloon.addEventListener("pointerenter", hold);
  balloon.addEventListener("pointerleave", resume);
  // Mengklik atau mengetuk elemen yang ditunjuk juga menutup penunjuk; kliknya tetap sampai ke
  // halaman. Geser layar sentuh yang mulai di atas elemen tidak menutupnya.
  el.addEventListener("click", clear, true);
  el.addEventListener("pointerenter", hold);
  el.addEventListener("pointerleave", resume);
  if (el.matches(":hover")) hold();
  window.addEventListener("scroll", move, { capture: true, passive: true });
  window.addEventListener("resize", move);
  globalThis.__appPointer = clear;
  place();
}
