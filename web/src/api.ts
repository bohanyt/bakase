import type { FileRef } from "./messages";

export type User = {
  id: number;
  email: string;
  name: string;
  department: string | null;
  role: "user" | "admin";
};

export type AuthOptions = {
  email_domain: string;
  departments: string[];
  guest: boolean;
  /** Nomor WhatsApp IT untuk tombol bantuan, kalau diatur. */
  support_whatsapp?: string | null;
  /** ID extension Edge yang boleh memasang Bakase di panel samping. */
  extension_ids?: string[];
};

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

/** Dipancarkan saat server menjawab 401, supaya aplikasi kembali ke halaman masuk. */
export const UNAUTHORIZED_EVENT = "app:unauthorized";

export async function api<T>(path: string, init: { method?: string; body?: unknown } = {}): Promise<T> {
  const method = init.method ?? (init.body === undefined ? "GET" : "POST");
  const res = await fetch(path, {
    method,
    headers: init.body === undefined ? undefined : { "Content-Type": "application/json" },
    body: init.body === undefined ? undefined : JSON.stringify(init.body),
  });
  if (res.status === 401 && !path.startsWith("/api/auth/")) {
    window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
  }
  if (!res.ok) {
    let message = `Server menjawab ${res.status}.`;
    try {
      const data = await res.json();
      if (typeof data?.message === "string") message = data.message;
    } catch {
      /* jawaban bukan JSON */
    }
    throw new ApiError(res.status, message);
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

/** Mengunggah satu lampiran. Pakai XMLHttpRequest karena fetch belum bisa melaporkan
 * kemajuan unggahan. Isi file dikirim apa adanya sebagai `application/octet-stream`;
 * server menolak form dan multipart dari situs lain. */
export function uploadAttachment(
  file: Blob,
  name: string,
  onProgress: (fraction: number) => void,
): { done: Promise<FileRef>; abort: () => void } {
  const xhr = new XMLHttpRequest();
  const done = new Promise<FileRef>((resolve, reject) => {
    xhr.open("POST", "/api/attachments");
    xhr.setRequestHeader("Content-Type", "application/octet-stream");
    xhr.setRequestHeader("X-File-Name", encodeURIComponent(name));
    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable && e.total > 0) onProgress(e.loaded / e.total);
    };
    xhr.onload = () => {
      let data: { message?: unknown } | null = null;
      try {
        data = JSON.parse(xhr.responseText);
      } catch {
        /* jawaban bukan JSON */
      }
      if (xhr.status === 401) window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
      if (xhr.status >= 200 && xhr.status < 300 && data) {
        resolve(data as unknown as FileRef);
        return;
      }
      const fallback = xhr.status === 413 ? "File terlalu besar." : `Server menjawab ${xhr.status}.`;
      reject(new ApiError(xhr.status, typeof data?.message === "string" ? data.message : fallback));
    };
    xhr.onerror = () => reject(new ApiError(0, "Gagal mengunggah. Periksa koneksi lalu coba lagi."));
    xhr.onabort = () => reject(new ApiError(0, "Unggahan dibatalkan."));
    xhr.send(file);
  });
  return { done, abort: () => xhr.abort() };
}

/** Menghapus lampiran yang belum dikirim. Gagal pun tidak apa-apa: server membersihkannya
 * setelah sehari. */
export function discardAttachment(id: string): void {
  api(`/api/attachments/${id}`, { method: "DELETE" }).catch(() => {});
}
