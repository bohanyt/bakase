import { useCallback, useEffect, useState } from "react";
import { api, type AuthOptions, type User } from "./api";

type Row = {
  id: number;
  email: string;
  name: string;
  requested_department: string | null;
  department: string | null;
  role: "user" | "admin";
  status: "pending" | "active" | "disabled";
  created_at: number;
};

const STATUS_LABEL: Record<Row["status"], string> = {
  pending: "Menunggu",
  active: "Aktif",
  disabled: "Nonaktif",
};

export function AdminUsers({ me }: { me: User }) {
  const [rows, setRows] = useState<Row[]>([]);
  const [departments, setDepartments] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(
    () =>
      api<Row[]>("/api/admin/users").then(setRows, (err) =>
        setError(err instanceof Error ? err.message : "Gagal memuat user."),
      ),
    [],
  );

  useEffect(() => {
    api<Row[]>("/api/admin/users").then(setRows, (err) =>
      setError(err instanceof Error ? err.message : "Gagal memuat user."),
    );
    api<AuthOptions>("/api/auth/options").then((o) => setDepartments(o.departments), () => {});
  }, []);

  async function update(id: number, body: Record<string, string>) {
    setError(null);
    setNotice(null);
    try {
      await api(`/api/admin/users/${id}`, { body });
      await load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Gagal menyimpan.");
    }
  }

  async function resetPassword(row: Row) {
    setError(null);
    setNotice(null);
    try {
      const res = await api<{ temporary_password: string }>(`/api/admin/users/${row.id}/reset-password`, { body: {} });
      setNotice(
        `Password sementara untuk ${row.email}: ${res.temporary_password}. Berikan langsung ke orangnya, lalu minta dia menggantinya.`,
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : "Gagal mereset password.");
    }
  }

  return (
    <div>
      {error && <p className="form-error">{error}</p>}
      {notice && <p className="notice selectable">{notice}</p>}
      <div className="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Nama</th>
              <th>Email</th>
              <th>Status</th>
              <th>Departemen</th>
              <th>Role</th>
              <th>Aksi</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => {
              const self = r.id === me.id;
              return (
                <tr key={r.id}>
                  <td>{r.name}</td>
                  <td>{r.email}</td>
                  <td>
                    <span className={`pill pill-${r.status}`}>{STATUS_LABEL[r.status]}</span>
                  </td>
                  <td>
                    <select
                      id={`dept-${r.id}`}
                      aria-label={`Departemen ${r.name}`}
                      value={r.department ?? ""}
                      onChange={(e) => update(r.id, { department: e.target.value })}
                    >
                      <option value="">
                        {r.requested_department ? `Belum ditetapkan (minta: ${r.requested_department})` : "Belum ditetapkan"}
                      </option>
                      {departments.map((d) => (
                        <option key={d} value={d}>
                          {d}
                        </option>
                      ))}
                    </select>
                  </td>
                  <td>
                    <select
                      id={`role-${r.id}`}
                      aria-label={`Role ${r.name}`}
                      value={r.role}
                      disabled={self}
                      onChange={(e) => update(r.id, { role: e.target.value })}
                    >
                      <option value="user">User</option>
                      <option value="admin">Admin</option>
                    </select>
                  </td>
                  <td className="actions">
                    {r.status !== "active" && !self && (
                      <button
                        type="button"
                        onClick={() =>
                          update(r.id, {
                            status: "active",
                            ...(r.department ? {} : r.requested_department ? { department: r.requested_department } : {}),
                          })
                        }
                      >
                        Setujui
                      </button>
                    )}
                    {r.status === "active" && !self && (
                      <button type="button" onClick={() => update(r.id, { status: "disabled" })}>
                        Nonaktifkan
                      </button>
                    )}
                    <button type="button" onClick={() => resetPassword(r)}>
                      Reset password
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}
