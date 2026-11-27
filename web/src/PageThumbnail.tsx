import { useEffect, useRef, useState } from "react";
import { placeLabel, sourceName, type SourceRef } from "./messages";

/** Lebar kolom jawaban terkecil yang masih nyaman dibagi dua. */
const MIN_COLUMN = 560;

/** Sesudah ini halaman dianggap gagal, supaya antreannya tetap jalan. */
const TIMEOUT_MS = 20_000;

/** Satu halaman dirender dalam satu waktu, supaya riwayat panjang tidak memenuhi jaringan. */
let queue: Promise<unknown> = Promise.resolve();

/** Halaman yang dikutip, di samping awal jawaban. Dimuat saat terlihat; kalau gagal, hilang saja. */
export function PageThumbnail({ source, onOpen }: { source: SourceRef; onOpen: (source: SourceRef) => void }) {
  const card = useRef<HTMLButtonElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [failed, setFailed] = useState(false);
  const { doc_id: docId, page } = source;

  useEffect(() => {
    const frame = card.current;
    const bitmap = canvas.current;
    if (!frame || !bitmap || page === null) return;
    const controller = new AbortController();
    let mounted = true;
    const watcher = new IntersectionObserver(([entry]) => {
      if (!entry.isIntersecting) return;
      watcher.disconnect();
      queue = queue.then(async () => {
        if (!mounted) return;
        const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);
        const stopped = new Promise<never>((_, reject) => {
          controller.signal.addEventListener("abort", () => reject(new Error("aborted")));
        });
        try {
          const { drawPdfThumbnail } = await import("./pdfPage");
          await Promise.race([drawPdfThumbnail(`/api/documents/${docId}/file`, page, bitmap, controller), stopped]);
        } catch {
          if (mounted) setFailed(true);
        } finally {
          clearTimeout(timer);
        }
      });
    }, { rootMargin: "200px" });
    watcher.observe(frame);
    return () => {
      mounted = false;
      watcher.disconnect();
      controller.abort();
    };
  }, [docId, page]);

  // Kolom yang sempit (panel samping, preview terbuka, HP) tidak dibagi dua.
  const [wide, setWide] = useState(false);
  const probe = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!probe.current) return;
    const observer = new ResizeObserver(([entry]) => setWide(entry.contentRect.width >= MIN_COLUMN));
    observer.observe(probe.current);
    return () => observer.disconnect();
  }, []);

  if (failed) return null;
  const name = sourceName(source);
  const place = placeLabel(source.kind, page);
  return (
    <>
      <div ref={probe} aria-hidden="true" />
      <button type="button" ref={card} className="page-thumb" hidden={!wide}
        aria-label={`Buka ${name}${place ? `, ${place}` : ""}`} onClick={() => onOpen(source)}>
        <span className="page-thumb-window" aria-hidden="true"><canvas ref={canvas} width={210} height={297} /></span>
        <span className="page-thumb-caption" aria-hidden="true">{[name, place].filter(Boolean).join(" · ")}</span>
      </button>
    </>
  );
}
