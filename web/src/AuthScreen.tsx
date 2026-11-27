import { useEffect, useState, type FormEvent } from "react";
import { ClipboardTextIcon, FileXlsIcon, MagnifyingGlassIcon } from "@phosphor-icons/react";
import { api, type AuthOptions, type User } from "./api";
import { GuestAsk } from "./GuestAsk";
import { Mascot } from "./Mascot";
import { MicrosoftAuth } from "./MicrosoftAuth";
import { microsoftParams, microsoftResult } from "./microsoftLogin";
import { inPanel } from "./pageBridge";
import { SupportLink } from "./SupportLink";
import { ThemeToggle } from "./ThemeToggle";

const HINTS = [
  { icon: MagnifyingGlassIcon, text: "Cari prosedur dan referensi" },
  { icon: ClipboardTextIcon, text: "Ringkas laporan kerja" },
  { icon: FileXlsIcon, text: "Buatkan file Excel dan Word" },
];

export function AuthScreen({ options, onLogin, note, panelTabReady = false }: {
  options: AuthOptions | null | undefined;
  /** Kenapa layar ini muncul, misalnya sesi berakhir di tengah kerja. */
  note?: string;
  onLogin: (user: User) => void;
  panelTabReady?: boolean;
}) {
  const [mode, setMode] = useState<"login" | "register">("login");
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const { pilot, tab, result } = microsoftParams();
  const panel = inPanel();
  const full = options?.microsoft === "full";
  const microsoft = full || (options?.microsoft === "pilot" && pilot);
  const initial = panelTabReady ? "done" : tab && result === "done" ? "expired" : microsoftResult(result);
  const showMicrosoft = microsoft || initial !== null;
  const panelLayout = showMicrosoft && panel;
  const hideVisual = showMicrosoft && (panel || tab);

  useEffect(() => {
    if (!tab) return;
    const theme = new URLSearchParams(location.search).get("ms_theme");
    if (theme === "light" || theme === "dark") document.documentElement.dataset.theme = theme;
  }, [tab]);

  function switchMode(next: "login" | "register") {
    setMode(next);
    setError(null);
  }

  function focusLogin() {
    if (showMicrosoft) {
      (document.querySelector("[data-microsoft-primary]") as HTMLElement | null)?.focus();
      return;
    }
    switchMode("login");
    requestAnimationFrame(() => document.getElementById("login-email")?.focus());
  }

  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const form = new FormData(e.currentTarget);
    const value = (k: string) => String(form.get(k) ?? "");
    setError(null);
    setNotice(null);
    setBusy(true);
    try {
      if (mode === "login") {
        onLogin(await api<User>("/api/auth/login", { body: { email: value("email"), password: value("password") } }));
      } else {
        if (value("password") !== value("password2")) throw new Error("Password dan ulangan password tidak sama.");
        const res = await api<{ message: string }>("/api/auth/register", {
          body: {
            name: value("name"),
            email: value("email"),
            password: value("password"),
            department: value("department"),
          },
        });
        setNotice(res.message);
        setMode("login");
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Terjadi kesalahan.");
    } finally {
      setBusy(false);
    }
  }

  const domain = options?.email_domain ?? "example.com";

  const passwordForm = (
    <form className="auth-form" onSubmit={submit} key={mode}>
      {notice && <p className="notice">{notice}</p>}
      {error && <p className="form-error" role="status">{error}</p>}
      {mode === "register" && (
        <label>
          Nama lengkap
          <input id="reg-name" name="name" required autoComplete="name" />
        </label>
      )}
      <label>
        Email kantor
        <input
          id={mode + "-email"}
          name="email"
          type="email"
          required
          autoComplete={full ? "off" : "email"}
          placeholder={"nama@" + domain}
        />
      </label>
      <label>
        Password
        <input
          id={mode + "-password"}
          name="password"
          type="password"
          required
          minLength={mode === "register" ? 10 : undefined}
          autoComplete={full ? "off" : mode === "login" ? "current-password" : "new-password"}
        />
      </label>
      {mode === "register" && (
        <>
          <label>
            Ulangi password
            <input id="reg-password2" name="password2" type="password" required autoComplete="new-password" />
          </label>
          <label>
            Departemen
            <select id="reg-department" name="department" defaultValue="">
              <option value="">Pilih departemen</option>
              {options?.departments.map((d) => <option key={d} value={d}>{d}</option>)}
            </select>
          </label>
          <p className="auth-note">
            Pakai password baru khusus Hub, minimal 10 karakter. Jangan pakai password Windows atau email
            kantor. Akun bisa dipakai setelah disetujui admin.
          </p>
        </>
      )}
      <button className={"auth-submit" + (full ? " ms-action" : "")} type="submit" disabled={busy}>
        {busy ? "Sebentar..." : full ? "Masuk sebagai admin" : mode === "login" ? "Masuk" : "Kirim pendaftaran"}
      </button>
    </form>
  );

  return (
    <div className={"auth" + (showMicrosoft ? " auth-microsoft" : "") + (hideVisual ? " auth-microsoft-single" : "") + (panelLayout ? " auth-microsoft-panel" : "")}>
      <div className="auth-theme">
        <SupportLink whatsapp={options?.support_whatsapp} />
        <ThemeToggle />
      </div>

      <main className="auth-main">
        <div className="auth-brand">
          <img src="/brand/mascot.png" alt="" width={32} height={32} />
          <span>Bakase</span>
        </div>

        <div className="auth-body">
          <Mascot className="auth-mascot-small" asset="welcome" placement="loginSmall" />
          <h1 className="auth-title">{panelLayout ? "Bakase" : "Tanya, cari, beres."}</h1>
          {!hideVisual && <p className="auth-lead">Bakase membantu mencari dan merangkum dokumen kerja, serta membuat file Excel dan Word.</p>}

          {(showMicrosoft || options !== undefined) && (
            <div className="auth-card">
              {note && <p className="auth-note" role="status">{note}</p>}
              {showMicrosoft && (
                <MicrosoftAuth pilot={pilot} panel={panel} tab={tab} initial={initial} visible={microsoft} legacyMessage={full && !panel && !tab} onLogin={onLogin}>
                  {full && !panel && !tab && (
                    <details className="ms-admin">
                      <summary className="ms-text">Masuk dengan password (admin)</summary>
                      {passwordForm}
                    </details>
                  )}
                </MicrosoftAuth>
              )}
              {!full && !initial && !hideVisual && (
                <>
                  <div className="auth-tabs" role="tablist" aria-label="Masuk atau daftar">
                    <button type="button" role="tab" aria-selected={mode === "login"} onClick={() => switchMode("login")}>
                      Masuk
                    </button>
                    <button type="button" role="tab" aria-selected={mode === "register"} onClick={() => switchMode("register")}>
                      Daftar
                    </button>
                  </div>
                  {passwordForm}
                </>
              )}
            </div>
          )}
        </div>
      </main>

      {!hideVisual && (options?.guest ? (
        <aside className="auth-visual" aria-label="Tanya tanpa login">
          <GuestAsk onWantLogin={focusLogin} />
        </aside>
      ) : (
        <aside className="auth-visual" aria-hidden="true">
          <div className="auth-stage">
            <p className="auth-bubble">Halo! Aku Bakase. Mau cari apa hari ini?</p>
            <Mascot className="auth-mascot" asset="welcome" placement="login" />
            <ul className="auth-hints">
              {HINTS.map(({ icon: Icon, text }) => (
                <li key={text}>
                  <Icon size={18} weight="duotone" />
                  {text}
                </li>
              ))}
            </ul>
          </div>
        </aside>
      ))}
    </div>
  );
}
