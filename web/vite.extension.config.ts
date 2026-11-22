import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

// Membundel pembaca halaman extension (extension/src/snapshot.js + dom-accessibility-api) jadi
// satu file IIFE di extension/lib/. Hasilnya ikut di-commit supaya extension bisa dipasang
// langsung dari folder repo (Load unpacked) tanpa build.
const here = (path: string) => fileURLToPath(new URL(path, import.meta.url));
// Lisensi MIT mewajibkan teks izinnya ikut di setiap salinan, jadi disalin utuh ke bundel.
const license = readFileSync(here("./node_modules/dom-accessibility-api/LICENSE.md"), "utf8").trim().replace(/\*\//g, "* /");

export default defineConfig({
  // Folder public/ milik web tidak ikut ke extension.
  publicDir: false,
  resolve: {
    // Sumbernya di luar web/, jadi node_modules di sini ditunjuk langsung.
    alias: { "dom-accessibility-api": here("./node_modules/dom-accessibility-api/dist/index.mjs") },
  },
  build: {
    lib: {
      entry: here("../extension/src/snapshot.js"),
      name: "AppSnapshot",
      formats: ["iife"],
      fileName: () => "snapshot.js",
    },
    outDir: here("../extension/lib"),
    emptyOutDir: true,
    minify: false,
    rolldownOptions: {
      output: {
        banner:
          "/*! Dibuat oleh `npm run build:extension` di web/ dari extension/src/snapshot.js. Jangan diedit langsung.\n" +
          ` * Berisi dom-accessibility-api 0.7.1:\n\n${license}\n */`,
      },
    },
  },
});
