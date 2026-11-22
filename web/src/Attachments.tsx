import { WarningCircleIcon, XIcon } from "@phosphor-icons/react";
import { useCallback, useContext, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { isImageKind } from "./attachment";
import type { AttachmentDraft, Draft } from "./attachmentDraft";
import { FileIcon } from "./FileIcon";
import { formatSize, kindLabel, type FileRef } from "./messages";
import { PreviewContext } from "./previewContext";

/** Penampil gambar layar penuh. Esc, klik di luar gambar, atau tombol × menutupnya. */
export function ImageViewer({ src, name, onClose }: { src: string; name: string; onClose: () => void }) {
  const closeRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    // Halaman di belakang tidak bisa difokus atau diklik selama gambar terbuka; penampil ada
    // di luar #root (portal), jadi tetap aktif.
    const root = document.getElementById("root");
    root?.setAttribute("inert", "");
    closeRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      } else if (e.key === "Tab") {
        // Tombol × satu-satunya yang bisa difokus di penampil.
        e.preventDefault();
        closeRef.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      root?.removeAttribute("inert");
      before?.focus?.();
    };
  }, [onClose]);
  return createPortal(
    <div className="overlay image-viewer" role="dialog" aria-modal="true" aria-label={name} onClick={onClose}>
      <figure onClick={(e) => e.stopPropagation()}>
        <img src={src} alt={name} />
        <figcaption>{name}</figcaption>
      </figure>
      <button ref={closeRef} type="button" className="icon-btn image-viewer-close" aria-label="Tutup gambar" onClick={onClose}>
        <XIcon size={20} />
      </button>
    </div>,
    document.body,
  );
}

type Viewing = { src: string; name: string } | null;

/** Lampiran di atas kotak tanya: thumbnail gambar, atau ikon, nama, dan ukuran file. */
export function AttachmentChips({ draft, onDone }: { draft: AttachmentDraft; onDone: () => void }) {
  const openPreview = useContext(PreviewContext);
  const [viewing, setViewing] = useState<Viewing>(null);
  const closeViewer = useCallback(() => setViewing(null), []);
  if (!draft.items.length && !draft.notice) return null;
  const errors = draft.items.filter((d) => d.status === "error");

  function open(d: Draft) {
    if (d.url) setViewing({ src: d.url, name: d.name });
    else if (d.ref) openPreview({ type: "file", file: d.ref });
  }

  return (
    <>
      {draft.items.length > 0 && (
        <ul className="attachments" aria-label="Lampiran untuk pesan ini">
          {draft.items.map((d) => {
            const image = !!d.url;
            const state = d.status === "uploading" ? `mengunggah ${Math.round(d.progress * 100)}%` : d.status === "error" ? "gagal" : "siap";
            return (
              <li key={d.key} className={`attachment${image ? " image" : ""}${d.status === "error" ? " failed" : ""}`}>
                <button
                  type="button"
                  className="attachment-open"
                  title={d.error ?? d.name}
                  aria-label={`${d.name}, ${formatSize(d.file.size)}, ${state}. Buka preview.`}
                  disabled={!image && d.status !== "ready"}
                  onClick={() => open(d)}
                  onKeyDown={(e) => {
                    if (e.key !== "Delete" && e.key !== "Backspace") return;
                    e.preventDefault();
                    draft.remove(d.key);
                    onDone();
                  }}
                >
                  {image ? (
                    <img className="attachment-thumb" src={d.url} alt="" />
                  ) : (
                    <>
                      <FileIcon kind={d.kind} size={26} />
                      <span className="attachment-meta">
                        <span className="attachment-name">{d.name}</span>
                        <span className="attachment-size">
                          {d.status === "error" ? "Gagal diunggah" : `${kindLabel(d.kind)} · ${formatSize(d.file.size)}`}
                        </span>
                      </span>
                    </>
                  )}
                  {d.status === "error" && image && <WarningCircleIcon size={22} weight="fill" className="attachment-warn" />}
                </button>
                {d.status === "uploading" && (
                  <span className="attachment-progress" aria-hidden="true">
                    <span style={{ width: `${Math.max(4, d.progress * 100)}%` }} />
                  </span>
                )}
                <button
                  type="button"
                  className="attachment-remove"
                  aria-label={`Hapus lampiran ${d.name}`}
                  onClick={() => {
                    draft.remove(d.key);
                    onDone();
                  }}
                >
                  <XIcon size={13} weight="bold" />
                </button>
              </li>
            );
          })}
        </ul>
      )}
      {(errors.length > 0 || draft.notice) && (
        <div className="attachment-errors" role="alert">
          {errors.map((d) => (
            <p key={d.key}>
              {d.name}: {d.error}
            </p>
          ))}
          {draft.notice && <p>{draft.notice}</p>}
        </div>
      )}
      {viewing && <ImageViewer src={viewing.src} name={viewing.name} onClose={closeViewer} />}
    </>
  );
}

/** Lampiran di pesan user yang sudah terkirim. Gambar dibuka di penampil, file di panel kanan. */
export function MessageAttachments({ files }: { files: readonly FileRef[] }) {
  const openPreview = useContext(PreviewContext);
  const [viewing, setViewing] = useState<Viewing>(null);
  const closeViewer = useCallback(() => setViewing(null), []);
  return (
    <div className="msg-attachments">
      {files.map((f) =>
        isImageKind(f.kind) ? (
          <button
            key={f.id}
            type="button"
            className="msg-image"
            aria-label={`Lihat gambar ${f.name}`}
            onClick={() => setViewing({ src: `/api/files/${f.id}`, name: f.name })}
          >
            <img src={`/api/files/${f.id}`} alt={f.name} loading="lazy" />
          </button>
        ) : (
          <button
            key={f.id}
            type="button"
            className="msg-file"
            title={f.name}
            onClick={() => openPreview({ type: "file", file: f })}
          >
            <FileIcon kind={f.kind} size={24} />
            <span className="attachment-meta">
              <span className="attachment-name">{f.name}</span>
              <span className="attachment-size">
                {kindLabel(f.kind)} · {formatSize(f.size)}
              </span>
            </span>
          </button>
        ),
      )}
      {viewing && <ImageViewer src={viewing.src} name={viewing.name} onClose={closeViewer} />}
    </div>
  );
}
