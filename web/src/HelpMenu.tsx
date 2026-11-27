import { QuestionIcon, SidebarSimpleIcon, XIcon } from "@phosphor-icons/react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { currentBrowser, type Browser } from "./installBrowser";
import { SupportLink } from "./SupportLink";
import "./HelpMenu.css";

export function HelpMenu({ whatsapp, canInstall, onInstall }: {
  whatsapp?: string | null;
  canInstall: boolean;
  onInstall: () => void;
}) {
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  if (!whatsapp && !canInstall) return null;
  return (
    <>
      <button ref={trigger} type="button" className="icon-btn help-trigger" aria-label="Bantuan" popoverTarget="help-menu">
        <QuestionIcon size={21} />
      </button>
      <div
        ref={menu}
        id="help-menu"
        popover="auto"
        className="help-menu"
        role="group"
        aria-label="Bantuan"
        onToggle={(event) => {
          if (event.newState === "open") event.currentTarget.querySelector<HTMLElement>("button, a")?.focus();
        }}
      >
        {canInstall && (
          <button type="button" onClick={() => {
            menu.current?.hidePopover();
            trigger.current?.focus();
            onInstall();
          }}>
            <SidebarSimpleIcon size={20} />
            <span>Pasang Bakase<small>Tanya sambil buka halaman lain</small></span>
          </button>
        )}
        {canInstall && whatsapp && <hr />}
        <SupportLink whatsapp={whatsapp} menu />
      </div>
    </>
  );
}

type Guide = { address: string; video: string; steps: ReactNode[]; note?: ReactNode };

/** Detik mulai setiap langkah di video; video kedua browser memakai waktu yang sama. */
const STEP_TIMES = [0, 4.4, 8.6, 11, 15.2];

const DROP_STEP = <>Seret file ZIP dari folder Downloads ke halaman itu.</>;
const GUIDES: Record<Exclude<Browser, "firefox">, Guide> = {
  edge: {
    address: "edge://extensions",
    video: "/brand/pasang/edge.mp4",
    steps: [
      <>Nyalakan <strong>Developer mode</strong> (Mode pengembang) di kiri bawah. Kalau tidak terlihat, klik <strong>☰</strong> dulu.</>,
      DROP_STEP,
      <>Klik ikon puzzle, sematkan Bakase, lalu klik <strong>Bakase</strong> dan masuk dengan akun kantor.</>,
    ],
    note: <p>Kalau Edge menawarkan <strong>Turn off extensions</strong>, pilih <strong>Not now</strong>.</p>,
  },
  chrome: {
    address: "chrome://extensions",
    video: "/brand/pasang/chrome.mp4",
    steps: [
      <>Nyalakan <strong>Developer mode</strong> (Mode developer) di kanan atas.</>,
      DROP_STEP,
      <>Klik ikon Bakase di kanan atas, lalu masuk dengan akun kantor.</>,
    ],
  },
};

const BROWSERS: [Browser, string][] = [["edge", "Edge"], ["chrome", "Chrome"], ["firefox", "Firefox"]];

export function ExtensionDialog({ version, onClose }: { version: string; onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [browser, setBrowser] = useState<Browser>(() => currentBrowser(navigator.userAgent));
  useEffect(() => {
    if (!dialog.current?.open) dialog.current?.showModal();
  }, []);
  return (
    <dialog ref={dialog} className="extension-dialog" aria-labelledby="extension-title" onClose={onClose}>
      <div className="extension-dialog-head">
        <h2 id="extension-title">Pasang Bakase</h2>
        <button type="button" className="icon-btn" aria-label="Tutup" onClick={() => dialog.current?.close()}>
          <XIcon size={18} />
        </button>
      </div>
      <p>Cukup sekali. Setelah itu Bakase bisa dibuka di samping halaman apa pun.</p>
      <div className="search-tabs extension-tabs" role="tablist" aria-label="Browser">
        {BROWSERS.map(([id, label]) => (
          <button key={id} type="button" role="tab" aria-selected={browser === id} onClick={() => setBrowser(id)}>
            {label}
          </button>
        ))}
      </div>
      {browser === "firefox"
        ? <p className="extension-none"><strong>Belum ada untuk Firefox.</strong> Pakai Edge atau Chrome dulu.</p>
        : <InstallGuide key={browser} guide={GUIDES[browser]} version={version} />}
    </dialog>
  );
}

function InstallGuide({ guide, version }: { guide: Guide; version: string }) {
  const video = useRef<HTMLVideoElement>(null);
  const [copyStatus, setCopyStatus] = useState("");
  const [step, setStep] = useState(-1);
  const [playback, setPlayback] = useState<"idle" | "playing" | "ended" | "missing">(
    () => matchMedia("(prefers-reduced-motion: reduce)").matches ? "idle" : "playing",
  );
  const blocked = () => setPlayback((now) => now === "playing" ? "idle" : now);
  useEffect(() => {
    if (!matchMedia("(prefers-reduced-motion: reduce)").matches) video.current?.play().catch(blocked);
  }, []);
  async function copyAddress() {
    try {
      await navigator.clipboard.writeText(guide.address);
      setCopyStatus("Disalin. Tempel di tab baru.");
    } catch {
      setCopyStatus("Salin alamat secara manual.");
    }
  }
  const steps = [
    <>
      Unduh file ZIP-nya. Tidak perlu diekstrak.
      <br />
      <a className="extension-download" href="/api/extension.zip" download>Unduh Bakase {version}</a>
    </>,
    <>
      Salin alamat ini, tempel di tab baru, lalu Enter.
      <br />
      <code>{guide.address}</code>
      <button type="button" className="extension-copy" onClick={() => void copyAddress()}>Salin</button>
      <span className="extension-copy-status" role="status">{copyStatus}</span>
    </>,
    ...guide.steps,
  ];
  const hasVideo = playback !== "missing";
  return (
    <>
      <div className={hasVideo ? "extension-guide" : undefined}>
        <ol>
          {steps.map((content, i) => (
            <li key={i} className={i === step ? "on" : undefined}><div>{content}</div></li>
          ))}
        </ol>
        {hasVideo && (
          <div className="extension-video">
            <video
              ref={video}
              src={guide.video}
              muted
              playsInline
              preload="metadata"
              aria-label="Video cara pasang"
              onError={() => setPlayback("missing")}
              onPlay={() => setPlayback("playing")}
              onEnded={() => {
                setPlayback("ended");
                setStep(-1);
              }}
              onTimeUpdate={(event) => {
                const { currentTime, ended } = event.currentTarget;
                if (!ended) setStep(STEP_TIMES.findLastIndex((start) => start <= currentTime));
              }}
            />
            {playback !== "playing" && (
              <button type="button" onClick={() => void video.current?.play().catch(blocked)}>{playback === "ended" ? "Putar lagi" : "Putar"}</button>
            )}
          </div>
        )}
      </div>
      <div className="extension-dialog-foot">
        {guide.note}
        <p><strong>Untuk memperbarui</strong>, unduh lagi lalu seret ke halaman yang sama.</p>
      </div>
    </>
  );
}

export function ExtensionUpdateNotice({ version, onUpdate }: { version: string; onUpdate: () => void }) {
  const storageKey = `app.extensionUpdate.${version}`;
  const [dismissed, setDismissed] = useState(() => {
    try {
      return localStorage.getItem(storageKey) === "dismissed";
    } catch {
      return false;
    }
  });
  if (dismissed) return null;
  return (
    <div className="extension-update">
      <span role="status">Versi baru extension tersedia.</span>
      <button type="button" className="extension-copy" onClick={onUpdate}>Perbarui</button>
      <button type="button" className="icon-btn" aria-label="Tutup pemberitahuan versi baru" onClick={() => {
        setDismissed(true);
        try {
          localStorage.setItem(storageKey, "dismissed");
        } catch {
          /* penyimpanan browser tidak tersedia */
        }
      }}>
        <XIcon size={16} />
      </button>
    </div>
  );
}
