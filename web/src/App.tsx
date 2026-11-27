import type { ThreadMessageLike } from "@assistant-ui/react";
import { ArrowLeftIcon, ListIcon } from "@phosphor-icons/react";
import { useCallback, useEffect, useEffectEvent, useRef, useState, lazy, Suspense } from "react";
import type { AdminTab } from "./AdminPage";
import { api, ApiError, SESSION_CHECK_EVENT, setTabUser, UNAUTHORIZED_EVENT, type AppCard, type AuthOptions, type User } from "./api";
import { AuthScreen } from "./AuthScreen";
import { microsoftParams } from "./microsoftLogin";
import { ChangePassword } from "./ChangePassword";
import { ChatView } from "./ChatView";
import { CiteContext } from "./citeContext";
import { AppsPage } from "./AppsPage";
import { DocumentsPage } from "./DocumentsPage";
import { FilesPage } from "./FilesPage";
import { DocumentPreview } from "./DocumentPreview";
import { FilePreview } from "./FilePreview";
import { ExtensionDialog, ExtensionUpdateNotice, HelpMenu } from "./HelpMenu";
import { extensionUpdateAvailable } from "./extensionVersion";
import { toThreadMessages, type ConversationDetail, type ConversationSummary } from "./messages";
import { inPanel } from "./pageBridge";
import { PreviewContext, ReloadAppContext, previewKey, type PreviewTarget } from "./previewContext";
import { newQuote, quoteError, type Quote, type QuoteDraft } from "./quote";
import { SelectionCite } from "./Quotes";
import { saveReloadDraft, takeReloadDraft, type DraftSnapshot } from "./reloadDraft";
import { SearchDialog } from "./SearchDialog";
import { SourcePreview } from "./SourcePreview";
import { Sidebar, type Conversation, type SidePage } from "./Sidebar";
import { ThemeToggle } from "./ThemeToggle";
import "./App.css";

type AdminPageModule = typeof import("./AdminPage");

/** Tab yang masih terbuka sesudah deploy bisa meminta berkas yang sudah tidak ada. */
const AdminPage = lazy(() =>
  import("./AdminPage").catch(() => ({ default: AdminLoadError as AdminPageModule["default"] })),
);

function AdminLoadError() {
  return (
    <div className="loading" role="alert">
      <div>
        <p>Halaman Admin gagal dimuat.</p>
        <button type="button" onClick={() => window.location.reload()}>
          Muat ulang
        </button>
      </div>
    </div>
  );
}

const SESSION_CHANNEL = "app-sesi";
const RETRY_EVENT = "app:coba-lagi";
/** Jarak terpendek antar pemeriksaan sesi saat tab kembali dibuka. */
const VISIBLE_CHECK_MS = 60_000;

type View = "chat" | "admin" | "password" | SidePage;
type Session = {
  key: string;
  conversationId: string | null;
  initialMessages: ThreadMessageLike[];
  initialSummary?: ConversationSummary | null;
  initialDraft?: DraftSnapshot;
};

const newSession = (): Session => ({ key: `baru-${Date.now()}`, conversationId: null, initialMessages: [] });
const READD_ATTACHMENTS = "Tambahkan kembali lampiran draf setelah memuat ulang aplikasi.";

const MIN_WIDTH = 200;
const MAX_WIDTH = 480;

function readWidth() {
  try {
    const w = Number(localStorage.getItem("app.sidebarWidth"));
    return w >= MIN_WIDTH && w <= MAX_WIDTH ? w : 260;
  } catch {
    return 260;
  }
}

const PREVIEW_MIN = 320;
/** Ruang minimal untuk chat saat panel preview dilebarkan. */
const CHAT_MIN = 380;

/** Lebar panel preview yang diingat, atau null untuk lebar bawaan dari CSS. */
function readPreviewWidth(): number | null {
  try {
    const w = Number(localStorage.getItem("app.previewWidth"));
    return w >= PREVIEW_MIN ? w : null;
  } catch {
    return null;
  }
}

function readCollapsed() {
  try {
    return localStorage.getItem("app.sidebar") === "collapsed";
  } catch {
    return false;
  }
}

function messageElement(messageId: number) {
  return document.querySelector<HTMLElement>(`.chat-pane [data-server-id="${messageId}"]`);
}

function Shell({ user, authOptions, initialNotice, onLogout }: {
  user: User;
  initialNotice: string | null;
  authOptions: AuthOptions | null | undefined;
  onLogout: () => Promise<void>;
}) {
  const [view, setView] = useState<View>("chat");
  const [adminTab, setAdminTab] = useState<AdminTab>("users");
  // File atau potongan dokumen yang sedang dibuka di panel preview kanan.
  const [preview, setPreview] = useState<PreviewTarget | null>(null);
  const [previewWidth, setPreviewWidth] = useState(readPreviewWidth);
  const [viewport, setViewport] = useState(() => window.innerWidth);
  // Pesan yang perlu disorot setelah percakapan terbuka (dari Admin → Feedback).
  const [focus, setFocus] = useState<{ messageId: number; seq: number } | null>(null);
  const focusSeq = useRef(0);
  const focusDone = useRef(0);
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [session, setSession] = useState<Session>(newSession);
  const draftRef = useRef<(() => DraftSnapshot) | null>(null);
  const [loadingConversation, setLoadingConversation] = useState(false);
  const [collapsed, setCollapsed] = useState(readCollapsed);
  const [width, setWidth] = useState(readWidth);
  const [installing, setInstalling] = useState(false);
  const extensionVersion = authOptions?.extension_version;
  const panel = inPanel();
  const canUpdateExtension = panel && extensionVersion &&
    extensionUpdateAvailable(new URLSearchParams(location.search).get("v"), extensionVersion);
  const openExtensionDialog = () => setInstalling(true);
  // Kutipan yang menunggu dikirim; ikut kosong saat pindah percakapan, seperti isi kotak tanya.
  const [quotes, setQuotes] = useState<Quote[]>([]);

  const [drawer, setDrawer] = useState(false);
  // Aplikasi yang boleh dipakai user ini; menu Aplikasi hanya tampil kalau ada.
  const [apps, setApps] = useState<AppCard[]>([]);
  useEffect(() => {
    let active = true;
    api<AppCard[]>("/api/apps")
      .then((list) => active && setApps(list))
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [user.id]);
  const [searchOpen, setSearchOpen] = useState(false);
  const [notice, setNotice] = useState(initialNotice);
  const [error, setError] = useState<string | null>(null);
  // Nomor urut navigasi: respons yang datang setelah user memilih hal lain diabaikan.
  const navSeq = useRef(0);
  // Percakapan aktif terbaru, untuk dicek setelah await (state di closure bisa usang).
  const activeRef = useRef<string | null>(null);
  // Percakapan yang sedang dimuat, supaya bisa dibatalkan kalau dihapus sebelum selesai.
  const loadingRef = useRef<string | null>(null);

  useEffect(() => {
    const onResize = () => setViewport(window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  const reload = useCallback(
    () => api<Conversation[]>("/api/conversations").then(setConversations, () => {}),
    [],
  );

  useEffect(() => {
    api<Conversation[]>("/api/conversations").then(setConversations, () => {});
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setSearchOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  function toggleSidebar() {
    setCollapsed((c) => {
      try {
        localStorage.setItem("app.sidebar", c ? "open" : "collapsed");
      } catch {
        /* penyimpanan browser tidak tersedia */
      }
      return !c;
    });
  }

  // Pembatas sidebar bisa ditarik; lebarnya diingat di browser ini.
  function startResize(e: React.PointerEvent<HTMLDivElement>) {
    e.preventDefault();
    const handle = e.currentTarget;
    handle.setPointerCapture(e.pointerId);
    let latest = width;
    const move = (ev: PointerEvent) => {
      latest = Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, ev.clientX));
      setWidth(latest);
    };
    const up = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
      document.body.classList.remove("resizing");
      try {
        localStorage.setItem("app.sidebarWidth", String(latest));
      } catch {
        /* penyimpanan browser tidak tersedia */
      }
    };
    document.body.classList.add("resizing");
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
  }

  // Pembatas chat dan preview juga bisa ditarik; lebarnya dihitung dari tepi kanan jendela.
  function startPreviewResize(e: React.PointerEvent<HTMLDivElement>) {
    e.preventDefault();
    const handle = e.currentTarget;
    handle.setPointerCapture(e.pointerId);
    let latest = previewWidth ?? PREVIEW_MIN;
    const move = (ev: PointerEvent) => {
      const sidebar = document.querySelector(".sidebar")?.getBoundingClientRect().width ?? 0;
      const max = Math.max(PREVIEW_MIN, window.innerWidth - sidebar - CHAT_MIN);
      latest = Math.min(max, Math.max(PREVIEW_MIN, window.innerWidth - ev.clientX));
      setPreviewWidth(latest);
    };
    const up = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
      document.body.classList.remove("resizing");
      try {
        localStorage.setItem("app.previewWidth", String(Math.round(latest)));
      } catch {
        /* penyimpanan browser tidak tersedia */
      }
    };
    document.body.classList.add("resizing");
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
  }

  function resetPreviewWidth() {
    setPreviewWidth(null);
    try {
      localStorage.removeItem("app.previewWidth");
    } catch {
      /* penyimpanan browser tidak tersedia */
    }
  }

  /** Membatalkan percakapan yang sedang dimuat; chat yang tampil tidak berubah. */
  function cancelLoading() {
    navSeq.current += 1;
    loadingRef.current = null;
    setLoadingConversation(false);
  }

  function newChat() {
    cancelLoading();
    setFocus(null);
    setView("chat");
    activeRef.current = null;
    setActiveId(null);
    setSession(newSession());
    setQuotes([]);
    setDrawer(false);
  }

  async function openConversation(id: string, messageId?: number, initialDraft?: DraftSnapshot) {
    setView("chat");
    setDrawer(false);
    setSearchOpen(false);
    setFocus(messageId === undefined ? null : { messageId, seq: ++focusSeq.current });
    // Chat yang sudah terbuka tidak dimuat ulang, supaya jawaban yang sedang mengalir
    // tidak putus. Dimuat ulang hanya kalau pesannya belum ada di layar, misalnya
    // jawaban itu dibuat dari tab lain.
    if (id === activeRef.current && (messageId === undefined || messageElement(messageId))) {
      cancelLoading();
      return;
    }
    const seq = ++navSeq.current;
    loadingRef.current = id;
    setError(null);
    setLoadingConversation(true);
    try {
      const detail = await api<ConversationDetail>(`/api/conversations/${encodeURIComponent(id)}`);
      if (seq !== navSeq.current) {
        if (initialDraft) saveReloadDraft(user.id, { ...initialDraft, conversationId: id });
        return;
      }
      // Chat lama baru diganti setelah percakapan baru berhasil dimuat.
      activeRef.current = id;
      setActiveId(id);
      setSession({
        key: `${id}:${seq}`,
        conversationId: id,
        initialMessages: toThreadMessages(detail.messages),
        initialSummary: detail.summary,
        initialDraft,
      });
      if (initialDraft?.hadAttachments) setNotice(READD_ATTACHMENTS);
      setQuotes([]);
    } catch (err) {
      if (initialDraft) saveReloadDraft(user.id, { ...initialDraft, conversationId: id });
      if (seq === navSeq.current) setError(err instanceof Error ? err.message : "Gagal membuka percakapan.");
    } finally {
      if (seq === navSeq.current) {
        loadingRef.current = null;
        setLoadingConversation(false);
      }
    }
  }

  const restoreDraft = useEffectEvent(() => {
    const draft = takeReloadDraft(user.id);
    if (!draft) return;
    if (draft.conversationId !== null) {
      void openConversation(draft.conversationId, undefined, draft);
    } else {
      setSession((current) => ({ ...current, initialDraft: draft }));
      if (draft.hadAttachments) setNotice(READD_ATTACHMENTS);
    }
  });
  useEffect(() => {
    let cancelled = false;
    queueMicrotask(() => { if (!cancelled) restoreDraft(); });
    return () => { cancelled = true; };
  }, [user.id]);

  // Sesi berakhir di tengah mengetik: draf dikembalikan setelah akun yang sama masuk lagi.
  const keepDraft = useEffectEvent(() => {
    const draft = draftRef.current?.();
    if (draft?.text || draft?.hadAttachments) {
      saveReloadDraft(user.id, { ...draft, conversationId: session.conversationId });
    }
  });
  useEffect(() => {
    const onUnauthorized = () => keepDraft();
    window.addEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
    return () => window.removeEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
  }, []);

  function reloadApp() {
    const draft = draftRef.current?.();
    if ((draft?.text || draft?.hadAttachments) && !saveReloadDraft(user.id, { ...draft, conversationId: session.conversationId })) {
      setNotice("Draf belum bisa disimpan. Salin teksnya sebelum memuat ulang aplikasi.");
      return;
    }
    window.location.reload();
  }

  // Gulir ke pesan yang dituju dan sorot sebentar, sekali per permintaan.
  useEffect(() => {
    if (!focus || view !== "chat" || loadingConversation || focusDone.current === focus.seq) return;
    let tries = 0;
    let frame = 0;
    const timers: number[] = [];
    let flashed: HTMLElement | null = null;
    const find = () => {
      const el = messageElement(focus.messageId);
      if (!el) {
        if (++tries < 60) frame = requestAnimationFrame(find);
        return;
      }
      focusDone.current = focus.seq;
      el.scrollIntoView({ block: "center" });
      // Diulang sekali setelah isi jawaban (tabel, kode) selesai digambar.
      timers.push(window.setTimeout(() => el.scrollIntoView({ block: "center" }), 300));
      // Lepas dulu lalu pasang lagi, supaya sorotan mulai dari awal walau pesannya sama.
      el.classList.remove("flash");
      void el.offsetWidth;
      el.classList.add("flash");
      flashed = el;
      timers.push(window.setTimeout(() => el.classList.remove("flash"), 2400));
    };
    frame = requestAnimationFrame(find);
    // Permintaan baru membatalkan guliran dan sorotan dari permintaan sebelumnya.
    return () => {
      cancelAnimationFrame(frame);
      timers.forEach((t) => window.clearTimeout(t));
      flashed?.classList.remove("flash");
    };
  }, [focus, view, loadingConversation, session.key]);

  async function rename(id: string, title: string) {
    try {
      await api(`/api/conversations/${id}`, { body: { title } });
      await reload();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Gagal mengganti nama.");
    }
  }

  async function remove(id: string) {
    try {
      await api(`/api/conversations/${id}/archive-v1`, { method: "DELETE" });
      if (id === activeRef.current) newChat();
      else if (id === loadingRef.current) cancelLoading();
      await reload();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Gagal menghapus percakapan.");
    }
  }

  const onConversation = useCallback((id: string, title: string) => {
    activeRef.current = id;
    setActiveId(id);
    // Simpan ID-nya juga di sesi, supaya chat yang dipasang ulang tetap melanjutkan percakapan ini.
    setSession((s) => (s.conversationId ? s : { ...s, conversationId: id }));
    setConversations((list) =>
      list.some((c) => c.id === id) ? list : [{ id, title, updated_at: Math.floor(Date.now() / 1000) }, ...list],
    );
  }, []);

  async function logout() {
    setError(null);
    try {
      await onLogout();
    } catch {
      setError("Gagal keluar karena server tidak terhubung. Periksa koneksi lalu coba lagi.");
    }
  }

  const firstName = user.name.split(/\s+/)[0] ?? user.name;
  // Kalau ruang tidak cukup untuk sidebar, chat, dan preview, preview tampil penuh di atas chat.
  const sidebarPx = collapsed && !drawer ? 52 : width;
  const previewOverlay = viewport - sidebarPx < CHAT_MIN + PREVIEW_MIN;

  // Preview yang menutupi chat ditutup setelah mengutip, supaya kutipannya langsung terlihat.
  const addQuote = useCallback(
    (draft: QuoteDraft) => {
      const error = quoteError(draft);
      if (error) return error;
      setQuotes((list) =>
        list.some((q) => q.text === draft.text && q.source === draft.source) ? list : [...list, newQuote(draft)],
      );
      setView("chat");
      if (previewOverlay) setPreview(null);
      return null;
    },
    [previewOverlay],
  );

  return (
    <CiteContext.Provider value={addQuote}>
    <div
      className={`app${drawer ? " drawer-open" : ""}`}
      style={{ "--sidebar-width": `${collapsed && !drawer ? 52 : width}px` } as React.CSSProperties}
    >
      <Sidebar
        user={user}
        conversations={conversations}
        activeId={view === "chat" ? activeId : null}
        collapsed={collapsed && !drawer}
        width={width}
        onToggle={drawer ? () => setDrawer(false) : toggleSidebar}
        onNewChat={newChat}
        onSearch={() => {
          setDrawer(false);
          setSearchOpen(true);
        }}
        onSelect={openConversation}
        onRename={rename}
        onDelete={remove}
        page={view === "documents" || view === "files" || view === "apps" ? view : null}
        onPage={(page) => (setView(page), setDrawer(false))}
        showApps={apps.length > 0}
        onAdmin={() => (setView("admin"), setDrawer(false))}
        onPassword={() => (setView("password"), setDrawer(false))}
        onLogout={logout}
      />
      {!collapsed && (
        <div
          className="resizer"
          role="separator"
          aria-orientation="vertical"
          aria-label="Ubah lebar sidebar"
          onPointerDown={startResize}
          onDoubleClick={() => {
            setWidth(260);
            try {
              localStorage.removeItem("app.sidebarWidth");
            } catch {
              /* penyimpanan browser tidak tersedia */
            }
          }}
        />
      )}
      {drawer && <div className="scrim" onClick={() => setDrawer(false)} />}

      <PreviewContext.Provider value={setPreview}>
      <main className="main">
        <header className="main-head">
          <button type="button" className="icon-btn mobile-only" aria-label="Buka menu" onClick={() => setDrawer(true)}>
            <ListIcon size={20} />
          </button>
          {view !== "chat" && (
            <button type="button" className="text-btn back" onClick={() => setView("chat")}>
              <ArrowLeftIcon size={16} /> Kembali ke chat
            </button>
          )}
          <span className="head-spacer" />
          <HelpMenu
            whatsapp={authOptions?.support_whatsapp}
            canInstall={!!extensionVersion && !panel && viewport > 800}
            onInstall={openExtensionDialog}
          />
          <ThemeToggle />
        </header>

        {extensionVersion && installing && (
          <ExtensionDialog version={extensionVersion} onClose={() => setInstalling(false)} />
        )}
        {canUpdateExtension && (
          <ExtensionUpdateNotice key={extensionVersion} version={extensionVersion} onUpdate={openExtensionDialog} />
        )}
        {user.role === "admin" && user.microsoft_secret_days !== undefined && (
          <p className="ms-callout ms-warning banner" role="status">
            <strong>Pengingat admin</strong><br />
            {user.microsoft_secret_days < 0 ? "Kunci masuk Microsoft sudah kedaluwarsa."
              : user.microsoft_secret_days === 0 ? "Kunci masuk Microsoft kedaluwarsa hari ini."
              : "Kunci masuk Microsoft kedaluwarsa dalam " + user.microsoft_secret_days + " hari."}
            {" Perbarui agar akses tetap berjalan."}
          </p>
        )}
        {error && (
          <p className="form-error banner" role="alert">
            {error}
            <button type="button" className="link" onClick={() => setError(null)}>
              Tutup
            </button>
          </p>
        )}
        {notice && (
          <p className="notice banner" role="status">
            {notice}
            <button type="button" className="link" onClick={() => setNotice(null)}>
              Tutup
            </button>
          </p>
        )}

        {/* Chat tetap terpasang saat membuka halaman lain supaya jawaban yang sedang berjalan tidak putus. */}
        {/* Selama memuat, chat lama tetap terpasang di bawah lapisan; kalau gagal, tidak ada yang hilang. */}
        <div className="chat-pane" hidden={view !== "chat"} aria-busy={loadingConversation}>
          <ChatView
            key={session.key}
            firstName={firstName}
            conversationId={session.conversationId}
            initialMessages={session.initialMessages}
            initialSummary={session.initialSummary}
            initialDraft={session.initialDraft}
            draftRef={draftRef}
            onConversation={onConversation}
            onDone={reload}
            quotes={quotes}
            onQuotesChange={setQuotes}
          />
          {loadingConversation && <div className="loading chat-loading">Memuat percakapan…</div>}
        </div>
        {view === "admin" && user.role === "admin" && (
          <div className="page-pane">
            <Suspense fallback={<div className="loading">Memuat Admin…</div>}>
              <AdminPage me={user} tab={adminTab} onTab={setAdminTab} onOpenChat={openConversation} />
            </Suspense>
          </div>
        )}
        {view === "documents" && (
          <div className="page-pane">
            <DocumentsPage
              openId={preview?.type === "document" ? preview.document.id : null}
              onOpen={(document) => setPreview({ type: "document", document })}
            />
          </div>
        )}
        {view === "files" && (
          <div className="page-pane">
            <FilesPage
              openId={preview?.type === "file" ? preview.file.id : null}
              onOpen={(file) => setPreview({ type: "file", file })}
              onOpenChat={openConversation}
            />
          </div>
        )}
        {view === "apps" && apps.length > 0 && (
          <div className="page-pane">
            <AppsPage apps={apps} />
          </div>
        )}
        {view === "password" && (
          <div className="page-pane">
            <ChangePassword
              onDone={() => {
                setNotice("Password sudah diganti.");
                setView("chat");
              }}
            />
          </div>
        )}
      </main>
      </PreviewContext.Provider>
      {preview && (
        <ReloadAppContext.Provider value={reloadApp}>
          {!previewOverlay && (
            <div
              className="resizer preview-resizer"
              role="separator"
              aria-orientation="vertical"
              aria-label="Ubah lebar preview"
              onPointerDown={startPreviewResize}
              onDoubleClick={resetPreviewWidth}
            />
          )}
          {preview.type === "file" ? (
            <FilePreview
              key={previewKey(preview)}
              file={preview.file}
              width={previewWidth}
              overlay={previewOverlay}
              onClose={() => setPreview(null)}
            />
          ) : preview.type === "source" ? (
            <SourcePreview
              key={previewKey(preview)}
              source={preview.source}
              originalPage={preview.originalPage}
              width={previewWidth}
              overlay={previewOverlay}
              onClose={() => setPreview(null)}
            />
          ) : (
            <DocumentPreview
              key={previewKey(preview)}
              document={preview.document}
              width={previewWidth}
              overlay={previewOverlay}
              onClose={() => setPreview(null)}
            />
          )}
        </ReloadAppContext.Provider>
      )}

      {searchOpen && (
        <SearchDialog
          onOpen={openConversation}
          onOpenSource={(source) => {
            setSearchOpen(false);
            setPreview({ type: "source", source });
          }}
          onClose={() => setSearchOpen(false)}
        />
      )}
      <SelectionCite />
    </div>
    </CiteContext.Provider>
  );
}

export default function App() {
  const [user, setUser] = useState<User | null | undefined>(undefined);
  const [authOptions, setAuthOptions] = useState<AuthOptions | null>();

  useEffect(() => {
    let active = true;
    let pending = false;
    async function loadOptions() {
      if (pending) return;
      pending = true;
      try {
        const options = await api<AuthOptions>("/api/auth/options");
        if (active) setAuthOptions(options);
        window.removeEventListener("online", loadOptions);
      } catch {
        if (active) setAuthOptions(null);
      } finally {
        pending = false;
      }
    }
    window.addEventListener("online", loadOptions);
    void loadOptions();
    return () => {
      active = false;
      window.removeEventListener("online", loadOptions);
    };
  }, []);

  const shown = useRef<User | null | undefined>(undefined);
  const [unreachable, setUnreachable] = useState(false);
  const [ended, setEnded] = useState(false);
  const [switched, setSwitched] = useState(false);

  const show = useCallback((next: User | null) => {
    const before = shown.current;
    shown.current = next;
    setTabUser(next?.id ?? null);
    setUnreachable(false);
    if (next) setEnded(false);
    setSwitched(!!before && !!next && before.id !== next.id);
    // Objek lama dipertahankan kalau orangnya sama, supaya Shell tidak dirender ulang.
    setUser((current) => (current && next && JSON.stringify(current) === JSON.stringify(next) ? current : next));
  }, []);

  useEffect(() => {
    let active = true;
    let pending = false;
    let again = false;
    let checkedAt = 0;
    async function check() {
      // Balasan yang sedang ditunggu bisa masih tentang akun lama, jadi dicek sekali lagi.
      again = pending;
      if (pending) return;
      pending = true;
      checkedAt = Date.now();
      try {
        const me = await api<User>("/api/me");
        if (active) show(me);
      } catch (err) {
        // 401 ditangani onUnauthorized. Gangguan lain tidak boleh mengeluarkan user.
        const unauthorized = err instanceof ApiError && err.status === 401;
        if (active && !unauthorized && shown.current === undefined) setUnreachable(true);
      } finally {
        pending = false;
        if (again && active) void check();
      }
    }
    const onUnauthorized = () => {
      if (shown.current) setEnded(true);
      show(null);
    };
    const onVisible = () => {
      if (document.visibilityState === "visible" && Date.now() - checkedAt > VISIBLE_CHECK_MS) void check();
    };
    const onSession = (e: MessageEvent<number | null>) => {
      if (e.data !== (shown.current?.id ?? null)) void check();
    };
    const channel = typeof BroadcastChannel === "undefined" ? null : new BroadcastChannel(SESSION_CHANNEL);
    channel?.addEventListener("message", onSession);
    window.addEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
    window.addEventListener(SESSION_CHECK_EVENT, check);
    window.addEventListener(RETRY_EVENT, check);
    window.addEventListener("online", check);
    document.addEventListener("visibilitychange", onVisible);
    void check();
    return () => {
      active = false;
      channel?.close();
      window.removeEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
      window.removeEventListener(SESSION_CHECK_EVENT, check);
      window.removeEventListener(RETRY_EVENT, check);
      window.removeEventListener("online", check);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [show]);

  // Tab lain memeriksa sesinya begitu akun di tab ini berganti.
  const userId = user === undefined ? undefined : (user?.id ?? null);
  useEffect(() => {
    if (userId === undefined || typeof BroadcastChannel === "undefined") return;
    const channel = new BroadcastChannel(SESSION_CHANNEL);
    channel.postMessage(userId);
    channel.close();
  }, [userId]);

  if (user === undefined) {
    return unreachable ? (
      <div className="loading full" role="alert">
        <div>
          <p>Bakase tidak bisa dihubungi. Sambungkan ke jaringan kantor atau VPN, lalu coba lagi.</p>
          <button type="button" className="link" onClick={() => window.dispatchEvent(new Event(RETRY_EVENT))}>
            Coba lagi
          </button>
        </div>
      </div>
    ) : (
      <div className="loading full">Memuat…</div>
    );
  }
  const { tab, result } = microsoftParams();
  if (user === null || tab) {
    return (
      <AuthScreen
        options={authOptions}
        onLogin={show}
        note={ended ? "Sesi berakhir. Masuk lagi untuk melanjutkan." : undefined}
        panelTabReady={tab && result === "done" && !!user}
      />
    );
  }

  // Shell dipasang ulang per user, jadi percakapan tidak terbawa ke akun lain.
  return (
    <Shell
      key={user.id}
      user={user}
      initialNotice={switched ? `Akun di tab ini berganti ke ${user.name}.` : null}
      authOptions={authOptions}
      onLogout={async () => {
        // Layar masuk hanya tampil setelah server benar-benar menghapus sesi.
        await api("/api/auth/logout", { body: {} });
        show(null);
      }}
    />
  );
}
