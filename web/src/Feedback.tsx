import { useAuiState } from "@assistant-ui/react";
import { ThumbsDownIcon, ThumbsUpIcon } from "@phosphor-icons/react";
import { useState, type FormEvent } from "react";
import { api } from "./api";
import type { AnswerMeta, Rating } from "./messages";

const REASONS = [
  { id: "salah", label: "Salah" },
  { id: "kurang_lengkap", label: "Kurang lengkap" },
  { id: "tidak_relevan", label: "Tidak relevan" },
  { id: "lainnya", label: "Lainnya" },
];

/** 👍/👎 untuk satu jawaban. 👎 membuka form alasan, komentar, dan pilihan berbagi. */
export function Feedback() {
  const meta = useAuiState((s) => s.message.metadata?.custom as AnswerMeta | undefined);
  const messageId = meta?.messageId;
  const [rating, setRating] = useState<Rating | null>(meta?.feedback ?? null);
  const [formOpen, setFormOpen] = useState(false);
  const [reason, setReason] = useState<string | null>(null);
  const [thanks, setThanks] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Jawaban yang gagal disimpan tidak punya ID, jadi tidak bisa dinilai.
  if (messageId === undefined) return null;

  async function send(body: Record<string, unknown>) {
    setError(null);
    try {
      await api(`/api/messages/${messageId}/feedback`, { body });
      return true;
    } catch (err) {
      setError(err instanceof Error ? err.message : "Gagal mengirim feedback.");
      return false;
    }
  }

  async function toggle(next: Rating) {
    const value = rating === next ? null : next;
    if (await send({ rating: value })) {
      setRating(value);
      setFormOpen(value === "down");
      setThanks(value === "up");
    }
  }

  async function submitDetail(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const form = new FormData(e.currentTarget);
    const ok = await send({
      rating: "down",
      reason,
      comment: String(form.get("comment") ?? ""),
      share: form.get("share") === "on",
    });
    if (ok) {
      setFormOpen(false);
      setThanks(true);
    }
  }

  return (
    <>
      <button
        type="button"
        className={`icon-btn${rating === "up" ? " on" : ""}`}
        aria-label="Jawaban bagus"
        aria-pressed={rating === "up"}
        onClick={() => toggle("up")}
      >
        <ThumbsUpIcon size={16} />
      </button>
      <button
        type="button"
        className={`icon-btn${rating === "down" ? " on" : ""}`}
        aria-label="Jawaban kurang bagus"
        aria-pressed={rating === "down"}
        onClick={() => toggle("down")}
      >
        <ThumbsDownIcon size={16} />
      </button>
      {thanks && !formOpen && <span className="fb-thanks">Terima kasih atas masukannya.</span>}
      {error && <span className="fb-error">{error}</span>}
      {formOpen && (
        <form className="fb-form" onSubmit={submitDetail}>
          <p className="fb-title">Apa yang kurang?</p>
          <div className="fb-reasons">
            {REASONS.map((r) => (
              <button
                key={r.id}
                type="button"
                className={`chip${reason === r.id ? " on" : ""}`}
                aria-pressed={reason === r.id}
                onClick={() => setReason(reason === r.id ? null : r.id)}
              >
                {r.label}
              </button>
            ))}
          </div>
          <textarea
            id={`fb-comment-${messageId}`}
            name="comment"
            rows={2}
            maxLength={1000}
            placeholder="Ceritakan lebih lanjut (opsional)"
          />
          <label className="fb-share">
            {/* Tercentang dari awal supaya admin bisa memperbaiki dari contoh nyata; user tetap
                bisa menghapus centangnya sebelum mengirim. */}
            <input id={`fb-share-${messageId}`} name="share" type="checkbox" defaultChecked />
            Kirim pertanyaan dan jawaban ini ke admin untuk perbaikan
          </label>
          <div className="fb-actions">
            <button type="button" className="text-btn" onClick={() => setFormOpen(false)}>
              Lewati
            </button>
            <button type="submit" className="primary small-btn">
              Kirim
            </button>
          </div>
        </form>
      )}
    </>
  );
}
