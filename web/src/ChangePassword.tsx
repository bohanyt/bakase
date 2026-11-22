import { useState, type FormEvent } from "react";
import { api } from "./api";

export function ChangePassword({ onDone }: { onDone: () => void }) {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const form = new FormData(e.currentTarget);
    const value = (k: string) => String(form.get(k) ?? "");
    setError(null);
    if (value("new") !== value("new2")) {
      setError("Password baru dan ulangannya tidak sama.");
      return;
    }
    setBusy(true);
    try {
      await api("/api/me/password", { body: { current: value("current"), new: value("new") } });
      onDone();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Gagal mengganti password.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="page narrow">
      <h2>Ganti password</h2>
      {error && <p className="form-error">{error}</p>}
      <form className="auth-form" onSubmit={submit}>
        <label>
          Password sekarang
          <input id="pw-current" name="current" type="password" required autoComplete="current-password" />
        </label>
        <label>
          Password baru
          <input id="pw-new" name="new" type="password" required minLength={10} autoComplete="new-password" />
        </label>
        <label>
          Ulangi password baru
          <input id="pw-new2" name="new2" type="password" required autoComplete="new-password" />
        </label>
        <p className="muted small">Perangkat lain yang masih masuk akan otomatis keluar.</p>
        <button className="primary" type="submit" disabled={busy}>
          {busy ? "Sebentar..." : "Simpan password"}
        </button>
      </form>
    </div>
  );
}
