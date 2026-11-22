const REF = /^(?:f[1-9][0-9]{0,9})?e[1-9][0-9]{0,15}$/;
const MAX_TEXT = 24_000;

export type PageAction = { id: string; remaining_ms: number } & (
  | { action: "sorot"; ref: string }
  | { action: "gulir"; ref: string | null; direction: "up" | "down" | "to" }
);

export type ActionReply =
  | { status: "highlighted" | "stale_ref" | "unavailable" | "failed" | "timeout" }
  | { status: "scrolled"; text: string; moved: boolean };

export function readPageAction(data: unknown): PageAction | null {
  if (!data || typeof data !== "object") return null;
  const { id, remaining_ms, action, ref, direction } = data as Record<string, unknown>;
  if (typeof id !== "string" || !/^[a-f0-9]{32}$/.test(id)) return null;
  if (typeof remaining_ms !== "number" || !Number.isSafeInteger(remaining_ms) || remaining_ms < 0) return null;
  const validRef = typeof ref === "string" && REF.test(ref);
  if (action === "sorot" && validRef) return { id, remaining_ms, action, ref };
  if (action === "gulir" && (validRef || ref === null) &&
      (direction === "up" || direction === "down" || (direction === "to" && validRef))) {
    return { id, remaining_ms, action, ref, direction };
  }
  return null;
}

export function pageActionTiming(remainingMs: number, clock: Pick<typeof performance, "now"> = performance) {
  const timeout = Math.min(remainingMs, 8000);
  return { timeout, deadline: clock.now() + timeout };
}

export function limitPageText(text: string): string {
  const bytes = new TextEncoder().encode(text);
  if (bytes.length <= MAX_TEXT) return text;
  const note = "\n[Terpotong: snapshot melewati batas baca.]";
  const head = new TextDecoder().decode(bytes.subarray(0, MAX_TEXT - note.length - 3));
  const line = head.lastIndexOf("\n");
  return (line > 0 ? head.slice(0, line) : head) + note;
}

export function readActionReply(data: unknown, id: string): ActionReply | null {
  if (!data || typeof data !== "object") return null;
  const msg = data as Record<string, unknown>;
  if (msg.type !== "app:hasil-aksi" || msg.id !== id) return null;
  switch (msg.status) {
    case "unavailable":
    case "stale_ref":
    case "highlighted":
    case "failed":
    case "timeout":
      return { status: msg.status };
    case "scrolled":
      if (typeof msg.text === "string" && typeof msg.moved === "boolean") {
        return { status: "scrolled", text: limitPageText(msg.text), moved: msg.moved };
      }
  }
  return { status: "failed" };
}
