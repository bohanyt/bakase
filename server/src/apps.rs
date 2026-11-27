//! Halaman Aplikasi: pekerjaan yang punya langkah sendiri, misalnya Laporan IT
//! dan olah Excel counting app. Daftar dan izinnya ada di server; halaman web hanya
//! menampilkan aplikasi yang dikembalikan di sini.

use crate::acl::{AuthUser, Role};
use axum::Json;
use serde::Serialize;

/// Satu kartu di halaman Aplikasi.
#[derive(Debug, Serialize)]
pub struct AppCard {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Belum bisa dipakai. Hanya admin yang melihat aplikasi yang belum siap.
    pub ready: bool,
}

/// Aplikasi yang dikenal hub. Aplikasi yang belum siap hanya tampil untuk admin. Saat sebuah
/// aplikasi siap, aturan siapa yang boleh memakainya ditambahkan di `allowed`.
const APPS: &[AppCard] = &[
    AppCard {
        id: "laporan-it",
        name: "Laporan IT",
        description: "Susun laporan harian dari catatanmu, periksa, lalu kirim ke GLPI.",
        ready: false,
    },
    AppCard {
        id: "counting-app",
        name: "Counting app",
        description: "Ubah Excel hasil counting jadi file yang siap di-upload ke app.",
        ready: false,
    },
];

fn allowed(app: &AppCard, user: &AuthUser) -> bool {
    user.role == Role::Admin || app.ready
}

/// Aplikasi yang boleh dilihat `user`, urut seperti `APPS`.
pub fn visible(user: &AuthUser) -> Vec<&'static AppCard> {
    APPS.iter().filter(|app| allowed(app, user)).collect()
}

/// `GET /api/apps`
pub async fn list(user: AuthUser) -> Json<Vec<&'static AppCard>> {
    Json(visible(&user))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(role: Role) -> AuthUser {
        AuthUser {
            id: 1,
            email: "alex@example.com".into(),
            name: "Alex".into(),
            departments: vec!["IT".into()],
            role,
        }
    }

    #[test]
    fn apps_that_are_not_ready_are_only_for_admins() {
        assert!(visible(&user(Role::User)).is_empty());
        let ids: Vec<_> = visible(&user(Role::Admin)).iter().map(|a| a.id).collect();
        assert_eq!(ids, ["laporan-it", "counting-app"]);
        assert!(APPS.iter().all(|a| !a.ready));
    }
}
