import { PencilSimpleIcon } from "@phosphor-icons/react";
import { useContext } from "react";
import { sourceName, type SourceRef } from "./messages";
import { PreviewContext } from "./previewContext";
import { scanPageLinks } from "./scanPageLinks";

export function ScanNotice({ sources }: { sources: readonly SourceRef[] }) {
  const open = useContext(PreviewContext);
  const links = scanPageLinks(sources);
  if (!links.length) return null;
  const unchecked = links.filter((link) => link.unchecked).length;
  const message = unchecked === 0
    ? "Ada tulisan tangan di sumber ini. Baca langsung dari halamannya."
    : unchecked === links.length
      ? "Sumber ini hasil scan. Kalau ada tulisan tangan, baca langsung dari halamannya."
      : "Ada sumber dengan tulisan tangan dan scan yang belum dicek. Baca langsung dari halamannya.";
  return (
    <div className="scan-notice">
      <PencilSimpleIcon size={18} aria-hidden="true" />
      <span>{message}</span>
      {links.map(({ source, page }) => {
        const title = sourceName(source);
        return (
          <button
            className="text-btn"
            key={source.doc_id}
            type="button"
            title={links.length > 1 ? title : undefined}
            onClick={() => open({ type: "source", source, originalPage: page })}
          >
            Lihat halaman{links.length > 1 && <span className="scan-notice-title"> · {title}</span>}
          </button>
        );
      })}
    </div>
  );
}
