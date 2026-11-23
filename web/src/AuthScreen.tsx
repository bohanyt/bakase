import { useEffect, useState, type FormEvent } from "react";
import { ClipboardTextIcon, FileXlsIcon, MagnifyingGlassIcon } from "@phosphor-icons/react";
import { api, type AuthOptions, type User } from "./api";
import { GuestAsk } from "./GuestAsk";
import { Mascot } from "./Mascot";
import { SupportLink } from "./SupportLink";
import { ThemeToggle } from "./ThemeToggle";

const HINTS = [
  { icon: MagnifyingGlassIcon, text: "Cari prosedur dan referensi" },
  { icon: ClipboardTextIcon, text: "Ringkas laporan kerja" },
  { icon: FileXlsIcon, text: "Buatkan file Excel dan Word" },
];

export function AuthScreen({ onLogin }: { onLogin: (user: User) => void }) {
  const [mode, setMode] = useState<"login" | "register">("login");
  const [options, setOptions] = useState<AuthOptions | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api<AuthOptions>("/api/auth/options").then(setOptions, () => setOptions(null));
  }, []);

  function switchMode(next: "login" | "register") {
    setMode(next);
    setError(null);
  }

  function focusLogin() {
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

  return (
    <div className="auth">
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
          <h1 className="auth-title">Tanya, cari, beres.</h1>
          <p className="auth-lead">Bakase membantu mencari dan merangkum dokumen kerja, serta membuat file Excel dan Word.</p>

          <div className="auth-card">
            <div className="auth-tabs" role="tablist" aria-label="Masuk atau daftar">
              <button type="button" role="tab" aria-selected={mode === "login"} onClick={() => switchMode("login")}>
                Masuk
              </button>
              <button type="button" role="tab" aria-selected={mode === "register"} onClick={() => switchMode("register")}>
                Daftar
              </button>
            </div>

            {notice && <p className="notice">{notice}</p>}
            {error && <p className="form-error">{error}</p>}

            <form className="auth-form" onSubmit={submit} key={mode}>
              {mode === "register" && (
                <label>
                  Nama lengkap
                  <input id="reg-name" name="name" required autoComplete="name" />
                </label>
              )}
              <label>
                Email kantor
                <input
                  id={`${mode}-email`}
                  name="email"
                  type="email"
                  required
                  autoComplete="email"
                  placeholder={`nama@${domain}`}
                />
              </label>
              <label>
                Password
                <input
                  id={`${mode}-password`}
                  name="password"
                  type="password"
                  required
                  minLength={mode === "register" ? 10 : undefined}
                  autoComplete={mode === "login" ? "current-password" : "new-password"}
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
                      {options?.departments.map((d) => (
                        <option key={d} value={d}>
                          {d}
                        </option>
                      ))}
                    </select>
                  </label>
                  <p className="auth-note">
                    Pakai password baru khusus Hub, minimal 10 karakter. Jangan pakai password Windows atau email
                    kantor. Akun bisa dipakai setelah disetujui admin.
                  </p>
                </>
              )}
              <button className="auth-submit" type="submit" disabled={busy}>
                {busy ? "Sebentar..." : mode === "login" ? "Masuk" : "Kirim pendaftaran"}
              </button>
            </form>
          </div>
        </div>
      </main>

      {options?.guest ? (
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
      )}
    </div>
  );
}
