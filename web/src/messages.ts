import type { ThreadMessage, ThreadMessageLike } from "@assistant-ui/react";

export type Rating = "up" | "down";

/** File buatan Bakase, misalnya Excel. */
export type FileRef = { id: string; name: string; kind: string; size: number };

/** Potongan dokumen yang dikutip jawaban; nomornya sama dengan [n] di teks. */
export type SourceRef = {
  n: number;
  chunk_id: string;
  doc_id: number;
  /** Nama dokumen asli. */
  title: string;
  /** Judul rapi tanpa awalan ARCHIVE, seperti di Sumber Data. */
  display_title?: string;
  source: string;
  kind: string;
  page: number | null;
  sheet?: string;
  image?: { key: string; place: string; similar: boolean };
};

/** Judul sumber untuk ditampilkan. */
export function sourceName(s: SourceRef): string {
  return s.display_title || s.title;
}

/** Satu langkah tool yang tampil di atas jawaban ("Membuat file Excel…"). */
export type Step = { id: string; label: string; done: boolean; failed: boolean };

export type ConversationSummary = { through_message_id: number; content: string };

export type StoredMessage = {
  id: number;
  role: "user" | "assistant";
  content: string;
  created_at: number;
  duration_ms: number | null;
  feedback: Rating | null;
  files: FileRef[];
  reasoning: string | null;
  steps: Step[];
  sources?: SourceRef[];
};

export type ConversationDetail = {
  id: string;
  messages: StoredMessage[];
  summary: ConversationSummary | null;
};

/** Data tambahan per jawaban, disimpan di metadata.custom milik assistant-ui. */
export type AnswerMeta = {
  startedAt?: number;
  compacting?: boolean;
  hasTextAfterTool?: boolean;
  durationMs?: number;
  /** ID pesan di server; dibutuhkan untuk mengirim feedback. */
  messageId?: number;
  feedback?: Rating | null;
  steps?: Step[];
  files?: FileRef[];
  sources?: SourceRef[];
  /** Lampiran di pesan user. */
  attachments?: FileRef[];
};

/** Pesan dari server ke bentuk yang dipakai assistant-ui. */
export function toThreadMessages(messages: StoredMessage[]): ThreadMessageLike[] {
  return messages.map((m) => {
    // Server mengirim lampiran user di `files` juga; di pesan user itu lampiran, bukan file buatan Bakase.
    const custom: AnswerMeta =
      m.role === "user"
        ? { messageId: m.id, attachments: m.files }
        : {
            durationMs: m.duration_ms ?? undefined,
            messageId: m.id,
            feedback: m.feedback,
            files: m.files,
            steps: m.steps,
            sources: m.sources ?? [],
          };
    // Pemikiran yang tersimpan dipasang lagi sebagai bagian "Pemikiran" di atas jawaban.
    const content = m.reasoning
      ? [
          { type: "reasoning" as const, text: m.reasoning },
          { type: "text" as const, text: m.content },
        ]
      : m.content;
    return {
      id: String(m.id),
      role: m.role,
      content,
      createdAt: new Date(m.created_at * 1000),
      metadata: { custom },
    };
  });
}

export function serverMessageId(message: ThreadMessage, questionIds: ReadonlyMap<string, number>) {
  return (message.metadata.custom as AnswerMeta | undefined)?.messageId ?? questionIds.get(message.id);
}

/** Sisipkan pesan lokal yang belum tersimpan setelah pesan server terakhir yang mendahuluinya. */
export function mergeThreadMessages(
  stored: StoredMessage[],
  local: readonly ThreadMessage[],
  questionIds: ReadonlyMap<string, number>,
): ThreadMessageLike[] {
  const persisted = new Set(stored.map((message) => message.id));
  const clientIds = new Map<number, string>();
  const localAfter = new Map<number | undefined, ThreadMessage[]>();
  let anchor: number | undefined;
  for (const message of local) {
    const id = serverMessageId(message, questionIds);
    if (id !== undefined && persisted.has(id)) {
      anchor = id;
      clientIds.set(id, message.id);
    } else {
      const bucket = localAfter.get(anchor) ?? [];
      bucket.push(message);
      localAfter.set(anchor, bucket);
    }
  }
  return [
    ...(localAfter.get(undefined) ?? []),
    ...toThreadMessages(stored).flatMap((message, index) => {
      const id = stored[index].id;
      return [{ ...message, id: clientIds.get(id) ?? message.id }, ...(localAfter.get(id) ?? [])];
    }),
  ];
}

/** Nama jenis file untuk ditampilkan. */
export function kindLabel(kind: string): string {
  if (kind === "xlsx" || kind === "xls" || kind === "xlsm") return "Excel";
  if (kind === "docx" || kind === "doc") return "Word";
  if (kind === "pptx" || kind === "ppt") return "PowerPoint";
  if (kind === "pdf") return "PDF";
  if (kind === "txt" || kind === "md" || kind === "csv") return "Teks";
  if (["png", "jpg", "jpeg", "gif", "webp"].includes(kind)) return "Gambar";
  return "File";
}

/** Letak potongan di dokumen: "hal. 3" untuk PDF, "slide 3" untuk presentasi. */
export function placeLabel(kind: string, page: number | null | undefined): string | null {
  if (page == null) return null;
  return kind === "pdf" ? `hal. ${page}` : `slide ${page}`;
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1).replace(".", ",")} MB`;
}
