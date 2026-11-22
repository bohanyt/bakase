//! Aturan hak akses dokumen. Dipakai oleh semua jalur yang menyentuh isi
//! dokumen: pencarian, baca file, sitasi, preview, dan unduhan.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Admin => "admin",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Role::User),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }
}

/// User yang sudah login dan berstatus aktif.
#[derive(Debug, Clone, Serialize)]
pub struct AuthUser {
    pub id: i64,
    pub email: String,
    pub name: String,
    pub department: Option<String>,
    pub role: Role,
}

/// Label hak akses yang melekat di dokumen dan diwariskan ke setiap chunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "departments", rename_all = "lowercase")]
pub enum Visibility {
    /// Semua user aktif.
    Umum,
    /// Hanya departemen yang disebut.
    Departemen(Vec<String>),
    /// Hanya admin.
    Admin,
}

/// Dokumen tanpa label dianggap tertutup; hanya admin yang bisa melihatnya,
/// supaya bisa diberi label.
pub fn can_read(user: &AuthUser, label: Option<&Visibility>) -> bool {
    if user.role == Role::Admin {
        return true;
    }
    match label {
        None | Some(Visibility::Admin) => false,
        Some(Visibility::Umum) => true,
        Some(Visibility::Departemen(allowed)) => user.department.as_deref().is_some_and(|d| {
            allowed
                .iter()
                .any(|a| a.trim().eq_ignore_ascii_case(d.trim()))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(department: Option<&str>, role: Role) -> AuthUser {
        AuthUser {
            id: 1,
            email: "a@example.com".into(),
            name: "A".into(),
            department: department.map(Into::into),
            role,
        }
    }

    fn dept(names: &[&str]) -> Visibility {
        Visibility::Departemen(names.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn rules() {
        let review = user(Some("Review"), Role::User);
        let operasional = user(Some("Operasional"), Role::User);
        let tanpa_dept = user(None, Role::User);
        let admin = user(None, Role::Admin);

        let cases: &[(&AuthUser, Option<Visibility>, bool)] = &[
            (&review, Some(Visibility::Umum), true),
            (&tanpa_dept, Some(Visibility::Umum), true),
            (&review, Some(dept(&["Review"])), true),
            (&review, Some(dept(&[" review "])), true),
            (&operasional, Some(dept(&["Review"])), false),
            (&operasional, Some(dept(&["Review", "Operasional"])), true),
            (&tanpa_dept, Some(dept(&["Review"])), false),
            (&review, Some(dept(&[])), false),
            (&review, Some(Visibility::Admin), false),
            (&review, None, false),
            (&admin, None, true),
            (&admin, Some(Visibility::Admin), true),
            (&admin, Some(dept(&["Review"])), true),
        ];
        for (u, label, expected) in cases {
            assert_eq!(
                can_read(u, label.as_ref()),
                *expected,
                "user {:?}/{:?}, label {:?}",
                u.department,
                u.role,
                label
            );
        }
    }
}
