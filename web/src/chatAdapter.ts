import type { ChatModelAdapter, ThreadMessage } from "@assistant-ui/react";
import { ERROR_MODE, SESSION_CHECK_EVENT, UNAUTHORIZED_EVENT, realStatus, tabUserHeader } from "./api.ts";
import { getJoke } from "./joke.ts";
import type { AnswerMeta, ConversationSummary, FileRef, SourceRef, Step } from "./messages.ts";
import { readPageAction } from "./pageAction.ts";
import { answerPageAction, consumePageRefresh, livePage, releasePage, verifyPageRefresh } from "./pageBridge.ts";

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

/** Jeda sebelum satu kali kirim ulang otomatis. */
const RETRY_MS = 1_000;

/** Bukan `crypto.randomUUID`, yang hanya ada di halaman HTTPS. */
function newTurnId() {
  return Array.from(crypto.getRandomValues(new Uint8Array(16)), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

/** Sekali lagi kalau jaringan putus sebelum server menjawab. ID kirimannya sama, jadi tidak tersimpan dua kali. */
async function send(init: RequestInit & { signal: AbortSignal }) {
  try {
    return await fetch("/api/chat", init);
  } catch (err) {
    if (init.signal.aborted) throw err;
    await new Promise((resolve) => setTimeout(resolve, RETRY_MS));
    if (init.signal.aborted) throw err;
    return fetch("/api/chat", init);
  }
}

const UNREACHABLE = "Bakase tidak bisa dihubungi. Cek jaringan, lalu kirim ulang.";
const CUT_OFF = "Jawaban terputus di tengah jalan. Kirim ulang pertanyaannya. Kalau terus terjadi, hubungi IT.";

/** Server mengirim tanda hidup setiap 15 detik, jadi diam selama ini berarti sambungannya mati. */
const SILENCE_MS = 60_000;

/** Potongan berikutnya, atau gagal kalau tidak ada satu byte pun selama `SILENCE_MS`. */
async function readChunk(reader: ReadableStreamDefaultReader<string>) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const silence = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(CUT_OFF)), SILENCE_MS);
  });
  try {
    return await Promise.race([reader.read(), silence]);
  } finally {
    clearTimeout(timer);
  }
}

/** Teks dan id lampiran pesan user terakhir. Lampiran ditaruh kotak tanya di metadata pesan. */
function lastUserMessage(messages: readonly ThreadMessage[]) {
  const last = messages.findLast((m) => m.role === "user");
  const text = last?.content.map((p) => (p.type === "text" ? p.text : "")).join("") ?? "";
  const files = (last?.metadata.custom as AnswerMeta | undefined)?.attachments ?? [];
  return { id: last?.id, text, attachments: files.map((f) => f.id) };
}

/** Server memegang riwayat; browser hanya mengirim pesan terbaru. */
export function createChatAdapter(hooks: ChatHooks) {
  let previousTurnId: string | undefined;
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
      const turnId = newTurnId();
      const previous = previousTurnId;
      previousTurnId = turnId;
      const res = await send({
        method: "POST",
        headers: { ...ERROR_MODE, "Content-Type": "application/json", ...tabUserHeader() },
        body: JSON.stringify({
          conversation_id: hooks.getConversationId(),
          message,
          style: getJoke() ? "joke" : "normal",
          attachments,
          turn_id: turnId,
          previous_turn_id: previous,
          page: page && {
            origin: page.origin,
            attachment_id: page.attachmentId,
            text: page.refresh?.text,
            ambil_tabel: page.ambilTabel,
          },
        }),
        signal: abortSignal,
      }).catch((err: unknown) => {
        if (abortSignal.aborted && page) releasePage(page);
        throw abortSignal.aborted ? err : new Error(UNREACHABLE);
      });
      const status = realStatus(res.status, res.headers.get("X-app-Status"));
      if (status === 401) {
        window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
        throw new Error("Sesi berakhir. Silakan masuk lagi.");
      }
      if (status === 409) window.dispatchEvent(new Event(SESSION_CHECK_EVENT));
      if (status < 200 || status >= 300 || !res.body) {
        let message = `Server menjawab ${status}.`;
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
      let receivedDone = false;
      let hasTextAfterTool: boolean | undefined;
      let steps: Step[] = [];
      let files: FileRef[] = [];
      let sources: SourceRef[] = [];

      const snapshot = () => ({
        content: [
          ...(reasoning ? [{ type: "reasoning" as const, text: reasoning }] : []),
          { type: "text" as const, text },
        ],
        metadata: { custom: { startedAt, compacting, durationMs, messageId, steps, files, sources, hasTextAfterTool } },
      });

      try {
        while (true) {
          const { value, done } = await readChunk(reader).catch((err: unknown) => {
            throw abortSignal.aborted ? err : new Error(CUT_OFF);
          });
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
              // Balasan untuk kiriman ulang tidak membawa ringkasan; yang tampil dibiarkan.
              if ("summary" in payload) hooks.onSummary(payload.summary);
              yield snapshot();
            } else if (event === "compaction") {
              compacting = payload.running === true;
              if (!compacting) hooks.onSummary(payload.summary ?? null);
              yield snapshot();
            } else if (event === "delta") {
              const delta = payload.text as string;
              text += delta;
              if (hasTextAfterTool === false && delta.trim()) hasTextAfterTool = true;
              if (payload.sources?.length) sources = [...sources, ...payload.sources as SourceRef[]];
              yield snapshot();
            } else if (event === "reasoning") {
              reasoning += payload.text as string;
              yield snapshot();
            } else if (event === "step") {
              const step = payload as Step;
              if (steps.some((s) => s.id === step.id)) {
                steps = steps.map((s) => (s.id === step.id ? step : s));
              } else {
                hasTextAfterTool = false;
                steps = [...steps, step];
              }
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
              receivedDone = true;
              text = finalText ?? text;
              if (!streamError) durationMs = payload.duration_ms as number;
              messageId = (payload.message_id as number | null) ?? undefined;
              yield snapshot();
            }
          }
        }
        if (abortSignal.aborted) return;
        if (streamError) throw streamError;
        if (!receivedDone) {
          throw new Error(CUT_OFF);
        }
      } catch (error) {
        if (!abortSignal.aborted) throw error;
      } finally {
        if (abortSignal.aborted && page) releasePage(page);
        pageAbort.abort();
        void reader.cancel().catch(() => {});
        hooks.onDone();
      }
    },
  } satisfies ChatModelAdapter;
}
