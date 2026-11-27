import assert from "node:assert/strict";
import test from "node:test";
import { extensionUpdateAvailable } from "./extensionVersion.ts";

test("membandingkan versi extension berdasarkan angka", () => {
  assert.equal(extensionUpdateAvailable("0.3.9", "0.3.10"), true);
  assert.equal(extensionUpdateAvailable("0.3.10", "0.3.9"), false);
  assert.equal(extensionUpdateAvailable("0.3.10", "0.3.10"), false);
  assert.equal(extensionUpdateAvailable("0.10.0", "1.0.0"), true);
});

test("versi yang hilang atau rusak dianggap versi lama", () => {
  assert.equal(extensionUpdateAvailable(null, "0.3.1"), true);
  assert.equal(extensionUpdateAvailable("rusak", "0.3.1"), true);
  assert.equal(extensionUpdateAvailable("0.3.10\n", "0.3.1"), true);
  assert.equal(extensionUpdateAvailable("0.3.9999999999999999999", "0.3.1"), true);
  assert.equal(extensionUpdateAvailable("0.3.1", "rusak"), false);
});
