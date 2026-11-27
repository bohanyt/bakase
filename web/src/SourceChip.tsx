import { createContext, useContext, useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { FileIcon } from "./FileIcon";
import { kindLabel, placeLabel, sourceName, type SourceRef } from "./messages";

type OpenCard = { group: string; chunks: string[]; pinned: boolean };
type CardContext = {
  id: string;
  active: OpenCard | null;
  show: (group: string, sources: SourceRef[], button: HTMLButtonElement, pinned: boolean, keyboard?: boolean) => void;
  close: () => void;
  connect: (group: string, button: HTMLButtonElement) => void;
  cancelClose: () => void;
  scheduleClose: () => void;
};

const SourceCardContext = createContext<CardContext | null>(null);

function positionCard(card: HTMLElement, button: HTMLElement) {
  const margin = 10;
  const gap = 5;
  const { left, top, bottom } = button.getBoundingClientRect();
  const { clientWidth, clientHeight } = document.documentElement;
  const above = top - margin - gap;
  const below = clientHeight - bottom - margin - gap;
  const putAbove = above > below;
  card.style.maxHeight = `${Math.max(0, putAbove ? above : below)}px`;
  card.style.left = `${Math.max(margin, Math.min(left, clientWidth - card.offsetWidth - margin))}px`;
  card.style.top = `${putAbove ? top - card.offsetHeight - gap : bottom + gap}px`;
}

export function SourceCards({ sources, onOpen, children }: {
  sources: readonly SourceRef[];
  onOpen: (source: SourceRef) => void;
  children: ReactNode;
}) {
  const id = useId();
  const root = useRef<HTMLDivElement>(null);
  const pop = useRef<HTMLSpanElement>(null);
  const anchor = useRef<HTMLButtonElement>(null);
  const closeTimer = useRef<number>(undefined);
  const focus = useRef<"trigger" | "first" | null>(null);
  const [active, setActive] = useState<OpenCard | null>(null);
  const selected = active?.chunks.flatMap((chunk) => sources.find((source) => source.chunk_id === chunk) ?? []) ?? [];

  function cancelClose() {
    window.clearTimeout(closeTimer.current);
  }

  function close() {
    cancelClose();
    pop.current?.hidePopover();
    setActive(null);
    focus.current = null;
  }

  function show(group: string, refs: SourceRef[], button: HTMLButtonElement, pinned: boolean, keyboard = false) {
    if (!pinned && document.querySelector(".source-pop[data-pinned]:popover-open")) return;
    cancelClose();
    focus.current = keyboard ? "first" : document.activeElement === button ? "trigger" : null;
    setActive({ group, chunks: refs.map((source) => source.chunk_id), pinned });
  }

  function scheduleClose() {
    cancelClose();
    if (active?.pinned) return;
    closeTimer.current = window.setTimeout(() => {
      if (!pop.current?.matches(":hover, :focus-within")) close();
    }, 220);
  }

  function connect(group: string, button: HTMLButtonElement) {
    const card = pop.current;
    if (!card || active?.group !== group || !card.matches(":popover-open")) return;
    positionCard(card, button);
    const restoreFocus = focus.current === "trigger" && button !== anchor.current && document.activeElement === document.body;
    anchor.current = button;
    if (restoreFocus) button.focus();
  }

  // Markdown bisa mengganti tombol saat daftar rapat berubah menjadi daftar longgar.
  useLayoutEffect(() => {
    const card = pop.current;
    if (!card || !active) return;
    const button = root.current?.querySelector<HTMLButtonElement>(`[data-source-group="${CSS.escape(active.group)}"]`);
    if (!button || !selected.length) {
      close();
      return;
    }
    if (!card.matches(":popover-open")) card.showPopover({ source: button });
    connect(active.group, button);
    if (focus.current === "first") {
      focus.current = null;
      card.querySelector("button")?.focus();
    }
  });

  const open = !!active;
  useEffect(() => {
    if (!open) return;
    const reposition = () => {
      if (pop.current && anchor.current?.isConnected) positionCard(pop.current, anchor.current);
    };
    document.addEventListener("scroll", reposition, true);
    window.addEventListener("resize", reposition);
    return () => {
      document.removeEventListener("scroll", reposition, true);
      window.removeEventListener("resize", reposition);
      cancelClose();
    };
  }, [open]);

  return (
    <SourceCardContext.Provider value={{ id, active, show, close, connect, cancelClose, scheduleClose }}>
      <div className="md" data-quote-source="Jawaban Bakase" ref={root} onKeyDown={(event) => {
        if (event.key !== "Escape" || !active || !pop.current?.matches(":popover-open")) return;
        event.preventDefault();
        close();
        anchor.current?.focus();
      }} onFocus={() => {
        if (!active) return;
        focus.current = document.activeElement === anchor.current ? "trigger" : null;
      }} onBlur={(event) => {
        const next = event.relatedTarget;
        if (active && next && next !== document.body && next !== anchor.current && !pop.current?.contains(next)) close();
      }}>
        {children}
        <span ref={pop} id={id} className="source-pop" popover="auto" data-pinned={active?.pinned || undefined}
          role="group" aria-label="Sumber bagian ini"
          onToggle={(event) => {
            if (!event.currentTarget.matches(":popover-open")) setActive(null);
          }}
          onPointerEnter={cancelClose} onPointerLeave={scheduleClose}>
          <span className="source-pop-title" aria-hidden="true">Sumber bagian ini</span>
          {selected.map((source) => {
            const place = source.image?.place || (source.sheet && `Sheet ${source.sheet}`) || placeLabel(source.kind, source.page);
            return (
              <button key={source.chunk_id} type="button" className="source-pop-row" onClick={() => {
                close();
                anchor.current?.focus();
                onOpen(source);
              }}>
                <span aria-hidden="true"><FileIcon kind={source.kind} size={17} /></span>
                <span>
                  <strong>{sourceName(source)}</strong>
                  <small>{place || kindLabel(source.kind)}{source.scan ? " · scan" : ""}</small>
                </span>
              </button>
            );
          })}
        </span>
      </div>
    </SourceCardContext.Provider>
  );
}

export function SourceChip({ group, sources }: { group: string; sources: SourceRef[] }) {
  const card = useContext(SourceCardContext);
  if (!card) return null;
  const first = sources[0];
  const more = sources.length - 1;
  const expanded = card.active?.group === group;
  return (
    <button type="button" className="source-chip" data-source-group={group}
      ref={(button) => { if (button) card.connect(group, button); }}
      aria-label={`Sumber bagian ini, ${sourceName(first)}${more ? ` dan ${more} sumber lain` : ""}`}
      aria-controls={card.id} aria-expanded={expanded}
      onPointerEnter={(event) => {
        card.cancelClose();
        if (event.pointerType === "mouse" && !event.buttons && !expanded && !card.active?.pinned) {
          card.show(group, sources, event.currentTarget, false);
        }
      }}
      onPointerLeave={() => { if (expanded) card.scheduleClose(); }}
      onClick={(event) => {
        if (expanded && card.active?.pinned) card.close();
        else card.show(group, sources, event.currentTarget, true, event.detail === 0);
      }}>
      <span className="source-chip-icon" aria-hidden="true"><FileIcon kind={first.kind} size={13} /></span>
      <span className="source-title">{sourceName(first)}</span>
      {more > 0 && <strong className="source-more">+{more}</strong>}
    </button>
  );
}
