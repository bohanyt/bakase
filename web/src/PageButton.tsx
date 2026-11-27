import { BrowserIcon, SpinnerGapIcon } from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";
import { api, type AuthOptions } from "./api";
import type { AttachmentDraft } from "./attachmentDraft";
import { inPanel, panelOrigin, requestPage } from "./pageBridge";
import { pageFiles } from "./pageRead";

let originRequest: Promise<string | null> | null = null;

/** Origin extension kalau Bakase dibuka di panelnya, dicek sekali per halaman. */
function loadOrigin(): Promise<string | null> {
  originRequest ??= inPanel()
    ? api<AuthOptions>("/api/auth/options").then(
        (o) => panelOrigin(o.extension_ids ?? []),
        () => null,
      )
    : Promise.resolve(null);
  return originRequest;
}

/** Tombol "Baca halaman" di kotak tanya, hanya di panel extension. Isi tab yang sedang
 * dibuka masuk sebagai lampiran, jadi user bisa melihat dan menghapusnya sebelum mengirim. */
export function PageButton({
  draft,
  onError,
  onDone,
}: {
  draft: AttachmentDraft;
  onError: (message: string | null) => void;
  onDone: () => void;
}) {
  const [origin, setOrigin] = useState<string | null>(null);
  const [reading, setReading] = useState(false);
  // Kabar progres dari extension ("Membaca tabel 1: 120 baris…"), supaya tidak terasa macet.
  const [status, setStatus] = useState<string | null>(null);
  // Tombol ini masih tampil (kotak tanya sambutan diganti kotak tanya bawah saat pesan pertama
  // terkirim). Hasil baca tetap masuk ke draf selama percakapannya masih terbuka; kalau user
  // sudah pindah percakapan, hasilnya dibuang.
  const alive = useRef(true);

  useEffect(() => {
    alive.current = true;
    loadOrigin().then((o) => alive.current && setOrigin(o));
    return () => {
      alive.current = false;
    };
  }, []);

  if (!origin) return null;

  async function read(from: string) {
    onError(null);
    setReading(true);
    setStatus(null);
    try {
      const page = await requestPage(from, (text) => alive.current && setStatus(text));
      // File Excel asli tidak membawa teks halaman, jadi fokusnya ikut sebagai catatan lampiran.
      if (draft.isAlive()) draft.add(pageFiles(page, new Date()), false, page.snapshotId, page.file ? page.focus : undefined);
    } catch (e) {
      if (alive.current) onError(e instanceof Error ? e.message : "Halaman tidak bisa dibaca.");
    } finally {
      if (alive.current) {
        setReading(false);
        onDone();
      }
    }
  }

  return (
    <button
      type="button"
      className="page-btn"
      disabled={reading}
      title={
        reading
          ? (status ?? "Membaca halaman…")
          : "Lampirkan isi tab yang sedang dibuka (isian form, tabel, dan teks) beserta screenshot-nya"
      }
      onClick={() => read(origin)}
    >
      {reading ? <SpinnerGapIcon size={18} className="spin" /> : <BrowserIcon size={18} />}
      <span className="page-btn-label" aria-live="polite">
        {reading ? (status ?? "Membaca halaman…") : "Baca halaman"}
      </span>
    </button>
  );
}
