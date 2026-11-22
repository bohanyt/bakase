import type { ChatModelAdapter, ThreadMessage } from "@assistant-ui/react";
import { UNAUTHORIZED_EVENT } from "./api";
import { getJoke } from "./joke";
import type { AnswerMeta, ConversationSummary, FileRef, SourceRef, Step } from "./messages";
import { readPageAction } from "./pageAction";
import { answerPageAction, consumePageRefresh, livePage, releasePage, verifyPageRefresh } from "./pageBridge";

export type ChatHooks = {
  /** ID percakapan yang sedang dibuka, atau null untuk percakapan baru. */
  getConversationId: () => string | null;
  /** Dipanggil saat server memberi tahu ID dan judul percakapan. */
  onConversation: (id: string, title: string) => void;
  onQuestion: (clientId: string, serverId: number) => void;
  onSummary: (summary: ConversationSummary | null) => void;
  /** Dipanggil setelah satu jawaban selesai. */
  onDone: () => void;
};

/** Teks dan id lampiran pesan user terakhir. Lampiran ditaruh kotak tanya di metadata pesan. */
function lastUserMessage(messages: readonly ThreadMessage[]) {
  const last = messages.findLast((m) => m.role === "user");
  const text = last?.content.map((p) => (p.type === "text" ? p.text : "")).join("") ?? "";
  const files = (last?.metadata.custom as AnswerMeta | undefined)?.attachments ?? [];
  return { id: last?.id, text, attachments: files.map((f) => f.id) };
}

/** Server memegang riwayat; browser hanya mengirim pesan terbaru. */
export function createChatAdapter(hooks: ChatHooks): ChatModelAdapter {
  return {
    async *run({ messages, abortSignal }) {
      const startedAt = Date.now();
      const { id: questionId, text: message, attachments } = lastUserMessage(messages);
      const pageAttachments = new Set(messages.flatMap(
        (m) => (m.role === "user" && (m.metadata.custom as AnswerMeta | undefined)?.attachments?.map((file) => file.id)) || [],
      ));
      let page = livePage(pageAttachments);
      if (page?.refresh) {
        const valid = await verifyPageRefresh(page, abortSignal);
        const current = livePage(pageAttachments);
        if (!valid || current?.id !== page.id || current.refresh !== page.refresh) page = null;
      }
      const res = await fetch("/api/chat", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          conversation_id: hooks.getConversationId(),
          message,
          style: getJoke() ? "joke" : "normal",
          attachments,
          page: page && {
            origin: page.origin,
            attachment_id: page.attachmentId,
            text: page.refresh?.text,
          },
        }),
        signal: abortSignal,
      }).catch((err: unknown) => {
        if (abortSignal.aborted && page) releasePage(page);
        throw err;
      });
      if (res.status === 401) {
        window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
        throw new Error("Sesi berakhir. Silakan masuk lagi.");
      }
      if (!res.ok || !res.body) {
        let message = `Server menjawab ${res.status}.`;
        try {
          const data = await res.json();
          if (typeof data?.message === "string") message = data.message;
        } catch {
          /* jawaban bukan JSON */
        }
        throw new Error(message);
      }

      if (page?.refresh) consumePageRefresh(page);
      const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
      const pageAbort = new AbortController();
      const pageSignal = AbortSignal.any([abortSignal, pageAbort.signal]);
      let buffer = "";
      let text = "";
      let finalText: string | undefined;
      let reasoning = "";
      let compacting = false;
      let durationMs: number | undefined;
      let messageId: number | undefined;
      let streamError: Error | undefined;
      let steps: Step[] = [];
      let files: FileRef[] = [];
      let sources: SourceRef[] = [];

      const snapshot = () => ({
        content: [
          ...(reasoning ? [{ type: "reasoning" as const, text: reasoning }] : []),
          { type: "text" as const, text },
        ],
        metadata: { custom: { startedAt, compacting, durationMs, messageId, steps, files, sources } },
      });

      try {
        while (true) {
          const { value, done } = await reader.read();
          if (done) break;
          buffer += value;

          let sep: number;
          while ((sep = buffer.indexOf("\n\n")) !== -1) {
            const raw = buffer.slice(0, sep);
            buffer = buffer.slice(sep + 2);
            let event = "message";
            let data = "";
            for (const line of raw.split("\n")) {
              if (line.startsWith("event:")) event = line.slice(6).trim();
              else if (line.startsWith("data:")) data += line.slice(5).trim();
            }
            if (!data) continue;
            const payload = JSON.parse(data);

            if (event === "conversation") {
              hooks.onConversation(payload.id, payload.title);
              if (questionId && typeof payload.question_id === "number") {
                hooks.onQuestion(questionId, payload.question_id);
              }
              hooks.onSummary(payload.summary ?? null);
              yield snapshot();
            } else if (event === "compaction") {
              compacting = payload.running === true;
              if (!compacting) hooks.onSummary(payload.summary ?? null);
              yield snapshot();
            } else if (event === "delta") {
              text += payload.text as string;
              if (payload.sources?.length) sources = [...sources, ...payload.sources as SourceRef[]];
              yield snapshot();
            } else if (event === "reasoning") {
              reasoning += payload.text as string;
              yield snapshot();
            } else if (event === "step") {
              const step = payload as Step;
              steps = steps.some((s) => s.id === step.id)
                ? steps.map((s) => (s.id === step.id ? step : s))
                : [...steps, step];
              yield snapshot();
            } else if (event === "page_action" && page) {
              const action = readPageAction(payload);
              if (action) {
                // Terus baca stream supaya putus koneksi segera membatalkan aksi yang menunggu.
                void answerPageAction(page, action, pageSignal).catch(() => {});
              }
            } else if (event === "file") {
              files = [...files, payload as FileRef];
              yield snapshot();
            } else if (event === "answer") {
              // Urutan nomor baru dan daftar sumbernya harus tampil bersamaan.
              finalText = payload.text as string;
            } else if (event === "sources") {
              text = finalText ?? text;
              finalText = undefined;
              sources = payload as SourceRef[];
              yield snapshot();
            } else if (event === "error") {
              // Server masih bisa menyimpan jawaban sebagian dan mengirim ID lewat done.
              streamError = new Error(payload.message as string);
            } else if (event === "done") {
              text = finalText ?? text;
              if (!streamError) durationMs = payload.duration_ms as number;
              messageId = (payload.message_id as number | null) ?? undefined;
              yield snapshot();
            }
          }
        }
        if (streamError) throw streamError;
      } finally {
        if (abortSignal.aborted && page) releasePage(page);
        pageAbort.abort();
        void reader.cancel().catch(() => {});
        hooks.onDone();
      }
    },
  };
}
