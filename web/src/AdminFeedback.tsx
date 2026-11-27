import { ArrowRightIcon, ChatCircleIcon, ThumbsDownIcon, ThumbsUpIcon } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import { api, departmentList } from "./api";
import { parseMessage } from "./quote";
import { fullDateTime } from "./time";

type Item = {
  id: number;
  rating: "up" | "down";
  reason: string | null;
  comment: string | null;
  question: string | null;
  answer: string | null;
  created_at: number;
  user_name: string;
  user_email: string;
  user_department: string | null;
  user_departments?: string[];
  user_role: "user" | "admin";
  /** Hanya terisi untuk feedback milik admin sendiri; chat user lain tetap privat. */
  conversation_id: string | null;
  conversation_title: string | null;
  message_id: number | null;
};
type List = { up: number; down: number; items: Item[] };

const REASON_LABEL: Record<string, string> = {
  salah: "Salah",
  kurang_lengkap: "Kurang lengkap",
  tidak_relevan: "Tidak relevan",
  lainnya: "Lainnya",
};

export type OpenChat = (conversationId: string, messageId: number) => void;

/** Pertanyaan tanpa bingkai kutipan; kutipannya cukup disebut jumlahnya. */
function questionText(message: string): string {
  const { quotes, question } = parseMessage(message);
  if (!quotes.length) return question;
  const note = `(${quotes.length} kutipan)`;
  return question ? `${question} ${note}` : note;
}

function FeedbackItem({ f, onOpenChat }: { f: Item; onOpenChat: OpenChat }) {
  const { conversation_id: conversationId, message_id: messageId } = f;
  return (
    <li className="fb-item">
      <div className="fb-head">
        {f.rating === "up" ? <ThumbsUpIcon size={16} className="up" /> : <ThumbsDownIcon size={16} className="down" />}
        <span className="fb-who">{f.user_name}</span>
        <span className={`pill ${f.user_role === "admin" ? "pill-pending" : "pill-disabled"}`}>
          {f.user_role === "admin" ? "Admin" : "User"}
        </span>
        {f.reason && <span className="pill pill-disabled">{REASON_LABEL[f.reason] ?? f.reason}</span>}
        <time className="fb-date" dateTime={new Date(f.created_at * 1000).toISOString()}>
          {fullDateTime(f.created_at)}
        </time>
      </div>
      <p className="fb-meta">
        {departmentList(f.user_departments, f.user_department).join(", ") || "Tanpa departemen"} · {f.user_email}
      </p>
      {f.comment && <p className="fb-comment">{f.comment}</p>}
      {conversationId && messageId !== null ? (
        <button type="button" className="fb-open" onClick={() => onOpenChat(conversationId, messageId)}>
          <ChatCircleIcon size={16} />
          <span className="fb-open-title">{f.conversation_title}</span>
          <span className="fb-open-cta">
            Buka chat <ArrowRightIcon size={14} />
          </span>
        </button>
      ) : f.answer ? (
        <details className="fb-shared">
          <summary>Lihat percakapan yang dibagikan</summary>
          {f.question && (
            <p>
              <strong>Pertanyaan:</strong> {questionText(f.question)}
            </p>
          )}
          <p className="fb-answer">
            <strong>Jawaban:</strong> {f.answer}
          </p>
        </details>
      ) : (
        f.rating === "down" && <p className="muted small">Percakapan tidak dibagikan.</p>
      )}
    </li>
  );
}

export function AdminFeedback({ onOpenChat }: { onOpenChat: OpenChat }) {
  const [list, setList] = useState<List | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api<List>("/api/admin/feedback").then(setList, (err) =>
      setError(err instanceof Error ? err.message : "Gagal memuat feedback."),
    );
  }, []);

  if (error) return <p className="form-error">{error}</p>;
  if (!list) return <p className="muted">Memuat…</p>;
  const total = list.up + list.down;

  return (
    <div className="fb-admin">
      <div className="fb-stats">
        <div className="stat">
          <ThumbsUpIcon size={18} /> <strong>{list.up}</strong> bagus
        </div>
        <div className="stat">
          <ThumbsDownIcon size={18} /> <strong>{list.down}</strong> kurang
        </div>
        {total > 0 && <div className="stat muted">{Math.round((list.up / total) * 100)}% puas</div>}
      </div>
      {list.items.length === 0 && <p className="muted">Belum ada feedback.</p>}
      <ul className="fb-list">
        {list.items.map((f) => (
          <FeedbackItem key={f.id} f={f} onOpenChat={onOpenChat} />
        ))}
      </ul>
    </div>
  );
}
