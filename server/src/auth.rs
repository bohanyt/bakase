//! Akun lokal: daftar, masuk, keluar, dan sesi.
//!
//! User mendaftar sendiri lalu menunggu persetujuan admin, karena tanpa
//! server email kepemilikan alamat email tidak bisa diverifikasi.
//! Sesi disimpan di server; cookie hanya membawa token acak, dan database
//! hanya menyimpan hash SHA-256 token itu.

use crate::{
    AppState,
    acl::{AuthUser, Role},
    db::now,
    error::ApiError,
};
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use axum::{
    Json,
    extract::{FromRequestParts, State},
    http::{StatusCode, request::Parts},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::sync::Semaphore;

pub const SESSION_COOKIE: &str = "app_session";
/// ID user yang sedang tampil di tab pengirim.
const TAB_USER_HEADER: &str = "x-app-user";
const MIN_PASSWORD: usize = 10;
const MAX_PASSWORD: usize = 200;
const MAX_FAILURES: u32 = 5;
const LOCK_SECS: i64 = 15 * 60;
/// Email yang dicatat pembatas sekaligus.
const MAX_TRACKED: usize = 10_000;
/// Batas alamat email di RFC 5321.
const MAX_EMAIL: usize = 254;
/// Satu hitungan Argon2 memakai sekitar 19 MiB. Test berjalan paralel dan berbagi jatah ini.
const ARGON_PARALLEL: usize = if cfg!(test) { 64 } else { 4 };
const ARGON_WAIT: Duration = Duration::from_secs(5);

static ARGON_SLOTS: Semaphore = Semaphore::const_new(ARGON_PARALLEL);

// ---------- password ----------

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!("gagal membuat hash password: {e}"))
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|parsed| {
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    })
}

/// Hash pengganti untuk email yang tidak terdaftar, supaya waktu respons
/// tidak membocorkan email mana yang ada.
fn dummy_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| hash_password("bukan-password-siapa-pun").unwrap_or_default())
}

/// Menjalankan Argon2 di luar worker async, paling banyak `ARGON_PARALLEL` sekaligus.
/// Izin ikut ke closure, jadi tetap terpakai sampai hitungan selesai walau klien putus.
async fn argon<T: Send + 'static>(
    slots: &'static Semaphore,
    job: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ApiError> {
    let permit = tokio::time::timeout(ARGON_WAIT, slots.acquire())
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "Server sibuk. Coba lagi sebentar lagi.",
            )
        })?;
    let done = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        job()
    });
    Ok(done.await.map_err(anyhow::Error::from)?)
}

pub async fn hash_blocking(password: String) -> Result<String, ApiError> {
    Ok(argon(&ARGON_SLOTS, move || hash_password(&password)).await??)
}

async fn verify_blocking(password: String, hash: String) -> Result<bool, ApiError> {
    argon(&ARGON_SLOTS, move || verify_password(&password, &hash)).await
}

pub fn validate_password(password: &str) -> Result<(), ApiError> {
    let len = password.chars().count();
    if len < MIN_PASSWORD {
        return Err(ApiError::bad_request(format!(
            "Password minimal {MIN_PASSWORD} karakter."
        )));
    }
    if len > MAX_PASSWORD {
        return Err(ApiError::bad_request("Password terlalu panjang."));
    }
    Ok(())
}

/// Mengembalikan email dalam huruf kecil kalau formatnya benar dan domainnya sesuai.
pub fn normalize_email(email: &str, domain: &str) -> Result<String, ApiError> {
    let email = email.trim().to_lowercase();
    let invalid = || ApiError::bad_request(format!("Pakai email kantor @{domain}."));
    let (local, host) = email.split_once('@').ok_or_else(invalid)?;
    let local_ok = !local.is_empty()
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._%+-".contains(c));
    if !local_ok || email.len() > MAX_EMAIL || !host.eq_ignore_ascii_case(domain) {
        return Err(invalid());
    }
    Ok(email)
}

/// Mengembalikan nama departemen sesuai penulisan di config, atau error.
pub fn canonical_department(name: &str, allowed: &[String]) -> Result<String, ApiError> {
    allowed
        .iter()
        .find(|d| d.eq_ignore_ascii_case(name.trim()))
        .cloned()
        .ok_or_else(|| ApiError::bad_request("Departemen tidak dikenal."))
}

// ---------- sesi ----------

pub(crate) fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    hex::encode(bytes)
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub(crate) fn session_cookie(token: String, state: &AppState) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, token))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(state.auth.secure_cookie)
        .max_age(time_duration(state.auth.session_hours * 3600))
        .build()
}

fn time_duration(secs: i64) -> time::Duration {
    time::Duration::seconds(secs)
}

/// Membuat sesi hanya kalau password yang tadi diverifikasi masih berlaku dan
/// akun masih aktif. Verifikasi Argon2 berjalan di luar database, jadi reset
/// password oleh admin bisa terjadi di antaranya.
pub(crate) async fn create_session(
    state: &AppState,
    user_id: i64,
    verified_hash: String,
) -> Result<Option<String>, ApiError> {
    let token = new_token();
    let hash = token_hash(&token);
    let ttl = state.auth.session_hours * 3600;
    let admins_only = state.auth.microsoft_full();
    let created = state
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            let created =
                insert_session(&tx, &hash, user_id, ttl, Some(&verified_hash), admins_only)?;
            tx.commit()?;
            Ok(created)
        })
        .await?;
    Ok(created.then_some(token))
}

pub(crate) fn insert_session(
    tx: &rusqlite::Transaction<'_>,
    hash: &str,
    user_id: i64,
    ttl: i64,
    password: Option<&str>,
    admins_only: bool,
) -> rusqlite::Result<bool> {
    let now = now();
    tx.execute("DELETE FROM sessions WHERE expires_at <= ?1", [now])?;
    let n = tx.execute(
        "INSERT INTO sessions (token_hash, user_id, created_at, expires_at)
         SELECT ?1, id, ?3, ?4 FROM users
         WHERE id = ?2 AND status = 'active' AND (?5 IS NULL OR password_hash = ?5)
           AND (NOT ?6 OR role = 'admin')",
        params![hash, user_id, now, now + ttl, password, admins_only],
    )?;
    Ok(n == 1)
}

/// Mencari user aktif pemilik token. Masa sesi diperpanjang kalau sudah lewat
/// separuh; nilai kedua bernilai `true` kalau perpanjangan itu terjadi.
async fn session_user(state: &AppState, token: &str) -> Result<(Option<AuthUser>, bool), ApiError> {
    let hash = token_hash(token);
    let ttl = state.auth.session_hours * 3600;
    let user = state
        .db
        .call(move |c| {
            let now = now();
            let row = c
                .query_row(
                    "SELECT u.id, u.email, u.name, u.departments, u.role, s.expires_at
                     FROM sessions s JOIN users_with_access u ON u.id = s.user_id
                     WHERE s.token_hash = ?1 AND s.expires_at > ?2 AND u.status = 'active'",
                    params![hash, now],
                    |r| {
                        Ok((
                            AuthUser {
                                id: r.get(0)?,
                                email: r.get(1)?,
                                name: r.get(2)?,
                                departments: crate::access::from_row(r, 3)?,
                                role: Role::parse(&r.get::<_, String>(4)?).unwrap_or(Role::User),
                            },
                            r.get::<_, i64>(5)?,
                        ))
                    },
                )
                .optional()?;
            let mut extended = false;
            if let Some((_, expires_at)) = &row
                && expires_at - now < ttl / 2
            {
                extended = c.execute(
                    "UPDATE sessions SET expires_at = ?1 WHERE token_hash = ?2",
                    params![now + ttl, hash],
                )? == 1;
            }
            Ok((row.map(|(u, _)| u), extended))
        })
        .await?;
    Ok(user)
}

/// Hash token sesi dari cookie, untuk memeriksa lagi sesinya di tengah giliran.
pub struct SessionHash(pub String);

impl FromRequestParts<Arc<AppState>> for SessionHash {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        _: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        CookieJar::from_headers(&parts.headers)
            .get(SESSION_COOKIE)
            .map(|c| Self(token_hash(c.value())))
            .ok_or_else(ApiError::unauthorized)
    }
}

/// Sesi masih berlaku dan hak akses user belum berubah sejak `user` dibaca. Membaca database
/// bersama, jadi perubahan lewat slot lain ikut terlihat.
pub async fn access_unchanged(
    db: &crate::db::Db,
    user: &AuthUser,
    session: &str,
) -> anyhow::Result<bool> {
    let (id, session) = (user.id, session.to_string());
    let current = db
        .call(move |c| {
            Ok(c.query_row(
                "SELECT u.departments, u.role
                 FROM sessions s JOIN users_with_access u ON u.id = s.user_id
                 WHERE s.token_hash = ?1 AND s.expires_at > ?2 AND u.id = ?3 AND u.status = 'active'",
                params![session, now(), id],
                |r| {
                    Ok((
                        crate::access::from_row(r, 0)?,
                        Role::parse(&r.get::<_, String>(1)?).unwrap_or(Role::User),
                    ))
                },
            )
            .optional()?)
        })
        .await?;
    Ok(current
        .is_some_and(|(departments, role)| departments == user.departments && role == user.role))
}

/// Tempat extractor `AuthUser` menitipkan token yang sesinya baru diperpanjang,
/// supaya middleware `refresh_session_cookie` mengirim ulang cookie-nya.
#[derive(Clone, Default)]
struct CookieRefresh(Arc<Mutex<Option<String>>>);

impl FromRequestParts<Arc<AppState>> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar
            .get(SESSION_COOKIE)
            .map(|c| c.value().to_string())
            .ok_or_else(ApiError::unauthorized)?;
        let (user, extended) = session_user(state, &token).await?;
        let user = user.ok_or_else(ApiError::unauthorized)?;
        if let Some(seen) = parts.extensions.get::<crate::request_log::SeenUser>() {
            seen.set(user.id);
        }
        // Cookie dipakai bersama semua tab. Tab yang masih menampilkan akun lain tidak boleh
        // menulis ke akun yang baru masuk.
        if let Some(expected) = parts.headers.get(TAB_USER_HEADER)
            && expected.to_str().ok().and_then(|id| id.parse().ok()) != Some(user.id)
        {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "Akun di tab ini sudah berganti. Muat ulang halaman.",
            ));
        }
        if extended
            && let Some(slot) = parts.extensions.get::<CookieRefresh>()
            && let Ok(mut slot) = slot.0.lock()
        {
            *slot = Some(token);
        }
        Ok(user)
    }
}

/// User aktif dengan role admin.
pub struct AdminUser(pub AuthUser);

impl FromRequestParts<Arc<AppState>> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if user.role != Role::Admin {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "Halaman ini khusus admin.",
            ));
        }
        Ok(AdminUser(user))
    }
}

// ---------- batas percobaan masuk ----------

#[derive(Default)]
pub struct LoginLimiter {
    attempts: Mutex<HashMap<String, (u32, i64)>>,
}

impl LoginLimiter {
    /// Memesan satu percobaan sebelum password diverifikasi. Pemeriksaan dan
    /// penambahan terjadi dalam satu kunci, jadi request paralel tidak bisa
    /// melewati batas. Mengembalikan detik tersisa kalau email ini dikunci.
    fn try_reserve(&self, email: &str) -> Result<(), i64> {
        let Ok(mut map) = self.attempts.lock() else {
            return Err(LOCK_SECS);
        };
        let now = now();
        map.retain(|_, (_, since)| now - *since < LOCK_SECS);
        // Saat penuh, yang belum terkunci dan paling lama dikeluarkan dulu, supaya banjir email
        // palsu tidak membuka kunci. Menolak email baru akan mengunci semua orang.
        if map.len() >= MAX_TRACKED
            && !map.contains_key(email)
            && let Some(oldest) = map
                .iter()
                .min_by_key(|(_, (failures, since))| (*failures >= MAX_FAILURES, *since))
                .map(|(email, _)| email.clone())
        {
            map.remove(&oldest);
        }
        let entry = map.entry(email.to_string()).or_insert((0, now));
        if entry.0 >= MAX_FAILURES {
            return Err(entry.1 + LOCK_SECS - now);
        }
        entry.0 += 1;
        Ok(())
    }

    /// Percobaan yang tidak sempat diverifikasi bukan kesalahan user.
    fn refund(&self, email: &str) {
        if let Ok(mut map) = self.attempts.lock()
            && let Some((failures, _)) = map.get_mut(email)
        {
            *failures = failures.saturating_sub(1);
        }
    }

    fn success(&self, email: &str) {
        if let Ok(mut map) = self.attempts.lock() {
            map.remove(email);
        }
    }
}

// ---------- handler ----------

#[derive(Serialize)]
pub struct Options {
    email_domain: String,
    departments: Vec<String>,
    /// Pertanyaan tanpa login di halaman masuk aktif.
    guest: bool,
    /// Nomor WhatsApp IT untuk tombol bantuan, kalau diatur.
    support_whatsapp: Option<String>,
    /// ID extension Edge Bakase yang sah. Web di panel extension hanya menjawab induk ini.
    extension_ids: Vec<String>,
    extension_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    microsoft: Option<&'static str>,
}

pub async fn options(State(state): State<Arc<AppState>>) -> Json<Options> {
    Json(Options {
        email_domain: state.auth.email_domain.clone(),
        departments: state.auth.departments.clone(),
        guest: state.guest.enabled,
        support_whatsapp: state.support_whatsapp.clone(),
        extension_ids: state
            .auth
            .extension_ids
            .iter()
            .filter(|id| crate::valid_extension_id(id))
            .cloned()
            .collect(),
        extension_version: crate::extension::version(),
        microsoft: state
            .auth
            .microsoft
            .as_ref()
            .filter(|m| m.enabled)
            .map(|m| if m.pilot.is_empty() { "full" } else { "pilot" }),
    })
}

#[derive(Deserialize)]
pub struct RegisterRequest {
    name: String,
    email: String,
    password: String,
    #[serde(default)]
    department: Option<String>,
}

pub async fn register(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    if state.auth.microsoft_full() {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Gunakan akun Microsoft kantor.",
        ));
    }
    let name = req.name.trim().to_string();
    if !(2..=100).contains(&name.chars().count()) {
        return Err(ApiError::bad_request("Isi nama lengkap."));
    }
    let email = normalize_email(&req.email, &state.auth.email_domain)?;
    validate_password(&req.password)?;
    let department = match req.department.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(d) => Some(canonical_department(d, &state.auth.departments)?),
    };
    let hash = hash_blocking(req.password).await?;

    // Jawaban sama untuk email baru maupun yang sudah terdaftar, supaya halaman
    // daftar tidak bisa dipakai menebak siapa yang punya akun. Akun lama tidak diubah.
    state
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO users (email, name, password_hash, requested_department, created_at)
                 SELECT ?1, ?2, ?3, ?4, ?5
                 WHERE NOT EXISTS (SELECT 1 FROM microsoft_addresses WHERE email = ?1)
                 ON CONFLICT(email) DO NOTHING",
                params![email, name, hash, department, now()],
            )?;
            Ok(())
        })
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "message": REGISTER_MESSAGE })),
    ))
}

const REGISTER_MESSAGE: &str = "Pendaftaran terkirim. Kamu bisa masuk setelah disetujui admin. \
Kalau email ini sudah punya akun, pakai akun itu atau minta admin mereset password.";

#[derive(Deserialize)]
pub struct LoginRequest {
    email: String,
    password: String,
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    Json(req): Json<LoginRequest>,
) -> Result<(CookieJar, Json<UserResponse>), ApiError> {
    // Alamat yang tidak mungkin punya akun ditolak sebelum dicatat dan sebelum Argon2.
    let email =
        normalize_email(&req.email, &state.auth.email_domain).map_err(|_| wrong_credentials())?;
    let admins_only = state.auth.microsoft_full();
    if let Err(secs) = state.limiter.try_reserve(&email) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            format!(
                "Terlalu banyak percobaan. Coba lagi dalam {} menit.",
                (secs + 59) / 60
            ),
        ));
    }

    let lookup = email.clone();
    let row = state
        .db
        .call(move |c| {
            Ok(c.query_row(
                "SELECT id, email, name, departments, role, status, password_hash FROM users_with_access WHERE email = ?1",
                params![lookup],
                |r| {
                    Ok((
                        AuthUser {
                            id: r.get(0)?,
                            email: r.get(1)?,
                            name: r.get(2)?,
                            departments: crate::access::from_row(r, 3)?,
                            role: Role::parse(&r.get::<_, String>(4)?).unwrap_or(Role::User),
                        },
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                    ))
                },
            )
            .optional()?)
        })
        .await?;

    let hash = row
        .as_ref()
        .filter(|(user, _, hash)| !hash.is_empty() && (!admins_only || user.role == Role::Admin))
        .map(|(_, _, h)| h.clone())
        .unwrap_or_else(|| dummy_hash().to_string());
    let ok = match verify_blocking(req.password, hash).await {
        Ok(ok) => ok,
        Err(busy) => {
            state.limiter.refund(&email);
            return Err(busy);
        }
    };
    let Some((user, status, verified_hash)) = row.filter(|(user, _, hash)| {
        ok && !hash.is_empty() && (!admins_only || user.role == Role::Admin)
    }) else {
        return Err(wrong_credentials());
    };
    state.limiter.success(&email);

    match status.as_str() {
        "active" => {}
        "pending" => {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "Akun belum disetujui admin.",
            ));
        }
        _ => {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "Akun dinonaktifkan. Hubungi admin.",
            ));
        }
    }

    let Some(token) = create_session(&state, user.id, verified_hash).await? else {
        // Password atau status berubah selagi diverifikasi.
        return Err(wrong_credentials());
    };
    Ok((
        jar.add(session_cookie(token, &state)),
        Json(UserResponse::new(user, &state)),
    ))
}

fn wrong_credentials() -> ApiError {
    ApiError::new(StatusCode::UNAUTHORIZED, "Email atau password salah.")
}

pub async fn logout(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    Json(_): Json<serde_json::Value>,
) -> Result<(CookieJar, StatusCode), ApiError> {
    if let Some(c) = jar.get(SESSION_COOKIE) {
        let hash = token_hash(c.value());
        state
            .db
            .call(move |c| {
                c.execute("DELETE FROM sessions WHERE token_hash = ?1", params![hash])?;
                Ok(())
            })
            .await?;
    }
    Ok((
        jar.remove(Cookie::build(SESSION_COOKIE).path("/")),
        StatusCode::NO_CONTENT,
    ))
}

#[derive(Serialize)]
pub struct UserResponse {
    #[serde(flatten)]
    user: AuthUser,
    department: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    microsoft_secret_days: Option<i64>,
}

impl UserResponse {
    fn new(user: AuthUser, state: &AppState) -> Self {
        let microsoft_secret_days = state
            .auth
            .microsoft
            .as_ref()
            .filter(|m| m.enabled && user.role == Role::Admin)
            .and_then(|m| m.expiry_warning(time::OffsetDateTime::now_utc().date()));
        Self {
            department: user.departments.first().cloned(),
            user,
            microsoft_secret_days,
        }
    }
}

pub async fn me(State(state): State<Arc<AppState>>, user: AuthUser) -> Json<UserResponse> {
    Json(UserResponse::new(user, &state))
}

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    current: String,
    new: String,
}

pub async fn change_password(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    jar: CookieJar,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    validate_password(&req.new)?;
    let id = user.id;
    let current_hash = state
        .db
        .call(move |c| {
            Ok(c.query_row(
                "SELECT password_hash FROM users WHERE id = ?1",
                params![id],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await?;
    if !verify_blocking(req.current, current_hash.clone()).await? {
        return Err(ApiError::bad_request("Password lama salah."));
    }
    let new_hash = hash_blocking(req.new).await?;
    let keep = jar
        .get(SESSION_COOKIE)
        .map(|c| token_hash(c.value()))
        .unwrap_or_default();
    let changed = state
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            // Hanya kalau password belum diganti orang lain (misalnya reset admin)
            // dan sesi ini masih berlaku selama hash dihitung.
            let n = tx.execute(
                "UPDATE users SET password_hash = ?1
                 WHERE id = ?2 AND password_hash = ?3 AND status = 'active'
                   AND EXISTS (SELECT 1 FROM sessions
                               WHERE token_hash = ?4 AND user_id = ?2 AND expires_at > ?5)",
                params![new_hash, id, current_hash, keep, now()],
            )?;
            if n == 1 {
                // Sesi di perangkat lain ikut keluar.
                tx.execute(
                    "DELETE FROM sessions WHERE user_id = ?1 AND token_hash <> ?2",
                    params![id, keep],
                )?;
            }
            tx.commit()?;
            Ok(n == 1)
        })
        .await?;
    if !changed {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Password atau sesimu berubah selagi diproses. Masuk lagi lalu coba ulang.",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Menolak request yang mengubah data kalau datang dari origin lain. SameSite=Lax
/// masih mengirim cookie dari port atau subdomain lain yang dianggap same-site.
pub async fn reject_cross_origin(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, ApiError> {
    use axum::http::{Method, header};
    let mutating = !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if mutating && let Some(origin) = req.headers().get(header::ORIGIN) {
        let host = req
            .headers()
            .get(header::HOST)
            .and_then(|h| h.to_str().ok());
        let same = origin
            .to_str()
            .ok()
            .zip(host)
            .is_some_and(|(o, h)| same_origin(o, h));
        if !same {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "Permintaan dari situs lain ditolak.",
            ));
        }
    }
    Ok(next.run(req).await)
}

/// Membandingkan header `Origin` dengan `Host`: nama host (tanpa beda huruf
/// besar/kecil) dan port efektif. Port kosong berarti port bawaan skema origin.
fn same_origin(origin: &str, host: &str) -> bool {
    let Some((scheme, authority)) = origin.split_once("://") else {
        return false;
    };
    let default_port = match scheme.to_ascii_lowercase().as_str() {
        "http" => 80,
        "https" => 443,
        _ => return false,
    };
    // Origin dan Host hanya berisi host[:port]. Userinfo, path, query, fragment,
    // dan spasi berarti header itu bukan bentuk yang sah.
    let parse = |s: &str| -> Option<(String, u16)> {
        if s.is_empty() || s.contains(['@', '/', '?', '#', ' ', '\\']) {
            return None;
        }
        let (name, port) = if let Some(rest) = s.strip_prefix('[') {
            let (name, after) = rest.split_once(']')?;
            let port = match after {
                "" => None,
                p => Some(p.strip_prefix(':')?),
            };
            (format!("[{name}]"), port)
        } else {
            match s.rsplit_once(':') {
                Some((name, port)) => (name.to_string(), Some(port)),
                None => (s.to_string(), None),
            }
        };
        let port = match port {
            Some(p) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => p.parse().ok()?,
            Some(_) => return None,
            None => default_port,
        };
        // Titik dua di luar kurung siku berarti ada lebih dari satu port.
        let valid_name = name.starts_with('[') || (!name.is_empty() && !name.contains(':'));
        valid_name.then(|| (name.to_ascii_lowercase(), port))
    };
    matches!((parse(authority), parse(host)), (Some(a), Some(b)) if a == b)
}

/// Mengirim ulang cookie sesi dengan masa berlaku baru saat sesi di server baru
/// saja diperpanjang. Hanya token yang sudah divalidasi extractor `AuthUser`
/// yang dikirim ulang, jadi token palsu atau yang sudah dicabut tidak pernah
/// diperpanjang di browser.
pub async fn refresh_session_cookie(
    State(state): State<Arc<AppState>>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::header;
    let slot = CookieRefresh::default();
    req.extensions_mut().insert(slot.clone());
    let mut res = next.run(req).await;
    let token = slot.0.lock().ok().and_then(|mut s| s.take());
    // Juga untuk respons gagal: sesinya sah dan sudah diperpanjang di database,
    // jadi cookie harus ikut supaya keduanya tetap sinkron.
    if let Some(token) = token
        && !res.headers().contains_key(header::SET_COOKIE)
        && let Ok(value) = session_cookie(token, &state).to_string().parse()
    {
        res.headers_mut().append(header::SET_COOKIE, value);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_rules() {
        let d = "example.com";
        assert_eq!(
            normalize_email(" Alex.S@example.com ", d).unwrap(),
            "alex.s@example.com"
        );
        for bad in [
            "alex@example.net",
            "alex",
            "@example.com",
            "demo user@example.com",
            "alex@example.com.evil.com",
            "a@b@example.com",
        ] {
            assert!(normalize_email(bad, d).is_err(), "{bad} seharusnya ditolak");
        }
        let host = "@example.com";
        let longest = "a".repeat(MAX_EMAIL - host.len());
        assert!(normalize_email(&format!("{longest}{host}"), d).is_ok());
        assert!(normalize_email(&format!("{longest}a{host}"), d).is_err());
    }

    #[test]
    fn password_hash_roundtrip() {
        let h = hash_password("rahasia-panjang-123").unwrap();
        assert!(verify_password("rahasia-panjang-123", &h));
        assert!(!verify_password("salah", &h));
        assert!(!verify_password("apa saja", "bukan-hash"));
    }

    #[test]
    fn department_must_be_listed() {
        let allowed = vec!["Umum".to_string(), "Review".to_string()];
        assert_eq!(
            canonical_department(" review ", &allowed).unwrap(),
            "Review"
        );
        assert!(canonical_department("Keuangan", &allowed).is_err());
    }

    #[test]
    fn origin_matches_host() {
        let ok = [
            ("http://hub.local", "hub.local"),
            ("http://hub.local", "hub.local:80"),
            ("https://hub.local", "hub.local:443"),
            ("http://HUB.local:8100", "hub.local:8100"),
            ("http://localhost:5173", "localhost:5173"),
            ("http://[::1]:8100", "[::1]:8100"),
            ("http://192.0.2.200:8100", "192.0.2.200:8100"),
        ];
        for (o, h) in ok {
            assert!(same_origin(o, h), "{o} seharusnya sama dengan {h}");
        }
        let bad = [
            ("http://hub.local:9000", "hub.local:8100"),
            ("http://evil.local", "hub.local"),
            ("https://hub.local", "hub.local:80"),
            ("null", "hub.local"),
            ("file://hub.local", "hub.local"),
            ("http://", "hub.local"),
            ("http://localhost:5173", "127.0.0.1:8100"),
            ("http://[::1]@evil.local", "[::1]"),
            ("http://[::1]/tail", "[::1]"),
            ("http://[::1]?x=evil", "[::1]"),
            ("http://[::1]x", "[::1]"),
            ("http://user@hub.local", "hub.local"),
            ("http://hub.local/path", "hub.local"),
            ("http://hub.local:+80", "hub.local:80"),
            ("http://hub.local:", "hub.local:80"),
            ("http://a:b:80", "a:b:80"),
        ];
        for (o, h) in bad {
            assert!(!same_origin(o, h), "{o} seharusnya beda dengan {h}");
        }
    }

    #[test]
    fn limiter_reserves_before_verifying() {
        let l = LoginLimiter::default();
        for _ in 0..MAX_FAILURES {
            assert!(l.try_reserve("a@example.com").is_ok());
        }
        assert!(
            l.try_reserve("a@example.com").is_err(),
            "percobaan ke-6 ditolak sebelum verifikasi"
        );
        assert!(
            l.try_reserve("b@example.com").is_ok(),
            "email lain tidak ikut terkunci"
        );
        l.success("a@example.com");
        assert!(l.try_reserve("a@example.com").is_ok());
    }

    #[test]
    fn limiter_forgets_old_emails_and_stays_bounded() {
        let l = LoginLimiter::default();
        {
            let mut map = l.attempts.lock().unwrap();
            map.insert("lama@example.com".into(), (MAX_FAILURES, now() - LOCK_SECS));
            map.insert(
                "terkunci@example.com".into(),
                (MAX_FAILURES, now() - LOCK_SECS + 30),
            );
            map.insert("tertua@example.com".into(), (1, now() - LOCK_SECS + 60));
            for n in 2..MAX_TRACKED {
                map.insert(format!("u{n}@example.com"), (1, now()));
            }
        }
        assert!(l.try_reserve("baru@example.com").is_ok());
        let map = l.attempts.lock().unwrap();
        assert_eq!(map.len(), MAX_TRACKED);
        assert!(!map.contains_key("lama@example.com"), "sudah lewat masanya");
        assert!(!map.contains_key("tertua@example.com"), "dikeluarkan");
        assert!(
            map.contains_key("terkunci@example.com"),
            "kunci tidak lepas karena banjir email"
        );
        drop(map);
        l.refund("baru@example.com");
        assert_eq!(l.attempts.lock().unwrap()["baru@example.com"].0, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn busy_argon_answers_503_instead_of_piling_up() {
        static SLOTS: Semaphore = Semaphore::const_new(1);
        let held = SLOTS.acquire().await.unwrap();
        let busy = argon(&SLOTS, || ()).await.err().unwrap();
        assert_eq!(
            axum::response::IntoResponse::into_response(busy).status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        drop(held);
        assert_eq!(argon(&SLOTS, || 7).await.ok(), Some(7));
    }
}
