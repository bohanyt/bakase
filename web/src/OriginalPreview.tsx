import { CaretLeftIcon, CaretRightIcon, MinusIcon, PlusIcon } from "@phosphor-icons/react";
import { Fragment, useContext, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import type { PDFDocumentLoadingTask, PDFDocumentProxy } from "pdfjs-dist";
import { api } from "./api";
import { isMissingPreviewAsset } from "./previewAsset";
import { ReloadAppContext } from "./previewContext";
import "./OriginalPreview.css";

const PAGE_WIDTH = 720;

type Pager = { page: number; total: number; move: (page: number) => void };

function MediaViewer({ pager, waiting, note, onScroll, onFit, children }: {
  pager?: Pager;
  waiting: boolean;
  note?: ReactNode;
  onScroll?: () => void;
  onFit?: () => void;
  children: ReactNode;
}) {
  const [zoom, setZoom] = useState(100);
  const viewport = useRef<HTMLDivElement>(null);

  function move(page: number) {
    pager?.move(page);
    viewport.current?.scrollTo(0, 0);
  }

  return (
    <div className="original-viewer" role="region" aria-label="Tampilan sumber">
      <div className="original-toolbar">
        {pager && (
          <div className="original-pager">
            <button type="button" className="icon-btn" aria-label="Halaman sebelumnya" title="Halaman sebelumnya"
              disabled={waiting || pager.page <= 1} onClick={() => move(pager.page - 1)}>
              <CaretLeftIcon size={16} />
            </button>
            <span aria-live="polite">hal. {pager.page} / {pager.total || "…"}</span>
            <button type="button" className="icon-btn" aria-label="Halaman berikutnya" title="Halaman berikutnya"
              disabled={waiting || pager.page >= pager.total} onClick={() => move(pager.page + 1)}>
              <CaretRightIcon size={16} />
            </button>
          </div>
        )}
        <div className="original-zoom" role="group" aria-label="Ukuran tampilan">
          <button type="button" className="text-btn" aria-pressed={zoom === 100} disabled={waiting} onClick={() => {
            setZoom(100);
            if (viewport.current) viewport.current.scrollLeft = 0;
            onFit?.();
          }}>Pas lebar</button>
          <button type="button" className="icon-btn" aria-label="Perkecil" title="Perkecil"
            disabled={waiting || zoom <= 75} onClick={() => setZoom(zoom - 25)}>
            <MinusIcon size={16} />
          </button>
          <span aria-live="polite">{zoom}%</span>
          <button type="button" className="icon-btn" aria-label="Perbesar" title="Perbesar"
            disabled={waiting || zoom >= 200} onClick={() => setZoom(zoom + 25)}>
            <PlusIcon size={16} />
          </button>
        </div>
      </div>
      {note}
      <div className="original-viewport" ref={viewport} tabIndex={0} aria-label="Area halaman" aria-busy={waiting}
        onScrollCapture={onScroll} onWheelCapture={onScroll} onTouchMoveCapture={onScroll}>
        <div className="original-stage">
          <div className="original-media" style={{ width: `${zoom}%` }}>{children}</div>
        </div>
      </div>
    </div>
  );
}

function PreviewFailure({ retry, asset }: { retry: () => void; asset?: boolean }) {
  const reloadApp = useContext(ReloadAppContext);
  return (
    <p className="preview-note original-failure">
      <span role="status">{asset ? "Pemuat PDF belum bisa dimuat." : "Halaman belum bisa dimuat."}</span>
      <button type="button" onClick={asset ? reloadApp : retry}>
        {asset ? "Muat ulang aplikasi" : "Coba lagi"}
      </button>
    </p>
  );
}

type Pdf = { document: PDFDocumentProxy; api: typeof import("./pdfPage") };
type Failure = "page" | "asset" | null;
type TextChunks = { chunks: { page: number | null; text: string }[] };

function PdfPaper({ pdf, page, excerpt, quoteSource, ready, aspect, setAspect, setReady, fail, onShown }: {
  pdf: Pdf | null;
  page: number;
  excerpt?: string;
  quoteSource: string;
  ready: boolean;
  aspect?: number;
  setAspect: (ratio: number) => void;
  setReady: (value: boolean) => void;
  fail: (value: Failure) => void;
  onShown: () => void;
}) {
  const frame = useRef<HTMLDivElement>(null);
  const sheet = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const layer = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const outer = frame.current;
    const inner = sheet.current;
    if (!outer || !inner || !aspect) return;
    const fit = ({ width, height }: { width: number; height: number }) => {
      inner.style.zoom = String(Math.min(width, height * aspect) / PAGE_WIDTH);
    };
    fit(outer.getBoundingClientRect());
    const observer = new ResizeObserver(([entry]) => fit(entry.contentRect));
    observer.observe(outer);
    return () => observer.disconnect();
  }, [aspect]);

  useEffect(() => {
    const bitmap = canvas.current;
    const text = layer.current;
    if (!pdf || !bitmap || !text) return;
    let stale = false;
    let task: ReturnType<Pdf["api"]["drawPdfPage"]> | undefined;
    let clearMark: (() => void) | undefined;
    void (async () => {
      try {
        const original = await pdf.document.getPage(page);
        if (stale) return;
        const natural = original.getViewport({ scale: 1 });
        const viewport = original.getViewport({ scale: PAGE_WIDTH / natural.width });
        setAspect(viewport.width / viewport.height);
        task = pdf.api.drawPdfPage(original, viewport, bitmap, text);
        await task.done;
        if (stale) return;
        if (excerpt) clearMark = pdf.api.markPdfCitation(task.layer, excerpt);
        setReady(true);
      } catch {
        if (!stale) fail("page");
      }
    })();
    return () => {
      stale = true;
      task?.cancel();
      clearMark?.();
      if (text.contains(window.getSelection()?.anchorNode ?? null)) window.getSelection()?.removeAllRanges();
    };
  }, [pdf, page, excerpt, setAspect, setReady, fail]);

  return (
    <div className="original-paper" ref={frame} inert={!ready}
      onTransitionEnd={(event) => {
        if (ready && event.target === event.currentTarget && event.propertyName === "opacity") onShown();
      }}>
      <div className="original-sheet" ref={sheet} style={{ width: PAGE_WIDTH }}>
        <canvas ref={canvas} aria-label={`Halaman ${page}`} role="img" />
        <div ref={layer} className="pdf-text-layer" data-quote-source={quoteSource} />
      </div>
    </div>
  );
}

export function PdfPreview({ docId, initialPage, pageCount, excerpt, pageTexts, documentText, title, failureFallback }: {
  docId: number;
  initialPage: number;
  pageCount: number;
  excerpt?: string;
  pageTexts?: ReadonlyMap<number | null, string>;
  documentText?: string;
  title: string;
  failureFallback?: ReactNode;
}) {
  const [page, setPage] = useState(initialPage);
  const [attempt, setAttempt] = useState(0);
  const [pdf, setPdf] = useState<Pdf | null>(null);
  const [failure, setFailure] = useState<Failure>(null);
  const [ready, setReady] = useState(false);
  const [aspect, setAspect] = useState<number>();
  const [frameAspect, setFrameAspect] = useState<number>();
  const [total, setTotal] = useState(pageCount);
  const [missingCitation, setMissingCitation] = useState(false);
  const [fallback, setFallback] = useState<{ page: number; text: string }>();
  const [unpagedFallback, setUnpagedFallback] = useState("");
  const scrolled = useRef(false);
  const shown = useRef(false);
  const excerptRegion = useRef<HTMLDivElement>(null);
  const citation = !missingCitation && page === initialPage ? excerpt : undefined;
  const indexedText = citation || pageTexts?.get(page);
  const pageText = indexedText || (fallback?.page === page ? fallback.text : undefined);
  const generalText = documentText || pageTexts?.get(null) || unpagedFallback;
  const showingGeneral = !!(failure && !pageText && generalText);
  const loadingText = showingGeneral ? generalText : pageText;
  const quoteSource = showingGeneral ? title : `${title} · hal. ${page}`;

  useEffect(() => {
    let stale = false;
    let task: PDFDocumentLoadingTask | undefined;
    const controller = new AbortController();
    const stop = () => {
      controller.abort();
      void task?.destroy().catch(() => {});
      task = undefined;
    };
    const failSession = (reason: Failure) => {
      if (stale) return;
      stale = true;
      stop();
      setPdf(null);
      setReady(false);
      setFailure(reason);
    };
    void (async () => {
      try {
        const api = await import("./pdfPage").catch(async (error: unknown) => {
          failSession(await isMissingPreviewAsset(error, controller.signal) ? "asset" : "page");
          return null;
        });
        if (!api || stale) return;
        task = await api.openPdf(`/api/documents/${docId}/file`, controller, () => failSession("page"));
        if (stale) {
          stop();
          return;
        }
        const document = await task.promise;
        if (stale || controller.signal.aborted) return;
        setMissingCitation(initialPage > document.numPages);
        setPage((current) => current > document.numPages ? 1 : current);
        setTotal(document.numPages);
        setPdf({ document, api });
      } catch {
        failSession("page");
      }
    })();
    return () => {
      stale = true;
      stop();
    };
  }, [docId, attempt, initialPage]);

  useEffect(() => {
    if (!failure || failureFallback || pageText || generalText) return;
    let stale = false;
    const url = `/api/documents/${docId}/chunks`;
    void (async () => {
      const result = await api<TextChunks>(`${url}?page=${page}`).catch(() => null);
      if (stale) return;
      const text = result?.chunks.map((chunk) => chunk.text).join("\n\n");
      if (text) setFallback({ page, text });
      else {
        const document = await api<TextChunks>(url).catch(() => null);
        if (!stale && document) setUnpagedFallback(document.chunks.filter((chunk) => chunk.page === null).map((chunk) => chunk.text).join("\n\n"));
      }
    })();
    return () => { stale = true; };
  }, [docId, page, failure, failureFallback, pageText, generalText]);

  useEffect(() => {
    const selection = window.getSelection();
    if (ready && excerptRegion.current?.contains(selection?.anchorNode ?? null)) selection?.removeAllRanges();
    if (ready && window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      shown.current = true;
      if (!scrolled.current) setFrameAspect(aspect);
    }
  }, [ready, aspect]);

  function retry() {
    setPdf(null);
    setReady(false);
    setFailure(null);
    shown.current = false;
    scrolled.current = false;
    setAttempt((current) => current + 1);
  }

  function move(next: number) {
    setReady(false);
    setFailure(null);
    shown.current = false;
    scrolled.current = false;
    setPage(next);
    if (!pdf) retry();
  }

  return (
    <MediaViewer waiting={!ready && !failure} pager={{ page, total, move }}
      onScroll={() => { scrolled.current = true; }} onFit={() => { if (shown.current) setFrameAspect(aspect); }}
      note={<>
        {missingCitation && <p className="preview-note original-notice" role="status">Halaman kutipan sudah tidak ada.</p>}
        {failure && <PreviewFailure retry={retry} asset={failure === "asset"} />}
      </>}>
      {failure && failureFallback ? failureFallback : <div className="original-stack" data-ready={ready} style={{ aspectRatio: frameAspect }}>
        <div className="original-excerpt" ref={excerptRegion} inert={ready || !loadingText}>
          {showingGeneral && <p className="original-text-label">Teks dokumen</p>}
          <div data-quote-source={quoteSource}>
            {!failure && citation && loadingText ? loadingText.split("\n").map((line, i) => (
              <Fragment key={i}>{i > 0 && "\n"}{line.trim() ? <mark>{line}</mark> : line}</Fragment>
            )) : loadingText}
          </div>
        </div>
        {!failure && <PdfPaper key={page} pdf={pdf} page={page} excerpt={citation} quoteSource={`${title} · hal. ${page}`}
          ready={ready} aspect={aspect} setAspect={setAspect} setReady={setReady} fail={setFailure} onShown={() => {
            shown.current = true;
            if (!scrolled.current) setFrameAspect(aspect);
          }} />}
      </div>}
    </MediaViewer>
  );
}

export function ImagePreview({ url, title, fallback }: { url: string; title: string; fallback?: ReactNode }) {
  const [state, setState] = useState<"waiting" | "ready" | "failed">("waiting");
  if (state === "failed") return <><PreviewFailure retry={() => setState("waiting")} />{fallback}</>;
  return (
    <MediaViewer waiting={state === "waiting"}>
      <img className="original-image" src={url} alt={title} draggable={false}
        onLoad={() => setState("ready")} onError={() => setState("failed")} />
    </MediaViewer>
  );
}
