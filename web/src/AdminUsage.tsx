import { useEffect, useState } from "react";
import { api } from "./api";

type Totals = {
  turns: number;
  input: number;
  cached: number;
  output: number;
  reasoning: number;
  requests: number;
  reported: number;
  cost_usd: number;
};
type Period = Totals & { period: string };
type UserUsage = Totals & {
  user_id: number | null;
  name: string | null;
  email: string | null;
  department: string | null;
  kind: string;
};
type Report = {
  month: string;
  months: Period[];
  total: Totals;
  days: Period[];
  users: UserUsage[];
  ingest: Totals | null;
  pricing: { input_usd: number; cached_input_usd: number; output_usd: number };
  rate: { usd_idr: number; date: string | null; source: "otomatis" | "config" } | null;
};

const rupiah = new Intl.NumberFormat("id-ID", { style: "currency", currency: "IDR", maximumFractionDigits: 0 });
const compact = new Intl.NumberFormat("id-ID", { notation: "compact", maximumFractionDigits: 1 });
const whole = new Intl.NumberFormat("id-ID");
const monthName = new Intl.DateTimeFormat("id-ID", { month: "long", year: "numeric", timeZone: "UTC" });
const dayName = new Intl.DateTimeFormat("id-ID", { weekday: "short", day: "numeric", month: "short", timeZone: "UTC" });
const rateDate = new Intl.DateTimeFormat("id-ID", { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" });

function usd(n: number): string {
  const digits = n === 0 ? 2 : n < 0.01 ? 4 : n < 1 ? 3 : 2;
  return `$${n.toLocaleString("id-ID", { minimumFractionDigits: digits, maximumFractionDigits: digits })}`;
}

/** Tanggal `YYYY-MM` atau `YYYY-MM-DD` dibaca sebagai UTC supaya tidak bergeser zona waktu. */
function utcDate(period: string): Date {
  return new Date(period.length === 7 ? `${period}-01T00:00:00Z` : `${period}T00:00:00Z`);
}

/** Batas atas sumbu yang bulat: 1, 2, 2,5, 5, atau 10 kali pangkat sepuluh. */
function niceMax(n: number): number {
  if (n <= 0) return 1;
  const p = 10 ** Math.floor(Math.log10(n));
  return ([1, 2, 2.5, 5, 10].find((s) => s * p >= n) ?? 10) * p;
}

/** Bulan berjalan dalam WIB (`YYYY-MM`). */
function currentMonth(): string {
  return new Date(Date.now() + 7 * 3600 * 1000).toISOString().slice(0, 7);
}

function userLabel(u: UserUsage): string {
  if (u.kind === "tamu") return "Tamu (tanpa login)";
  return u.name ?? u.email ?? `User #${u.user_id}`;
}

function DailyChart({ report, money }: { report: Report; money: (usd: number) => string }) {
  // Sumbu dibulatkan dalam mata uang yang tampil (Rupiah kalau ada kurs).
  const scale = report.rate?.usd_idr ?? 1;
  const [hover, setHover] = useState<number | null>(null);
  const start = utcDate(report.month);
  const daysInMonth = new Date(Date.UTC(start.getUTCFullYear(), start.getUTCMonth() + 1, 0)).getUTCDate();
  const byDate = new Map(report.days.map((d) => [d.period, d]));
  const columns = Array.from({ length: daysInMonth }, (_, i) => {
    const period = `${report.month}-${String(i + 1).padStart(2, "0")}`;
    return { period, day: byDate.get(period) };
  });
  const peak = Math.max(0, ...report.days.map((d) => d.cost_usd));
  const top = niceMax(peak * scale) / scale;
  const peakIndex = peak > 0 ? columns.findIndex((c) => c.day?.cost_usd === peak) : -1;
  const ticks = [top, top / 2, 0];

  return (
    <figure className="uc">
      <figcaption className="uc-title">Biaya per hari</figcaption>
      <div className="uc-plot">
        <div className="uc-axis" aria-hidden="true">
          {ticks.map((t) => (
            <span key={t} style={{ bottom: `${(t / top) * 100}%` }}>
              {money(t)}
            </span>
          ))}
        </div>
        <div className="uc-area">
          {ticks.map((t) => (
            <div key={t} className="uc-grid" style={{ bottom: `${(t / top) * 100}%` }} aria-hidden="true" />
          ))}
          {columns.map((c, i) => {
            const cost = c.day?.cost_usd ?? 0;
            const label = `${dayName.format(utcDate(c.period))}: ${money(cost)}, ${c.day?.turns ?? 0} giliran`;
            return (
              <button
                key={c.period}
                type="button"
                className={`uc-col${hover === i ? " on" : ""}`}
                aria-label={label}
                onPointerEnter={() => setHover(i)}
                onPointerLeave={() => setHover(null)}
                onFocus={() => setHover(i)}
                onBlur={() => setHover(null)}
              >
                {cost > 0 && <span className="uc-bar" style={{ height: `${(cost / top) * 100}%` }} />}
                {i === peakIndex && hover === null && (
                  <span className="uc-peak" style={{ bottom: `${(cost / top) * 100}%` }}>
                    {money(cost)}
                  </span>
                )}
                {hover === i && (
                  <span
                    className={`uc-tip${i < 4 ? " left" : i > daysInMonth - 5 ? " right" : ""}`}
                    role="presentation"
                  >
                    <strong>{money(cost)}</strong>
                    <span>
                      {dayName.format(utcDate(c.period))} · {c.day?.turns ?? 0} giliran
                    </span>
                  </span>
                )}
              </button>
            );
          })}
        </div>
      </div>
      <div className="uc-days" aria-hidden="true">
        {columns.map((c, i) => (
          <span key={c.period}>{i === 0 || (i + 1) % 7 === 1 ? i + 1 : ""}</span>
        ))}
      </div>
    </figure>
  );
}

export function AdminUsage() {
  const [month, setMonth] = useState<string | null>(null);
  const [report, setReport] = useState<Report | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Dinaikkan oleh tombol "Coba lagi" untuk memuat ulang bulan yang sama.
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let live = true;
    api<Report>(month ? `/api/admin/usage?month=${encodeURIComponent(month)}` : "/api/admin/usage").then(
      (r) => live && setReport(r),
      (err) => live && setError(err instanceof Error ? err.message : "Gagal memuat pemakaian."),
    );
    return () => {
      live = false;
    };
  }, [month, attempt]);

  const retry = (
    <button type="button" className="link-btn" onClick={() => (setError(null), setAttempt((n) => n + 1))}>
      Coba lagi
    </button>
  );
  if (!report) {
    return error ? (
      <p className="form-error">
        {error} {retry}
      </p>
    ) : (
      <p className="muted">Memuat…</p>
    );
  }

  const rate = report.rate;
  const money = (n: number) => (rate ? rupiah.format(n * rate.usd_idr) : usd(n));
  const t = report.total;
  const people = report.users.filter((u) => u.kind !== "tamu").length;
  const rows: { key: string; label: string; email: string | null; department: string; totals: Totals }[] = report.users.map((u) => ({
    key: `${u.kind}-${u.user_id ?? "tamu"}`,
    label: userLabel(u),
    email: u.name ? u.email : null,
    department: u.kind === "tamu" ? "–" : (u.department ?? "Tanpa departemen"),
    totals: u,
  }));
  if (report.ingest) {
    rows.push({
      key: "ingest_vision",
      label: "Baca scan dan foto (ingest)",
      email: null,
      department: "–",
      totals: report.ingest,
    });
  }
  const cachedShare = t.input > 0 ? Math.round((t.cached / t.input) * 100) : 0;
  const unreported = t.requests - t.reported;
  // Bulan berjalan dan bulan yang sedang tampil selalu bisa dipilih, walau belum ada pemakaian.
  const monthOptions = [
    ...new Set([currentMonth(), report.month, ...report.months.map((m) => m.period)]),
  ].sort((a, b) => b.localeCompare(a));
  const p = report.pricing;
  // Bulan yang dipilih belum selesai dimuat; data di bawah masih bulan sebelumnya.
  const loading = !error && month !== null && month !== report.month;

  return (
    <div className={`usage${loading ? " loading" : ""}`} aria-busy={loading}>
      <div className="usage-bar">
        <label>
          <span className="muted small">Bulan</span>
          <select value={month ?? report.month} onChange={(e) => (setError(null), setMonth(e.target.value))}>
            {monthOptions.map((m) => (
              <option key={m} value={m}>
                {monthName.format(utcDate(m))}
              </option>
            ))}
          </select>
        </label>
        {loading && <span className="muted small">Memuat…</span>}
        {error && (
          <span className="form-error small">
            {error} {retry}
          </span>
        )}
      </div>

      <div className="usage-tiles">
        <div className="tile">
          <span className="tile-label">Biaya</span>
          <strong className="tile-value">{money(t.cost_usd)}</strong>
          <span className="tile-sub">{rate ? usd(t.cost_usd) : "kurs belum tersedia"}</span>
        </div>
        <div className="tile">
          <span className="tile-label">Giliran</span>
          <strong className="tile-value">{whole.format(t.turns)}</strong>
          <span className="tile-sub">{people} user login</span>
        </div>
        <div className="tile">
          <span className="tile-label">Token input</span>
          <strong className="tile-value">{compact.format(t.input)}</strong>
          <span className="tile-sub">{cachedShare}% dari cache</span>
        </div>
        <div className="tile">
          <span className="tile-label">Token output</span>
          <strong className="tile-value">{compact.format(t.output)}</strong>
          <span className="tile-sub">{compact.format(t.reasoning)} untuk berpikir</span>
        </div>
      </div>

      {t.requests === 0 ? (
        <p className="muted">Belum ada pemakaian di bulan ini.</p>
      ) : (
        <>
          <DailyChart report={report} money={money} />
          <details className="usage-days">
            <summary>Tabel per hari</summary>
            <div className="table-wrap">
              <table>
                <thead>
                  <tr>
                    <th>Tanggal</th>
                    <th className="num">Giliran</th>
                    <th className="num">Input</th>
                    <th className="num">Cached</th>
                    <th className="num">Output</th>
                    <th className="num">Biaya</th>
                  </tr>
                </thead>
                <tbody>
                  {report.days.map((d) => (
                    <tr key={d.period}>
                      <td>{dayName.format(utcDate(d.period))}</td>
                      <td className="num">{whole.format(d.turns)}</td>
                      <td className="num">{whole.format(d.input)}</td>
                      <td className="num">{whole.format(d.cached)}</td>
                      <td className="num">{whole.format(d.output)}</td>
                      <td className="num">{money(d.cost_usd)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </details>

          <h3 className="usage-h">Pemakaian menurut sumber</h3>
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>Sumber</th>
                  <th>Departemen</th>
                  <th className="num">Giliran</th>
                  <th className="num">Input</th>
                  <th className="num">Output</th>
                  <th className="num">Biaya</th>
                  <th className="num">Bagian</th>
                </tr>
              </thead>
              <tbody>
                {rows.map(({ key, label, email, department, totals }) => (
                  <tr key={key}>
                    <td>
                      <div>{label}</div>
                      {email && <div className="muted small">{email}</div>}
                    </td>
                    <td>{department}</td>
                    <td className="num">{key === "ingest_vision" ? "–" : whole.format(totals.turns)}</td>
                    <td className="num">{compact.format(totals.input)}</td>
                    <td className="num">{compact.format(totals.output)}</td>
                    <td className="num">{money(totals.cost_usd)}</td>
                    <td className="num">
                      {t.cost_usd > 0 ? `${Math.round((totals.cost_usd / t.cost_usd) * 100)}%` : "–"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}

      {report.months.length > 1 && (
        <>
          <h3 className="usage-h">Per bulan</h3>
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>Bulan</th>
                  <th className="num">Giliran</th>
                  <th className="num">Input</th>
                  <th className="num">Output</th>
                  <th className="num">Biaya</th>
                </tr>
              </thead>
              <tbody>
                {report.months.map((m) => (
                  <tr key={m.period}>
                    <td>
                      <button type="button" className="link-btn" onClick={() => setMonth(m.period)}>
                        {monthName.format(utcDate(m.period))}
                      </button>
                    </td>
                    <td className="num">{whole.format(m.turns)}</td>
                    <td className="num">{compact.format(m.input)}</td>
                    <td className="num">{compact.format(m.output)}</td>
                    <td className="num">{money(m.cost_usd)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}

      <p className="muted small usage-note">
        Tarif per 1 juta token: input {usd(p.input_usd)}, cached {usd(p.cached_input_usd)}, output{" "}
        {usd(p.output_usd)}.{" "}
        {rate
          ? rate.source === "otomatis" && rate.date
            ? `Kurs 1 USD = ${rupiah.format(rate.usd_idr)} (kurs referensi ECB, ${rateDate.format(utcDate(rate.date))}), diperbarui otomatis.`
            : `Kurs 1 USD = ${rupiah.format(rate.usd_idr)} dari config, karena kurs otomatis belum berhasil diambil.`
          : "Kurs USD-IDR belum tersedia, jadi biaya ditampilkan dalam USD."}{" "}
        Rupiah memakai kurs terbaru untuk semua tanggal.
        {unreported > 0 &&
          ` ${whole.format(unreported)} permintaan terputus sebelum selesai; tokennya tidak dilaporkan Azure, jadi belum masuk biaya.`}
      </p>
    </div>
  );
}
