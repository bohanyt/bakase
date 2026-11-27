const REF = /^(?:f[1-9][0-9]{0,9})?e[1-9][0-9]{0,15}$/;
const MAX_TEXT = 24_000;
const TABLE_TIMEOUT = 62_000;
const MAX_TABLE_ROWS = 5_000;
const MAX_TABLE_COLUMNS = 100;
const MAX_TABLE_BYTES = 2_000_000;
const MAX_CELL_BYTES = 8_000;
const TABLE_ISSUES = [
  "row_limit", "column_limit", "size_limit", "cell_limit", "time_limit",
  "unknown_rows", "missing_rows", "missing_columns", "changed",
] as const;

type Table = {
  title: string;
  columns: string[];
  rows: string[][];
  issues: typeof TABLE_ISSUES[number][];
};

export type PageAction = { id: string; remaining_ms: number } & (
  | { action: "sorot" | "ambil_tabel"; ref: string }
  | { action: "gulir"; ref: string | null; direction: "up" | "down" | "to" }
);

export type ActionReply =
  | { status: "highlighted" | "stale_ref" | "unavailable" | "failed" | "timeout" }
  | { status: "scrolled"; text: string; moved: boolean }
  | { status: "table"; table: Table };

export function readPageAction(data: unknown): PageAction | null {
  if (!data || typeof data !== "object") return null;
  const { id, remaining_ms, action, ref, direction } = data as Record<string, unknown>;
  if (typeof id !== "string" || !/^[a-f0-9]{32}$/.test(id)) return null;
  if (typeof remaining_ms !== "number" || !Number.isSafeInteger(remaining_ms) || remaining_ms < 0) return null;
  const validRef = typeof ref === "string" && REF.test(ref);
  if ((action === "sorot" || action === "ambil_tabel") && validRef && direction === undefined) {
    return { id, remaining_ms, action, ref };
  }
  if (action === "gulir" && (validRef || ref === null) &&
      (direction === "up" || direction === "down" || (direction === "to" && validRef))) {
    return { id, remaining_ms, action, ref, direction };
  }
  return null;
}

export function pageActionTiming(action: PageAction, clock: Pick<typeof performance, "now"> = performance) {
  const timeout = Math.min(action.remaining_ms, action.action === "ambil_tabel" ? TABLE_TIMEOUT : 8000);
  return { timeout, deadline: clock.now() + timeout };
}

function readTable(value: unknown): Table | null {
  if (!value || typeof value !== "object") return null;
  const { title, columns, rows, issues } = value as Record<string, unknown>;
  const encoder = new TextEncoder();
  const fits = (value: unknown, max: number): value is string =>
    typeof value === "string" && value.length <= max && encoder.encode(value).length <= max;
  const isIssue = (v: unknown): v is Table["issues"][number] => TABLE_ISSUES.some((issue) => issue === v);
  if (!fits(title, 400) || !Array.isArray(columns) || !columns.length || columns.length > MAX_TABLE_COLUMNS ||
      !Array.isArray(rows) || rows.length > MAX_TABLE_ROWS ||
      !Array.isArray(issues) || issues.length > TABLE_ISSUES.length || !issues.every(isIssue)) return null;
  const wellFormed = (text: string) => !/[\uD800-\uDFFF]/u.test(text);
  const cell = (value: unknown): value is string => fits(value, MAX_CELL_BYTES) && wellFormed(value);
  if (!wellFormed(title) || !columns.every(cell) ||
      !rows.every((row) => Array.isArray(row) && row.length === columns.length && row.every(cell))) return null;
  const table = { title, columns, rows, issues };
  return encoder.encode(JSON.stringify(table)).length <= MAX_TABLE_BYTES ? table : null;
}

export function readTableProgress(data: unknown, id: string): number | null {
  if (!data || typeof data !== "object") return null;
  const msg = data as Record<string, unknown>;
  return msg.type === "app:progres" && msg.id === id && typeof msg.rows === "number" &&
    Number.isInteger(msg.rows) && msg.rows > 0 && msg.rows <= MAX_TABLE_ROWS ? msg.rows : null;
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
      break;
    case "table": {
      const table = readTable(msg.table);
      if (table) return { status: "table", table };
      break;
    }
  }
  return { status: "failed" };
}
