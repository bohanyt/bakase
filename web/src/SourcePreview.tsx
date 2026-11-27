import { DownloadSimpleIcon } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import { Streamdown } from "streamdown";
import { api } from "./api";
import { BlockedImage } from "./BlockedImage";
import { isSourceImage, sourcePreviewPage, sourcePreviewText, statusNote, type DocumentItem } from "./documents";
import { FileIcon } from "./FileIcon";
import { ImageSourceThumb } from "./ImageGallery";
import { DOCUMENT_REMARK_PLUGINS } from "./markdownText";
import { PreviewPanel } from "./FilePreview";
import { kindLabel, placeLabel, sourceName, type SourceRef } from "./messages";
import { ImagePreview, PdfPreview } from "./OriginalPreview";

type Chunk = {
  page: number | null;
  text: string;
  document: Pick<DocumentItem, "pages" | "status" | "sheet_count">;
};

/** Spreadsheet dan dokumen Office dibaca sebagai markdown (tabel, judul). */
const MARKDOWN_KINDS = new Set(["xlsx", "xls", "xlsm", "docx", "doc", "pptx", "ppt", "md"]);
const PREVIEW_COMPONENTS = { img: BlockedImage };

const TABLE_SEPARATOR = /^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)*\|?\s*$/;

/** PDF, teks, dan hasil bacaan foto: format markdown (tebal, daftar, tabel) ditampilkan, dan
 * setiap baris asli tetap jadi baris baru. Baris tabel markdown (blok baris `|` yang memuat
 * baris pemisah `|---|`) tidak diberi jeda supaya tabelnya utuh. */
function keepLines(text: string): string {
  const lines = text.split("\n");
  const inTable = new Array<boolean>(lines.length).fill(false);
  lines.forEach((line, i) => {
    if (!TABLE_SEPARATOR.test(line) || !line.includes("-")) return;
    let start = i;
    while (start > 0 && lines[start - 1].trimStart().startsWith("|")) start--;
    let end = i;
    while (end + 1 < lines.length && lines[end + 1].trimStart().startsWith("|")) end++;
    for (let j = start; j <= end; j++) inTable[j] = true;
  });
  return lines.map((line, i) => (line.trim() === "" || inTable[i] ? line : `${line}  `)).join("\n");
}

/** Teks satu potongan dokumen, dengan format markdown yang aman. */
export function ChunkText({ kind, text }: { kind: string; text: string }) {
  return (
    <div className="md">
      <Streamdown
        mode="static"
        tableMaxHeight="none"
        components={PREVIEW_COMPONENTS}
        // Isi dokumen tidak pernah dibaca sebagai HTML: `<Review-01>` tetap tampil apa adanya.
        remarkPlugins={DOCUMENT_REMARK_PLUGINS}
        controls={{ table: false, code: false, mermaid: false }}
      >
        {MARKDOWN_KINDS.has(kind) ? text : keepLines(text)}
      </Streamdown>
    </div>
  );
}

/** Teks dokumen yang bisa diblok lalu dikutip: potongan sumber, atau isi lampiran dari user. */
export function DocumentText({ text, kind, quoteSource }: { text: string; kind: string; quoteSource: string }) {
  return (
    <div className="chunk-view" data-quote-source={quoteSource}>
      <ChunkText kind={kind} text={text} />
    </div>
  );
}

/** Panel kanan untuk dokumen yang dikutip jawaban. */
export function SourcePreview({
  source,
  originalPage,
  width,
  overlay,
  onClose,
}: {
  source: SourceRef;
  originalPage?: number;
  width: number | null;
  overlay: boolean;
  onClose: () => void;
}) {
  const [data, setData] = useState<Chunk | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    if (source.image) return;
    let stale = false;
    api<Chunk>(`/api/documents/chunks/${encodeURIComponent(source.chunk_id)}`).then(
      (d) => !stale && setData(d),
      (err) => !stale && setError(err instanceof Error ? err.message : "Gagal memuat dokumen."),
    );
    return () => {
      stale = true;
    };
  }, [source.chunk_id, source.image, attempt]);

  const title = sourceName(source);
  const page = originalPage ?? sourcePreviewPage(source.page, data?.page);
  const pdf = source.kind === "pdf" && (originalPage != null || source.image || data || (error && source.doc_id > 0));
  const url = `/api/documents/${source.doc_id}/file`;
  const quoteSource = [title, placeLabel(source.kind, page)].filter(Boolean).join(" · ");
  const text = data?.text ? (
    <DocumentText text={sourcePreviewText(data.text, source.kind, data.document.status, data.document.sheet_count)}
      kind={source.kind} quoteSource={quoteSource} />
  ) : null;
  const thumbnail = source.image ? (
    <div className="source-image-location"><ImageSourceThumb source={source} /></div>
  ) : undefined;

  return (
    <PreviewPanel
      title={title}
      titleHint={source.title}
      icon={<FileIcon kind={source.kind} size={22} />}
      actions={
        <a className="text-btn" href={url} download>
          <DownloadSimpleIcon size={16} /> Unduh asli
        </a>
      }
      width={width}
      overlay={overlay}
      onClose={onClose}
    >
      {source.image && (
        <p className="source-meta">{[`Sumber ${source.source}`, kindLabel(source.kind), source.image.place].filter(Boolean).join(" · ")}</p>
      )}
      {pdf ? (
        <PdfPreview docId={source.doc_id} title={title} initialPage={page ?? 1} pageCount={data?.document.pages ?? 0}
          excerpt={data?.page != null && data.page === page ? data.text : undefined}
          documentText={data?.page === null ? data.text : undefined} failureFallback={thumbnail} />
      ) : thumbnail && originalPage == null ? thumbnail : isSourceImage(source.kind) ? (
        <ImagePreview url={url} title={title} fallback={text} />
      ) : !data && !error ? (
        <div className="loading">Memuat potongan…</div>
      ) : error ? (
        <p className="preview-note original-failure">
          <span role="status">{error}</span>
          <button type="button" onClick={() => { setError(null); setAttempt((n) => n + 1); }}>Coba lagi</button>
        </p>
      ) : text ? (
        <>
          {text}
          <p className="preview-note">Ini potongan yang dibaca Bakase. Isi lengkapnya ada di dokumen asli.</p>
        </>
      ) : data && <p className="doc-note">{statusNote(data.document.status)}</p>}
    </PreviewPanel>
  );
}
