import { useEffect, useRef, useState } from "react";
import { sourceName, type SourceRef } from "./messages";

export function ImageSourceThumb({ source, decorative = false }: { source: SourceRef; decorative?: boolean }) {
  const [failed, setFailed] = useState(false);
  if (!source.image) return null;
  if (failed) return <span className="image-source-unavailable">Gambar tidak tersedia.</span>;
  return (
    <img
      src={`/api/documents/${source.doc_id}/images/${encodeURIComponent(source.image.key)}/thumbnail`}
      alt={decorative ? "" : sourceName(source)}
      loading="lazy"
      decoding="async"
      onError={() => setFailed(true)}
    />
  );
}

export function ImageGallery({
  sources,
  similar = false,
  onOpen,
  active,
  onHighlight,
}: {
  sources: SourceRef[];
  similar?: boolean;
  onOpen: (source: SourceRef) => void;
  active?: number;
  onHighlight?: (index: number) => void;
}) {
  const selected = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    selected.current?.scrollIntoView({ block: "nearest" });
  }, [active]);
  if (!sources.length) return null;
  return (
    <div className={`image-sources${similar ? " image-sources-similar" : ""}`} aria-label={similar ? "Gambar mirip" : "Gambar dari dokumen"}>
      {sources.map((source, index) => (
        <button
          type="button"
          className={`image-source-card${active === index ? " active" : ""}`}
          ref={active === index ? selected : undefined}
          onMouseEnter={() => onHighlight?.(index)}
          key={source.chunk_id}
          onClick={() => onOpen(source)}
          title={sourceName(source)}
        >
          <span className="image-source-art"><ImageSourceThumb source={source} decorative /></span>
          <span className="image-source-caption">
            <strong>{sourceName(source)}</strong>
            <span>{source.n > 0 && `[${source.n}] · `}{source.image?.place}</span>
          </span>
        </button>
      ))}
    </div>
  );
}
