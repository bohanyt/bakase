import { AnnotationMode, GlobalWorkerOptions, PDFDataRangeTransport, TextLayer, getDocument, version, type PDFPageProxy } from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url&no-inline";
import { findPdfHighlight } from "./pdfHighlight";
import "./pdfPage.css";

GlobalWorkerOptions.workerSrc = workerUrl;

const assets = import.meta.env.DEV ? "/node_modules/pdfjs-dist/" : `/assets/pdfjs/${version}/`;

export async function openPdf(url: string, controller: AbortController, fail: () => void) {
  const { signal } = controller;
  const info = await fetch(url, { method: "HEAD", signal });
  const size = Number(info.headers.get("Content-Length"));
  if (!info.ok || !Number.isSafeInteger(size) || size <= 0 || info.headers.get("Accept-Ranges") !== "bytes") {
    throw new Error("PDF range unavailable");
  }
  const validator = info.headers.get("ETag") ?? info.headers.get("Last-Modified");
  signal.throwIfAborted();
  const range = new PDFDataRangeTransport(size, new Uint8Array(), true);
  range.requestDataRange = (begin, end) => {
    const headers = { Range: `bytes=${begin}-${end - 1}`, ...(validator && { "If-Range": validator }) };
    void (async () => {
      try {
        const response = await fetch(url, { headers, signal });
        if (response.status !== 206 || response.headers.get("Content-Range") !== `bytes ${begin}-${end - 1}/${size}`) {
          throw new Error("PDF range changed");
        }
        const bytes = new Uint8Array(await response.arrayBuffer());
        if (bytes.length !== end - begin) throw new Error("Incomplete PDF range");
        if (!signal.aborted) range.onDataRange(begin, bytes);
      } catch {
        if (!signal.aborted) fail();
      }
    })();
  };
  range.abort = () => controller.abort();
  const options = {
    range,
    disableAutoFetch: true,
    disableStream: true,
    isEvalSupported: false,
    enableXfa: false,
    stopAtErrors: true,
    cMapUrl: `${assets}cmaps/`,
    standardFontDataUrl: `${assets}standard_fonts/`,
    wasmUrl: `${assets}wasm/`,
  };
  return getDocument(options);
}

/** Satu halaman sebagai gambar kecil, tanpa lapisan teks. */
export async function drawPdfThumbnail(url: string, number: number, canvas: HTMLCanvasElement, controller: AbortController) {
  const { signal } = controller;
  const task = await openPdf(url, controller, () => controller.abort());
  const stop = () => void task.destroy().catch(() => {});
  signal.addEventListener("abort", stop);
  try {
    const page = await (await task.promise).getPage(number);
    // Lebarnya nol kalau kolom menyempit selama halaman dimuat.
    const width = (canvas.clientWidth || 340) * Math.min(window.devicePixelRatio || 1, 2);
    const viewport = page.getViewport({ scale: width / page.getViewport({ scale: 1 }).width });
    canvas.width = Math.max(1, Math.floor(viewport.width));
    canvas.height = Math.max(1, Math.floor(viewport.height));
    await page.render({ canvas, viewport, annotationMode: AnnotationMode.DISABLE }).promise;
  } finally {
    signal.removeEventListener("abort", stop);
    stop();
  }
}

export function drawPdfPage(page: PDFPageProxy, viewport: ReturnType<PDFPageProxy["getViewport"]>, canvas: HTMLCanvasElement, container: HTMLDivElement) {
  const density = Math.min(window.devicePixelRatio || 1, 2, Math.sqrt(8_000_000 / (viewport.width * viewport.height)));
  canvas.width = Math.max(1, Math.floor(viewport.width * density));
  canvas.height = Math.max(1, Math.floor(viewport.height * density));
  canvas.style.height = `${viewport.height}px`;
  container.style.setProperty("--total-scale-factor", String(viewport.scale * viewport.userUnit));
  container.replaceChildren();
  const render = page.render({
    canvas,
    viewport,
    transform: [canvas.width / viewport.width, 0, 0, canvas.height / viewport.height, 0, 0],
    annotationMode: AnnotationMode.DISABLE,
  });
  const layer = new TextLayer({ textContentSource: page.streamTextContent(), container, viewport });
  return { done: Promise.all([render.promise, layer.render()]), layer, cancel: () => { render.cancel(); layer.cancel(); } };
}

export function markPdfCitation(layer: TextLayer, excerpt: string) {
  if (!CSS.highlights || !excerpt) return;
  const match = findPdfHighlight(layer.textContentItemsStr, excerpt);
  if (!match) return;
  const { start, end } = match;
  const ranges: Range[] = [];
  for (let i = start.item; i <= end.item; i++) {
    const text = layer.textDivs[i]?.firstChild;
    if (!(text instanceof Text)) continue;
    const range = new Range();
    range.setStart(text, i === start.item ? start.offset : 0);
    range.setEnd(text, i === end.item ? end.offset : text.length);
    ranges.push(range);
  }
  const highlight = new Highlight(...ranges);
  CSS.highlights.set("pdf-citation", highlight);
  return () => {
    if (CSS.highlights.get("pdf-citation") === highlight) CSS.highlights.delete("pdf-citation");
  };
}
