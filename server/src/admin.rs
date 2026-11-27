//! Kelola user: menyetujui pendaftaran, menetapkan departemen dan role,
//! menonaktifkan, dan mereset password.

use crate::{
    AppState, access,
    acl::Role,
    auth::{AdminUser, canonical_department, hash_blocking},
    db::now,
    error::ApiError,
};
use axum::{
    Json,
    extract::rejection::JsonRejection,
    extract::{Path, State},
    http::StatusCode,
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize)]
pub struct UserRow {
    id: i64,
    email: String,
    name: String,
    requested_department: Option<String>,
    department: Option<String>,
    departments: Vec<String>,
    role: String,
    status: String,
    created_at: i64,
}

pub async fn list_users(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Vec<UserRow>>, ApiError> {
    let rows = state
        .db
        .call(|c| {
            let mut stmt = c.prepare(
                "SELECT id, email, name, requested_department, department, role, status, created_at, departments
                 FROM users_with_access WHERE id <> ?1
                 ORDER BY CASE status WHEN 'pending' THEN 0 WHEN 'active' THEN 1 ELSE 2 END, created_at DESC",
            )?;
            let rows = stmt
                .query_map([crate::archive::SYSTEM_ID], |r| {
                    Ok(UserRow {
                        id: r.get(0)?,
                        email: r.get(1)?,
                        name: r.get(2)?,
                        requested_department: r.get(3)?,
                        department: r.get(4)?,
                        role: r.get(5)?,
                        status: r.get(6)?,
                        created_at: r.get(7)?,
                        departments: access::from_row(r, 8)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct UpdateUser {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    role: Option<String>,
    /// Field tunggal untuk halaman admin versi lama.
    #[serde(default)]
    department: Option<String>,
    #[serde(default)]
    departments: Option<Vec<String>>,
}

pub async fn update_user(
    State(state): State<Arc<AppState>>,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
    body: Result<Json<UpdateUser>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    reject_system_account(id)?;
    let Json(req) = body.map_err(|_| {
        ApiError::bad_request("Data user tidak valid. Muat ulang halaman dan coba lagi.")
    })?;
    let status = match req.status.as_deref() {
        None => None,
        Some(s @ ("pending" | "active" | "disabled")) => Some(s.to_string()),
        Some(_) => return Err(ApiError::bad_request("Status tidak dikenal.")),
    };
    let role = match req.role.as_deref() {
        None => None,
        Some(r) => Some(
            Role::parse(r)
                .ok_or_else(|| ApiError::bad_request("Role tidak dikenal."))?
                .as_str()
                .to_string(),
        ),
    };
    let departments = match (req.departments, req.department.as_deref().map(str::trim)) {
        (Some(_), Some(_)) => return Err(ApiError::bad_request("Kirim satu pilihan akses.")),
        (Some(mut departments), None) => {
            for department in &mut departments {
                *department = canonical_department(department, &state.auth.departments)?;
            }
            access::sort_departments(&mut departments);
            Some(departments)
        }
        (None, Some("")) => Some(vec![]),
        (None, Some(d)) => Some(vec![canonical_department(d, &state.auth.departments)?]),
        (None, None) => None,
    };
    if id == admin.id && (status.is_some() || role.is_some()) {
        return Err(ApiError::bad_request(
            "Kamu tidak bisa mengubah status atau role akunmu sendiri.",
        ));
    }

    let admin_id = admin.id;
    state
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            let current = tx
                .query_row(
                    "SELECT status, role, departments, approved_at FROM users_with_access WHERE id = ?1",
                    params![id],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            access::from_row(r, 2)?,
                            r.get::<_, Option<i64>>(3)?,
                        ))
                    },
                )
                .optional()?;
            let Some((cur_status, cur_role, cur_departments, approved_at)) = current else {
                return Ok(Err(ApiError::not_found("User tidak ditemukan.")));
            };

            let new_status = status.unwrap_or(cur_status.clone());
            let new_role = role.unwrap_or(cur_role.clone());
            if new_status == "active" && cur_status != "active" && new_role == "user"
                && cur_departments.is_empty() && departments.is_none()
            {
                return Ok(Err(ApiError::bad_request(
                    "Pilih akses departemen atau setujui tanpa akses khusus.",
                )));
            }

            let approving = new_status == "active" && approved_at.is_none();
            tx.execute(
                "UPDATE users SET status = ?1, role = ?2,
                    approved_at = CASE WHEN ?3 THEN ?4 ELSE approved_at END,
                    approved_by = CASE WHEN ?3 THEN ?5 ELSE approved_by END
                 WHERE id = ?6",
                params![
                    new_status,
                    new_role,
                    approving,
                    now(),
                    admin_id,
                    id
                ],
            )?;
            // Perubahan hak akses berlaku segera: sesi lama dihapus.
            if let Some(departments) = departments.filter(|d| *d != cur_departments) {
                access::replace(&tx, id, &departments)?;
            } else if new_status != cur_status || new_role != cur_role {
                tx.execute("DELETE FROM sessions WHERE user_id = ?1", params![id])?;
            }
            tx.commit()?;
            Ok(Ok(()))
        })
        .await??;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct ResetResult {
    temporary_password: String,
}

pub async fn reset_password(
    State(state): State<Arc<AppState>>,
    AdminUser(_admin): AdminUser,
    Path(id): Path<i64>,
    // Body JSON wajib (boleh `{}`), sama seperti semua request yang mengubah data.
    Json(_): Json<serde_json::Value>,
) -> Result<Json<ResetResult>, ApiError> {
    reject_system_account(id)?;
    let temporary = temporary_password();
    let hash = hash_blocking(temporary.clone()).await?;
    let found = state
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            let n = tx.execute(
                "UPDATE users SET password_hash = ?1 WHERE id = ?2",
                params![hash, id],
            )?;
            tx.execute("DELETE FROM sessions WHERE user_id = ?1", params![id])?;
            tx.commit()?;
            Ok(n == 1)
        })
        .await?;
    if !found {
        return Err(ApiError::not_found("User tidak ditemukan."));
    }
    Ok(Json(ResetResult {
        temporary_password: temporary,
    }))
}

/// 14 karakter tanpa huruf yang mudah tertukar (0/O, 1/l/I).
fn reject_system_account(id: i64) -> Result<(), ApiError> {
    if id == crate::archive::SYSTEM_ID {
        return Err(ApiError::not_found("User tidak ditemukan."));
    }
    Ok(())
}

fn temporary_password() -> String {
    use rand::RngExt;
    const CHARS: &[u8] = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut rng = rand::rng();
    (0..14)
        .map(|_| CHARS[rng.random_range(0..CHARS.len())] as char)
        .collect()
}
