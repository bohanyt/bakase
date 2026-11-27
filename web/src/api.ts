import type { FileRef } from "./messages.ts";

export type User = {
  id: number;
  email: string;
  name: string;
  department: string | null;
  departments?: string[];
  role: "user" | "admin";
  microsoft_secret_days?: number;
};

export function departmentList(departments: string[] | undefined, legacy: string | null): string[] {
  return departments ?? (legacy ? [legacy] : []);
}

export type AuthOptions = {
  email_domain: string;
  departments: string[];
  guest: boolean;
  /** Nomor WhatsApp IT untuk tombol bantuan, kalau diatur. */
  support_whatsapp?: string | null;
  /** ID extension Edge yang boleh memasang Bakase di panel samping. */
  extension_ids?: string[];
  extension_version?: string;
  microsoft?: "pilot" | "full";
};

/** Satu file di File Saya (`GET /api/files`). */
export type MyFile = {
  id: string;
  name: string;
  kind: string;
  size: number;
  created_at: number;
  conversation_id: string;
  conversation_title: string;
  /** Lampiran dari user; selain itu dibuat Bakase. */
  upload: boolean;
};

/** Satu kartu di halaman Aplikasi (`GET /api/apps`). */
export type AppCard = { id: string; name: string; description: string; ready: boolean };

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

/** Dipancarkan saat server menjawab 401, supaya aplikasi kembali ke halaman masuk. */
export const UNAUTHORIZED_EVENT = "app:unauthorized";

/** Minta App memeriksa lagi siapa yang masuk, misalnya setelah server menolak karena akun berganti. */
export const SESSION_CHECK_EVENT = "app:cek-sesi";

let tabUserId: number | null = null;

/** Akun yang sedang tampil di tab ini. */
export function setTabUser(id: number | null): void {
  tabUserId = id;
}

/** Proxy kantor mengganti isi balasan error (400, 404, …) dengan halamannya sendiri, jadi pesan
 * Bakase hilang. Dengan header ini server mengirim error sebagai 200 dan kode aslinya di
 * `X-app-Status`. */
export const ERROR_MODE: Record<string, string> = { "X-app-Error-Mode": "200" };

/** Kode asli dari server: `X-app-Status` kalau error dibungkus, selain itu status HTTP. */
export function realStatus(status: number, header: string | null): number {
  const code = Number(header);
  return Number.isInteger(code) && code >= 400 && code <= 599 ? code : status;
}

/** Dikirim bersama request yang mengubah data, supaya tab lama tidak menulis ke akun yang baru masuk. */
export function tabUserHeader(): Record<string, string> {
  return tabUserId === null ? {} : { "X-app-User": String(tabUserId) };
}

export async function api<T>(path: string, init: { method?: string; body?: unknown } = {}): Promise<T> {
  const method = init.method ?? (init.body === undefined ? "GET" : "POST");
  const res = await fetch(path, {
    method,
    headers: {
      ...ERROR_MODE,
      ...(init.body === undefined ? {} : { "Content-Type": "application/json" }),
      ...(method === "GET" || path.startsWith("/api/auth/") ? {} : tabUserHeader()),
    },
    body: init.body === undefined ? undefined : JSON.stringify(init.body),
  });
  const status = realStatus(res.status, res.headers.get("X-app-Status"));
  if (status === 401 && !path.startsWith("/api/auth/")) {
    window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
  }
  if (status === 409) window.dispatchEvent(new Event(SESSION_CHECK_EVENT));
  if (status < 200 || status >= 300) {
    let message = `Server menjawab ${status}.`;
    try {
      const data = await res.json();
      if (typeof data?.message === "string") message = data.message;
    } catch {
      /* jawaban bukan JSON */
    }
    throw new ApiError(status, message);
  }
  if (status === 204) return undefined as T;
  return (await res.json()) as T;
}

/** Ukuran potongan unggahan, sama dengan `PART_BYTES` di server. Proxy jaringan di depan hub
 * menolak body di atas sekitar 10,5 MB, jadi file yang lebih besar dikirim per 8 MB
 * lalu disambung lagi oleh server. */
export const PART_BYTES = 8 * 1024 * 1024;

/** Awal dan akhir tiap potongan untuk file sebesar `size`. */
export function uploadParts(size: number, part = PART_BYTES): Array<[number, number]> {
  const parts: Array<[number, number]> = [];
  for (let start = 0; start < size; start += part) parts.push([start, Math.min(size, start + part)]);
  return parts;
}

/** Mengunggah satu lampiran. Pakai XMLHttpRequest karena fetch belum bisa melaporkan
 * kemajuan unggahan. Isi file dikirim apa adanya sebagai `application/octet-stream`;
 * server menolak form dan multipart dari situs lain. File di atas `PART_BYTES` dikirim
 * berpotongan dan berurutan ke `/api/attachments/parts`. */
export function uploadAttachment(
  file: Blob,
  name: string,
  onProgress: (fraction: number) => void,
  /** Fokus user saat file diambil dari halaman (sheet dan sel aktif), disimpan bersama lampiran. */
  focus?: string,
): { done: Promise<FileRef>; abort: () => void } {
  const noted: Record<string, string> = {};
  try {
    if (focus) noted["X-app-Fokus"] = encodeURIComponent(focus);
  } catch {
    // Teks yang tidak bisa dikodekan: unggah tetap jalan tanpa catatan fokus.
  }
  let current: XMLHttpRequest | null = null;
  let aborted = false;
  const cancelled = () => new ApiError(0, "Unggahan dibatalkan.");
  const send = (url: string, body: Blob, extra: Record<string, string>, sent: number) =>
    new Promise<unknown>((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      current = xhr;
      xhr.open("POST", url);
      xhr.setRequestHeader("Content-Type", "application/octet-stream");
      xhr.setRequestHeader("X-File-Name", encodeURIComponent(name));
      for (const [header, value] of Object.entries({ ...ERROR_MODE, ...tabUserHeader(), ...noted, ...extra })) {
        xhr.setRequestHeader(header, value);
      }
      xhr.upload.onprogress = (e) => {
        if (e.lengthComputable && e.total > 0 && file.size > 0) {
          onProgress(Math.min(1, (sent + (e.loaded / e.total) * body.size) / file.size));
        }
      };
      xhr.onload = () => {
        let data: { message?: unknown } | null = null;
        try {
          data = JSON.parse(xhr.responseText);
        } catch {
          /* jawaban bukan JSON */
        }
        const status = realStatus(xhr.status, xhr.getResponseHeader("X-app-Status"));
        if (status === 401) window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
        if (status === 409) window.dispatchEvent(new Event(SESSION_CHECK_EVENT));
        if (status >= 200 && status < 300 && data) {
          resolve(data);
          return;
        }
        const fallback = status === 413 ? "File terlalu besar." : `Server menjawab ${status}.`;
        reject(new ApiError(status, typeof data?.message === "string" ? data.message : fallback));
      };
      xhr.onerror = () => reject(new ApiError(0, "Gagal mengunggah. Periksa koneksi lalu coba lagi."));
      xhr.onabort = () => reject(cancelled());
      xhr.send(body);
    });
  const done = (async () => {
    if (file.size <= PART_BYTES) return (await send("/api/attachments", file, {}, 0)) as FileRef;
    const random = new Uint8Array(16);
    crypto.getRandomValues(random);
    const id = Array.from(random, (b) => b.toString(16).padStart(2, "0")).join("");
    let last: unknown = null;
    for (const [start, end] of uploadParts(file.size)) {
      if (aborted) throw cancelled();
      last = await send("/api/attachments/parts", file.slice(start, end), {
        "X-Upload-Id": id,
        "X-Upload-Offset": String(start),
        "X-Upload-Size": String(file.size),
      }, start);
    }
    if (typeof (last as { id?: unknown } | null)?.id !== "string") {
      throw new ApiError(0, "Unggahan tidak selesai. Lampirkan ulang filenya.");
    }
    return last as FileRef;
  })();
  return {
    done,
    abort: () => {
      aborted = true;
      current?.abort();
    },
  };
}

/** Menghapus lampiran yang belum dikirim. Gagal pun tidak apa-apa: server membersihkannya
 * setelah sehari. */
export function discardAttachment(id: string): void {
  api(`/api/attachments/${id}`, { method: "DELETE" }).catch(() => {});
}
