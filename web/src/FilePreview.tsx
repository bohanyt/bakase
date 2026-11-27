import { DownloadSimpleIcon, QuotesIcon, XIcon } from "@phosphor-icons/react";
import { Fragment, memo, useContext, useEffect, useId, useRef, useState } from "react";
import { api } from "./api";
import { CiteContext } from "./citeContext";
import { FileIcon } from "./FileIcon";
import type { FileRef } from "./messages";
import { isImageKind } from "./attachment";
import { columnName, edgeScrollStep, markdownTable, rangeA1, rangeBounds, type CellPos } from "./quote";
import { DocumentText } from "./SourcePreview";

/** Potong ke `max` karakter (code point, bukan satuan UTF-16), dengan elipsis. */
function clipChars(text: string, max: number): string {
  const chars = [...text];
  return chars.length > max ? `${chars.slice(0, max).join("")}…` : text;
}

type Cell = string | number | boolean | null;
type Column = { header: string; format?: string | null };
type Sheet = { name: string; columns: Column[]; rows: Cell[][] };
type XlsxSpec = { filename: string; sheets: Sheet[] };

type Block =
  | { type: "heading"; level?: number; text: string }
  | { type: "paragraph"; text: string }
  | { type: "bullets"; items: string[] }
  | { type: "numbered"; items: string[] }
  | { type: "table"; columns: string[]; rows: Cell[][] }
  | { type: "page_break" };
type DocxSpec = { filename: string; title?: string | null; blocks: Block[] };

type Slide = { title: string } & (
  | { type: "title"; subtitle: string | null }
  | { type: "bullets"; items: string[] }
  | { type: "table"; columns: string[]; rows: Cell[][] }
  | { type: "chart"; chart_type: "bar" | "line" | "pie"; unit: string; categories: string[]; series: { name: string; values: number[] }[] }
);

type Preview = { name: string; kind: string; spec: unknown };

/** Baris tabel yang digambar; sisanya cukup dilihat di file aslinya. */
const MAX_ROWS = 500;

const number = new Intl.NumberFormat("id-ID");
const decimal = new Intl.NumberFormat("id-ID", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
const percent = new Intl.NumberFormat("id-ID", { style: "percent", maximumFractionDigits: 1 });

function formatCell(value: Cell, format?: string | null): string {
  if (value === null || value === undefined) return "";
  if (typeof value === "boolean") return value ? "TRUE" : "FALSE";
  if (typeof value === "string") return value;
  // File lama atau data tak terduga: jangan sampai preview crash.
  if (typeof value !== "number") return JSON.stringify(value) ?? "";
  switch (format) {
    case "desimal":
      return decimal.format(value);
    case "rupiah":
      return `Rp ${number.format(value)}`;
    case "persen":
      return percent.format(value);
    default:
      return number.format(value);
  }
}

/** Teks dengan **tebal**; tanda yang tidak berpasangan dibiarkan apa adanya. */
function Rich({ text }: { text: string }) {
  const parts = text.split("**");
  if (parts.length % 2 === 0) return <>{text}</>;
  return (
    <>
      {parts.map((p, i) => (i % 2 === 1 ? <strong key={i}>{p}</strong> : <Fragment key={i}>{p}</Fragment>))}
    </>
  );
}

function cellClass(...names: (string | false)[]): string | undefined {
  return names.filter(Boolean).join(" ") || undefined;
}

/** Satu baris data. Hanya baris yang pilihannya berubah yang digambar ulang. */
const SheetRow = memo(function SheetRow({
  row,
  rowNum,
  columns,
  selLeft,
  selRight,
  focusCol,
}: {
  row: Cell[];
  rowNum: number;
  columns: Column[];
  /** Kolom pertama dan terakhir yang dipilih di baris ini, atau -1. */
  selLeft: number;
  selRight: number;
  focusCol: number;
}) {
  return (
    <tr>
      <td className="rownum">{rowNum}</td>
      {columns.map((c, j) => {
        const v = row[j] ?? null;
        // Di kolom teks, "=..." tetap teks biasa.
        const formula = typeof v === "string" && v.startsWith("=") && c.format !== "teks";
        const numeric = typeof v === "number";
        return (
          <td
            key={j}
            data-cell={`${rowNum}:${j}`}
            className={cellClass(
              formula ? "formula" : numeric && "num",
              j >= selLeft && j <= selRight && "sel",
              j === focusCol && "sel-focus",
            )}
          >
            {formatCell(v, c.format)}
          </td>
        );
      })}
    </tr>
  );
});

type SheetSelection = { anchor: CellPos; focus: CellPos };

function cellAt(target: EventTarget | null): CellPos | null {
  const el = target instanceof Element ? target.closest("[data-cell]") : null;
  const [row, col] = (el?.getAttribute("data-cell") ?? "").split(":").map(Number);
  return Number.isInteger(row) && Number.isInteger(col) ? { row, col } : null;
}

const samePos = (a: CellPos, b: CellPos) => a.row === b.row && a.col === b.col;

const ARROWS: Record<string, [number, number]> = {
  ArrowUp: [-1, 0],
  ArrowDown: [1, 0],
  ArrowLeft: [0, -1],
  ArrowRight: [0, 1],
};

/** Tabel Excel. Sel atau rentang bisa dipilih (klik, Shift+klik, seret, atau panah) lalu dikutip. */
function SheetPreview({ spec, fileName }: { spec: XlsxSpec; fileName: string }) {
  const cite = useContext(CiteContext);
  const [active, setActive] = useState(0);
  const [sel, setSel] = useState<SheetSelection | null>(null);
  const [citeError, setCiteError] = useState<string | null>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);
  const pointerType = useRef("mouse");
  const statusId = useId();

  useEffect(() => {
    let point: { x: number; y: number } | null = null;
    let frame = 0;
    // Seret sampai tepi: tabel ikut bergulir dan pilihan melebar ke sel di bawah kursor,
    // seperti Excel. Berhenti saat tombol dilepas atau tabel sudah mentok.
    const step = () => {
      frame = 0;
      const wrap = wrapRef.current;
      if (!dragging.current || !wrap || !point) return;
      const r = wrap.getBoundingClientRect();
      const left = r.left + wrap.clientLeft;
      const top = r.top + wrap.clientTop;
      const right = left + wrap.clientWidth;
      const bottom = top + wrap.clientHeight;
      // Nomor baris dan judul kolom menempel, jadi tepi sel yang terlihat ada di balik keduanya.
      const start = left + (wrap.querySelector<HTMLElement>("thead .rownum")?.offsetWidth ?? 0);
      const head = top + (wrap.querySelector<HTMLElement>("thead")?.offsetHeight ?? 0);
      const dx = edgeScrollStep(point.x, start, right);
      const dy = edgeScrollStep(point.y, head, bottom);
      const x0 = wrap.scrollLeft;
      const y0 = wrap.scrollTop;
      if (dx || dy) wrap.scrollBy({ left: dx, top: dy, behavior: "instant" });
      if (wrap.scrollLeft === x0 && wrap.scrollTop === y0) return;
      const x = Math.min(Math.max(point.x, start + 1), right - 2);
      const y = Math.min(Math.max(point.y, head + 1), bottom - 2);
      const pos = cellAt(document.elementFromPoint(x, y));
      if (pos) setSel((s) => (s && !samePos(s.focus, pos) ? { anchor: s.anchor, focus: pos } : s));
      frame = requestAnimationFrame(step);
    };
    const move = (e: PointerEvent) => {
      if (!dragging.current) return;
      point = { x: e.clientX, y: e.clientY };
      if (!frame) frame = requestAnimationFrame(step);
    };
    const up = () => {
      dragging.current = false;
      point = null;
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
    };
  }, []);

  const sheets = spec.sheets ?? [];
  const sheet = sheets[Math.min(active, Math.max(sheets.length - 1, 0))];
  if (!sheet) return null;
  const sheetLabel = sheets.length > 1 ? sheet.name : null;
  const rows = sheet.rows.slice(0, MAX_ROWS);
  // Baris 1 adalah judul kolom, data mulai di baris 2, seperti di Excel.
  const lastRow = rows.length + 1;
  const lastCol = sheet.columns.length - 1;
  const box = sel && rangeBounds(sel.anchor, sel.focus);
  const a1 = sel ? rangeA1(sheetLabel, sel.anchor, sel.focus) : "";
  // Sel aktif (judul kolom dan isinya) di status, supaya pembaca layar tahu apa yang akan
  // dikutip; pilihan di tabel hanya ditandai warna.
  const focusCell = (() => {
    if (!sel) return "";
    const { row, col } = sel.focus;
    const header = sheet.columns[col]?.header?.trim() ?? "";
    const name = clipChars(header, 40) || columnName(col);
    if (row === 1) return `sel aktif: judul kolom ${name}${header ? "" : " (kosong)"}`;
    const value = formatCell(rows[row - 2]?.[col] ?? null, sheet.columns[col]?.format).trim();
    return `sel aktif ${name}: ${clipChars(value, 80) || "kosong"}`;
  })();

  function select(pos: CellPos, extend: boolean) {
    setCiteError(null);
    setSel((s) => (extend && s ? { anchor: s.anchor, focus: pos } : { anchor: pos, focus: pos }));
  }

  /** Isi rentang sebagai tabel kecil, dengan judul kolom supaya angkanya punya arti. */
  function rangeText(b: NonNullable<typeof box>): string {
    const cols = Array.from({ length: b.right - b.left + 1 }, (_, i) => b.left + i);
    const header = cols.map((j) => sheet.columns[j]?.header?.trim() || columnName(j));
    const body: string[][] = [];
    for (let r = Math.max(b.top, 2); r <= b.bottom; r++) {
      body.push(cols.map((j) => formatCell(rows[r - 2]?.[j] ?? null, sheet.columns[j]?.format)));
    }
    return markdownTable(header, body);
  }

  function citeRange() {
    if (!box) return;
    const err = cite({ text: rangeText(box), source: `${fileName} · ${a1}` });
    setCiteError(err);
    if (!err) setSel(null);
  }

  function onPointerDown(e: React.PointerEvent) {
    pointerType.current = e.pointerType;
    const pos = cellAt(e.target);
    if (!pos || e.pointerType !== "mouse" || e.button !== 0) return;
    wrapRef.current?.focus({ preventScroll: true });
    dragging.current = true;
    select(pos, e.shiftKey);
  }

  function onPointerOver(e: React.PointerEvent) {
    const pos = dragging.current ? cellAt(e.target) : null;
    if (pos) select(pos, true);
  }

  // Layar sentuh: ketuk sel awal, lalu sel akhir. Ketukan berikutnya mulai pilihan baru.
  function onClick(e: React.MouseEvent) {
    const pos = cellAt(e.target);
    if (!pos || pointerType.current === "mouse") return;
    setCiteError(null);
    setSel((s) =>
      s && samePos(s.anchor, s.focus) && !samePos(s.anchor, pos) ? { anchor: s.anchor, focus: pos } : { anchor: pos, focus: pos },
    );
  }

  function onKeyDown(e: React.KeyboardEvent) {
    const arrow = ARROWS[e.key];
    if (arrow && !e.altKey && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      const from = sel?.focus;
      const next = from
        ? {
            row: Math.min(Math.max(from.row + arrow[0], 1), lastRow),
            col: Math.min(Math.max(from.col + arrow[1], 0), lastCol),
          }
        : { row: Math.min(2, lastRow), col: 0 };
      select(next, e.shiftKey && !!from);
      requestAnimationFrame(() =>
        wrapRef.current
          ?.querySelector(`[data-cell="${next.row}:${next.col}"]`)
          ?.scrollIntoView({ block: "nearest", inline: "nearest" }),
      );
    } else if (e.key === "Escape" && sel) {
      e.preventDefault();
      setSel(null);
      setCiteError(null);
    } else if (e.key === "Enter" && box) {
      e.preventDefault();
      citeRange();
    } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "c" && box) {
      // Teks di tabel ini tidak bisa diblok, jadi Ctrl+C menyalin rentang yang dipilih.
      e.preventDefault();
      const lines: string[] = [];
      for (let r = box.top; r <= box.bottom; r++) {
        const cells: string[] = [];
        for (let j = box.left; j <= box.right; j++) {
          cells.push(r === 1 ? sheet.columns[j]?.header ?? "" : formatCell(rows[r - 2]?.[j] ?? null, sheet.columns[j]?.format));
        }
        lines.push(cells.join("\t"));
      }
      void navigator.clipboard?.writeText(lines.join("\n")).catch(() => {});
    }
  }

  const headerSel = (j: number) => !!box && box.top === 1 && j >= box.left && j <= box.right;
  return (
    <>
      {sheets.length > 1 && (
        <select className="sheet-picker" aria-label="Pilih sheet" value={active} onChange={(e) => {
          setActive(Number(e.target.value));
          setSel(null);
          setCiteError(null);
          wrapRef.current?.scrollTo(0, 0);
        }}>
          {sheets.map((s, i) => <option key={s.name} value={i}>{s.name}</option>)}
        </select>
      )}
      <div
        ref={wrapRef}
        className="preview-table-wrap sheet-grid"
        tabIndex={0}
        role="group"
        aria-label={`${sheetLabel ? `Sheet ${sheetLabel}` : "Tabel"}. Panah memilih sel, Shift+panah memilih rentang, Enter mengutip.`}
        aria-describedby={statusId}
        onKeyDown={onKeyDown}
      >
        <table
          className="preview-table"
          onPointerDown={onPointerDown}
          onPointerOver={onPointerOver}
          onClick={onClick}
          onMouseDown={(e) => {
            // Blok teks biasa dimatikan supaya seret memilih sel.
            if (cellAt(e.target)) e.preventDefault();
          }}
        >
          <thead>
            <tr>
              <th className="rownum">1</th>
              {sheet.columns.map((c, j) => (
                <th
                  key={j}
                  data-cell={`1:${j}`}
                  className={cellClass(headerSel(j) && "sel", sel?.focus.row === 1 && sel.focus.col === j && "sel-focus")}
                >
                  {c.header}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((row, i) => {
              const rowNum = i + 2;
              const inRange = !!box && rowNum >= box.top && rowNum <= box.bottom;
              return (
                <SheetRow
                  key={i}
                  row={row}
                  rowNum={rowNum}
                  columns={sheet.columns}
                  selLeft={inRange ? box.left : -1}
                  selRight={inRange ? box.right : -1}
                  focusCol={sel?.focus.row === rowNum ? sel.focus.col : -1}
                />
              );
            })}
          </tbody>
        </table>
      </div>
      <div className="range-bar">
        <span id={statusId} className="range-status" aria-live="polite">
          {box ? (
            <>
              <strong className="range-ref">{a1}</strong> · {(box.bottom - box.top + 1) * (box.right - box.left + 1)} sel ·{" "}
              {focusCell}
            </>
          ) : (
            <>
              <span className="hint-mouse">Klik sel untuk mengutip. Shift+klik atau seret untuk rentang.</span>
              <span className="hint-touch">Ketuk sel untuk mengutip. Ketuk sel kedua untuk rentang.</span>
            </>
          )}
        </span>
        {box && (
          <>
            <button
              type="button"
              className="text-btn"
              onClick={() => {
                setSel(null);
                setCiteError(null);
              }}
            >
              Batal
            </button>
            <button type="button" className="text-btn range-cite" onClick={citeRange}>
              <QuotesIcon size={16} weight="fill" /> Kutip
            </button>
          </>
        )}
      </div>
      {citeError && (
        <p className="form-error range-error" role="alert">
          {citeError}
        </p>
      )}
      <p className="preview-note">
        {sheet.rows.length > MAX_ROWS && `Menampilkan ${MAX_ROWS} dari ${sheet.rows.length} baris. `}
        Rumus dihitung saat file dibuka di Excel.
      </p>
    </>
  );
}

function DocPreview({ spec, fileName }: { spec: DocxSpec; fileName: string }) {
  return (
    <article className="doc-preview" data-quote-source={fileName}>
      {spec.title && <h1>{spec.title}</h1>}
      {(spec.blocks ?? []).map((block, i) => {
        switch (block.type) {
          case "heading": {
            const Tag = block.level === 1 ? "h2" : block.level === 3 ? "h4" : "h3";
            return <Tag key={i}>{block.text}</Tag>;
          }
          case "paragraph":
            return block.text.split("\n").map((line, j) => (
              <p key={`${i}-${j}`}>
                <Rich text={line} />
              </p>
            ));
          case "bullets":
            return (
              <ul key={i}>
                {block.items.map((t, j) => (
                  <li key={j}>
                    <Rich text={t} />
                  </li>
                ))}
              </ul>
            );
          case "numbered":
            return (
              <ol key={i}>
                {block.items.map((t, j) => (
                  <li key={j}>
                    <Rich text={t} />
                  </li>
                ))}
              </ol>
            );
          case "table":
            return <PreviewTable key={i} columns={block.columns} rows={block.rows} />;
          case "page_break":
            return <hr key={i} className="page-break" aria-label="Halaman baru" />;
          default:
            return null;
        }
      })}
    </article>
  );
}

function PreviewTable({ columns, rows }: { columns: string[]; rows: Cell[][] }) {
  return (
    <div className="doc-table-wrap">
      <table className="preview-table doc-table">
        <thead>
          <tr>{columns.map((column, index) => <th key={index} scope="col">{column}</th>)}</tr>
        </thead>
        <tbody>
          {rows.slice(0, MAX_ROWS).map((row, index) => (
            <tr key={index}>
              {columns.map((_, column) => <td key={column}>{formatCell(row[column] ?? null)}</td>)}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

const CHART_LABELS = { bar: "Grafik batang", line: "Grafik garis", pie: "Grafik pai" };

function SlideContent({ slide }: { slide: Slide }) {
  switch (slide.type) {
    case "title":
      return slide.subtitle && <p>{slide.subtitle}</p>;
    case "bullets":
      return <ul>{slide.items.map((item, i) => <li key={i}>{item}</li>)}</ul>;
    case "table":
      return <PreviewTable columns={slide.columns} rows={slide.rows} />;
    case "chart":
      return (
        <>
          <p>{CHART_LABELS[slide.chart_type]} · {slide.unit}</p>
          <ul>
            {slide.series.map((series, i) => (
              <li key={i}>
                <strong>{series.name}</strong>{" · "}
                {series.values.map((value, j) => `${slide.categories[j]} ${value.toString().replace(".", ",")}`).join(", ")}
              </li>
            ))}
          </ul>
        </>
      );
  }
}

function SlidePreview({ slides }: { slides: Slide[] }) {
  return (
    <div className="doc-preview">
      <ol className="slide-list" aria-label="Daftar slide">
        {slides.map((slide, index) => (
          <li key={index}>
            <h2>{slide.title}</h2>
            <SlideContent slide={slide} />
          </li>
        ))}
      </ol>
    </div>
  );
}

/** Kerangka panel kanan: judul, tombol aksi, tombol tutup, lalu isinya. */
export function PreviewPanel({
  title,
  titleHint = title,
  icon,
  actions,
  width,
  overlay,
  onClose,
  children,
}: {
  title: string;
  titleHint?: string;
  icon: React.ReactNode;
  actions?: React.ReactNode;
  /** Lebar hasil tarikan pembatas; null berarti lebar bawaan. */
  width: number | null;
  /** Tampil penuh di atas chat karena ruangnya tidak cukup. */
  overlay: boolean;
  onClose: () => void;
  children: React.ReactNode;
}) {
  return (
    <aside
      className={`preview-panel${overlay ? " full" : ""}`}
      aria-label={`Preview ${title}`}
      style={width ? ({ "--preview-width": `${width}px` } as React.CSSProperties) : undefined}
    >
      <header className="preview-head">
        {icon}
        <span className="preview-title" title={titleHint}>
          {title}
        </span>
        {actions}
        <button type="button" className="icon-btn" aria-label="Tutup preview" onClick={onClose}>
          <XIcon size={18} />
        </button>
      </header>
      {children}
    </aside>
  );
}

/** Panel kanan: isi file buatan Bakase, digambar dari isi terstruktur yang dipakai membuat file. */
export function FilePreview({
  file,
  width,
  overlay,
  onClose,
}: {
  file: FileRef;
  /** Lebar hasil tarikan pembatas; null berarti lebar bawaan. */
  width: number | null;
  /** Tampil penuh di atas chat karena ruangnya tidak cukup. */
  overlay: boolean;
  onClose: () => void;
}) {
  const [data, setData] = useState<Preview | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let stale = false;
    api<Preview>(`/api/files/${file.id}/preview`).then(
      (d) => !stale && setData(d),
      (err) => !stale && setError(err instanceof Error ? err.message : "Gagal memuat preview."),
    );
    return () => {
      stale = true;
    };
  }, [file.id]);

  return (
    <PreviewPanel
      title={file.name}
      icon={<FileIcon kind={file.kind} size={22} />}
      actions={
        <a className="text-btn" href={`/api/files/${file.id}`} download>
          <DownloadSimpleIcon size={16} /> Unduh
        </a>
      }
      width={width}
      overlay={overlay}
      onClose={onClose}
    >
      {error && <p className="form-error">{error}</p>}
      {!data && !error && <div className="loading">Memuat preview…</div>}
      {data && <PreviewBody data={data} file={file} />}
    </PreviewPanel>
  );
}

/** Isi lampiran yang ditampilkan; sisanya cukup dibuka dari file aslinya. */
const MAX_TEXT = 60_000;
const textNumber = new Intl.NumberFormat("id-ID");

/** File buatan Bakase punya isi terstruktur (sheet atau blok). Lampiran dari user membawa
 * teks hasil baca server, dan lampiran Excel yang tidak terlalu besar juga sel per sheet. */
function PreviewBody({ data, file }: { data: Preview; file: FileRef }) {
  const spec = (data.spec ?? {}) as { sheets?: unknown; blocks?: unknown; slides?: unknown; text?: unknown; truncated?: unknown };
  if (isImageKind(data.kind)) {
    return (
      <div className="image-preview">
        <img src={`/api/files/${file.id}`} alt={file.name} />
      </div>
    );
  }
  if (Array.isArray(spec.sheets)) return <SheetPreview spec={data.spec as XlsxSpec} fileName={file.name} />;
  if (Array.isArray(spec.blocks)) return <DocPreview spec={data.spec as DocxSpec} fileName={file.name} />;
  if (data.kind === "pptx" && Array.isArray(spec.slides)) return <SlidePreview slides={spec.slides as Slide[]} />;
  if (typeof spec.text === "string") {
    const text = spec.text;
    const cut = text.length > MAX_TEXT || spec.truncated === true;
    return (
      <>
        {text.trim() ? (
          <DocumentText text={text.slice(0, MAX_TEXT)} kind={data.kind} quoteSource={file.name} />
        ) : (
          <p className="preview-note">File ini tidak memuat teks yang bisa dibaca, misalnya PDF hasil scan. Bakase tetap membacanya langsung dari file saat pesan dikirim.</p>
        )}
        <p className="preview-note">
          {cut && `Menampilkan ${textNumber.format(Math.min(text.length, MAX_TEXT))} karakter pertama. `}
          Ini teks yang dibaca Bakase dari lampiranmu. Isi lengkapnya ada di file asli.
        </p>
      </>
    );
  }
  return <p className="preview-note">Preview belum tersedia untuk file ini. Unduh untuk membukanya.</p>;
}
