import { createContext } from "react";
import type { DocumentItem } from "./documents";
import type { FileRef, SourceRef } from "./messages";

/** Yang bisa dibuka di panel kanan: file buatan Bakase, potongan dokumen sumber, atau
 * seluruh teks satu dokumen dari Sumber Data. */
export type PreviewTarget =
  | { type: "file"; file: FileRef }
  | { type: "source"; source: SourceRef; originalPage?: number }
  | { type: "document"; document: DocumentItem };

/** Membuka panel preview kanan. Disediakan oleh Shell. */
export const PreviewContext = createContext<(target: PreviewTarget) => void>(() => {});

export const ReloadAppContext = createContext(() => window.location.reload());

/** Kunci unik per isi panel, supaya panel dimuat ulang saat isinya berganti. */
export function previewKey(target: PreviewTarget): string {
  switch (target.type) {
    case "file":
      return `file:${target.file.id}`;
    case "source":
      return `source:${target.source.chunk_id}:${target.originalPage ?? ""}`;
    case "document":
      return `document:${target.document.id}`;
  }
}
