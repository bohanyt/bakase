import { departmentList } from "./api.ts";

export type AdminUserRow = {
  id: number;
  email: string;
  name: string;
  requested_department: string | null;
  department: string | null;
  departments?: string[];
  role: "user" | "admin";
  status: "pending" | "active" | "disabled";
};

export const STATUS_LABEL = { active: "Aktif", pending: "Menunggu", disabled: "Nonaktif" } as const;
export const ROLE_LABEL = { user: "User", admin: "Admin" } as const;
export const NO_ACCESS = "Tanpa akses khusus";

export const COLUMNS = {
  name: { label: "Nama", size: 215, min: 106 },
  email: { label: "Email", size: 245, min: 96 },
  status: { label: "Status", size: 226, min: 170 },
  access: { label: "Akses", size: 200, min: 105 },
  role: { label: "Role", size: 96, min: 80 },
  action: { label: "Aksi", size: 54, min: 54 },
};
export type UserColumn = keyof typeof COLUMNS;
const COLUMN_IDS = Object.keys(COLUMNS) as UserColumn[];
export const MOVABLE = COLUMN_IDS.filter((id) => id !== "name" && id !== "action");
export const MAX_WIDTH = 950;
const STORAGE_KEY = "app.admin-user-table";
const collator = new Intl.Collator("id", { numeric: true, sensitivity: "base" });
export const compareUserText = collator.compare;

export function accesses(row: AdminUserRow) {
  return departmentList(row.departments, row.department);
}

export function accessText(row: AdminUserRow) {
  return accesses(row).join(", ") || NO_ACCESS;
}

export type UserFilters = { query: string; statuses: string[]; access: string[] };
export const NO_FILTERS: UserFilters = { query: "", statuses: [], access: [] };

export function matchesAccess(row: AdminUserRow, access: string) {
  const current = accesses(row);
  return access === "" ? current.length === 0 : current.includes(access);
}

export function filterUsers(rows: AdminUserRow[], filters: UserFilters) {
  const query = filters.query.trim().toLowerCase();
  return rows.filter((row) =>
    (!query || [row.name, row.email].some((value) => value.toLowerCase().includes(query))) &&
    (!filters.statuses.length || filters.statuses.includes(row.status)) &&
    (!filters.access.length || filters.access.some((access) => matchesAccess(row, access))),
  );
}

export function sameAccess(initial: string[], selected: string[]) {
  return initial.length === selected.length && selected.every((value) => initial.includes(value));
}

export function accessUpdate(row: AdminUserRow, selected: string[]) {
  return row.departments === undefined ? { department: selected[0] ?? "" } : { departments: selected };
}

export function approvalUpdate(row: AdminUserRow) {
  if (accesses(row).length) return { status: "active" };
  const requested = row.requested_department ? [row.requested_department] : [];
  return { status: "active", ...accessUpdate(row, requested) };
}

export type TableLayout = {
  widths: Record<string, number>;
  order: UserColumn[];
  visibility: Record<string, boolean>;
};

export const DEFAULT_LAYOUT: TableLayout = { widths: {}, order: COLUMN_IDS, visibility: {} };

export function moveColumn(order: UserColumn[], id: UserColumn, offset: -1 | 1) {
  const index = order.indexOf(id);
  const target = index + offset;
  if (index <= 0 || index >= order.length - 1 || target <= 0 || target >= order.length - 1) {
    return order;
  }
  const next = [...order];
  [next[index], next[target]] = [next[target]!, next[index]!];
  return next;
}

type LayoutStorage = Pick<Storage, "getItem" | "setItem">;

export function loadUserLayout(storage?: LayoutStorage): TableLayout {
  try {
    const saved = JSON.parse((storage ?? window.localStorage).getItem(STORAGE_KEY) ?? "null");
    const widths: TableLayout["widths"] = {};
    const visibility: TableLayout["visibility"] = {};
    for (const id of COLUMN_IDS) {
      const width: unknown = saved?.widths?.[id];
      if (id !== "action" && typeof width === "number" && Number.isFinite(width)) {
        widths[id] = Math.max(COLUMNS[id].min, Math.min(MAX_WIDTH, width));
      }
      if (id !== "name" && id !== "action" && saved?.visibility?.[id] === false) visibility[id] = false;
    }
    const order = Array.isArray(saved?.order) ? saved.order : [];
    const middle = MOVABLE.slice().sort((a, b) => {
      const position = (id: UserColumn) => order.includes(id) ? order.indexOf(id) : order.length;
      return position(a) - position(b);
    });
    return { widths, visibility, order: ["name", ...middle, "action"] };
  } catch {
    return DEFAULT_LAYOUT;
  }
}

export function saveUserLayout(layout: TableLayout, storage?: LayoutStorage) {
  try {
    (storage ?? window.localStorage).setItem(STORAGE_KEY, JSON.stringify(layout));
  } catch {
    // Pengaturan masih berlaku selama tabel terbuka.
  }
}
