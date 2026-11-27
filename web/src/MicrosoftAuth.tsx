import { useEffect, useRef, useState, type ReactNode } from "react";
import { ArrowSquareOutIcon } from "@phosphor-icons/react";
import type { User } from "./api";
import { currentTheme } from "./theme";

import { COPY, type Status } from "./microsoftLogin";

const ERROR_STATUSES: readonly Status[] = ["wrong", "unavailable", "invalid", "disabled", "linked", "pilot"];
const FLOW_MS = 10 * 60 * 1000;

function startMicrosoft(pilot: boolean, tab: boolean, signal: AbortSignal) {
  const target = new URL(location.href);
  target.searchParams.delete("microsoft");
  target.searchParams.delete("ms");
  target.searchParams.delete("ms_tab");
  target.searchParams.delete("ms_theme");
  return fetch("/api/auth/microsoft/start", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      pilot,
      panel_tab: tab,
      return_to: target.pathname + target.search + target.hash,
    }),
    signal,
  }).then(async (response) => {
    if (!response.ok) throw new Error();
    const { url } = await response.json() as { url: string };
    if (!signal.aborted) location.assign(url);
  });
}

function MicrosoftLogo() {
  return (
    <svg className="ms-logo" viewBox="0 0 21 21" aria-hidden="true">
      <path fill="#f25022" d="M0 0h10v10H0z" /><path fill="#7fba00" d="M11 0h10v10H11z" />
      <path fill="#00a4ef" d="M0 11h10v10H0z" /><path fill="#ffb900" d="M11 11h10v10H11z" />
    </svg>
  );
}

type Props = {
  pilot: boolean;
  panel: boolean;
  tab: boolean;
  initial: Status | null;
  visible: boolean;
  legacyMessage: boolean;
  children?: ReactNode;
  onLogin: (user: User) => void;
};

export function MicrosoftAuth({ pilot, panel, tab, initial, visible, legacyMessage, children, onLogin }: Props) {
  const [status, setStatus] = useState<Status | null>(initial);
  const [panelAttempt, setPanelAttempt] = useState(0);
  const fetchStart = useRef<AbortController | null>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const signedInUser = useRef<User | null>(null);
  const page = pilot ? "/?microsoft=pilot" : "/";
  const tabUrl = () => page + (pilot ? "&" : "?") + "ms_tab=1&ms_theme=" + currentTheme();

  useEffect(() => {
    if (!tab || initial || !visible) return;
    const control = new AbortController();
    // Strict Mode cancels the first setup before the request begins.
    queueMicrotask(() => {
      if (control.signal.aborted) return;
      void startMicrosoft(pilot, true, control.signal).catch(() => {
        if (!control.signal.aborted) setStatus("unavailable");
      });
    });
    return () => control.abort();
  }, [initial, pilot, tab, visible]);

  useEffect(() => {
    heading.current?.focus();
  }, [status]);

  useEffect(() => () => fetchStart.current?.abort(), []);

  useEffect(() => {
    if (!panel || status !== "waiting") return;
    const control = new AbortController();
    const end = performance.now() + FLOW_MS;
    let timer = 0;
    async function check() {
      try {
        const response = await fetch("/api/me", { cache: "no-store", signal: control.signal });
        if (response.ok) {
          const user = await response.json() as User;
          if (control.signal.aborted) return;
          signedInUser.current = user;
          setStatus("done");
          return;
        }
      } catch {
        // Koneksi ke panel bisa terputus sesaat saat tab login sedang dibuka.
      }
      if (control.signal.aborted) return;
      if (performance.now() >= end) setStatus("expired");
      else timer = window.setTimeout(() => void check(), 1500);
    }
    timer = window.setTimeout(() => void check(), 1000);
    const deadline = window.setTimeout(() => setStatus("expired"), FLOW_MS);
    return () => {
      control.abort();
      window.clearTimeout(timer);
      window.clearTimeout(deadline);
    };
  }, [panel, panelAttempt, status]);

  useEffect(() => {
    const user = signedInUser.current;
    if (!panel || status !== "done" || !user) return;
    const timer = window.setTimeout(() => onLogin(user), 900);
    return () => window.clearTimeout(timer);
  }, [onLogin, panel, status]);

  function begin() {
    fetchStart.current?.abort();
    const control = new AbortController();
    fetchStart.current = control;
    setStatus("redirect");
    void startMicrosoft(pilot, tab, control.signal).catch(() => {
      if (!control.signal.aborted) setStatus("unavailable");
    });
  }

  function waitForTab() {
    setStatus("waiting");
    setPanelAttempt((count) => count + 1);
  }

  function back() {
    fetchStart.current?.abort();
    if (tab || initial) location.replace(page);
    else setStatus(null);
  }

  const shown = status ?? (tab && visible ? "redirect" : null);
  if (shown) {
    const [title, note, retry] = COPY[shown];
    return (
      <div className={"ms-status" + (ERROR_STATUSES.includes(shown) ? " ms-error" : "")}>
        <h2 tabIndex={-1} ref={heading}>{title}</h2>
        <p>{note}</p>
        {shown === "waiting" && (
          <a className="auth-submit ms-action" href={tabUrl()} target="_blank" rel="noopener noreferrer" onClick={waitForTab}>
            Buka ulang tab masuk
          </a>
        )}
        {retry && visible && (panel ? (
          <a className="auth-submit ms-action" href={tabUrl()} target="_blank" rel="noopener noreferrer" onClick={waitForTab}>
            {retry}
          </a>
        ) : (
          <button type="button" className="auth-submit ms-action" onClick={begin}>{retry}</button>
        ))}
        {shown !== "waiting" && shown !== "done" && !(shown === "redirect" && tab) && (
          <button type="button" className="ms-text" onClick={back}>Kembali ke halaman masuk</button>
        )}
      </div>
    );
  }

  if (panel) {
    return (
      <>
        <a className="auth-submit ms-action" data-microsoft-primary href={tabUrl()} target="_blank" rel="noopener noreferrer" onClick={waitForTab}>
          <MicrosoftLogo /> Masuk dengan Microsoft <ArrowSquareOutIcon aria-hidden="true" size={18} />
        </a>
        <p className="ms-center">Masuk dengan akun kantor di tab baru.</p>
      </>
    );
  }
  return (
    <>
      {legacyMessage && <p className="ms-callout">Kalau dulu kamu masuk dengan password, sekarang gunakan akun Microsoft kantor dan riwayatmu tetap ada.</p>}
      <button className="auth-submit ms-action" type="button" data-microsoft-primary onClick={begin}>
        <MicrosoftLogo /> Masuk dengan Microsoft
      </button>
      <p className="ms-center">Gunakan akun Microsoft kantor.</p>
      {children}
    </>
  );
}
