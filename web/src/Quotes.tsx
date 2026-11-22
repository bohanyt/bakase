import { ArrowBendDownRightIcon, QuotesIcon, XIcon } from "@phosphor-icons/react";
import { useContext, useEffect, useId, useRef, useState } from "react";
import { CiteContext } from "./citeContext";
import { cleanQuoteText, parseMessage, parseTable, type Quote } from "./quote";

/** Area yang teksnya boleh dikutip; nilai atributnya menjadi sumber kutipan. */
const REGION = "[data-quote-source]";

type Spot = { x: number; y: number; below: boolean };
type Picked = { text: string; source?: string; spot: Spot | null };

function regionOf(node: Node | null): HTMLElement | null {
  const el = node instanceof Element ? node : node?.parentElement;
  return el?.closest<HTMLElement>(REGION) ?? null;
}

function readSelection() {
  const sel = window.getSelection();
  if (!sel || sel.isCollapsed || sel.rangeCount === 0) return null;
  const region = regionOf(sel.anchorNode);
  if (!region || region !== regionOf(sel.focusNode)) return null;
  const text = cleanQuoteText(sel.toString());
  if (!text) return null;
  return { text, source: region.dataset.quoteSource || undefined, range: sel.getRangeAt(0) };
}

/** Di atas pilihan; di layar sentuh di bawahnya supaya tidak menutupi menu bawaan browser. */
function place(range: Range): Spot | null {
  const box = range.getBoundingClientRect();
  if (box.bottom < 0 || box.top > window.innerHeight) return null;
  const below = window.matchMedia("(pointer: coarse)").matches || box.top < 56;
  const x = Math.min(Math.max(box.left + box.width / 2, 60), window.innerWidth - 60);
  const y = below ? Math.min(box.bottom + 8, window.innerHeight - 48) : box.top - 8;
  return { x, y, below };
}

function isTyping(el: Element | null): boolean {
  return el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || (el instanceof HTMLElement && el.isContentEditable);
}

/** Tombol "Kutip" yang muncul di dekat teks yang diblok di jawaban atau preview. */
export function SelectionCite() {
  const cite = useContext(CiteContext);
  const [picked, setPicked] = useState<Picked | null>(null);
  const [error, setError] = useState<string | null>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const rangeRef = useRef<Range | null>(null);
  // Menekan tombol di layar sentuh bisa menghapus blok teks sebelum klik sampai.
  const holdRef = useRef(false);

  useEffect(() => {
    let frame = 0;
    let dragging = false;
    const update = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const found = readSelection();
        if (!found) {
          if (holdRef.current || (buttonRef.current && document.activeElement === buttonRef.current)) return;
          rangeRef.current = null;
          setPicked(null);
          setError(null);
          return;
        }
        rangeRef.current = found.range;
        setError(null);
        setPicked({ text: found.text, source: found.source, spot: place(found.range) });
      });
    };
    const reposition = () => {
      const range = rangeRef.current;
      if (!range) return;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => setPicked((p) => p && { ...p, spot: place(range) }));
    };
    const onPointerDown = (e: PointerEvent) => {
      holdRef.current = !!buttonRef.current?.contains(e.target as Node);
      if (!holdRef.current && e.pointerType === "mouse") dragging = true;
    };
    const onPointerUp = () => {
      if (!dragging) return;
      dragging = false;
      update();
    };
    const onSelectionChange = () => {
      if (!dragging) update();
    };
    // Tab dari teks yang diblok langsung ke tombol Kutip, supaya bisa dipakai dengan keyboard.
    const onKeyDown = (e: KeyboardEvent) => {
      const button = buttonRef.current;
      if (!button || e.key !== "Tab" || e.shiftKey || e.altKey || e.ctrlKey || e.metaKey) return;
      if (document.activeElement === button || isTyping(document.activeElement)) return;
      e.preventDefault();
      button.focus();
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("pointerup", onPointerUp, true);
    document.addEventListener("pointercancel", onPointerUp, true);
    window.addEventListener("blur", onPointerUp);
    document.addEventListener("selectionchange", onSelectionChange);
    document.addEventListener("scroll", reposition, true);
    window.addEventListener("resize", reposition);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      cancelAnimationFrame(frame);
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("pointerup", onPointerUp, true);
      document.removeEventListener("pointercancel", onPointerUp, true);
      window.removeEventListener("blur", onPointerUp);
      document.removeEventListener("selectionchange", onSelectionChange);
      document.removeEventListener("scroll", reposition, true);
      window.removeEventListener("resize", reposition);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, []);

  function close() {
    holdRef.current = false;
    rangeRef.current = null;
    setPicked(null);
    setError(null);
  }

  function onCite() {
    holdRef.current = false;
    if (!picked) return;
    const err = cite({ text: picked.text, source: picked.source });
    if (err) {
      setError(err);
      return;
    }
    window.getSelection()?.removeAllRanges();
    close();
  }

  if (!picked?.spot) return null;
  const { x, y, below } = picked.spot;
  return (
    <div className={`cite-pop${below ? " below" : ""}`} style={{ left: x, top: y }}>
      {error ? (
        <p className="cite-pop-error" role="alert">
          {error}
        </p>
      ) : (
        <button
          ref={buttonRef}
          type="button"
          className="cite-btn"
          title="Kutip ke kotak tanya"
          onMouseDown={(e) => e.preventDefault()}
          onClick={onCite}
          onKeyDown={(e) => {
            if (e.key !== "Escape") return;
            e.preventDefault();
            window.getSelection()?.removeAllRanges();
            close();
          }}
        >
          <QuotesIcon size={15} weight="fill" /> Kutip
        </button>
      )}
    </div>
  );
}

function QuoteChip({
  quote,
  n,
  onComment,
  onRemove,
  onDone,
}: {
  quote: Quote;
  n: number;
  onComment: (comment: string) => void;
  onRemove: () => void;
  /** Kembali ke kotak tanya. */
  onDone: () => void;
}) {
  const tipId = useId();
  // Esc menyembunyikan tooltip sampai kursor atau fokus pindah.
  const [tipOff, setTipOff] = useState(false);
  const table = parseTable(quote.text);
  const snippet = table
    ? `Tabel ${table.header.join(", ")} · ${table.rows.length} baris`
    : quote.text.replace(/\s+/g, " ");
  return (
    <li
      className={`quote-chip${tipOff ? " tip-off" : ""}`}
      onMouseLeave={() => setTipOff(false)}
      onKeyDown={(e) => {
        if (e.key === "Escape") setTipOff(true);
      }}
    >
      <span
        className="quote-main"
        tabIndex={0}
        aria-label={`Kutipan ${n}`}
        aria-describedby={tipId}
        onBlur={() => setTipOff(false)}
        onKeyDown={(e) => {
          if (e.key !== "Delete" && e.key !== "Backspace") return;
          e.preventDefault();
          onRemove();
        }}
      >
        <QuotesIcon size={14} weight="fill" className="quote-icon" />
        <span className="quote-snippet">{snippet}</span>
        {quote.source && <span className="quote-src">{quote.source}</span>}
      </span>
      <input
        className="quote-comment"
        value={quote.comment}
        placeholder="Komentar (opsional)"
        aria-label={`Komentar untuk kutipan ${n}`}
        onChange={(e) => onComment(e.target.value)}
        onKeyDown={(e) => {
          // Enter di sini tidak mengirim pesan; kembali ke kotak tanya.
          if (e.key !== "Enter" || e.nativeEvent.isComposing) return;
          e.preventDefault();
          onDone();
        }}
      />
      <button type="button" className="icon-btn quote-remove" aria-label={`Hapus kutipan ${n}`} onClick={onRemove}>
        <XIcon size={14} />
      </button>
      <div className="quote-tip" role="tooltip" id={tipId}>
        <div className="quote-tip-box">
          <div className="quote-tip-text">
            <QuoteBody text={quote.text} />
          </div>
          {quote.source && <p className="quote-tip-src">— {quote.source}</p>}
          {quote.comment.trim() && <p className="quote-tip-comment">Komentar: {quote.comment.trim()}</p>}
        </div>
      </div>
    </li>
  );
}

/** Kutipan yang menunggu dikirim, di atas kotak tanya. */
export function QuoteChips({
  quotes,
  onChange,
  onDone,
}: {
  quotes: Quote[];
  onChange: (quotes: Quote[]) => void;
  onDone: () => void;
}) {
  if (!quotes.length) return null;
  return (
    <ul className="quotes" aria-label="Kutipan untuk pesan ini">
      {quotes.map((q, i) => (
        <QuoteChip
          key={q.id}
          quote={q}
          n={i + 1}
          onComment={(comment) => onChange(quotes.map((x) => (x.id === q.id ? { ...x, comment } : x)))}
          onRemove={() => {
            onChange(quotes.filter((x) => x.id !== q.id));
            onDone();
          }}
          onDone={onDone}
        />
      ))}
    </ul>
  );
}

function QuoteBody({ text }: { text: string }) {
  const table = parseTable(text);
  if (!table) return <>{text}</>;
  return (
    <div className="msg-quote-table">
      <table>
        <thead>
          <tr>
            {table.header.map((h, j) => (
              <th key={j}>{h}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {table.rows.map((row, i) => (
            <tr key={i}>
              {row.map((cell, j) => (
                <td key={j}>{cell}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Isi bubble user: kutipan sebagai blockquote dengan komentarnya, lalu pertanyaannya. */
export function UserText({ text }: { text: string }) {
  const { quotes, question } = parseMessage(text);
  if (!quotes.length) return <>{question}</>;
  return (
    <>
      {quotes.map((q, i) => (
        <figure key={i} className="msg-quote">
          <blockquote>
            <QuoteBody text={q.text} />
          </blockquote>
          {q.source && <figcaption>— {q.source}</figcaption>}
          {q.comment && (
            <p className="msg-quote-comment">
              <ArrowBendDownRightIcon size={14} aria-hidden="true" />
              <span>{q.comment}</span>
            </p>
          )}
        </figure>
      ))}
      {question && <div className="msg-question">{question}</div>}
    </>
  );
}
