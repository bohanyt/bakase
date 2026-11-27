import assert from "node:assert/strict";
import { test } from "node:test";
import { PART_BYTES, realStatus, uploadAttachment, uploadParts } from "./api.ts";

type Sent = { url: string; headers: Record<string, string>; size: number };
type Reply = { status: number; body: unknown; headers?: Record<string, string> };

/** XMLHttpRequest palsu: mencatat tiap request dan menjawab sesuai `reply`. */
class FakeXhr {
  static sent: Sent[] = [];
  static reply: (index: number, sent: Sent) => Reply = () => ({ status: 500, body: null });
  static after: (index: number) => void = () => {};
  url = "";
  headers: Record<string, string> = {};
  status = 0;
  responseText = "";
  responseHeaders: Record<string, string> = {};
  upload: { onprogress: ((e: { lengthComputable: boolean; loaded: number; total: number }) => void) | null } = {
    onprogress: null,
  };
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onabort: (() => void) | null = null;
  open(_method: string, url: string) {
    this.url = url;
  }
  setRequestHeader(header: string, value: string) {
    this.headers[header] = value;
  }
  send(body: Blob) {
    const sent = { url: this.url, headers: this.headers, size: body.size };
    const index = FakeXhr.sent.push(sent) - 1;
    queueMicrotask(() => {
      this.upload.onprogress?.({ lengthComputable: true, loaded: body.size, total: body.size });
      const reply = FakeXhr.reply(index, sent);
      this.status = reply.status;
      this.responseText = JSON.stringify(reply.body);
      this.responseHeaders = reply.headers ?? {};
      this.onload?.();
      FakeXhr.after(index);
    });
  }
  abort() {
    this.onabort?.();
  }
  getResponseHeader(name: string) {
    return this.responseHeaders[name.toLowerCase()] ?? null;
  }
}

function useFake(reply: (index: number, sent: Sent) => Reply, after: (index: number) => void = () => {}) {
  FakeXhr.sent = [];
  FakeXhr.reply = reply;
  FakeXhr.after = after;
  (globalThis as { XMLHttpRequest?: unknown }).XMLHttpRequest = FakeXhr;
}

const fileRef = { id: "a".repeat(32), name: "Rekap.xlsx", kind: "xlsx", size: 0 };

test("potongan unggahan berurutan, masing-masing paling besar 8 MB", () => {
  assert.deepEqual(uploadParts(0), []);
  assert.deepEqual(uploadParts(5), [[0, 5]]);
  assert.deepEqual(uploadParts(PART_BYTES), [[0, PART_BYTES]]);
  assert.deepEqual(uploadParts(2 * PART_BYTES + 3), [
    [0, PART_BYTES],
    [PART_BYTES, 2 * PART_BYTES],
    [2 * PART_BYTES, 2 * PART_BYTES + 3],
  ]);
});

test("file kecil tetap satu request ke /api/attachments", async () => {
  useFake(() => ({ status: 201, body: fileRef }));
  const { done } = uploadAttachment(new Blob([new Uint8Array(10)]), "Rekap.xlsx", () => {});
  assert.deepEqual(await done, fileRef);
  assert.equal(FakeXhr.sent.length, 1);
  assert.equal(FakeXhr.sent[0].url, "/api/attachments");
  assert.equal(FakeXhr.sent[0].headers["X-Upload-Id"], undefined);
});

test("file besar dikirim per potongan dengan id, offset, dan ukuran total", async () => {
  const size = 2 * PART_BYTES + 3;
  useFake((index) => (index < 2 ? { status: 202, body: { received: (index + 1) * PART_BYTES } } : { status: 201, body: fileRef }));
  const progress: number[] = [];
  const { done } = uploadAttachment(new Blob([new Uint8Array(size)]), "Piping.xlsx", (p) => progress.push(p));
  assert.deepEqual(await done, fileRef);
  assert.deepEqual(FakeXhr.sent.map((s) => s.url), Array(3).fill("/api/attachments/parts"));
  assert.deepEqual(FakeXhr.sent.map((s) => s.size), [PART_BYTES, PART_BYTES, 3]);
  assert.deepEqual(FakeXhr.sent.map((s) => s.headers["X-Upload-Offset"]), ["0", String(PART_BYTES), String(2 * PART_BYTES)]);
  assert.ok(FakeXhr.sent.every((s) => s.headers["X-Upload-Size"] === String(size)));
  const ids = new Set(FakeXhr.sent.map((s) => s.headers["X-Upload-Id"]));
  assert.equal(ids.size, 1);
  assert.match([...ids][0] ?? "", /^[0-9a-f]{32}$/);
  assert.equal(FakeXhr.sent[0].headers["X-File-Name"], "Piping.xlsx");
  assert.deepEqual(progress.map((p) => Math.round(p * size)), [PART_BYTES, 2 * PART_BYTES, size]);
});

test("potongan yang ditolak menghentikan unggahan dengan pesan server", async () => {
  useFake((index) => (index === 0 ? { status: 202, body: { received: PART_BYTES } } : { status: 400, body: { message: "Potongan ditolak." } }));
  const { done } = uploadAttachment(new Blob([new Uint8Array(3 * PART_BYTES)]), "Piping.xlsx", () => {});
  await assert.rejects(done, (e: Error & { status?: number }) => e.status === 400 && e.message === "Potongan ditolak.");
  assert.equal(FakeXhr.sent.length, 2);
});

test("batal di antara dua potongan tidak mengirim potongan berikutnya", async () => {
  let abort = () => {};
  useFake(() => ({ status: 202, body: { received: PART_BYTES } }), (index) => {
    if (index === 0) abort();
  });
  const upload = uploadAttachment(new Blob([new Uint8Array(3 * PART_BYTES)]), "Piping.xlsx", () => {});
  abort = upload.abort;
  await assert.rejects(upload.done, /dibatalkan/);
  assert.equal(FakeXhr.sent.length, 1);
});

test("batal saat potongan dikirim tidak mengirim potongan berikutnya", async () => {
  let abort = () => {};
  useFake((index) => {
    if (index === 0) abort();
    return { status: 202, body: { received: PART_BYTES } };
  });
  const upload = uploadAttachment(new Blob([new Uint8Array(3 * PART_BYTES)]), "Piping.xlsx", () => {});
  abort = upload.abort;
  await assert.rejects(upload.done, /dibatalkan/);
  assert.equal(FakeXhr.sent.length, 1);
});

test("error yang dibungkus server (200 + X-app-Status) tetap jadi error dengan pesan aslinya", async () => {
  useFake(() => ({ status: 200, body: { message: "File Excel ini terlalu besar." }, headers: { "x-app-status": "400" } }));
  const { done } = uploadAttachment(new Blob([new Uint8Array(10)]), "Rekap.xlsx", () => {});
  await assert.rejects(done, (e: Error & { status?: number }) => e.status === 400 && e.message === "File Excel ini terlalu besar.");
  assert.equal(FakeXhr.sent[0].headers["X-app-Error-Mode"], "200");
});

test("kode asli dari X-app-Status hanya dipakai kalau berupa kode error", () => {
  assert.equal(realStatus(200, "404"), 404);
  assert.equal(realStatus(200, null), 200);
  assert.equal(realStatus(200, "abc"), 200);
  assert.equal(realStatus(200, "302"), 200);
  assert.equal(realStatus(413, null), 413);
});

test("fokus halaman ikut sebagai header di setiap request unggah", async () => {
  useFake((index) => (index === 0 ? { status: 202, body: { received: PART_BYTES } } : { status: 201, body: fileRef }));
  const focus = 'sheet aktif "Rekap Juni"; sel dipilih B12';
  const { done } = uploadAttachment(new Blob([new Uint8Array(PART_BYTES + 5)]), "Rekap.xlsx", () => {}, focus);
  await done;
  assert.ok(FakeXhr.sent.every((s) => decodeURIComponent(s.headers["X-app-Fokus"] ?? "") === focus));
  useFake(() => ({ status: 201, body: fileRef }));
  await uploadAttachment(new Blob([new Uint8Array(4)]), "Rekap.xlsx", () => {}).done;
  assert.equal(FakeXhr.sent[0].headers["X-app-Fokus"], undefined);
});
