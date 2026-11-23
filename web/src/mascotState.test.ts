import assert from "node:assert/strict";
import { test } from "node:test";
import { workingMascotState } from "./mascotState.ts";

const text = { type: "text", text: "Contoh jawaban" } as const;
const step = { id: "1", label: "Mencari dokumen", done: false, failed: false };
const tool = { type: "tool-call", toolCallId: "1", toolName: "search", args: {}, argsText: "{}" } as const;

test("memakai status merangkum selama pemadatan berjalan", () => {
  assert.equal(workingMascotState([text], { compacting: true, steps: [step] }), "tired");
});

test("langkah tool dari server didahulukan selama belum selesai", () => {
  assert.equal(workingMascotState([text], { steps: [step] }), "focus");
  assert.equal(workingMascotState([text], { steps: [{ ...step, done: true, failed: true }] }), "bicara");
});

test("tool-call assistant-ui fokus sampai hasilnya final", () => {
  assert.equal(workingMascotState([text, tool]), "focus");
  assert.equal(workingMascotState([text, { ...tool, result: [], isPreliminary: true }]), "focus");
  assert.equal(workingMascotState([text, { ...tool, result: null }]), "mikir");
  assert.equal(workingMascotState([{ ...tool, isError: true }]), "mikir");
});

test("memakai status menjawab hanya ketika pesan memuat teks jawaban", () => {
  assert.equal(workingMascotState([text]), "bicara");
  assert.equal(workingMascotState([{ type: "reasoning", text: "Contoh pemikiran" }, { ...text, text: " " }]), "mikir");
  assert.equal(workingMascotState([]), "mikir");
});

test("mengabaikan teks dan pemikiran sebelum atau sesudah tool terakhir sampai ada teks baru", () => {
  const completed = { ...tool, result: null };
  const second = { ...completed, toolCallId: "2" };
  const reasoning = { type: "reasoning", text: "Contoh pemikiran" } as const;
  assert.equal(workingMascotState([text, completed, second, reasoning]), "mikir");
  assert.equal(workingMascotState([text, completed, { ...text, text: "Ini hasilnya." }]), "bicara");
});
