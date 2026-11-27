import { AdminFeedback, type OpenChat } from "./AdminFeedback";
import { AdminUsage } from "./AdminUsage";
import { AdminUsers } from "./AdminUsers";
import type { User } from "./api";

export type AdminTab = "users" | "feedback" | "usage";

/** Tab dipegang oleh Shell, jadi tetap di Feedback setelah bolak-balik membuka chat. */
export default function AdminPage({
  me,
  tab,
  onTab,
  onOpenChat,
}: {
  me: User;
  tab: AdminTab;
  onTab: (tab: AdminTab) => void;
  onOpenChat: OpenChat;
}) {
  return (
    <div className={tab === "users" ? "page admin-users-page" : "page"}>
      <h2>Admin</h2>
      <div className="search-tabs admin-tabs" role="tablist">
        <button type="button" role="tab" aria-selected={tab === "users"} onClick={() => onTab("users")}>
          Kelola user
        </button>
        <button type="button" role="tab" aria-selected={tab === "feedback"} onClick={() => onTab("feedback")}>
          Feedback
        </button>
        <button type="button" role="tab" aria-selected={tab === "usage"} onClick={() => onTab("usage")}>
          Pemakaian
        </button>
      </div>
      {tab === "users" ? (
        <AdminUsers me={me} />
      ) : tab === "feedback" ? (
        <AdminFeedback onOpenChat={onOpenChat} />
      ) : (
        <AdminUsage />
      )}
    </div>
  );
}
