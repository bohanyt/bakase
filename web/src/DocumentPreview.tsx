import { CaretLeftIcon, CaretRightIcon, DownloadSimpleIcon } from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";
import { api } from "./api";
import { attentionStatus, isSourceImage, sourcePreviewText, statusNote, type DocumentItem } from "./documents";
import { FileIcon } from "./FileIcon";
import { PreviewPanel } from "./FilePreview";
import { ChunkText, DocumentText } from "./SourcePreview";
import { ImagePreview, PdfPreview } from "./OriginalPreview";

type ChunkPage = {
  total: number;
  from: number;
  limit: number;
  chunks: { chunk_id: string; page: number | null; text: string }[];
};

/** Panel kanan di Sumber Data. */
export function DocumentPreview({
  document: doc,
  width,
  overlay,
  onClose,
}: {
  document: DocumentItem;
  width: number | null;
  overlay: boolean;
  onClose: () => void;
}) {
  // Halaman yang diminta; yang tampil tetap `data.from` sampai jawabannya datang.
  const [from, setFrom] = useState(0);
  // Naik setiap kali pindah halaman, supaya halaman yang sama bisa dicoba lagi setelah gagal.
  const [attempt, setAttempt] = useState(0);
  const [data, setData] = useState<ChunkPage | null>(null);
  const [error, setError] = useState<string | null>(null);
  const viewRef = useRef<HTMLDivElement>(null);
  const readable = doc.available && doc.chunks > 0;
  const url = `/api/documents/${doc.id}/file`;
  const original = doc.available && (doc.kind === "pdf" || isSourceImage(doc.kind));

  useEffect(() => {
    if (!readable) return;
    let stale = false;
    api<ChunkPage>(`/api/documents/${doc.id}/chunks?from=${from}`).then(
      (d) => {
        if (stale) return;
        setData(d);
        setError(null);
        viewRef.current?.scrollTo({ top: 0 });
      },
      (err) => !stale && setError(err instanceof Error ? err.message : "Gagal memuat isi dokumen."),
    );
    return () => {
      stale = true;
    };
  }, [doc.id, from, attempt, readable]);

  function go(next: number) {
    setError(null);
    setFrom(next);
    setAttempt((a) => a + 1);
  }

  const pageTexts = new Map<number | null, string>();
  for (const chunk of data?.chunks ?? []) {
    const previous = pageTexts.get(chunk.page);
    pageTexts.set(chunk.page, previous ? `${previous}\n\n${chunk.text}` : chunk.text);
  }
  const imageText = data?.chunks.map((c) => c.text).join("\n\n");
  const status = attentionStatus(doc.status);
  const pageSize = data?.limit ?? 20;
  const shown = data?.chunks.length ?? 0;
  // Halaman berikutnya sedang dimuat (belum gagal): tombol menunggu.
  const moving = data !== null && data.from !== from && !error;

  return (
    <PreviewPanel
      title={doc.display_title}
      titleHint={doc.title}
      icon={<FileIcon kind={doc.kind} size={22} />}
      actions={
        doc.available && (
          <a className="text-btn" href={url} download aria-label={`Unduh asli ${doc.display_title}`}>
            <DownloadSimpleIcon size={16} /> Unduh asli
          </a>
        )
      }
      width={width}
      overlay={overlay}
      onClose={onClose}
    >
      {original ? doc.kind === "pdf" ? (
        <PdfPreview docId={doc.id} title={doc.display_title} initialPage={1} pageCount={doc.pages} pageTexts={pageTexts} />
      ) : (
        <ImagePreview url={url} title={doc.display_title} fallback={imageText && <DocumentText text={imageText} kind={doc.kind} quoteSource={doc.display_title} />} />
      ) : (
        <>
          {doc.status !== "ok" && doc.status !== "vision" && doc.status !== "archive" && (
            <p className="doc-note">
              <strong>{status?.label ?? "Belum terindeks"}.</strong> {statusNote(doc.status)}
              {!doc.available && " File aslinya juga belum bisa diunduh dari sini."}
            </p>
          )}
          {error && (
            <p className="form-error doc-error">
              {error}{" "}
              <button type="button" className="text-btn" onClick={() => go(from)}>
                Coba lagi
              </button>
            </p>
          )}
          {readable && !data && !error && <div className="loading">Memuat isi…</div>}
          {data && shown === 0 && (
            <p className="doc-note">Teks file ini belum bisa ditampilkan. Unduh file aslinya untuk melihat isinya.</p>
          )}
          {data && shown > 0 && (
            <div className="chunk-view" ref={viewRef}>
              {data.chunks.map((c) => (
                <section key={c.chunk_id} className="doc-chunk">
                  <ChunkText kind={doc.kind} text={sourcePreviewText(c.text, doc.kind, doc.status, doc.sheet_count)} />
                </section>
              ))}
            </div>
          )}
          {data && data.total > pageSize && (
            <div className="doc-pager" aria-busy={moving}>
              <button
                type="button"
                className="text-btn"
                disabled={moving || data.from === 0}
                onClick={() => go(Math.max(0, data.from - pageSize))}
              >
                <CaretLeftIcon size={15} /> Sebelumnya
              </button>
              <span aria-live="polite">
                {moving ? "Memuat…" : `Bagian ${data.from + 1}–${data.from + shown} dari ${data.total}`}
              </span>
              <button
                type="button"
                className="text-btn"
                disabled={moving || data.from + pageSize >= data.total}
                onClick={() => go(data.from + pageSize)}
              >
                Berikutnya <CaretRightIcon size={15} />
              </button>
            </div>
          )}
          {readable && (
            <p className="preview-note">
              {doc.status === "archive"
                ? "Ini teks hasil baca ARCHIVE, karena Bakase tidak bisa membaca file ini sendiri. Tampilan aslinya ada di file asli."
                : "Ini teks yang dibaca Bakase dari file ini. Tampilan aslinya ada di file asli."}
            </p>
          )}
        </>
      )}
    </PreviewPanel>
  );
}
