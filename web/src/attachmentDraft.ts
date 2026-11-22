import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { discardAttachment, uploadAttachment } from "./api";
import { MAX_ATTACHMENTS, checkFile, extensionOf, isImageKind, pastedName, totalError } from "./attachment";
import type { FileRef } from "./messages";
import { bindPageAttachment } from "./pageBridge";

/** Lampiran di kotak tanya yang belum dikirim. */
export type Draft = {
  key: string;
  file: File;
  name: string;
  kind: string;
  /** Alamat sementara untuk thumbnail gambar, dari file di browser. */
  url?: string;
  progress: number;
  status: "uploading" | "ready" | "error";
  error?: string;
  /** File di server setelah unggahan selesai. */
  ref?: FileRef;
  abort?: () => void;
};

let seq = 0;

/** Menghentikan unggahan yang dibuang. Selama isinya belum terkirim semua, unggahan dibatalkan
 * (server melepas kuotanya). Kalau sudah terkirim, server mungkin sedang memeriksanya:
 * unggahan dibiarkan selesai, lalu lampirannya dihapus begitu id-nya diketahui (lihat `add`),
 * supaya tidak ada lampiran yang tertinggal tanpa diketahui id-nya. */
function stop(draft: Draft) {
  if (draft.status === "uploading" && draft.progress < 1) draft.abort?.();
}

/** Lampiran yang sedang disiapkan untuk satu pesan. Dipegang per percakapan, jadi kalau
 * user pindah percakapan, unggahan dibatalkan dan lampiran yang belum dikirim dihapus. */
export function useAttachmentDraft() {
  const [items, setItems] = useState<Draft[]>([]);
  const [notice, setNotice] = useState<string | null>(null);
  // Sumber kebenaran yang langsung terbaru, supaya beberapa tempelan beruntun tidak saling menimpa.
  const listRef = useRef<Draft[]>([]);
  const alive = useRef(true);

  const commit = useCallback((next: Draft[]) => {
    listRef.current = next;
    setItems(next);
  }, []);
  const update = useCallback(
    (key: string, patch: Partial<Draft>) => commit(listRef.current.map((d) => (d.key === key ? { ...d, ...patch } : d))),
    [commit],
  );

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      for (const d of listRef.current) {
        stop(d);
        if (d.ref) discardAttachment(d.ref.id);
        if (d.url) URL.revokeObjectURL(d.url);
      }
      listRef.current = [];
    };
  }, []);

  const add = useCallback(
    (files: readonly File[], pasted = false, pageId?: string) => {
      setNotice(null);
      const problems: string[] = [];
      const added: Draft[] = [];
      for (const file of files) {
        const name = pasted || !file.name ? pastedName(file.name, file.type, new Date()) : file.name;
        const error = checkFile(name, file.size);
        if (error) {
          problems.push(`${name}: ${error}`);
          continue;
        }
        if (listRef.current.length + added.length >= MAX_ATTACHMENTS) {
          problems.push(`Maksimal ${MAX_ATTACHMENTS} lampiran per pesan.`);
          break;
        }
        const kind = extensionOf(name);
        seq += 1;
        added.push({
          key: `lampiran-${seq}`,
          file,
          name,
          kind,
          url: isImageKind(kind) ? URL.createObjectURL(file) : undefined,
          progress: 0,
          status: "uploading",
        });
      }
      if (problems.length) setNotice(problems.join(" "));
      if (!added.length) return;
      commit([...listRef.current, ...added]);
      for (const draft of added) {
        const { done, abort } = uploadAttachment(draft.file, draft.name, (progress) => update(draft.key, { progress }));
        update(draft.key, { abort });
        done.then(
          (ref) => {
            // Selesai setelah user pindah percakapan atau menghapusnya: buang di server juga.
            if (!alive.current || !listRef.current.some((d) => d.key === draft.key)) {
              discardAttachment(ref.id);
              return;
            }
            if (pageId && draft.file === files[0]) bindPageAttachment(pageId, ref.id);
            update(draft.key, { status: "ready", ref, progress: 1, abort: undefined });
          },
          (err: unknown) => {
            if (!listRef.current.some((d) => d.key === draft.key)) return;
            const error = err instanceof Error ? err.message : "Gagal mengunggah.";
            update(draft.key, { status: "error", error, abort: undefined });
          },
        );
      }
    },
    [commit, update],
  );

  const remove = useCallback(
    (key: string) => {
      const draft = listRef.current.find((d) => d.key === key);
      if (!draft) return;
      commit(listRef.current.filter((d) => d.key !== key));
      stop(draft);
      if (draft.ref) discardAttachment(draft.ref.id);
      if (draft.url) URL.revokeObjectURL(draft.url);
      setNotice(null);
    },
    [commit],
  );

  /** Setelah pesan terkirim. Lampirannya sudah milik pesan itu, jadi tidak dihapus di server. */
  const clear = useCallback(() => {
    for (const d of listRef.current) if (d.url) URL.revokeObjectURL(d.url);
    commit([]);
    setNotice(null);
  }, [commit]);

  /** Draf ini masih dipakai (percakapannya masih terbuka). */
  const isAlive = useCallback(() => alive.current, []);

  const ready = useMemo(() => items.flatMap((d) => (d.status === "ready" && d.ref ? [d.ref] : [])), [items]);
  const uploading = items.some((d) => d.status === "uploading");
  const failed = items.some((d) => d.status === "error");
  // Alasan pesan belum bisa dikirim, kalau ada.
  const blocker = failed
    ? "Hapus lampiran yang gagal sebelum mengirim."
    : totalError(items.map((d) => ({ name: d.name, size: d.file.size })));

  return { items, ready, uploading, blocker, notice, add, remove, clear, isAlive };
}

export type AttachmentDraft = ReturnType<typeof useAttachmentDraft>;
