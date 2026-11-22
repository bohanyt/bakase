// Grafik dan diagram di jawaban Bakase. Library-nya (Chart.js, Mermaid) baru diunduh browser
// saat blok pertama muncul, jadi halaman awal tidak bertambah berat.
import { createElement, type ComponentProps } from "react";
import type { DiagramPlugin, MermaidErrorComponentProps, MermaidOptions, Streamdown } from "streamdown";
import { ChartBlock } from "./ChartBlock";
import { allowedDiagram } from "./diagramGuard";
import type { Theme } from "./theme";

type Plugins = NonNullable<ComponentProps<typeof Streamdown>["plugins"]>;
type MermaidConfig = NonNullable<MermaidOptions["config"]>;
type MermaidInstance = ReturnType<DiagramPlugin["getMermaid"]>;

let loading: Promise<MermaidInstance> | null = null;
// Konfigurasi Mermaid bersifat global, jadi initialize dan render dijalankan satu per satu.
let queue: Promise<unknown> = Promise.resolve();

/** Pengganti `@streamdown/mermaid` yang mengunduh Mermaid saat diagram pertama digambar.
 * Setiap pemanggil menyimpan konfigurasinya sendiri. */
const lazyMermaid: DiagramPlugin = {
  name: "mermaid",
  type: "diagram",
  language: "mermaid",
  getMermaid(initial) {
    let config: MermaidConfig | undefined = initial;
    return {
      initialize(next) {
        config = next;
      },
      async render(id, source) {
        if (!allowedDiagram(source)) throw new Error("Diagram ini tidak didukung.");
        const own = config;
        loading ??= import("@streamdown/mermaid").then(({ createMermaidPlugin }) => createMermaidPlugin().getMermaid());
        const mermaid = await loading;
        const run = queue.then(() => {
          if (own) mermaid.initialize(own);
          return mermaid.render(id, source);
        });
        queue = run.catch(() => undefined);
        return run;
      },
    };
  },
};

export const ANSWER_PLUGINS: Plugins = {
  mermaid: lazyMermaid,
  renderers: [{ language: "chart", component: ChartBlock }],
};

/** Diagram yang tidak bisa atau tidak boleh digambar tampil sebagai kode biasa. */
function DiagramAsCode({ chart }: MermaidErrorComponentProps) {
  return createElement("pre", null, createElement("code", null, chart));
}

/** Warna dan huruf diagram dari token tema, supaya satu keluarga dengan teks jawaban. Diagram
 * duduk di kartu `--side` (lihat App.css): kotak bersudut bulat dengan garis tipis, jarak
 * antar langkah rapat, label Ya/Tidak kecil dan redup. */
export function mermaidOptions(theme: Theme): MermaidOptions {
  const root = getComputedStyle(document.documentElement);
  const v = (name: string) => root.getPropertyValue(name).trim();
  const base = parseFloat(root.fontSize);
  const card = v("--side");
  const faint = v("--faint");
  return {
    errorComponent: DiagramAsCode,
    config: {
      startOnLoad: false,
      securityLevel: "strict",
      theme: "base",
      fontFamily: root.fontFamily,
      themeVariables: {
        darkMode: theme === "dark",
        fontFamily: root.fontFamily,
        fontSize: `${base * 0.9}px`,
        background: card,
        mainBkg: v("--hover"),
        primaryColor: v("--hover"),
        primaryTextColor: v("--fg"),
        primaryBorderColor: v("--composer-line"),
        nodeBorder: v("--composer-line"),
        secondaryColor: card,
        tertiaryColor: card,
        clusterBkg: card,
        lineColor: faint,
        textColor: v("--fg"),
        edgeLabelBackground: card,
      },
      flowchart: { curve: "basis", padding: 10, nodeSpacing: 28, rankSpacing: 30 },
      themeCSS: [
        ".node rect { rx: 10px; ry: 10px; }",
        ".node rect, .node polygon, .node path { stroke-width: 1px; }",
        ".flowchart-link { stroke-width: 1.25px; }",
        `.edgeLabel, .edgeLabel p, .edgeLabel span { font-size: ${base * 0.77}px; color: ${faint}; fill: ${faint}; background: ${card}; }`,
      ].join(" "),
    },
  };
}
