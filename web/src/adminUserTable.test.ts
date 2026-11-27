import assert from "node:assert/strict";
import test from "node:test";
import {
  accessUpdate, accesses, approvalUpdate, compareUserText, DEFAULT_LAYOUT, filterUsers,
  loadUserLayout, moveColumn, NO_FILTERS, sameAccess, saveUserLayout, type AdminUserRow,
} from "./adminUserTable.ts";

const base: AdminUserRow = {
  id: 1, name: "Éka 10", email: "review@example.test", status: "active", role: "user",
  department: "Review", departments: ["Operasional", "Review"], requested_department: null,
};
const pending: AdminUserRow = {
  ...base, id: 2, name: "Eka 2", email: "baru@example.test", status: "pending",
  department: null, departments: [], requested_department: "Review",
};
const legacy: AdminUserRow = {
  ...base, id: 3, name: "Adi", email: "legacy@example.test", department: "Contoh", departments: undefined,
};

test("mencari nama atau email dan menggabungkan filter status dengan salah satu akses", () => {
  const rows = [base, pending, legacy];
  assert.deepEqual(filterUsers(rows, { ...NO_FILTERS, query: "  EXAMPLE.TEST ", statuses: ["active"], access: ["Contoh", "Review"] }), [base, legacy]);
  assert.deepEqual(filterUsers(rows, { ...NO_FILTERS, query: "Eka", access: [""] }), [pending]);
  assert.deepEqual(filterUsers(rows, { ...NO_FILTERS, statuses: ["pending"], access: ["Review"] }), []);
});

test("urutan Indonesia mengabaikan kapital dan aksen serta membandingkan angka alami", () => {
  assert.equal(compareUserText("ÉKA", "eka"), 0);
  assert.deepEqual(["Éka 10", "Eka 2", "Adi"].sort(compareUserText), ["Adi", "Eka 2", "Éka 10"]);
});

test("perubahan akses mendukung payload ganda dan respons lama", () => {
  assert.deepEqual(accesses(legacy), ["Contoh"]);
  assert.deepEqual(accesses({ ...legacy, departments: [] }), []);
  assert.deepEqual(accessUpdate(legacy, ["Review"]), { department: "Review" });
  assert.deepEqual(accessUpdate(legacy, []), { department: "" });
  assert.deepEqual(accessUpdate(base, ["Review"]), { departments: ["Review"] });
  assert.deepEqual(accessUpdate(base, []), { departments: [] });
  assert.ok(sameAccess(["Review", "Operasional"], ["Operasional", "Review"]));
  assert.ok(!sameAccess(["Review", "Operasional"], ["Operasional"]));
});

test("persetujuan memberi akses yang diminta hanya bila belum ada akses yang ditetapkan", () => {
  assert.deepEqual(approvalUpdate(pending), { status: "active", departments: ["Review"] });
  assert.deepEqual(approvalUpdate({ ...pending, departments: undefined }), { status: "active", department: "Review" });
  assert.deepEqual(approvalUpdate({ ...pending, requested_department: null }), { status: "active", departments: [] });
  assert.deepEqual(approvalUpdate({ ...pending, departments: ["Operasional", "Contoh"] }), { status: "active" });
});

test("lebar, kolom tersembunyi dan urutan tersimpan, dengan nama dan aksi tetap di tepi", () => {
  let saved: string | null = null;
  const storage = { getItem: () => saved, setItem: (_key: string, value: string) => { saved = value; } };
  const order = moveColumn(DEFAULT_LAYOUT.order, "status", -1);
  assert.deepEqual(order, ["name", "status", "email", "access", "role", "action"]);
  assert.equal(moveColumn(order, "status", -1), order);
  const layout = { order, widths: { email: 330 }, visibility: { role: false } };
  saveUserLayout(layout, storage);
  assert.deepEqual(loadUserLayout(storage), layout);
  saved = '{"order":["action","email","email","nama"],"widths":{"email":99999,"status":10,"role":"bad","action":800},"visibility":{"name":false,"action":false,"email":false}}';
  assert.deepEqual(loadUserLayout(storage), {
    ...DEFAULT_LAYOUT, widths: { email: 950, status: 170 }, visibility: { email: false },
  });
  saved = "{rusak";
  assert.deepEqual(loadUserLayout(storage), DEFAULT_LAYOUT);
  const denied = () => { throw new Error("storage ditolak"); };
  assert.deepEqual(loadUserLayout({ getItem: denied, setItem: denied }), DEFAULT_LAYOUT);
  assert.doesNotThrow(() => saveUserLayout(layout, { getItem: denied, setItem: denied }));
});
