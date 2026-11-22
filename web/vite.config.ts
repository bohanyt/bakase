import { readdirSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";

function pdfAssets(): Plugin {
  const require = createRequire(import.meta.url);
  const manifest = require.resolve("pdfjs-dist/package.json");
  const root = dirname(manifest);
  const { version } = JSON.parse(readFileSync(manifest, "utf8")) as { version: string };
  // CMaps and image decoders load on demand by filename. The version keeps their cache immutable.
  const assets = [
    "LICENSE",
    "wasm/qcms_bg.wasm",
    "wasm/LICENSE_QCMS",
    "wasm/LICENSE_PDFJS_QCMS",
    ...readdirSync(join(root, "cmaps")).map((file) => `cmaps/${file}`),
    ...readdirSync(join(root, "standard_fonts")).filter((file) => /^(Foxit|LICENSE_FOXIT)/.test(file))
      .map((file) => `standard_fonts/${file}`),
    ...["jbig2", "openjpeg"].flatMap((decoder) => [
      `wasm/${decoder}.wasm`,
      `wasm/${decoder}_nowasm_fallback.js`,
      `wasm/LICENSE_${decoder.toUpperCase()}`,
      `wasm/LICENSE_PDFJS_${decoder.toUpperCase()}`,
    ]),
  ];
  return {
    name: "local-pdf-assets",
    apply: "build",
    generateBundle() {
      for (const file of assets) {
        this.emitFile({ type: "asset", fileName: `assets/pdfjs/${version}/${file}`, source: readFileSync(join(root, file)) });
      }
    },
  };
}

// Saat development, panggilan /api diteruskan ke server Rust.
// changeOrigin: false menjaga header Host tetap localhost:5173, supaya cocok
// dengan header Origin dan lolos pemeriksaan lintas origin di server.
export default defineConfig({
  plugins: [react(), pdfAssets()],
  server: {
    proxy: {
      "/api": { target: "http://127.0.0.1:8100", changeOrigin: false },
    },
  },
});
