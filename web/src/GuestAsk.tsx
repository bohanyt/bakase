import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from "react";
import { ArrowUpIcon, SignInIcon } from "@phosphor-icons/react";
import { Streamdown } from "streamdown";
import { ERROR_MODE, realStatus } from "./api";
import { BlockedImage } from "./BlockedImage";
import { Mascot } from "./Mascot";
import { saveFollowUp } from "./guestFollowUp";

const MAX_CHARS = 1000;
const EXAMPLES = ["Apa isi ringkasan rapat yang baik?", "Cara membuat tabel anggaran sederhana", "Bantu rapikan kalimat email ke pelanggan"];

type Phase = "ask" | "answering" | "done";
const ANSWER_COMPONENTS = { img: BlockedImage };

/** Satu pertanyaan tanpa login. Lanjutannya disimpan dan dipakai setelah masuk. */
export function GuestAsk({ onWantLogin }: { onWantLogin: () => void }) {
  const [phase, setPhase] = useState<Phase>("ask");
  const [draft, setDraft] = useState("");
  const [asked, setAsked] = useState("");
  const [answer, setAnswer] = useState("");
  const [error, setError] = useState<string | null>(null);
  const abort = useRef<AbortController | null>(null);

  // Jawaban yang masih mengalir dihentikan kalau user keburu masuk.
  useEffect(() => () => abort.current?.abort(), []);

  async function ask(text: string) {
    const question = text.trim();
    if (!question || phase !== "ask") return;
    setAsked(question);
    setDraft("");
    setAnswer("");
    setError(null);
    setPhase("answering");
    const controller = new AbortController();
    abort.current = controller;
    try {
      const res = await fetch("/api/guest/chat/archive-v1", {
        method: "POST",
        headers: { ...ERROR_MODE, "Content-Type": "application/json" },
        body: JSON.stringify({ message: question }),
        signal: controller.signal,
      });
      const status = realStatus(res.status, res.headers.get("X-app-Status"));
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
      const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
      let buffer = "";
      let finished = false;
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
          if (event === "delta") setAnswer((a) => a + payload.text);
          else if (event === "error") {
            setError(payload.message);
            finished = true;
          } else if (event === "done") finished = true;
        }
      }
      if (!finished) setError("Jawaban terputus sebelum selesai. Masuk untuk bertanya lagi.");
    } catch (err) {
      if (!controller.signal.aborted) setError(err instanceof Error ? err.message : "Terjadi kesalahan.");
    } finally {
      setPhase("done");
    }
  }

  function submitQuestion(e: FormEvent) {
    e.preventDefault();
    void ask(draft);
  }

  function continueAfterLogin(e: FormEvent) {
    e.preventDefault();
    saveFollowUp(draft);
    onWantLogin();
  }

  function sendOnEnter(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      e.currentTarget.form?.requestSubmit();
    }
  }

  if (phase === "ask") {
    return (
      <div className="guest">
        <div className="guest-head">
          <Mascot asset="welcome" placement="welcome" />
          <h2>Mau tanya apa?</h2>
          <p>Satu pertanyaan cepat tanpa login.</p>
        </div>
        <form className="composer guest-composer" onSubmit={submitQuestion}>
          <textarea
            className="composer-input"
            aria-label="Pertanyaan tanpa login"
            placeholder="Tanya apa saja"
            rows={1}
            maxLength={MAX_CHARS}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={sendOnEnter}
          />
          <div className="composer-bar">
            <span className="bar-spacer" />
            <button className="send-btn" type="submit" aria-label="Kirim" disabled={!draft.trim()}>
              <ArrowUpIcon size={18} weight="bold" />
            </button>
          </div>
        </form>
        <div className="guest-examples">
          {EXAMPLES.map((q) => (
            <button key={q} type="button" onClick={() => void ask(q)}>
              {q}
            </button>
          ))}
        </div>
        <p className="hint">Tanpa login, Bakase tidak membuka dokumen kantor. Pertanyaan dan jawaban disimpan di arsip IT.</p>
      </div>
    );
  }

  return (
    <div className="guest guest-chat">
      <div className="guest-thread" aria-live="polite">
        <p className="guest-question">{asked}</p>
        <div className="guest-answer">
          <Mascot
            className="guest-mascot"
            asset={error ? "kaget" : phase === "done" ? "diam" : answer ? "bicara" : "mikir"}
            placement={phase === "done" && !error ? "guestIdle" : "guest"}
          />
          <div className="md">
            {answer ? (
              <Streamdown
                components={ANSWER_COMPONENTS}
                controls={{ table: { copy: true, download: false, fullscreen: false }, code: { copy: true, download: false }, mermaid: false }}
              >
                {answer}
              </Streamdown>
            ) : phase === "answering" ? (
              <p className="guest-thinking">Bakase sedang berpikir…</p>
            ) : null}
            {error && <p className="form-error">{error}</p>}
          </div>
        </div>
      </div>

      {phase === "done" && (
        <form className="guest-next" onSubmit={continueAfterLogin}>
          <p>
            <strong>Mau lanjut?</strong> Masuk untuk bertanya lagi, mencari di dokumen kantor, dan membuat file Excel atau
            Word. Pertanyaan lanjutanmu langsung muncul di chat.
          </p>
          <div className="composer guest-composer">
            <textarea
              className="composer-input"
              aria-label="Pertanyaan lanjutan"
              placeholder="Tulis pertanyaan lanjutan (boleh kosong)"
              rows={1}
              maxLength={20000}
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={sendOnEnter}
            />
          </div>
          <button className="auth-submit" type="submit">
            <SignInIcon size={18} weight="bold" />
            Masuk untuk lanjut
          </button>
        </form>
      )}
    </div>
  );
}
