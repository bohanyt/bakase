import {
  ActionBarPrimitive,
  AssistantRuntimeProvider,
  AuiIf,
  ComposerPrimitive,
  MessagePrimitive,
  ThreadPrimitive,
  useAui,
  useAuiState,
  useLocalRuntime,
  type ThreadMessageLike,
} from "@assistant-ui/react";
import {
  ArrowUpIcon,
  BrainIcon,
  CaretDownIcon,
  CheckCircleIcon,
  CheckIcon,
  CopyIcon,
  DownloadSimpleIcon,
  EyeIcon,
  PaperclipIcon,
  PencilSimpleIcon,
  SpinnerGapIcon,
  StopIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { createContext, useContext, useEffect, useImperativeHandle, useMemo, useRef, useState, type ComponentProps, type FormEvent, type Ref } from "react";
import { Block, Streamdown, defaultComponents, defaultRemarkPlugins, parseMarkdownIntoBlocks, type BlockProps, type ExtraProps } from "streamdown";
import { ANSWER_PLUGINS, mermaidOptions } from "./answerVisuals";
import { api } from "./api";
import { AnswerRunning } from "./answerRunning";
import { remarkCitations, SOURCE_LINK } from "./citations";
import { createChatAdapter } from "./chatAdapter";
import { Feedback } from "./Feedback";
import { BlockedImage } from "./BlockedImage";
import { takeFollowUp } from "./guestFollowUp";
import { getJoke, setJoke } from "./joke";
import { FileIcon } from "./FileIcon";
import { PageButton } from "./PageButton";
import { formatSize, kindLabel, mergeThreadMessages, serverMessageId, type AnswerMeta, type ConversationDetail, type ConversationSummary, type SourceRef } from "./messages";
import { PreviewContext } from "./previewContext";
import { SourceCards, SourceChip } from "./SourceChip";
import { ImageGallery } from "./ImageGallery";
import { formatMessage, lengthError, type Quote } from "./quote";
import { QuoteChips, UserText } from "./Quotes";
import type { DraftSnapshot } from "./reloadDraft";
import { AttachmentChips, MessageAttachments } from "./Attachments";
import { useAttachmentDraft, type AttachmentDraft } from "./attachmentDraft";
import { ACCEPT } from "./attachment";
import { useTheme } from "./theme";

function formatSeconds(ms: number) {
  const s = Math.max(1, Math.round(ms / 1000));
  return s < 60 ? `${s} dtk` : `${Math.floor(s / 60)} mnt ${s % 60} dtk`;
}

type RemarkPlugins = NonNullable<BlockProps["remarkPlugins"]>;
const CitationContext = createContext({ known: new Map<number, SourceRef>(), tail: -1 });
const CitationBlockIndex = createContext(0);
const DEFAULT_REMARK_PLUGINS = Object.values(defaultRemarkPlugins);

function CitationBlock(props: BlockProps) {
  const { known, tail } = useContext(CitationContext);
  const streaming = tail === props.index;
  const remarkPlugins = useMemo((): RemarkPlugins => [
    ...(props.remarkPlugins ?? DEFAULT_REMARK_PLUGINS),
    [remarkCitations, { numbers: [...known.keys()], streaming }],
  ], [props.remarkPlugins, known, streaming]);
  return <CitationBlockIndex.Provider value={props.index}>
    <Block {...props} remarkPlugins={remarkPlugins} />
  </CitationBlockIndex.Provider>;
}

function CitationLink(props: ComponentProps<"a"> & ExtraProps) {
  const { known } = useContext(CitationContext);
  const block = useContext(CitationBlockIndex);
  // Hanya link buatan plugin yang tidak memiliki posisi di Markdown asli.
  if (!props.node?.position && props.href?.startsWith(SOURCE_LINK)) {
    const [numbers, group] = props.href.slice(SOURCE_LINK.length).split("/");
    const sources = numbers.split(",").flatMap((n) => known.get(Number(n)) ?? []);
    return sources.length ? <SourceChip sources={sources} group={`${block}:${group}`} /> : null;
  }
  return <defaultComponents.a {...props} />;
}

const ANSWER_COMPONENTS = { img: BlockedImage, a: CitationLink };

function MarkdownText({ text }: { text: string }) {
  const sources = useAuiState((s) => (s.message.metadata?.custom as AnswerMeta | undefined)?.sources);
  const open = useContext(PreviewContext);
  const known = useMemo(() => new Map((sources ?? []).map((s) => [s.n, s])), [sources]);
  // Diagram dibaca ulang dengan warna tema baru saat tema berganti.
  const theme = useTheme();
  const mermaid = useMemo(() => mermaidOptions(theme), [theme]);
  const running = useAuiState((s) => s.message.status?.type === "running");
  const tail = useMemo(() => running ? parseMarkdownIntoBlocks(text).length - 1 : -1, [running, text]);
  const citations = useMemo(() => ({ known, tail }), [known, tail]);
  return (
    <SourceCards sources={sources ?? []} onOpen={(source) => open({ type: "source", source })}>
      <AnswerRunning.Provider value={running}>
        <CitationContext.Provider value={citations}>
          <Streamdown
            key={theme}
            BlockComponent={CitationBlock}
            tableMaxHeight="none"
            plugins={ANSWER_PLUGINS}
            mermaid={mermaid}
            components={ANSWER_COMPONENTS}
            controls={{ table: { copy: true, download: false, fullscreen: false }, code: { copy: true, download: false }, mermaid: { copy: true, download: true, fullscreen: false, panZoom: false } }}
          >
            {text}
          </Streamdown>
        </CitationContext.Provider>
      </AnswerRunning.Provider>
    </SourceCards>
  );
}

function Reasoning({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="reasoning">
      <button type="button" className="reasoning-toggle" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <BrainIcon size={15} /> Pemikiran <CaretDownIcon size={14} className={open ? "rot" : ""} />
      </button>
      {open && <div className="reasoning-body">{text}</div>}
    </div>
  );
}

/** "Sedang berpikir · 5 dtk" saat jawaban berjalan, "Selesai dalam 12 dtk" sesudahnya. */
function WorkingTime() {
  const running = useAuiState((s) => s.message.status?.type === "running");
  const custom = useAuiState((s) => s.message.metadata?.custom as AnswerMeta | undefined);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (!running) return;
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [running]);

  if (running && custom?.compacting) {
    return (
      <div className="working compaction-status" role="status">
        <span className="compaction-mascot" aria-hidden="true">
          <img src="/brand/mascot.png" width={28} height={28} alt="" />
          <PencilSimpleIcon className="compaction-pen" size={14} weight="bold" />
        </span>
        Merangkum obrolan sejauh ini…
      </div>
    );
  }
  if (running) {
    const elapsed = custom?.startedAt ? now - custom.startedAt : 0;
    return (
      <div className="working">
        <img className="working-mascot" src="/brand/mascot.png" width={24} height={24} alt="" />
        Sedang berpikir · {formatSeconds(elapsed)}
      </div>
    );
  }
  if (custom?.durationMs) {
    return <div className="worked">Selesai dalam {formatSeconds(custom.durationMs)}</div>;
  }
  return null;
}

/** ID pesan di server, dipasang sebagai `data-server-id` supaya pesan bisa dicari
 * (mis. dari Admin → Feedback) tanpa memuat ulang chat yang sedang berjalan. */
function useServerId() {
  return useAuiState((s) => (s.message.metadata?.custom as AnswerMeta | undefined)?.messageId);
}

function UserMessage() {
  const serverId = useServerId();
  const attachments = useAuiState((s) => (s.message.metadata?.custom as AnswerMeta | undefined)?.attachments);
  // Pesan yang hanya berisi lampiran tidak diberi bubble kosong.
  const hasText = useAuiState((s) => s.message.content.some((p) => p.type === "text" && p.text.trim() !== ""));
  return (
    <MessagePrimitive.Root className="row row-user" data-server-id={serverId}>
      <div className="user-stack">
        {attachments?.length ? <MessageAttachments files={attachments} /> : null}
        {hasText && (
          <div className="bubble">
            <MessagePrimitive.Parts components={{ Text: UserText }} />
          </div>
        )}
      </div>
    </MessagePrimitive.Root>
  );
}

/** Langkah tool di atas jawaban, misalnya "Membuat file Excel…". */
function Steps() {
  const steps = useAuiState((s) => (s.message.metadata?.custom as AnswerMeta | undefined)?.steps);
  const running = useAuiState((s) => s.message.status?.type === "running");
  if (!steps?.length) return null;
  return (
    <ul className="steps">
      {steps.map((step) => {
        // Spinner hanya selama jawaban berjalan; langkah yang tertinggal tanpa hasil dianggap gagal.
        const failed = step.done ? step.failed : !running;
        const state = failed ? "failed" : step.done ? "done" : undefined;
        return (
          <li key={step.id} className={state}>
            {failed ? (
              <WarningCircleIcon size={16} weight="fill" />
            ) : step.done ? (
              <CheckCircleIcon size={16} weight="fill" />
            ) : (
              <SpinnerGapIcon size={16} className="spin" />
            )}
            <span>{step.label}</span>
          </li>
        );
      })}
    </ul>
  );
}

function AnswerImages() {
  const sources = useAuiState((s) => (s.message.metadata?.custom as AnswerMeta | undefined)?.sources);
  const open = useContext(PreviewContext);
  const show = (source: SourceRef) => open({ type: "source", source });
  return (
    <>
      <ImageGallery sources={sources?.filter((source) => source.image && !source.image.similar) ?? []} onOpen={show} />
      <ImageGallery sources={sources?.filter((source) => source.image?.similar) ?? []} similar onOpen={show} />
    </>
  );
}

/** Kartu file buatan Bakase: preview di panel kanan atau unduh. */
function FileCards() {
  const files = useAuiState((s) => (s.message.metadata?.custom as AnswerMeta | undefined)?.files);
  const openPreview = useContext(PreviewContext);
  if (!files?.length) return null;
  return (
    <div className="file-cards">
      {files.map((f) => (
        <div key={f.id} className="file-card">
          <FileIcon kind={f.kind} size={30} />
          <div className="file-meta">
            <span className="file-name" title={f.name}>
              {f.name}
            </span>
            <span className="file-size">
              {kindLabel(f.kind)} · {formatSize(f.size)}
            </span>
          </div>
          <button type="button" className="text-btn" onClick={() => openPreview({ type: "file", file: f })}>
            <EyeIcon size={16} /> Preview
          </button>
          <a className="text-btn" href={`/api/files/${f.id}`} download>
            <DownloadSimpleIcon size={16} /> Unduh
          </a>
        </div>
      ))}
    </div>
  );
}

function AssistantMessage() {
  const serverId = useServerId();
  return (
    <MessagePrimitive.Root className="row row-assistant" data-server-id={serverId}>
      <WorkingTime />
      <Steps />
      <MessagePrimitive.Parts components={{ Text: MarkdownText, Reasoning }} />
      <AnswerImages />
      <FileCards />
      <MessagePrimitive.Error>
        <p className="msg-error">Gagal mendapat jawaban. Coba kirim ulang.</p>
      </MessagePrimitive.Error>
      <ActionBarPrimitive.Root hideWhenRunning autohide="never" className="actions-bar">
        <ActionBarPrimitive.Copy className="icon-btn" aria-label="Salin jawaban">
          <AuiIf condition={(s) => s.message.isCopied}>
            <CheckIcon size={16} />
          </AuiIf>
          <AuiIf condition={(s) => !s.message.isCopied}>
            <CopyIcon size={16} />
          </AuiIf>
        </ActionBarPrimitive.Copy>
        <Feedback />
      </ActionBarPrimitive.Root>
    </MessagePrimitive.Root>
  );
}

function SummaryDivider({ content }: { content: string }) {
  return (
    <details className="compaction-divider">
      <summary>
        <span>Obrolan sebelumnya sudah diringkas</span>
        <CaretDownIcon size="1em" aria-hidden="true" />
      </summary>
      <div className="compaction-content">{content}</div>
    </details>
  );
}

function JokeToggle() {
  const [on, setOn] = useState(getJoke);
  function toggle() {
    setJoke(!on);
    setOn(!on);
  }
  return (
    <button
      type="button"
      className={`joke-btn${on ? " on" : ""}`}
      aria-pressed={on}
      title={on ? "Bakase menjawab lebih santai, dengan humor ringan" : "Nyalakan supaya Bakase menjawab lebih santai"}
      onClick={toggle}
    >
      <img src="/brand/mascot.png" width={22} height={22} alt="" />
      Joke {on ? "on" : "off"}
    </button>
  );
}

type QuoteProps = { quotes: Quote[]; onQuotesChange: (quotes: Quote[]) => void };

/** File yang diseret ke kotak tanya, bukan teks atau tautan. */
function draggingFiles(e: DragEvent) {
  return Array.from(e.dataTransfer?.types ?? []).includes("Files");
}

function Composer({ quotes, onQuotesChange, draft }: QuoteProps & { draft: AttachmentDraft }) {
  const aui = useAui();
  const text = useAuiState((s) => s.composer.text);
  const running = useAuiState((s) => s.thread.isRunning);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const pickerRef = useRef<HTMLInputElement>(null);
  const [dragging, setDragging] = useState(false);
  const [pageError, setPageError] = useState<string | null>(null);
  const message = formatMessage(quotes, text);
  const tooLong = lengthError(message, quotes.length > 0);
  const hasAttachments = draft.items.length > 0;
  const canSend = !running && (message !== "" || draft.ready.length > 0) && !tooLong && !draft.uploading && !draft.blocker;

  // Kutipan baru langsung siap ditanyakan.
  const quoteCount = useRef(quotes.length);
  useEffect(() => {
    if (quotes.length > quoteCount.current) inputRef.current?.focus();
    quoteCount.current = quotes.length;
  }, [quotes.length]);

  // Kirim bawaan assistant-ui dilewati supaya kutipan ikut di teks pesan dan boleh tanpa pertanyaan.
  // Lampiran ikut di metadata pesan; adapter chat mengirim id-nya ke server.
  function submit(e: FormEvent) {
    e.preventDefault();
    if (!canSend) return;
    setPageError(null);
    if (draft.ready.length) {
      aui.thread.append({
        role: "user",
        content: [{ type: "text", text: message }],
        metadata: { custom: { attachments: draft.ready } },
      });
      aui.composer.setText("");
      draft.clear();
    } else {
      // Teks selalu lewat formatMessage: penanda kutipan yang diketik user di-escape.
      aui.composer.setText(message);
      aui.composer.send();
    }
    if (quotes.length) onQuotesChange([]);
  }

  return (
    <ComposerPrimitive.Root
      className={`composer${dragging ? " dragging" : ""}`}
      onSubmit={submit}
      onDragOver={(e) => {
        if (!draggingFiles(e.nativeEvent)) return;
        e.preventDefault();
        setDragging(true);
      }}
      onDragLeave={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false);
      }}
      onDrop={(e) => {
        if (!draggingFiles(e.nativeEvent)) return;
        e.preventDefault();
        setDragging(false);
        draft.add(Array.from(e.dataTransfer.files));
        inputRef.current?.focus();
      }}
    >
      <AttachmentChips draft={draft} onDone={() => inputRef.current?.focus()} />
      <QuoteChips quotes={quotes} onChange={onQuotesChange} onDone={() => inputRef.current?.focus()} />
      <ComposerPrimitive.Input
        ref={inputRef}
        className="composer-input"
        placeholder={
          hasAttachments ? "Tanya soal lampiran ini" : quotes.length ? "Tanya soal kutipan ini" : "Tanya apa saja soal kantor"
        }
        rows={1}
        autoFocus
        onPaste={(e) => {
          // Tangkapan layar (Ctrl+V) jadi lampiran. Kalau clipboard juga berisi teks (misalnya
          // sel dari Excel atau paragraf dari Word), yang ditempel tetap teksnya.
          const files = Array.from(e.clipboardData.files);
          if (!files.length || e.clipboardData.getData("text/plain")) return;
          e.preventDefault();
          draft.add(files, true);
        }}
      />
      {(tooLong || draft.blocker || pageError) && (
        <p className="composer-error" role="alert">
          {tooLong ?? draft.blocker ?? pageError}
        </p>
      )}
      <div className="composer-bar">
        <button
          type="button"
          className="icon-btn attach-btn"
          aria-label="Lampirkan file"
          title="Lampirkan gambar atau file (PDF, Word, Excel, PowerPoint, teks). Bisa juga tempel tangkapan layar dengan Ctrl+V."
          onClick={() => pickerRef.current?.click()}
        >
          <PaperclipIcon size={20} />
        </button>
        <input
          ref={pickerRef}
          type="file"
          multiple
          accept={ACCEPT}
          hidden
          onChange={(e) => {
            draft.add(Array.from(e.target.files ?? []));
            e.target.value = "";
            inputRef.current?.focus();
          }}
        />
        <PageButton draft={draft} onError={setPageError} onDone={() => inputRef.current?.focus()} />
        <JokeToggle />
        <span className="bar-spacer" />
        <AuiIf condition={(s) => !s.thread.isRunning}>
          <button
            type="submit"
            className="send-btn"
            aria-label="Kirim"
            title={draft.uploading ? "Tunggu lampiran selesai diunggah" : undefined}
            disabled={!canSend}
          >
            <ArrowUpIcon size={18} weight="bold" />
          </button>
        </AuiIf>
        <AuiIf condition={(s) => s.thread.isRunning}>
          <ComposerPrimitive.Cancel className="send-btn" aria-label="Hentikan">
            <StopIcon size={14} weight="fill" />
          </ComposerPrimitive.Cancel>
        </AuiIf>
      </div>
    </ComposerPrimitive.Root>
  );
}

/** Pertanyaan lanjutan yang diketik di halaman masuk langsung mengisi kotak tanya. */
function FollowUpFromLogin() {
  const aui = useAui();
  useEffect(() => {
    const text = takeFollowUp();
    if (text) aui.composer.setText(text);
  }, [aui]);
  return null;
}

export function ChatView({
  firstName,
  conversationId,
  initialMessages,
  initialSummary,
  initialDraft,
  draftRef,
  onConversation,
  onDone,
  quotes,
  onQuotesChange,
}: QuoteProps & {
  firstName: string;
  conversationId: string | null;
  initialMessages: ThreadMessageLike[];
  initialSummary?: ConversationSummary | null;
  initialDraft?: DraftSnapshot;
  draftRef: Ref<() => DraftSnapshot>;
  onConversation: (id: string, title: string) => void;
  onDone: () => void;
}) {
  // Adapter dibuat sekali per percakapan. ID percakapan yang baru dibuat langsung
  // dipakai pesan berikutnya tanpa memasang ulang runtime (yang akan memutus
  // jawaban yang sedang mengalir). `onConversation` dan `onDone` harus stabil.
  const [summary, setSummary] = useState(initialSummary);
  const [questionIds, setQuestionIds] = useState(() => new Map<string, number>());
  const [adapter] = useState(() => {
    let id = conversationId;
    return createChatAdapter({
      getConversationId: () => id,
      onConversation: (newId, title) => {
        id = newId;
        onConversation(newId, title);
      },
      onSummary: setSummary,
      onQuestion: (clientId, serverId) => setQuestionIds((ids) => new Map(ids).set(clientId, serverId)),
      onDone,
    });
  });

  const runtime = useLocalRuntime(adapter, { initialMessages });
  const boundary = summary?.through_message_id;
  useEffect(() => {
    if (conversationId === null || boundary === undefined) return;
    const thread = runtime.thread;
    let active = true;
    let requested = false;
    const syncBoundary = () => {
      const { isRunning, messages } = thread.getState();
      if (requested || isRunning || messages.some((message) => serverMessageId(message, questionIds) === boundary)) return;
      requested = true;
      void api<ConversationDetail>(`/api/conversations/${encodeURIComponent(conversationId)}`).then((detail) => {
        const current = thread.getState();
        if (!active || current.isRunning) return;
        thread.reset(mergeThreadMessages(detail.messages, current.messages, questionIds));
        setSummary(detail.summary);
      }).catch(() => {});
    };
    const unsubscribe = thread.subscribe(syncBoundary);
    syncBoundary();
    return () => {
      active = false;
      unsubscribe();
    };
  }, [runtime, conversationId, boundary, questionIds]);
  // Satu draf lampiran untuk kedua kotak tanya (sambutan dan bawah percakapan).
  const draft = useAttachmentDraft();
  const composer = runtime.thread.composer;
  const restored = useRef(false);
  useImperativeHandle(draftRef, () => () => ({ text: composer.getState().text, hadAttachments: draft.items.length > 0 }),
    [composer, draft.items.length]);
  useEffect(() => {
    if (!initialDraft || restored.current) return;
    restored.current = true;
    composer.setText(initialDraft.text);
  }, [composer, initialDraft]);

  return (
    <AssistantRuntimeProvider runtime={runtime}>
      {conversationId === null && <FollowUpFromLogin />}
      <ThreadPrimitive.Root className="thread">
        <AuiIf condition={(s) => s.thread.isEmpty}>
          <div className="welcome">
            <div className="welcome-head">
              <img className="welcome-mascot" src="/brand/mascot.png" width={104} height={104} alt="" />
              <h1>Mau tanya apa, {firstName}?</h1>
            </div>
            <Composer quotes={quotes} onQuotesChange={onQuotesChange} draft={draft} />
            <p className="hint">Jawaban bisa keliru. Cek lagi angka dan keputusan penting.</p>
          </div>
        </AuiIf>
        <AuiIf condition={(s) => !s.thread.isEmpty}>
          <ThreadPrimitive.Viewport className="viewport">
            <div className="messages">
              <ThreadPrimitive.Messages>
                {({ message }) => {
                  const serverId = serverMessageId(message, questionIds);
                  return (
                    <>
                      {message.role === "user" ? <UserMessage /> : <AssistantMessage />}
                      {summary && serverId === summary.through_message_id && (
                        <SummaryDivider key={summary.through_message_id} content={summary.content} />
                      )}
                    </>
                  );
                }}
              </ThreadPrimitive.Messages>
            </div>
            <ThreadPrimitive.ViewportFooter className="footer">
              <Composer quotes={quotes} onQuotesChange={onQuotesChange} draft={draft} />
              <p className="hint">Jawaban bisa keliru. Cek lagi angka dan keputusan penting.</p>
            </ThreadPrimitive.ViewportFooter>
          </ThreadPrimitive.Viewport>
        </AuiIf>
      </ThreadPrimitive.Root>
    </AssistantRuntimeProvider>
  );
}
