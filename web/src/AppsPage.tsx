import { ClipboardTextIcon, SquaresFourIcon, TableIcon } from "@phosphor-icons/react";
import type { AppCard } from "./api";

function AppIcon({ id }: { id: string }) {
  switch (id) {
    case "laporan-it":
      return <ClipboardTextIcon size={22} />;
    case "counting-app":
      return <TableIcon size={22} />;
    default:
      return <SquaresFourIcon size={22} />;
  }
}

/** Halaman Aplikasi. Daftar dan izinnya dari server (`/api/apps`). */
export function AppsPage({ apps }: { apps: AppCard[] }) {
  return (
    <div className="page docs-page apps-page">
      <h2>Aplikasi</h2>
      <p className="docs-lead muted">Pekerjaan yang punya langkah sendiri. Hanya aplikasi yang boleh kamu pakai yang muncul di sini.</p>
      <ul className="app-grid">
        {apps.map((app) => (
          <li key={app.id} className="app-card" aria-disabled={!app.ready || undefined}>
            <span className="app-icon" aria-hidden>
              <AppIcon id={app.id} />
            </span>
            <span className="app-text">
              <span className="app-name">
                {app.name}
                {!app.ready && <span className="app-soon">Disiapkan</span>}
              </span>
              <span className="app-desc">{app.description}</span>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}
