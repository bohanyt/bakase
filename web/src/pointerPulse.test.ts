import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

// Penunjuk (pointer.js) menggambar di halaman; di sini hanya kunci animasinya yang diuji.
const source = readFileSync(new URL("../../extension/src/pointer.js", import.meta.url), "utf8")
  .replaceAll("export function", "function").replaceAll("export const", "const");
const context = createContext({});
runInContext(`${source}\nglobalThis.pulseFrames = pulseFrames;`, context);
const pulseFrames = context.pulseFrames as (pulses?: number, pulseMs?: number, pauseMs?: number) => {
  frames: { opacity: number; offset: number }[];
  total: number;
};

test("denyut penunjuk: tiga kali terang lalu diam 3 detik dalam satu putaran", () => {
  const { frames, total } = pulseFrames();
  assert.equal(total, 3 * 600 + 3000);
  assert.equal(frames.filter((f) => f.opacity === 1).length, 3);
  assert.deepEqual([frames[0].offset, frames.at(-1)!.offset], [0, 1]);
  assert.ok(frames.every((f, i) => i === 0 || f.offset > frames[i - 1].offset), "offset harus naik");
  // Denyut terakhir selesai sebelum jeda; sisa putaran redup.
  const lastPeak = frames.filter((f) => f.opacity === 1).at(-1)!.offset;
  assert.ok(lastPeak < 1800 / total && frames.at(-2)!.opacity === 0);
});
