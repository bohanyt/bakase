import { DotsThreeIcon, FoldersIcon, KeyIcon, MagnifyingGlassIcon, NotePencilIcon, PencilSimpleIcon, SidebarSimpleIcon, SignOutIcon, TrashIcon, UsersIcon } from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";
import type { User } from "./api";
import { groupLabel } from "./time";

export type Conversation = { id: string; title: string; updated_at: number };

const GROUP_ORDER = ["Hari ini", "Kemarin", "7 hari terakhir", "30 hari terakhir", "Lebih lama"];

function initials(name: string) {
  return name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]!.toUpperCase())
    .join("");
}

function ConversationItem({
  c,
  active,
  onSelect,
  onRename,
  onDelete,
}: {
  c: Conversation;
  active: boolean;
  onSelect: () => void;
  onRename: (title: string) => void;
  onDelete: () => void;
}) {
  const [menu, setMenu] = useState(false);
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(c.title);

  if (editing) {
    return (
      <form
        className="conv-item editing"
        onSubmit={(e) => {
          e.preventDefault();
          setEditing(false);
          if (title.trim() && title.trim() !== c.title) onRename(title.trim());
        }}
      >
        <input
          id={`rename-${c.id}`}
          value={title}
          autoFocus
          onChange={(e) => setTitle(e.target.value)}
          onBlur={() => {
            setEditing(false);
            setTitle(c.title);
          }}
          onKeyDown={(e) => e.key === "Escape" && (setEditing(false), setTitle(c.title))}
          aria-label="Judul percakapan"
        />
      </form>
    );
  }

  return (
    <div className={`conv-item${active ? " active" : ""}`}>
      <button type="button" className="conv-title" onClick={onSelect} title={c.title}>
        {c.title}
      </button>
      <button type="button" className="conv-more" aria-label="Opsi percakapan" onClick={() => setMenu((m) => !m)}>
        <DotsThreeIcon size={16} />
      </button>
      {menu && (
        <div className="menu conv-menu" onMouseLeave={() => setMenu(false)}>
          <button
            type="button"
            onClick={() => {
              setMenu(false);
              setTitle(c.title);
              setEditing(true);
            }}
          >
            <PencilSimpleIcon size={15} /> Ganti nama
          </button>
          <button
            type="button"
            className="danger"
            onClick={() => {
              setMenu(false);
              if (window.confirm(`Hapus percakapan "${c.title}"?`)) onDelete();
            }}
          >
            <TrashIcon size={15} /> Hapus
          </button>
        </div>
      )}
    </div>
  );
}

export function Sidebar({
  user,
  conversations,
  activeId,
  collapsed,
  width,
  onToggle,
  onNewChat,
  onSearch,
  onSelect,
  onRename,
  onDelete,
  documentsActive,
  onDocuments,
  onAdmin,
  onPassword,
  onLogout,
}: {
  user: User;
  conversations: Conversation[];
  activeId: string | null;
  collapsed: boolean;
  width: number;
  onToggle: () => void;
  onNewChat: () => void;
  onSearch: () => void;
  onSelect: (id: string) => void;
  onRename: (id: string, title: string) => void;
  onDelete: (id: string) => void;
  documentsActive: boolean;
  onDocuments: () => void;
  onAdmin: () => void;
  onPassword: () => void;
  onLogout: () => void;
}) {
  const [userMenu, setUserMenu] = useState(false);
  const userRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!userMenu) return;
    const close = (e: MouseEvent) => {
      if (!userRef.current?.contains(e.target as Node)) setUserMenu(false);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [userMenu]);

  const groups = new Map<string, Conversation[]>();
  for (const c of conversations) {
    const g = groupLabel(c.updated_at);
    groups.set(g, [...(groups.get(g) ?? []), c]);
  }

  return (
    <aside className={`sidebar${collapsed ? " collapsed" : ""}`} style={collapsed ? undefined : { width }}>
      <div className="side-top">
        <button type="button" className="brand" onClick={onNewChat} title="Bakase">
          <span className="ribbon" aria-hidden>
            <img src="/brand/mascot.png" alt="" />
          </span>
          {!collapsed && <span className="side-brand">Bakase</span>}
        </button>
        <button type="button" className="icon-btn" aria-label={collapsed ? "Buka sidebar" : "Tutup sidebar"} onClick={onToggle}>
          <SidebarSimpleIcon size={18} />
        </button>
      </div>

      <nav className="side-actions">
        <button type="button" className="side-row" onClick={onNewChat} title="Chat baru">
          <NotePencilIcon size={18} /> {!collapsed && <span>Chat baru</span>}
        </button>
        <button type="button" className="side-row" onClick={onSearch} title="Cari (Ctrl+K)">
          <MagnifyingGlassIcon size={18} />
          {!collapsed && (
            <>
              <span>Cari</span>
              <kbd>Ctrl K</kbd>
            </>
          )}
        </button>
        <button
          type="button"
          className={`side-row${documentsActive ? " active" : ""}`}
          onClick={onDocuments}
          title="Sumber Data"
          aria-current={documentsActive ? "page" : undefined}
        >
          <FoldersIcon size={18} /> {!collapsed && <span>Sumber Data</span>}
        </button>
      </nav>

      {!collapsed && (
        <div className="side-list">
          {conversations.length === 0 && <p className="side-empty">Belum ada percakapan.</p>}
          {GROUP_ORDER.filter((g) => groups.has(g)).map((g) => (
            <section key={g}>
              <h3>{g}</h3>
              {groups.get(g)!.map((c) => (
                <ConversationItem
                  key={c.id}
                  c={c}
                  active={c.id === activeId}
                  onSelect={() => onSelect(c.id)}
                  onRename={(t) => onRename(c.id, t)}
                  onDelete={() => onDelete(c.id)}
                />
              ))}
            </section>
          ))}
        </div>
      )}

      <div className="side-user" ref={userRef}>
        <button type="button" className="side-row user-btn" onClick={() => setUserMenu((m) => !m)} title={user.email}>
          <span className="avatar">{initials(user.name)}</span>
          {!collapsed && (
            <span className="user-text">
              <span className="user-name">{user.name}</span>
              <span className="user-dept">{user.department ?? (user.role === "admin" ? "Admin" : "")}</span>
            </span>
          )}
        </button>
        {userMenu && (
          <div className="menu user-menu">
            <div className="menu-email">{user.email}</div>
            {user.role === "admin" && (
              <button type="button" onClick={() => (setUserMenu(false), onAdmin())}>
                <UsersIcon size={15} /> Admin
              </button>
            )}
            <button type="button" onClick={() => (setUserMenu(false), onPassword())}>
              <KeyIcon size={15} /> Ganti password
            </button>
            <button type="button" onClick={() => (setUserMenu(false), onLogout())}>
              <SignOutIcon size={15} /> Keluar
            </button>
          </div>
        )}
      </div>
    </aside>
  );
}
