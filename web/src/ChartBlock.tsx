import { DownloadSimpleIcon, TableIcon } from "@phosphor-icons/react";
import { useContext, useEffect, useMemo, useRef, useState } from "react";
import type { CustomRendererProps } from "streamdown";
import { AnswerRunning } from "./answerRunning";
import { parseChartSpec, tableRows, type ChartSpec } from "./chartSpec";
import type { DrawnChart } from "./chartDraw";
import { useTheme } from "./theme";

/** Blok ```chart di jawaban. Selama jawaban masih mengalir, JSON yang belum bisa dibaca
 * dianggap belum lengkap. Setelah selesai, blok yang tidak bisa dibaca tampil sebagai kode
 * supaya isinya tidak hilang. */
export function ChartBlock({ code, isIncomplete }: CustomRendererProps) {
  const running = useContext(AnswerRunning);
  const spec = useMemo(() => parseChartSpec(code), [code]);
  if (!spec) {
    if (isIncomplete || running) return <div className="chart chart-wait">Menyiapkan grafik…</div>;
    return (
      <pre>
        <code>{code}</code>
      </pre>
    );
  }
  return <ChartView spec={spec} />;
}

function ChartView({ spec }: { spec: ChartSpec }) {
  const theme = useTheme();
  const canvas = useRef<HTMLCanvasElement>(null);
  const tip = useRef<HTMLDivElement>(null);
  const drawn = useRef<DrawnChart | null>(null);
  const [table, setTable] = useState(false);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let alive = true;
    import("./chartDraw")
      .then(({ drawChart }) => {
        if (alive && canvas.current && tip.current) drawn.current = drawChart(canvas.current, tip.current, spec);
      })
      .catch(() => alive && setFailed(true));
    return () => {
      alive = false;
      drawn.current?.destroy();
      drawn.current = null;
    };
  }, [spec, theme]);

  const download = async () => {
    const blob = await drawn.current?.png();
    if (!blob) return;
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = `${spec.title.replace(/[\\/:*?"<>|]+/g, "").trim() || "grafik"}.png`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 1000);
  };

  const keys = spec.type === "pie" ? spec.labels : spec.series.length > 1 ? spec.series.map((s) => s.name) : [];
  const rows = tableRows(spec);
  return (
    <figure className={`chart chart-${spec.type}`}>
      {spec.title && <figcaption className="chart-title">{spec.title}</figcaption>}
      {keys.length > 0 && (
        <div className="chart-keys">
          {keys.map((k, i) => (
            <span key={i}>
              <i className={spec.type === "line" ? "line" : undefined} style={{ background: `var(--series-${(i % 6) + 1})` }} />
              {k}
            </span>
          ))}
        </div>
      )}
      {failed ? (
        <p className="chart-wait">Grafik tidak bisa dimuat. Angkanya ada di tabel.</p>
      ) : (
        <div className="chart-plot">
          <canvas ref={canvas} role="img" aria-label={spec.title || "Grafik"} />
          <div ref={tip} className="chart-tip" hidden />
        </div>
      )}
      <div className="chart-actions">
        <button type="button" className="text-btn" aria-expanded={table || failed} onClick={() => setTable((t) => !t)}>
          <TableIcon size={16} /> Tabel
        </button>
        {!failed && (
          <button type="button" className="text-btn" onClick={download}>
            <DownloadSimpleIcon size={16} /> Unduh PNG
          </button>
        )}
      </div>
      {(table || failed) && (
        <table className="chart-table">
          <thead>
            <tr>
              {rows.head.map((h, i) => (
                <th key={i}>{h}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.rows.map((r, i) => (
              <tr key={i}>
                {r.map((c, j) => (
                  <td key={j} className={j ? "num" : undefined}>
                    {c}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </figure>
  );
}
