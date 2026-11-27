import assert from "node:assert/strict";
import test from "node:test";
import { currentBrowser } from "./installBrowser.ts";

test("mengenali browser dari user agent", () => {
  const chromium = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/158.0.0.0 Safari/537.36";
  assert.equal(currentBrowser(chromium), "chrome");
  assert.equal(currentBrowser(`${chromium} Edg/158.0.0.0`), "edge");
  assert.equal(currentBrowser("Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:150.0) Gecko/20100101 Firefox/150.0"), "firefox");
});
