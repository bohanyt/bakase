import { useLayoutEffect, useRef, type ReactNode } from "react";

function positionPopover(popover: HTMLElement, anchor: HTMLElement, alignRight = false) {
  const margin = 8;
  const { left, right, top, bottom } = anchor.getBoundingClientRect();
  const { clientWidth, clientHeight } = document.documentElement;
  const start = alignRight ? right - popover.offsetWidth : left;
  popover.style.left = Math.max(margin, Math.min(start, clientWidth - popover.offsetWidth - margin)) + "px";
  popover.style.top = (bottom + popover.offsetHeight + margin < clientHeight
    ? bottom + 4
    : Math.max(margin, top - popover.offsetHeight - 4)) + "px";
}

export function UserTextTip({ anchor }: { anchor: HTMLElement }) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const element = ref.current!;
    if (!element.matches(":popover-open")) element.showPopover();
    positionPopover(element, anchor);
  }, [anchor]);
  return <div ref={ref} popover="manual" className="user-table-popover user-table-tip" aria-hidden="true">{anchor.textContent}</div>;
}
export function UserPopover({ id, anchor, label, menu, onClose, children }: {
  id: string;
  anchor: HTMLButtonElement;
  label: string;
  menu?: boolean;
  onClose: () => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const popover = ref.current!;
    const position = () => positionPopover(popover, anchor, menu);
    if (!popover.matches(":popover-open")) {
      anchor.focus({ preventScroll: true });
      popover.showPopover({ source: anchor });
    }
    position();
    popover.querySelector<HTMLElement>("input:enabled, button:enabled")?.focus({ preventScroll: true });
    window.addEventListener("resize", position);
    return () => window.removeEventListener("resize", position);
  }, [anchor, menu]);

  return (
    <div
      id={id}
      ref={ref}
      popover="auto"
      role={menu ? "group" : "dialog"}
      aria-label={label}
      className={menu ? "user-table-popover user-table-menu" : "user-table-popover"}
      onKeyDown={(event) => {
        if (event.key !== "Escape" || event.defaultPrevented) return;
        event.preventDefault();
        event.currentTarget.hidePopover();
        onClose();
        requestAnimationFrame(() => anchor.focus({ preventScroll: true }));
      }}
      onToggle={(event) => {
        if (event.currentTarget.matches(":popover-open")) return;
        onClose();
        if (document.activeElement === document.body) anchor.focus({ preventScroll: true });
      }}
    >
      {children}
    </div>
  );
}

export type UserConfirmation = {
  anchor: HTMLElement;
  title: string;
  description: string;
  label: string;
  run: () => void;
};

export function UserConfirm({ confirmation, onClose }: { confirmation: UserConfirmation; onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  useLayoutEffect(() => {
    const element = dialog.current!;
    if (!element.open) {
      confirmation.anchor.focus({ preventScroll: true });
      element.showModal();
    }
  }, [confirmation.anchor]);

  return (
    <dialog
      ref={dialog}
      className="user-table-confirm"
      aria-label={confirmation.title}
      onClose={(event) => {
        const accepted = event.currentTarget.returnValue === "ok";
        onClose();
        confirmation.anchor.focus({ preventScroll: true });
        if (accepted) confirmation.run();
      }}
    >
      <form method="dialog">
        <h3>{confirmation.title}</h3>
        <p>{confirmation.description}</p>
        <div className="user-table-pop-actions">
          <button value="cancel" autoFocus>Batal</button>
          <button className="user-table-strong" value="ok">{confirmation.label}</button>
        </div>
      </form>
    </dialog>
  );
}
