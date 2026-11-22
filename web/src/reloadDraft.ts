export type DraftSnapshot = { text: string; hadAttachments: boolean };
type ReloadDraft = DraftSnapshot & { conversationId: string | null };

const key = (userId: number) => `app.reloadDraft.${userId}`;

export function saveReloadDraft(userId: number, draft: ReloadDraft): boolean {
  try {
    window.sessionStorage.setItem(key(userId), JSON.stringify(draft));
    return true;
  } catch {
    return false;
  }
}

export function takeReloadDraft(userId: number): ReloadDraft | null {
  try {
    const raw = window.sessionStorage.getItem(key(userId));
    if (!raw) return null;
    window.sessionStorage.removeItem(key(userId));
    const draft = JSON.parse(raw) as Partial<ReloadDraft> | null;
    if (typeof draft?.text !== "string" || typeof draft.hadAttachments !== "boolean") return null;
    if (draft.conversationId !== null && typeof draft.conversationId !== "string") return null;
    return { conversationId: draft.conversationId, text: draft.text, hadAttachments: draft.hadAttachments };
  } catch {
    return null;
  }
}
