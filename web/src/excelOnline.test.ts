import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

// Hanya fungsi pemetaan alamat dari panel extension; sisanya butuh API chrome.
const panel = readFileSync(new URL("../../extension/panel.js", import.meta.url), "utf8");
const start = panel.indexOf("// excel-online:mulai");
const end = panel.indexOf("// excel-online:akhir");
const context = createContext({ URL });
runInContext(`${panel.slice(start, end)}\nglobalThis.excelDownloads = excelDownloads;`, context);
// Array dari realm vm dibuat ulang supaya bisa dibandingkan dengan deepEqual.
const excelDownloads = (address: string) => Array.from((context.excelDownloads as (a: string) => string[])(address));

test("Excel Online: tautan berbagi diunduh dengan download=1", () => {
  assert.deepEqual(excelDownloads("https://appid-my.example.com/:x:/g/personal/uji_contoh_test/IQAabc?e=XyZ12#tab"),
    ["https://appid-my.example.com/:x:/g/personal/uji_contoh_test/IQAabc?e=XyZ12&download=1"]);
  assert.deepEqual(excelDownloads("https://appid.example.com/:x:/r/sites/IT/Shared%20Documents/Rekap.xlsx?d=w1&csf=1"),
    ["https://appid.example.com/:x:/r/sites/IT/Shared%20Documents/Rekap.xlsx?d=w1&csf=1&download=1"]);
});

test("Excel Online: file dari pustaka dokumen diunduh lewat UniqueId", () => {
  assert.deepEqual(
    excelDownloads("https://appid.example.com/sites/IT/_layouts/15/Doc.aspx?sourcedoc=%7B12345678-ABCD-4ef0-9abc-1234567890ab%7D&file=Rekap.xlsx&action=default"),
    ["https://appid.example.com/sites/IT/_layouts/15/download.aspx?UniqueId=12345678-ABCD-4ef0-9abc-1234567890ab"]);
});

test("Excel Online: alamat lain tidak diunduh", () => {
  for (const address of [
    "https://contoh.test/:x:/g/abc",
    "http://appid.example.com/:x:/g/abc",
    "https://tenantlain.example.com/:x:/g/personal/uji/abc",
    "https://appid-my.example.com/:w:/g/personal/uji/abc",
    "https://appid.example.com.palsu.test/:x:/g/abc",
    "https://x.appid.example.com/:x:/g/abc",
    "http://contoh.example.com/:x:/g/abc",
    "https://contoh.example.com/:w:/g/personal/uji/abc",
    "https://contoh.example.com.palsu.test/:x:/g/abc",
    "https://contoh.example.com/sites/IT/_layouts/15/Doc.aspx?sourcedoc=bukan-guid",
    "bukan alamat",
  ]) assert.deepEqual(excelDownloads(address), [], address);
});
