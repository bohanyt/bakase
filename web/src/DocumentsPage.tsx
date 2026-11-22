import { DownloadSimpleIcon, MagnifyingGlassIcon } from "@phosphor-icons/react";
import { useEffect, useId, useRef, useState } from "react";
import { api } from "./api";
import { attentionStatus, statusNote, UNSEARCHABLE_STATUSES, unsearchableSummary, type DocumentItem, type DocumentList } from "./documents";
import { FileIcon } from "./FileIcon";
import { calendarDate, fullDateTime, longDate } from "./time";

type Sort = "judul" | "terbaru";

const PAGE_SIZE = 50;

const DATE_FROM: Record<string, string> = {
  judul: "dari judul",
  isi: "dari isi",
  metadata: "dari metadata file",
  archive: "dari tanggal upload ARCHIVE",
};

/** Tanggal dokumen kalau diketahui (asalnya di tooltip); kalau tidak, tanggal file diubah. */
function rowDate(doc: DocumentItem): { text: string; tip: string } | null {
  const date = calendarDate(doc.doc_date);
  if (date) {
    const text = longDate(date);
    const from = doc.doc_date_from ? DATE_FROM[doc.doc_date_from] : undefined;
    return { text, tip: [`Tanggal dokumen ${text}`, from].filter(Boolean).join(" · ") };
  }
  const seconds = Math.floor(doc.modified / 1000);
  if (seconds <= 0) return null;
  return { text: `diubah ${longDate(new Date(seconds * 1000))}`, tip: `Diubah ${fullDateTime(seconds)}` };
}

type Filter = { source: string | null; query: string; attentionOnly: boolean; sort: Sort };

function listRequest({ source, query, attentionOnly, sort }: Filter, offset: number) {
  const status = attentionOnly ? UNSEARCHABLE_STATUSES : [];
  // POST supaya kata pencarian tidak tercatat di log akses server.
  return api<DocumentList>("/api/documents/list", {
    body: { source, query: query.trim(), status, sort, offset, limit: PAGE_SIZE },
  });
}

function DocumentRow({
  doc,
  showSource,
  active,
  onOpen,
}: {
  doc: DocumentItem;
  showSource: boolean;
  active: boolean;
  onOpen: () => void;
}) {
  const status = attentionStatus(doc.status);
  const reasonId = useId();
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [tipOff, setTipOff] = useState(false);
  const tipActive = Boolean(status) && (hovered || focused);

  useEffect(() => {
    if (!tipActive) return;
    function dismiss(event: KeyboardEvent) {
      if (event.key === "Escape") setTipOff(true);
    }
    window.addEventListener("keydown", dismiss);
    return () => window.removeEventListener("keydown", dismiss);
  }, [tipActive]);
  const folder = [showSource || !doc.folder ? doc.source : null, doc.folder].filter(Boolean).join(" / ");
  const date = rowDate(doc);
  return (
    <li className={`doc-row${active ? " active" : ""}${tipOff ? " tip-off" : ""}`}>
      <button
        type="button"
        className="doc-open"
        aria-describedby={status ? reasonId : undefined}
        onClick={onOpen}
        onFocus={status ? () => {
          if (!hovered) setTipOff(false);
          setFocused(true);
        } : undefined}
        onBlur={status ? () => setFocused(false) : undefined}
      >
        <FileIcon kind={doc.kind} size={20} />
        <span className="doc-text">
          <span className="doc-title" title={doc.title}>{doc.display_title}</span>
          <span className="doc-sub">
            {folder}
            {date && <> · <span title={date.tip}>{date.text}</span></>}
          </span>
        </span>
      </button>
      {status && (
        <span
          className="doc-status"
          onMouseEnter={() => {
            if (!focused) setTipOff(false);
            setHovered(true);
          }}
          onMouseLeave={() => setHovered(false)}
        >
          <span className={`status-dot st-${status.tone}`} aria-hidden />
          {status.label}
          <span className="doc-status-reason" role="tooltip" id={reasonId}>{statusNote(doc.status)}</span>
        </span>
      )}
      {doc.available && (
        <a className="icon-btn doc-dl" href={`/api/documents/${doc.id}/file`} download aria-label={`Unduh ${doc.display_title}`} title="Unduh asli">
          <DownloadSimpleIcon size={17} />
        </a>
      )}
    </li>
  );
}

function DocumentAttention({
  statuses,
  attentionOnly,
  onFilter,
}: {
  statuses: DocumentList["statuses"];
  attentionOnly: boolean;
  onFilter: (active: boolean) => void;
}) {
  const summary = unsearchableSummary(statuses);
  return (
    <p className="docs-attention">
      {summary && <>{summary} </>}
      {(summary || attentionOnly) && (
        <button type="button" onClick={() => onFilter(!attentionOnly)}>
          {attentionOnly ? "Tampilkan semua" : "Lihat"}
        </button>
      )}
    </p>
  );
}

/** Halaman Sumber Data: file per sumber yang boleh dibuka user, dengan status bacanya. */
export function DocumentsPage({ openId, onOpen }: { openId: number | null; onOpen: (doc: DocumentItem) => void }) {
  const [source, setSource] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [attentionOnly, setAttentionOnly] = useState(false);
  const [sort, setSort] = useState<Sort>("judul");
  const [result, setResult] = useState<{ page: DocumentList; source: string | null } | null>(null);
  const page = result?.page ?? null;
  const [items, setItems] = useState<DocumentItem[]>([]);
  const [error, setError] = useState<string | null>(null);
  // Naik saat user menekan "Coba lagi" setelah halaman pertama gagal dimuat.
  const [retry, setRetry] = useState(0);
  // Kunci permintaan sekarang (filter + percobaan), kunci yang halaman pertamanya sudah selesai
  // (berhasil atau gagal), kunci data yang sedang tersimpan di `page`/`items`, dan kunci yang
  // "Tampilkan lagi"-nya sedang dimuat. Status dihitung dari keempatnya, jadi permintaan untuk
  // filter lama tidak pernah membuat tombol macet, dan daftar filter lama tidak pernah
  // disambung dengan halaman filter baru.
  const key = JSON.stringify([source, query, attentionOnly, sort, retry]);
  const [loadedKey, setLoadedKey] = useState<string | null>(null);
  const [dataKey, setDataKey] = useState<string | null>(null);
  const [moreKey, setMoreKey] = useState<string | null>(null);
  const loading = loadedKey !== key;
  const current = dataKey === key;
  const loadingMore = moreKey === key;
  // Selama memuat, daftar sebelumnya tetap tampil; setelah gagal (juga saat mencoba lagi),
  // daftar filter lain disembunyikan.
  const shown = current || (loading && !error) ? items : [];
  // Nomor urut permintaan halaman pertama: jawaban untuk filter lama diabaikan.
  const seq = useRef(0);

  useEffect(() => {
    const s = ++seq.current;
    const k = JSON.stringify([source, query, attentionOnly, sort, retry]);
    // Ketikan ditunggu sebentar supaya tidak setiap huruf memanggil server.
    const t = window.setTimeout(
      () => {
        listRequest({ source, query, attentionOnly, sort }, 0).then(
          (r) => {
            if (s !== seq.current) return;
            setResult({ page: r, source });
            setItems(r.documents);
            setError(null);
            setDataKey(k);
            setLoadedKey(k);
          },
          (err) => {
            if (s !== seq.current) return;
            setError(err instanceof Error ? err.message : "Gagal memuat daftar dokumen.");
            setLoadedKey(k);
          },
        );
      },
      query.trim() ? 200 : 0,
    );
    return () => window.clearTimeout(t);
  }, [source, query, attentionOnly, sort, retry]);

  function loadMore() {
    // Offset diambil dari daftar filter yang sedang tampil, jadi hanya kalau daftar itu milik
    // filter sekarang.
    if (loading || loadingMore || !current) return;
    const s = seq.current;
    const k = key;
    setMoreKey(k);
    listRequest({ source, query, attentionOnly, sort }, items.length)
      .then(
        (r) => {
          if (s !== seq.current) return;
          setResult({ page: r, source });
          setError(null);
          setItems((list) => {
            const ids = new Set(list.map((d) => d.id));
            return [...list, ...r.documents.filter((d) => !ids.has(d.id))];
          });
        },
        (err) => s === seq.current && setError(err instanceof Error ? err.message : "Gagal memuat daftar dokumen."),
      )
      .finally(() => setMoreKey((m) => (m === k ? null : m)));
  }

  const sources = page?.sources ?? [];
  const allTotal = sources.reduce((n, s) => n + s.total, 0);
  const statuses = result?.source === source ? result.page.statuses : {};
  const filtered = query.trim() !== "" || attentionOnly;

  return (
    <div className="page docs-page">
      <h2>Sumber Data</h2>
      <p className="docs-lead muted">File dokumen kantor yang boleh kamu buka. Pilih file untuk melihat isinya.</p>

      {page && sources.length === 0 ? (
        <p className="docs-empty">
          Belum ada sumber dokumen yang bisa kamu buka. Kalau pekerjaanmu butuh dokumen departemen, minta admin mengatur departemenmu.
        </p>
      ) : (
        <>
          {sources.length > 1 && (
            <div className="search-tabs docs-tabs" role="tablist" aria-label="Sumber">
              <button type="button" role="tab" aria-selected={source === null} onClick={() => setSource(null)}>
                Semua <span className="tab-count">{allTotal}</span>
              </button>
              {sources.map((s) => (
                <button key={s.name} type="button" role="tab" aria-selected={source === s.name} onClick={() => setSource(s.name)}>
                  {s.name} <span className="tab-count">{s.total}</span>
                </button>
              ))}
            </div>
          )}

          <div className="docs-bar">
            <label className="docs-search">
              <MagnifyingGlassIcon size={16} aria-hidden />
              <input
                id="docs-search"
                type="search"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Cari judul atau folder…"
                aria-label="Cari judul atau folder"
                autoComplete="off"
              />
            </label>
            <select value={sort} onChange={(e) => setSort(e.target.value as Sort)} aria-label="Urutkan">
              <option value="judul">Nama A–Z</option>
              <option value="terbaru">Terbaru</option>
            </select>
          </div>

          <DocumentAttention statuses={statuses} attentionOnly={attentionOnly} onFilter={setAttentionOnly} />

          {error && (
            <p className="form-error">
              {error}{" "}
              {!loading && !current && (
                <button type="button" className="text-btn" onClick={() => setRetry((r) => r + 1)}>
                  Coba lagi
                </button>
              )}
            </p>
          )}
          <p className="docs-count" aria-live="polite">
            {loading ? "Memuat…" : current && page ? `${page.total} file` : ""}
          </p>

          <ul className="docs-list" aria-busy={loading}>
            {shown.map((d) => (
              <DocumentRow
                key={d.id}
                doc={d}
                showSource={source === null && sources.length > 1}
                active={d.id === openId}
                onOpen={() => onOpen(d)}
              />
            ))}
          </ul>
          {!loading && current && page && items.length === 0 && (
            <p className="docs-empty">{filtered ? "Tidak ada file yang cocok." : "Belum ada file di sumber ini."}</p>
          )}
          {!loading && current && page && items.length < page.total && (
            <button type="button" className="text-btn docs-more" disabled={loadingMore} onClick={loadMore}>
              {loadingMore ? "Memuat…" : `Tampilkan lagi (${page.total - items.length} file)`}
            </button>
          )}
        </>
      )}
    </div>
  );
}
