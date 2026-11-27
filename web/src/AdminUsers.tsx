import {
  ArrowDownIcon, ArrowsDownUpIcon, ArrowLeftIcon, ArrowRightIcon, ArrowUpIcon, CaretDownIcon,
  CaretLeftIcon, CaretRightIcon, DotsThreeIcon, MagnifyingGlassIcon,
} from "@phosphor-icons/react";
import {
  functionalUpdate, getCoreRowModel, getPaginationRowModel, getSortedRowModel, useReactTable,
  type ColumnDef, type Header, type PaginationState, type SortingState,
} from "@tanstack/react-table";
import { useEffect, useId, useMemo, useRef, useState, type MouseEvent } from "react";
import { UserConfirm, UserPopover, UserTextTip, type UserConfirmation } from "./AdminUserOverlays";
import {
  accessText, accessUpdate, accesses, approvalUpdate, COLUMNS, compareUserText, DEFAULT_LAYOUT,
  filterUsers, loadUserLayout, matchesAccess, MAX_WIDTH, moveColumn, NO_ACCESS, NO_FILTERS, ROLE_LABEL,
  sameAccess, saveUserLayout, STATUS_LABEL, type AdminUserRow, type UserColumn, type UserFilters,
} from "./adminUserTable";
import { api, type AuthOptions, type User } from "./api";
import "./AdminUsers.css";

const EMPTY: AdminUserRow[] = [];
type RowOverlay = { kind: "edit" | "menu"; row: AdminUserRow; anchor: HTMLButtonElement };
type Overlay = RowOverlay | { kind: "status" | "access" | "layout"; anchor: HTMLButtonElement };

function Clip({ children, className = "" }: { children: string; className?: string }) {
  return <span className={`user-table-clip ${className}`} title={children}>{children}</span>;
}

function sortColumn(id: Exclude<UserColumn, "action">, read: (row: AdminUserRow) => string): ColumnDef<AdminUserRow> {
  return {
    id,
    accessorFn: read,
    size: COLUMNS[id].size,
    minSize: COLUMNS[id].min,
    maxSize: MAX_WIDTH,
    sortDescFirst: false,
    sortingFn: (a, b) => compareUserText(read(a.original), read(b.original)),
  };
}

function ColumnGrip({ header, id, maximum, onFit }: {
  header: Header<AdminUserRow, unknown>;
  id: UserColumn;
  maximum: number;
  onFit: () => void;
}) {
  const column = header.column;
  const { min, label } = COLUMNS[id];
  const resize = header.getResizeHandler();
  return (
    <div
      className="user-table-grip"
      data-resizing={column.getIsResizing() || undefined}
      role="separator"
      tabIndex={0}
      aria-orientation="vertical"
      aria-label={`Lebar kolom ${label}`}
      aria-valuemin={min}
      aria-valuemax={maximum}
      aria-valuenow={Math.round(column.getSize())}
      title="Geser untuk mengubah lebar. Klik ganda untuk menyesuaikan isi."
      onMouseDown={resize}
      onTouchStart={resize}
      onDoubleClick={onFit}
      onKeyDown={(event) => {
        const step = event.shiftKey ? 24 : 8;
        const choices: Record<string, number> = {
          ArrowLeft: column.getSize() - step,
          ArrowRight: column.getSize() + step,
          Home: min,
          End: maximum,
        };
        if (event.key === "Enter") onFit();
        else if (event.key in choices) {
          header.getContext().table.setColumnSizing((previous) => ({
            ...previous, [id]: Math.max(min, Math.min(maximum, choices[event.key]!)),
          }));
        } else return;
        event.preventDefault();
      }}
    />
  );
}

export function AdminUsers({ me }: { me: User }) {
  const [rows, setRows] = useState<AdminUserRow[] | null>(null);
  const [departments, setDepartments] = useState<string[]>([]);
  const [filters, setFilters] = useState(NO_FILTERS);
  const [sorting, setSorting] = useState<SortingState>([{ id: "name", desc: false }]);
  const [pagination, setPagination] = useState<PaginationState>({ pageIndex: 0, pageSize: 50 });
  const [layout, setLayout] = useState(loadUserLayout);
  const [tip, setTip] = useState<HTMLElement | null>(null);
  const [overlay, setOverlay] = useState<Overlay | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [confirmation, setConfirmation] = useState<UserConfirmation | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [viewport, setViewport] = useState(0);
  const overlayId = useId();
  const scroller = useRef<HTMLDivElement>(null);
  const pendingButton = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    let mounted = true;
    api<AdminUserRow[]>("/api/admin/users").then(
      (data) => { if (mounted) setRows(data); },
      (problem: Error) => { if (mounted) setError(problem.message || "Gagal memuat user."); },
    );
    api<AuthOptions>("/api/auth/options").then(
      (options) => { if (mounted) setDepartments(options.departments); },
      () => { if (mounted) setError("Pilihan departemen gagal dimuat. Muat ulang halaman."); },
    );
    return () => { mounted = false; };
  }, []);
  useEffect(() => {
    const element = scroller.current!;
    const observer = new ResizeObserver(() => setViewport(element.clientWidth));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  function focus(anchor: HTMLElement) {
    requestAnimationFrame(() => {
      if (!anchor.isConnected) pendingButton.current?.focus({ preventScroll: true });
      else if (document.activeElement === document.body) anchor.focus({ preventScroll: true });
    });
  }

  async function mutate(row: AdminUserRow, anchor: HTMLElement, message: string, body?: object) {
    if (busy) return;
    setBusy(true);
    setError("");
    setNotice("");
    let written = false;
    try {
      if (body) {
        await api(`/api/admin/users/${row.id}`, { body });
        written = true;
        const refreshed = await api<AdminUserRow[]>("/api/admin/users");
        setRows(refreshed);
        setNotice(message);
      } else {
        const res = await api<{ temporary_password: string }>(`/api/admin/users/${row.id}/reset-password`, { body: {} });
        setNotice(`Password sementara untuk ${row.email}: ${res.temporary_password}. Berikan langsung ke orangnya, lalu minta dia menggantinya.`);
      }
      setOverlay(null);
      focus(anchor);
    } catch (problem) {
      setError(written
        ? "Perubahan tersimpan. Muat ulang halaman untuk melihat data terbaru."
        : problem instanceof Error ? problem.message : "Gagal menyimpan. Coba lagi.");
      requestAnimationFrame(() => {
        if (overlay?.kind === "edit" && document.activeElement === document.body) {
          document.getElementById(overlayId)?.querySelector<HTMLButtonElement>('button[type="submit"]')?.focus();
        } else focus(anchor);
      });
    } finally {
      setBusy(false);
    }
  }

  function open(event: MouseEvent<HTMLButtonElement>, kind: "status" | "access" | "layout") {
    const anchor = event.currentTarget;
    setOverlay((current) => current?.anchor === anchor ? null : { kind, anchor });
  }

  function openRow(event: MouseEvent<HTMLButtonElement>, kind: RowOverlay["kind"], row: AdminUserRow) {
    const anchor = event.currentTarget;
    if (overlay?.anchor === anchor) {
      setOverlay(null);
      return;
    }
    if (kind === "edit") setSelected(accesses(row));
    setOverlay({ kind, row, anchor });
  }

  function overlayAttributes(kind: Overlay["kind"], rowId?: number) {
    const expanded = overlay?.kind === kind && (rowId === undefined || ("row" in overlay && overlay.row.id === rowId));
    return {
      "aria-expanded": expanded,
      "aria-controls": expanded ? overlayId : undefined,
      "aria-haspopup": kind === "menu" ? undefined : "dialog" as const,
    };
  }

  const all = rows ?? EMPTY;
  const filtered = useMemo(() => filterUsers(all, filters), [all, filters]);
  const nameMaximum = viewport ? Math.max(COLUMNS.name.min, Math.min(MAX_WIDTH, Math.floor(viewport * 0.55))) : MAX_WIDTH;
  const compact = viewport > 0 && viewport < 600;
  const nameSize = compact ? 129 : COLUMNS.name.size;
  const nameWidth = Math.max(COLUMNS.name.min, Math.min(nameMaximum, layout.widths.name ?? nameSize));
  const statusSize = compact
    ? Math.max(COLUMNS.status.min, Math.min(COLUMNS.status.size, viewport - nameWidth - COLUMNS.action.size))
    : COLUMNS.status.size;
  const visibleIds = layout.order.filter((id) => layout.visibility[id] !== false);
  const totalWidth = visibleIds.reduce((width, id) => {
    const standard = id === "name" ? nameSize : id === "status" ? statusSize : COLUMNS[id].size;
    const wanted = layout.widths[id] ?? standard;
    return width + Math.max(COLUMNS[id].min, Math.min(id === "name" ? nameMaximum : MAX_WIDTH, wanted));
  }, 0);
  const fillId = visibleIds.find((id) => id !== "name" && id !== "action" && layout.widths[id] === undefined);
  const extraWidth = Math.max(0, viewport - totalWidth);
  function renderCell(id: UserColumn, r: AdminUserRow) {
    switch (id) {
      case "name": return (
        <div className="user-table-person">
          <Clip>{r.name}</Clip>
          {me.id === r.id && <span className="user-table-self">Anda</span>}
        </div>
      );
      case "status": return (
        <div className="user-table-status-line">
          <span className={`user-table-status ${r.status}`}>{STATUS_LABEL[r.status]}</span>
          {r.status === "pending" && r.id !== me.id && (
            <>
              <button
                type="button"
                disabled={busy}
                aria-label={`Setujui ${r.name}${r.requested_department ? `, minta ${r.requested_department}` : ""}`}
                onClick={(event) => {
                  const request = r.requested_department;
                  const granted = request && !accesses(r).length;
                  void mutate(r, event.currentTarget, granted ? `User disetujui. Akses ${request} diberikan.` : "User disetujui.", approvalUpdate(r));
                }}
              >
                Setujui
              </button>
              {r.requested_department && <Clip className="user-table-request">{`Minta ${r.requested_department}`}</Clip>}
            </>
          )}
        </div>
      );
      case "access": return (
        <button
          type="button"
          className="user-table-access user-table-flat"
          disabled={busy}
          aria-label={`Ubah akses ${r.name}: ${accessText(r)}`}
          {...overlayAttributes("edit", r.id)}
          onClick={(event) => openRow(event, "edit", r)}
        >
          <Clip className={accesses(r).length ? "" : "user-table-quiet"}>{accessText(r)}</Clip>
          <CaretDownIcon size={12} aria-hidden="true" />
        </button>
      );
      case "role": return (
        <select
          className="user-table-role"
          aria-label={`Role ${r.name}`}
          title={me.id === r.id ? "Role akun sendiri tidak dapat diubah." : undefined}
          disabled={me.id === r.id || busy}
          value={r.role}
          onChange={(event) => {
            const anchor = event.currentTarget;
            const role = anchor.value;
            anchor.value = r.role;
            const run = () => void mutate(r, anchor, "Role disimpan.", { role });
            if (role === "admin") {
              setConfirmation({
                anchor, run, title: "Jadikan Admin?", label: "Jadikan Admin",
                description: `${r.name} akan bisa mengelola akun dan akses user lain.`,
              });
            } else run();
          }}
        >
          <option value="user">User</option>
          <option value="admin">Admin</option>
        </select>
      );
      case "action": return (
        <button
          type="button"
          className="user-table-more user-table-flat"
          disabled={busy}
          aria-label={`Aksi ${r.name}`}
          {...overlayAttributes("menu", r.id)}
          onClick={(event) => openRow(event, "menu", r)}
        >
          <DotsThreeIcon size={20} weight="bold" aria-hidden="true" />
        </button>
      );
      case "email": return <Clip className="user-table-quiet">{r.email}</Clip>;
    }
  }

  const columns: ColumnDef<AdminUserRow>[] = [
    { ...sortColumn("name", (row) => row.name), size: nameSize, maxSize: nameMaximum },
    sortColumn("email", (row) => row.email),
    { ...sortColumn("status", (row) => STATUS_LABEL[row.status]), size: statusSize },
    sortColumn("access", accessText),
    sortColumn("role", (row) => ROLE_LABEL[row.role]),
    {
      id: "action", size: COLUMNS.action.size, minSize: COLUMNS.action.min, maxSize: COLUMNS.action.size,
      enableSorting: false, enableResizing: false,
    },
  ];
  if (fillId) {
    const column = columns.find(({ id }) => id === fillId)!;
    column.size = (column.size ?? COLUMNS[fillId].size) + extraWidth;
  }

  const pages = Math.ceil(filtered.length / pagination.pageSize);
  const page = Math.min(pagination.pageIndex, Math.max(0, pages - 1));
  const table = useReactTable({
    data: filtered, columns, getRowId: (row) => String(row.id),
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getPaginationRowModel: getPaginationRowModel(),
    enableSortingRemoval: false,
    enableMultiSort: false,
    autoResetPageIndex: false,
    columnResizeMode: "onChange",
    state: {
      sorting, pagination: { ...pagination, pageIndex: page }, columnOrder: layout.order,
      columnSizing: layout.widths, columnVisibility: layout.visibility,
    },
    onSortingChange: (change) => {
      setSorting(change);
      firstPage();
    },
    onPaginationChange: (change) => {
      const next = functionalUpdate(change, { ...pagination, pageIndex: page });
      setPagination({ ...next, pageIndex: next.pageSize !== pagination.pageSize ? 0 : next.pageIndex });
      if (scroller.current) scroller.current.scrollTop = 0;
    },
    onColumnSizingChange: (change) => setLayout((current) => ({
      ...current, widths: functionalUpdate(change, current.widths),
    })),
  });

  const resizing = table.getState().columnSizingInfo.isResizingColumn;
  useEffect(() => {
    if (!resizing) saveUserLayout(layout);
  }, [layout, resizing]);
  function firstPage() {
    setPagination((current) => ({ ...current, pageIndex: 0 }));
    if (scroller.current) scroller.current.scrollTop = 0;
  }

  function changeFilters(next: UserFilters) {
    setFilters(next);
    firstPage();
  }

  function autoSize(id: UserColumn, header: Header<AdminUserRow, unknown>) {
    const context = document.createElement("canvas").getContext("2d");
    if (!context) return;
    context.font = `600 14px ${getComputedStyle(scroller.current!).fontFamily}`;
    let width = context.measureText(COLUMNS[id].label).width + 44;
    for (const row of all) {
      const value = String(header.column.accessorFn?.(row, 0) ?? "");
      const extra = id === "status" && row.status === "pending" ? " Setujui Minta " + (row.requested_department ?? "") : "";
      width = Math.max(width, context.measureText(value + extra).width + 42);
    }
    table.setColumnSizing((current) => ({
      ...current,
      [id]: Math.min(id === "name" ? nameMaximum : MAX_WIDTH, Math.max(COLUMNS[id].min, Math.ceil(width))),
    }));
  }

  function closeOverlay() {
    if (!overlay) return;
    setOverlay(null);
    overlay.anchor.focus({ preventScroll: true });
  }

  function confirmAction(overlay: RowOverlay, action: "disable" | "password" | "enable") {
    const { row, anchor } = overlay;
    setOverlay(null);
    const position = {
      disable: {
        title: "Nonaktifkan user?", label: "Nonaktifkan",
        description: `${row.name} akan kehilangan akses dan tidak bisa masuk lagi.`,
        run: () => void mutate(row, anchor, "User dinonaktifkan.", { status: "disabled" }),
      },
      password: {
        title: "Reset password?", label: "Reset password",
        description: `${row.name} harus masuk dengan password sementara lalu menggantinya.`,
        run: () => void mutate(row, anchor, ""),
      },
      enable: {
        title: "Aktifkan kembali user?", label: "Aktifkan kembali",
        description: `${row.name} bisa masuk kembali dengan akses yang ditetapkan.`,
        run: () => void mutate(row, anchor, "User diaktifkan kembali.", approvalUpdate({ ...row, requested_department: null })),
      },
    }[action];
    setConfirmation({ anchor, ...position });
  }

  const statusChoices: [string, string][] = Object.entries(STATUS_LABEL);
  const accessChoices = [...new Set([...departments, ...all.flatMap(accesses)]), ""];
  const selectedStatus = filters.statuses.map((id) => STATUS_LABEL[id as keyof typeof STATUS_LABEL]);
  const selectedAccess = filters.access.map((id) => id || NO_ACCESS);
  const pending = filters.statuses.length === 1 && filters.statuses[0] === "pending";
  const columnCount = table.getVisibleLeafColumns().length;
  const visibleRows = table.getRowModel().rows;
  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      scroller.current?.querySelectorAll<HTMLElement>("td .user-table-clip").forEach((label) => {
        if (label.closest("button")) return;
        if (label.scrollWidth > label.clientWidth) label.tabIndex = 0;
        else label.removeAttribute("tabindex");
      });
    });
    return () => cancelAnimationFrame(frame);
  }, [visibleRows, layout, viewport]);

  function showClipped(target: EventTarget) {
    if (target instanceof HTMLElement && target.matches(".user-table-clip[tabindex]")) setTip(target);
  }

  return (
    <section className="admin-users" aria-label="Kelola user">
      <div className="user-table-filters">
        <label className="user-table-search">
          <MagnifyingGlassIcon size={16} aria-hidden="true" />
          <input type="search" aria-label="Cari nama atau email" placeholder="Cari nama atau email" value={filters.query}
            onChange={(event) => changeFilters({ ...filters, query: event.target.value })} />
        </label>
        {(["status", "access"] as const).map((kind) => {
          const values = kind === "status" ? selectedStatus : selectedAccess;
          return (
            <button key={kind} type="button" className="user-table-filter" data-active={!!values.length || undefined}
              {...overlayAttributes(kind)} onClick={(event) => open(event, kind)}>
              {kind === "status" ? "+ Status" : "+ Akses"}
              {!!values.length && <span title={values.join(", ")}> · {values.join(", ")}</span>}
            </button>
          );
        })}
        <button ref={pendingButton} type="button" aria-pressed={pending} onClick={() =>
          changeFilters(pending ? { ...filters, statuses: [] } : { ...NO_FILTERS, statuses: ["pending"] })}>
          Menunggu ({all.filter((row) => row.status === "pending").length})
        </button>
        {(filters.query || selectedStatus.length > 0 || selectedAccess.length > 0) && (
          <button className="user-table-flat" type="button" onClick={() => changeFilters(NO_FILTERS)}>Reset</button>
        )}
        <button type="button" className="user-table-layout" {...overlayAttributes("layout")}
          onClick={(event) => open(event, "layout")}>Tampilan</button>
      </div>
      <div className="user-table-scroll" role="region" aria-label="Tabel user" tabIndex={0} ref={scroller}
        onFocus={(event) => showClipped(event.target)}
        onClick={(event) => showClipped(event.target)}
        onBlur={() => setTip(null)}
        onKeyDown={(event) => {
          if (tip && event.key === "Escape") {
            event.preventDefault();
            setTip(null);
          }
        }}
        onScroll={() => {
          setTip(null);
          if (overlay && "row" in overlay) setOverlay(null);
        }}>
        <table aria-label="Daftar user" style={{ width: table.getTotalSize() }}>
          <colgroup>
            {table.getVisibleLeafColumns().map((column) => <col key={column.id} style={{ width: column.getSize() }} />)}
          </colgroup>
          <thead>
            {table.getHeaderGroups().map((group) => (
              <tr key={group.id}>
                {group.headers.map((header) => {
                  const id = header.column.id as UserColumn;
                  const sort = header.column.getIsSorted();
                  const Icon = sort === "asc" ? ArrowUpIcon : sort === "desc" ? ArrowDownIcon : ArrowsDownUpIcon;
                  return (
                    <th key={id} scope="col" data-column={id} aria-sort={sort === "asc" ? "ascending" : sort === "desc" ? "descending" : undefined}>
                      {id === "action" ? COLUMNS[id].label : (
                        <>
                          <button type="button" className="user-table-sort user-table-flat" onClick={header.column.getToggleSortingHandler()}>
                            {COLUMNS[id].label}<Icon size={11} aria-hidden="true" />
                          </button>
                          <ColumnGrip header={header} id={id} maximum={id === "name" ? nameMaximum : MAX_WIDTH} onFit={() => autoSize(id, header)} />
                        </>
                      )}
                    </th>
                  );
                })}
              </tr>
            ))}
          </thead>
          <tbody>
            {visibleRows.map((row) => (
              <tr key={row.id}>
                {row.getVisibleCells().map((cell) => (
                  <td key={cell.id} data-column={cell.column.id}>{renderCell(cell.column.id as UserColumn, row.original)}</td>
                ))}
              </tr>
            ))}
            {filtered.length === 0 && (
              <tr>
                <td colSpan={columnCount} className="user-table-empty">
                  <strong>{rows === null ? error ? "Daftar user gagal dimuat" : "Memuat user…" : all.length ? "Tidak ada hasil" : "Belum ada user"}</strong>
                  {rows !== null && <p>{all.length ? "Coba kata kunci lain atau hapus filter." : "User akan muncul setelah mendaftar."}</p>}
                  {all.length > 0 && <button type="button" onClick={() => changeFilters(NO_FILTERS)}>Hapus filter</button>}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      <footer className="user-table-footer">
        <strong role="status">{filtered.length} dari {all.length} user</strong>
        <div className="user-table-pages">
          <label>Baris per halaman <select value={pagination.pageSize} onChange={(event) => table.setPageSize(Number(event.target.value))}>
            {[25, 50, 100].map((size) => <option key={size}>{size}</option>)}
          </select></label>
          <span role="status">Hal. {pages ? page + 1 : 0} / {pages}</span>
          <button type="button" aria-label="Halaman sebelumnya" disabled={!table.getCanPreviousPage()} onClick={() => table.previousPage()}>
            <CaretLeftIcon size={14} aria-hidden="true" />
          </button>
          <button type="button" aria-label="Halaman berikutnya" disabled={!table.getCanNextPage()} onClick={() => table.nextPage()}>
            <CaretRightIcon size={14} aria-hidden="true" />
          </button>
        </div>
        {viewport > 0 && table.getTotalSize() > viewport && <small>Geser tabel untuk melihat kolom lainnya.</small>}
      </footer>
      {error && <p className="form-error user-table-feedback" role="alert">{error}</p>}
      {!error && (busy || notice) && <p className="notice selectable user-table-feedback" role="status">{busy ? "Menyimpan…" : notice}</p>}
      {tip?.isConnected && !overlay && <UserTextTip anchor={tip} />}
      {overlay && (
        <UserPopover key={overlay.kind + ("row" in overlay ? overlay.row.id : "")} id={overlayId} anchor={overlay.anchor}
          label={overlay.kind === "layout" ? "Kolom terlihat" : overlay.kind === "status" ? "Status" :
            overlay.kind === "access" ? "Akses" : overlay.kind === "edit" ? "Ubah akses" : "row" in overlay ? `Aksi ${overlay.row.name}` : "Aksi user"}
          menu={overlay.kind === "menu"} onClose={() => setOverlay((current) => current === overlay ? null : current)}>
          {(overlay.kind === "status" || overlay.kind === "access") && (() => {
            const isStatus = overlay.kind === "status";
            const choices = isStatus ? statusChoices : accessChoices.map((id): [string, string] => [id, id || NO_ACCESS]);
            const key = isStatus ? "statuses" : "access";
            const values = filters[key];
            const pool = filterUsers(all, { ...filters, [key]: [] });
            return (
              <>
                <h3>{isStatus ? "Status" : "Akses"}</h3>
                <div className="user-table-facets">
                  {choices.map(([id, label]) => (
                    <label key={id}>
                      <input type="checkbox" checked={values.includes(id)} onChange={(event) => {
                        const next = event.target.checked ? [...values, id] : values.filter((value) => value !== id);
                        changeFilters({ ...filters, [key]: next });
                      }} />
                      {label}
                      <small>{pool.filter((row) => isStatus ? row.status === id : matchesAccess(row, id)).length}</small>
                    </label>
                  ))}
                </div>
              </>
            );
          })()}
          {overlay.kind === "layout" && (
            <>
              <h3>Kolom terlihat</h3>
              <div className="user-table-facets">
                {layout.order.map((id, index) => {
                  const pinned = id === "name" || id === "action";
                  const { label } = COLUMNS[id];
                  return (
                    <div className="user-table-column-option" key={id}>
                      <label><input type="checkbox" disabled={pinned} checked={layout.visibility[id] !== false}
                        onChange={(event) => {
                          const visible = event.target.checked;
                          setLayout((current) => {
                            const visibility = { ...current.visibility };
                            if (visible) delete visibility[id];
                            else visibility[id] = false;
                            return { ...current, visibility };
                          });
                        }} />{label}</label>
                      {pinned ? <small>Tetap</small> : (
                        <>
                          <button className="user-table-flat" type="button" aria-label={`Pindah ${label} ke kiri`}
                            disabled={index === 1} onClick={() => setLayout((current) => ({ ...current, order: moveColumn(current.order, id, -1) }))}>
                            <ArrowLeftIcon size={14} aria-hidden="true" />
                          </button>
                          <button className="user-table-flat" type="button" aria-label={`Pindah ${label} ke kanan`}
                            disabled={index === layout.order.length - 2} onClick={() => setLayout((current) => ({ ...current, order: moveColumn(current.order, id, 1) }))}>
                            <ArrowRightIcon size={14} aria-hidden="true" />
                          </button>
                        </>
                      )}
                    </div>
                  );
                })}
              </div>
              <button className="user-table-flat user-table-reset" type="button" onClick={() => {
                setLayout((current) => ({ ...current, widths: {} }));
                if (scroller.current) scroller.current.scrollLeft = 0;
                closeOverlay();
                setNotice("Lebar kolom dikembalikan.");
              }}>Reset lebar kolom</button>
              <button className="user-table-flat user-table-reset" type="button" onClick={() => {
                setLayout(DEFAULT_LAYOUT);
                if (scroller.current) scroller.current.scrollLeft = 0;
                closeOverlay();
                setNotice("Tampilan dikembalikan.");
              }}>Reset tampilan</button>
            </>
          )}
          {overlay.kind === "edit" && (
            <form onSubmit={(event) => {
              event.preventDefault();
              void mutate(overlay.row, overlay.anchor, "Akses disimpan. User harus masuk lagi.", accessUpdate(overlay.row, selected));
            }}>
              <h3>Ubah akses</h3>
              <p className="user-table-overlay-name">{overlay.row.name}</p>
              <div className="user-table-checks">
                {[...new Set([...departments, ...accesses(overlay.row)])].map((department) => (
                  <label key={department}>
                    <input type="checkbox" checked={selected.includes(department)} disabled={busy}
                      onChange={(event) => {
                        const checked = event.target.checked;
                        const single = overlay.row.departments === undefined;
                        setSelected((current) => checked ? single ? [department] : [...current, department] : current.filter((d) => d !== department));
                      }} />
                    {department}
                  </label>
                ))}
              </div>
              <div className="user-table-pop-actions">
                <button type="button" disabled={busy} onClick={closeOverlay}>Batal</button>
                <button type="submit" className="user-table-strong" disabled={busy || sameAccess(accesses(overlay.row), selected)}>Simpan</button>
              </div>
            </form>
          )}
          {overlay.kind === "menu" && (
            <>
              <button type="button" className="user-table-flat" onClick={() => confirmAction(overlay, "password")}>Reset password</button>
              {overlay.row.status === "disabled" && overlay.row.id !== me.id ? (
                <button type="button" className="user-table-flat" onClick={() => confirmAction(overlay, "enable")}>Aktifkan kembali</button>
              ) : (
                <button type="button" className="user-table-flat user-table-danger" disabled={overlay.row.status !== "active" || overlay.row.id === me.id}
                  onClick={() => confirmAction(overlay, "disable")}>Nonaktifkan</button>
              )}
            </>
          )}
        </UserPopover>
      )}
      {confirmation && <UserConfirm confirmation={confirmation} onClose={() => setConfirmation(null)} />}
    </section>
  );
}
