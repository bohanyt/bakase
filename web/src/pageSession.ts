import type { ActionReply, PageAction } from "./pageAction.js";

type Page = { id: string; origin: string; ambilTabel?: true; attachmentId?: string; refresh?: { text: string } };
export type LivePage = Page & { attachmentId: string };

export function pageChanged(data: unknown, id: string): boolean {
  if (!data || typeof data !== "object") return false;
  const msg = data as Record<string, unknown>;
  return msg.type === "app:halaman-berubah" && msg.pageId === id;
}

/** Hanya menyimpan halaman yang masih terhubung ke panel dan lampiran di chat. */
export function createPageSession() {
  let latest: Page | null = null;

  function forget(id?: string) {
    if (!latest || (id !== undefined && latest.id !== id)) return false;
    latest = null;
    return true;
  }

  return {
    remember(id: string, origin: string, ambilTabel?: true) {
      latest = { id, origin, ...(ambilTabel ? { ambilTabel } : {}) };
    },
    forget,
    changed(origin: string, data: unknown) {
      if (latest?.origin === origin && pageChanged(data, latest.id)) forget(latest.id);
    },
    bind(id: string, attachmentId: string) {
      if (latest?.id === id) latest.attachmentId = attachmentId;
    },
    live(attachmentIds: ReadonlySet<string>): LivePage | null {
      const attachmentId = latest?.attachmentId;
      return latest && attachmentId && attachmentIds.has(attachmentId) ? { ...latest, attachmentId } : null;
    },
    consumed(page: LivePage) {
      if (latest?.id === page.id && latest.refresh === page.refresh) latest.refresh = undefined;
    },
    async answer(
      page: LivePage,
      action: PageAction,
      signal: AbortSignal,
      ask: (page: LivePage, action: PageAction, signal: AbortSignal) => Promise<ActionReply>,
      post: (id: string, reply: ActionReply, signal: AbortSignal) => Promise<void>,
    ) {
      if (signal.aborted) return;
      let reply: ActionReply = latest?.id === page.id
        ? await ask(page, action, signal)
        : { status: "unavailable" };
      if (signal.aborted) {
        forget(page.id);
        return;
      }
      if (reply.status === "scrolled" || reply.status === "highlighted" || reply.status === "table") {
        if (latest?.id !== page.id) reply = { status: "unavailable" };
        else if (reply.status === "scrolled") latest.refresh = { text: reply.text };
      } else if (reply.status !== "stale_ref") {
        forget(page.id);
      }
      try {
        await post(action.id, reply, signal);
      } catch (err) {
        forget(page.id);
        throw err;
      }
    },
  };
}
