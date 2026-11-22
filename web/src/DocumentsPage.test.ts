import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { resolve } from "node:path";
import { test } from "node:test";
import { createServer } from "vite";

const fixture = `
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { DocumentsPage } from "/src/DocumentsPage.tsx";
import "/src/App.css";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
document.documentElement.dataset.theme = "light";
const nativeFetch = window.fetch;
const statuses = { scan: 1, tidak_didukung: 2, ok: 1 };
const sources = [{ name: "Review", total: 2 }, { name: "Umum", total: 2 }];
const wanted = ["scan", "gambar", "gagal", "tidak_didukung", "terlalu_besar", "kosong"];
const ready = {
  id: 1, title: "cafef00d_Catatan_Kegiatan_EXTRACTED", display_title: "Catatan Kegiatan",
  source: "Review", folder: "", kind: "txt", status: "ok", modified: Date.UTC(2026, 10, 19, 12),
  doc_date: "2026-02-13", doc_date_from: "judul",
  size: 2048, pages: 0, chunks: 1, available: true,
};
const problem = {
  ...ready, id: 2, title: "Form_IK_Review", display_title: "Form_IK_Review",
  status: "tidak_didukung", available: false, doc_date: null, doc_date_from: null,
};
let onRequest;
window.fetch = (url, init) => new Promise((respond) => {
  if (url !== "/api/documents/list" || init.method !== "POST") {
    throw new Error("Permintaan tak terduga: " + url);
  }
  onRequest({ body: JSON.parse(init.body), respond });
});

function equal(actual, expected) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(JSON.stringify({ actual, expected }));
  }
}

async function request(action) {
  const next = new Promise((received) => { onRequest = received; });
  await act(action);
  return next;
}

async function reply({ respond }, counts, documents = []) {
  const total = documents.length;
  await act(() => respond(Response.json({ documents, total, offset: 0, limit: 50, sources, statuses: counts })));
}

const summary = () => document.querySelector(".docs-attention").textContent.trim();
const click = (selector) => document.querySelector(selector).click();
const tabs = () => [...document.querySelectorAll('[role="tab"]')];
const clickTab = (name) => tabs().find((tab) => tab.textContent.startsWith(name + " ")).click();

async function run() {
  const root = createRoot(document.getElementById("root"));
  const first = await request(() => root.render(createElement(DocumentsPage, { openId: null, onOpen() {} })));
  equal(first.body, { source: null, query: "", status: [], sort: "judul", offset: 0, limit: 50 });
  await reply(first, statuses, [ready, problem]);
  equal(tabs().map((tab) => tab.textContent), ["Semua 4", "Review 2", "Umum 2"]);
  equal(summary(), "3 file belum bisa dicari Bakase: 1 menunggu dibaca, 2 tidak bisa dibaca. Lihat");
  const title = document.querySelector(".doc-title");
  equal([title.textContent, title.title], [ready.display_title, ready.title]);
  equal(document.querySelector("[download]").getAttribute("aria-label"), "Unduh Catatan Kegiatan");
  // Tanggal dokumen menggantikan tanggal diubah; asalnya hanya di tooltip.
  const dates = [...document.querySelectorAll(".doc-sub span[title]")];
  equal(dates.map((d) => d.textContent), ["13 Feb 2026", "diubah 19 Nov 2026"]);
  equal(dates[0].title, "Tanggal dokumen 13 Feb 2026 · dari judul");
  equal(dates[1].title.startsWith("Diubah Kamis, 19 Nov 2026 · "), true);

  const problemRow = document.querySelectorAll(".doc-row")[1];
  const open = problemRow.querySelector(".doc-open");
  const tip = document.getElementById(open.getAttribute("aria-describedby"));
  equal(tip.getAttribute("role"), "tooltip");
  equal(tip.textContent, "Format file ini belum didukung Bakase.");
  equal(problemRow.querySelector(".doc-status").tabIndex, -1);
  await act(() => open.focus());
  equal(getComputedStyle(tip).display, "block");
  await act(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
  equal(getComputedStyle(tip).display, "none");
  equal(getComputedStyle(open).outlineStyle, "solid");
  equal(getComputedStyle(open).outlineWidth, "2px");
  equal(getComputedStyle(open).outlineColor, "rgb(13, 13, 13)");
  document.documentElement.dataset.theme = "dark";
  equal(getComputedStyle(open).outlineColor, "rgb(236, 236, 236)");
  document.documentElement.dataset.theme = "light";
  await act(() => open.blur());
  const status = problemRow.querySelector(".doc-status");
  await act(() => status.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
  equal(problemRow.classList.contains("tip-off"), false);
  await act(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
  equal(problemRow.classList.contains("tip-off"), true);
  await act(() => status.dispatchEvent(new MouseEvent("mouseout", { bubbles: true })));

  const filtered = await request(() => click(".docs-attention button"));
  equal(filtered.body.status, wanted);
  await reply(filtered, statuses, [problem]);

  const review = await request(() => clickTab("Review"));
  equal([review.body.source, review.body.status], ["Review", wanted]);
  equal(summary(), "Tampilkan semua");
  await reply(review, { tidak_didukung: 1, ok: 1 }, [problem]);
  equal(summary(), "1 file belum bisa dicari Bakase: 1 tidak bisa dibaca. Tampilkan semua");

  const search = await request(() => {
    const input = document.querySelector('input[type="search"]');
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(input, "Form");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  equal(summary(), "1 file belum bisa dicari Bakase: 1 tidak bisa dibaca. Tampilkan semua");
  equal([search.body.source, search.body.query, search.body.status], ["Review", "Form", wanted]);
  await reply(search, { tidak_didukung: 1 }, [problem]);

  const umum = await request(() => clickTab("Umum"));
  equal([umum.body.source, umum.body.status], ["Umum", wanted]);
  await reply(umum, {});
  equal(summary(), "Tampilkan semua");

  const all = await request(() => click(".docs-attention button"));
  equal([all.body.source, all.body.status], ["Umum", []]);
  await reply(all, {});
}

run().then(
  () => nativeFetch("/__test_result", { method: "POST", body: "ok" }),
  (error) => nativeFetch("/__test_result", { method: "POST", body: String(error.stack || error) }),
);
`;

const browser = [
  process.env.CHROME_BIN,
  ...["ProgramFiles", "ProgramFiles(x86)"].flatMap((key) => {
    const base = process.env[key];
    return base ? [resolve(base, "Google/Chrome/Application/chrome.exe"), resolve(base, "Microsoft/Edge/Application/msedge.exe")] : [];
  }),
  "/usr/bin/google-chrome",
  "/usr/bin/chromium",
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
].find((path) => path && existsSync(path));

test("Sumber Data menampilkan jumlah tiap tab dan mempertahankan filter saat berpindah tab", {
  skip: browser ? false : "Chromium tidak tersedia (atur CHROME_BIN)",
  timeout: 45_000,
}, async (t) => {
  const profile = await mkdtemp(resolve("node_modules", ".documents-browser-"));
  let report: (value: string) => void;
  const result = new Promise<string>((resolve) => { report = resolve; });
  const vite = await createServer({
    configFile: false,
    cacheDir: "node_modules/.vite-documents-test",
    server: { host: "127.0.0.1", port: 0, ws: false, watch: null },
    plugins: [{
      name: "documents-page-test",
      configureServer(server) {
        server.middlewares.use("/__test_result", (req, res) => {
          req.setEncoding("utf8");
          let body = "";
          req.on("data", (chunk) => { body += chunk; });
          req.on("end", () => {
            res.end();
            report(body);
          });
        });
        server.middlewares.use("/__documents_test", async (_req, res) => {
          res.setHeader("Content-Type", "text/html");
          res.end(await server.transformIndexHtml(
            "/__documents_test",
            `<!doctype html><div id="root"></div><script type="module">${fixture}</script>`,
          ));
        });
      },
    }],
  });
  await vite.listen();
  const url = vite.resolvedUrls?.local[0];
  assert.ok(url);
  const child = spawn(
    browser!,
    ["--headless", "--no-first-run", `--user-data-dir=${profile}`, `${url}__documents_test`],
    { stdio: "ignore", windowsHide: true },
  );
  const closed = new Promise<string>((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (code, signal) => resolve(`Chromium berhenti: ${code ?? signal}`));
  });
  t.after(async () => {
    child.kill();
    await closed.catch(() => {});
    await vite.close();
    await rm(profile, { recursive: true, force: true, maxRetries: 3 });
  });
  const cancelled = new Promise<string>((resolve) => {
    t.signal.addEventListener("abort", () => resolve("Tes Chromium melewati batas waktu"), { once: true });
  });
  assert.equal(await Promise.race([result, closed, cancelled]), "ok");
});
