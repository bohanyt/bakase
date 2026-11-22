import { ChatCircleIcon, XIcon } from "@phosphor-icons/react";
import { Fragment, useEffect, useRef, useState, type ReactNode } from "react";
import { ApiError, api } from "./api";
import { FileIcon } from "./FileIcon";
import { ImageGallery } from "./ImageGallery";
import { placeLabel, type SourceRef } from "./messages";
import { shortDate } from "./time";

type ChatHit = { id: string; title: string; snippet: string | null; updated_at: number };
type DocHit = {
  chunk_id: string;
  page: number | null;
  snippet: { text: string; hit: boolean }[];
  document: { id: number; source: string; folder?: string; title: string; display_title: string; kind: string };
};
type Tab = "all" | "chats" | "documents";
type Item = { type: "chat"; hit: ChatHit } | { type: "doc"; hit: DocHit };
type ResultItem = Item | { type: "image"; hit: SourceRef };

const TABS: { id: Tab; label: string }[] = [
  { id: "all", label: "Semua" },
  { id: "chats", label: "Chat" },
  { id: "documents", label: "Dokumen" },
];

/** Di tab Semua, dokumen dibatasi supaya chat tetap terlihat. */
const DOCS_IN_ALL = 5;

/** Menebalkan setiap kata kunci di dalam teks, tanpa peduli huruf besar/kecil. */
function highlight(text: string, words: string[]): ReactNode {
  if (words.length === 0) return text;
  const escaped = words.map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  const parts = text.split(new RegExp(`(${escaped.join("|")})`, "gi"));
  return parts.map((p, i) =>
    i % 2 === 1 ? <mark key={i}>{p}</mark> : <Fragment key={i}>{p}</Fragment>,
  );
}

function toSource(h: DocHit): SourceRef {
  return {
    n: 0,
    chunk_id: h.chunk_id,
    doc_id: h.document.id,
    title: h.document.title,
    display_title: h.document.display_title,
    source: h.document.source,
    kind: h.document.kind,
    page: h.page,
  };
}

export function SearchDialog({
  onOpen,
  onOpenSource,
  onClose,
}: {
  onOpen: (id: string) => void;
  onOpenSource: (source: SourceRef) => void;
  onClose: () => void;
}) {
  const [q, setQ] = useState("");
  const [tab, setTab] = useState<Tab>("all");
  const [chats, setChats] = useState<ChatHit[]>([]);
  const [docs, setDocs] = useState<DocHit[]>([]);
  const [images, setImages] = useState<SourceRef[]>([]);
  const [docError, setDocError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const words = q.trim().split(/\s+/).filter(Boolean);

  useEffect(() => {
    inputRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    const query = q.trim();
    if (!query) return;
    let stale = false;
    const t = window.setTimeout(() => {
      setLoading(true);
      // POST supaya kata kunci tidak tercatat di log akses server.
      const chatSearch = api<ChatHit[]>("/api/conversations/search", { body: { q: query } }).then(
        (r) => !stale && setChats(r),
        () => !stale && setChats([]),
      );
      const docSearch = api<{ hits: DocHit[]; images?: SourceRef[] }>("/api/documents/search", { body: { query } }).then(
        (r) => {
          if (stale) return;
          setDocs(r.hits);
          setImages(r.images ?? []);
          setDocError(null);
        },
        (err) => {
          if (stale) return;
          setDocs([]);
          setImages([]);
          setDocError(
            err instanceof ApiError && err.status === 404
              ? "Pencarian dokumen belum aktif di server ini."
              : "Pencarian dokumen sedang bermasalah.",
          );
        },
      );
      Promise.all([chatSearch, docSearch]).finally(() => {
        if (!stale) {
          setLoading(false);
          setActive(0);
        }
      });
    }, 200);
    return () => {
      stale = true;
      window.clearTimeout(t);
    };
  }, [q]);

  const typed = words.length > 0;
  const shownChats = typed && tab !== "documents" ? chats : [];
  const shownDocs = typed && tab !== "chats" ? (tab === "all" ? docs.slice(0, DOCS_IN_ALL) : docs) : [];
  const shownImages = typed && tab !== "chats" && !loading ? images : [];
  const textItems: Item[] = [
    ...shownChats.map((hit) => ({ type: "chat" as const, hit })),
    ...shownDocs.map((hit) => ({ type: "doc" as const, hit })),
  ];
  const items: ResultItem[] = [...textItems, ...shownImages.map((hit) => ({ type: "image" as const, hit }))];

  function choose(item: ResultItem) {
    if (item.type === "chat") onOpen(item.hit.id);
    else onOpenSource(item.type === "image" ? item.hit : toSource(item.hit));
  }

  function onInputKey(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => Math.min(a + 1, Math.max(0, items.length - 1)));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => Math.max(a - 1, 0));
    } else if (e.key === "Enter" && items[active]) {
      choose(items[active]);
    }
  }

  const emptyText =
    tab === "documents"
      ? "Ketik kata kunci untuk mencari di dokumen kantor yang boleh kamu buka."
      : "Ketik kata apa saja. Semua kata harus ada di percakapan, urutannya bebas.";

  return (
    <div className="overlay" onMouseDown={onClose}>
      <div className="search-dialog" role="dialog" aria-label="Cari" onMouseDown={(e) => e.stopPropagation()}>
        <div className="search-head">
          <input
            id="search-input"
            ref={inputRef}
            value={q}
            onChange={(e) => {
              setQ(e.target.value);
              if (!e.target.value.trim()) {
                setChats([]);
                setDocs([]);
                setImages([]);
              }
            }}
            onKeyDown={onInputKey}
            placeholder="Cari percakapan atau dokumen…"
            autoComplete="off"
          />
          {q && (
            <button type="button" className="text-btn" onClick={() => setQ("")}>
              Hapus
            </button>
          )}
          <button type="button" className="icon-btn" aria-label="Tutup" onClick={onClose}>
            <XIcon size={18} />
          </button>
        </div>

        <div className="search-tabs" role="tablist">
          {TABS.map((t) => (
            <button
              key={t.id}
              type="button"
              role="tab"
              aria-selected={tab === t.id}
              onClick={() => {
                setTab(t.id);
                setActive(0);
              }}
            >
              {t.label}
            </button>
          ))}
        </div>

        <div className="search-results">
          {!typed && <p className="search-empty">{emptyText}</p>}
          {typed && !loading && items.length === 0 && !(tab === "documents" && docError) && (
            <p className="search-empty">
              {tab === "documents" ? "Tidak ada dokumen yang cocok." : tab === "chats" ? "Tidak ada percakapan yang cocok." : "Tidak ada yang cocok."}
            </p>
          )}
          {textItems.map((item, i) => {
            const activeClass = i === active ? " active" : "";
            if (item.type === "chat") {
              const h = item.hit;
              return (
                <button
                  key={`c:${h.id}`}
                  type="button"
                  className={`search-hit${activeClass}`}
                  onMouseEnter={() => setActive(i)}
                  onClick={() => choose(item)}
                >
                  <ChatCircleIcon size={18} className="hit-icon" />
                  <span className="hit-text">
                    <span className="hit-title">{highlight(h.title, words)}</span>
                    {h.snippet && <span className="hit-snippet">{highlight(h.snippet, words)}</span>}
                  </span>
                  <span className="hit-date">{shortDate(h.updated_at)}</span>
                </button>
              );
            }
            const h = item.hit;
            const place = placeLabel(h.document.kind, h.page);
            return (
              <Fragment key={`d:${h.chunk_id}`}>
                {tab === "all" && i === shownChats.length && <p className="search-section">Dokumen</p>}
                <button
                  type="button"
                  className={`search-hit${activeClass}`}
                  onMouseEnter={() => setActive(i)}
                  onClick={() => choose(item)}
                >
                  <span className="hit-icon">
                    <FileIcon kind={h.document.kind} size={18} />
                  </span>
                  <span className="hit-text">
                    <span className="hit-title" title={h.document.title}>{highlight(h.document.display_title, words)}</span>
                    <span className="hit-snippet">
                      {h.snippet.some((s) => s.text) ? h.snippet.map((s, j) => (
                        s.hit ? <mark key={j}>{s.text}</mark> : <Fragment key={j}>{s.text}</Fragment>
                      )) : "Isi belum terbaca"}
                    </span>
                  </span>
                  <span className="hit-date">
                    {h.document.source}
                    {h.document.folder && ` · ${h.document.folder}`}
                    {place && ` · ${place}`}
                  </span>
                </button>
              </Fragment>
            );
          })}
          {typed && tab !== "chats" && docError && <p className="search-note">{docError}</p>}
          {shownImages.length > 0 && <p className="search-section">Gambar</p>}
          <ImageGallery sources={shownImages} onOpen={onOpenSource}
            active={active - textItems.length} onHighlight={(i) => setActive(textItems.length + i)} />
          {typed && tab === "all" && docs.length > DOCS_IN_ALL && (
            <button type="button" className="link search-more" onClick={() => setTab("documents")}>
              Lihat semua {docs.length} hasil dokumen
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
