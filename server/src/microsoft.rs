use crate::{AppState, access, archive, auth, db::now};
use anyhow::{Result, anyhow, ensure};
use axum::{
    Json,
    extract::{Query, State, rejection::QueryRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{
    Algorithm, DecodingKey, Validation, decode, decode_header,
    jwk::{AlgorithmParameters, Jwk, KeyAlgorithm, KeyOperations, PublicKeyUse},
};
use reqwest::{Client, Url};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc, time::Duration};
use subtle::ConstantTimeEq;
use time::{Date, Month};
use tokio::{sync::Mutex, time::Instant};
use uuid::Uuid;

pub const CALLBACK: &str = "/api/auth/callback/microsoft";
const REGISTERED_REDIRECT: &str = "https://hub.example.com/api/auth/callback/microsoft";
const AUTHORITY: &str = "https://login.microsoftonline.com";
const BROWSER_COOKIE: &str = "__Host-app_ms";
const FLOW_SECS: i64 = 600;
const KEY_TTL: Duration = Duration::from_secs(3600);
const KEY_RETRY: Duration = Duration::from_secs(5);
const TIMEOUT: Duration = Duration::from_secs(5);

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS microsoft_identities (
    tenant_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    user_id INTEGER NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE CHECK (user_id <> -1),
    PRIMARY KEY (tenant_id, object_id)
);
CREATE TABLE IF NOT EXISTS microsoft_flows (
    state_hash TEXT PRIMARY KEY,
    browser_hash TEXT NOT NULL,
    registration TEXT NOT NULL,
    nonce_hash TEXT NOT NULL,
    verifier TEXT NOT NULL,
    return_to TEXT NOT NULL,
    panel_tab INTEGER NOT NULL,
    pilot INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS microsoft_flows_expiry ON microsoft_flows (expires_at);
"#;

pub(crate) const ADDRESS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS microsoft_addresses (
    email TEXT NOT NULL PRIMARY KEY COLLATE NOCASE,
    user_id INTEGER NOT NULL REFERENCES microsoft_identities(user_id) ON DELETE CASCADE,
    is_mail INTEGER NOT NULL CHECK (is_mail IN (0, 1))
);
CREATE INDEX IF NOT EXISTS microsoft_addresses_user ON microsoft_addresses (user_id);
CREATE TABLE IF NOT EXISTS imported_access_bans (
    email TEXT NOT NULL PRIMARY KEY COLLATE NOCASE
);
CREATE TRIGGER IF NOT EXISTS microsoft_address_user_insert AFTER INSERT ON users
BEGIN
    DELETE FROM microsoft_addresses WHERE email = NEW.email AND user_id <> NEW.id;
END;
CREATE TRIGGER IF NOT EXISTS microsoft_address_user_update AFTER UPDATE OF email ON users
BEGIN
    DELETE FROM microsoft_addresses WHERE email = NEW.email AND user_id <> NEW.id;
END;
"#;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub tenant_id: Uuid,
    pub client_id: Uuid,
    pub redirect_uri: String,
    pub secret_expires: Option<toml::value::Datetime>,
    pub pilot: Vec<String>,
}

impl Settings {
    pub fn validate(&mut self, domain: &str, secure_cookie: bool) -> Result<()> {
        if self.enabled {
            self.check_registration()?;
            ensure!(
                secure_cookie,
                "Login Microsoft membutuhkan auth.secure_cookie = true."
            );
            ensure!(
                self.expiry_date().is_some(),
                "Isi auth.microsoft.secret_expires dengan tanggal TOML."
            );
            for email in &mut self.pilot {
                *email = auth::normalize_email(email, domain).map_err(|_| {
                    anyhow!("Semua alamat pilot Microsoft harus memakai domain kantor.")
                })?;
            }
        }
        Ok(())
    }

    pub fn check_registration(&self) -> Result<()> {
        ensure!(
            !self.tenant_id.is_nil() && !self.client_id.is_nil(),
            "Isi tenant_id dan client_id Microsoft."
        );
        ensure!(
            self.redirect_uri == REGISTERED_REDIRECT,
            "redirect_uri Microsoft harus {REGISTERED_REDIRECT}."
        );
        Ok(())
    }

    fn expiry_date(&self) -> Option<Date> {
        let value = self.secret_expires?;
        if value.time.is_some() || value.offset.is_some() {
            return None;
        }
        let date = value.date?;
        Date::from_calendar_date(
            i32::from(date.year),
            Month::try_from(date.month).ok()?,
            date.day,
        )
        .ok()
    }

    pub fn expiry_warning(&self, today: Date) -> Option<i64> {
        let days = (self.expiry_date()? - today).whole_days();
        (days <= 60).then_some(days)
    }

    fn registration(&self) -> String {
        format!(
            "{}/{}/{}",
            self.tenant_id, self.client_id, self.redirect_uri
        )
    }
}

pub struct Microsoft {
    http: Client,
    authorization: Url,
    token_endpoint: Url,
    jwks: Url,
    issuer: String,
    client_id: String,
    redirect_uri: String,
    secret: Option<String>,
    validation: Validation,
    keys: Mutex<KeyCache>,
}

#[derive(Default)]
struct KeyCache {
    keys: HashMap<String, DecodingKey>,
    expires_at: Option<Instant>,
    last_retry: Option<Instant>,
}

#[derive(Deserialize)]
struct KeySet {
    keys: Vec<SigningKey>,
}

#[derive(Deserialize)]
struct SigningKey {
    #[serde(flatten)]
    jwk: Jwk,
    issuer: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: String,
}

#[derive(Deserialize)]
struct Claims {
    aud: String,
    tid: Uuid,
    oid: Uuid,
    nonce: String,
    #[serde(default)]
    acct: serde_json::Value,
    preferred_username: Option<String>,
    email: Option<String>,
    name: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failure {
    Expired,
    Wrong,
    Pending,
    Disabled,
    Linked,
    Pilot,
    Unavailable,
    Canceled,
    Invalid,
}

impl Failure {
    fn slug(self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::Wrong => "wrong",
            Self::Pending => "access",
            Self::Disabled => "disabled",
            Self::Linked => "linked",
            Self::Pilot => "pilot",
            Self::Unavailable => "unavailable",
            Self::Canceled => "canceled",
            Self::Invalid => "invalid",
        }
    }
}

impl Microsoft {
    pub fn load(settings: &Settings) -> Result<Self> {
        let secret = crate::secret::load_microsoft_secret(settings)
            .ok()
            .flatten();
        if secret.is_none() {
            tracing::warn!("Secret login Microsoft belum tersedia. Jalankan set-microsoft-secret.");
        }
        Self::new(settings, AUTHORITY, secret)
    }

    fn new(settings: &Settings, authority: &str, secret: Option<String>) -> Result<Self> {
        let base = format!("{authority}/{}", settings.tenant_id);
        let issuer = format!("{base}/v2.0");
        let client_id = settings.client_id.to_string();
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&client_id]);
        validation.set_issuer(&[&issuer]);
        validation.set_required_spec_claims(&["exp", "nbf", "aud", "iss", "sub"]);
        validation.validate_nbf = true;
        validation.leeway = 0;
        validation.reject_tokens_expiring_in_less_than = 1;
        Ok(Self {
            http: Client::builder()
                .timeout(TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            authorization: Url::parse(&format!("{base}/oauth2/v2.0/authorize"))?,
            token_endpoint: Url::parse(&format!("{base}/oauth2/v2.0/token"))?,
            jwks: Url::parse(&format!("{base}/discovery/v2.0/keys"))?,
            issuer,
            client_id,
            redirect_uri: settings.redirect_uri.clone(),
            secret,
            validation,
            keys: Mutex::new(KeyCache::default()),
        })
    }

    fn authorization_url(&self, state: &str, nonce: &str, verifier: &str) -> Url {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut url = self.authorization.clone();
        url.query_pairs_mut().extend_pairs([
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", &self.redirect_uri),
            ("response_type", "code"),
            ("response_mode", "query"),
            ("scope", "openid profile email"),
            ("prompt", "select_account"),
            ("state", state),
            ("nonce", nonce),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
        ]);
        url
    }

    async fn claims(&self, code: &str, flow: &Flow) -> Result<Claims, Failure> {
        let secret = self.secret.as_deref().ok_or(Failure::Unavailable)?;
        let response = self
            .http
            .post(self.token_endpoint.clone())
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", secret),
                ("redirect_uri", &self.redirect_uri),
                ("grant_type", "authorization_code"),
                ("code", code),
                ("code_verifier", &flow.verifier),
            ])
            .send()
            .await
            .map_err(|_| Failure::Unavailable)?;
        if !response.status().is_success() {
            #[derive(Deserialize)]
            struct TokenError {
                error: String,
            }
            let invalid_grant = response.status() == StatusCode::BAD_REQUEST
                && response
                    .json::<TokenError>()
                    .await
                    .is_ok_and(|e| e.error == "invalid_grant");
            return Err(if invalid_grant {
                Failure::Invalid
            } else {
                Failure::Unavailable
            });
        }
        let token: TokenResponse = response.json().await.map_err(|error| {
            if error.is_timeout() {
                Failure::Unavailable
            } else {
                Failure::Invalid
            }
        })?;
        let header = decode_header(&token.id_token).map_err(|_| Failure::Invalid)?;
        let kid = header
            .kid
            .as_deref()
            .filter(|kid| !kid.is_empty() && kid.len() <= 256)
            .ok_or(Failure::Invalid)?;
        if header.alg != Algorithm::RS256 {
            return Err(Failure::Invalid);
        }
        let key = self.cached_keys(Some(kid)).await?.ok_or(Failure::Invalid)?;
        let claims: Claims = decode(&token.id_token, &key, &self.validation)
            .map_err(|_| Failure::Invalid)?
            .claims;
        if claims.aud != self.client_id
            || !bool::from(
                auth::token_hash(&claims.nonce)
                    .as_bytes()
                    .ct_eq(flow.nonce_hash.as_bytes()),
            )
        {
            return Err(Failure::Invalid);
        }
        Ok(claims)
    }

    async fn cached_keys(&self, kid: Option<&str>) -> Result<Option<DecodingKey>, Failure> {
        let mut cache = self.keys.lock().await;
        let now = Instant::now();
        let fresh = cache.expires_at.is_some_and(|expires| expires > now);
        let found = kid.and_then(|kid| cache.keys.get(kid).cloned());
        if fresh && (kid.is_none() || found.is_some()) {
            return Ok(found);
        }
        if cache
            .last_retry
            .is_some_and(|last| now.duration_since(last) < KEY_RETRY)
        {
            return Err(if fresh {
                Failure::Invalid
            } else {
                Failure::Unavailable
            });
        }
        if fresh {
            cache.last_retry = Some(now);
        }
        let set = self
            .fetch_keys()
            .await
            .inspect_err(|_| cache.last_retry = Some(now))?;
        cache.keys = set;
        cache.expires_at = Some(Instant::now() + KEY_TTL);
        let found = kid.and_then(|kid| cache.keys.get(kid).cloned());
        if kid.is_some() && found.is_none() {
            cache.last_retry = Some(now);
            return Err(Failure::Invalid);
        }
        Ok(found)
    }

    async fn fetch_keys(&self) -> Result<HashMap<String, DecodingKey>, Failure> {
        let keys: KeySet = self
            .http
            .get(self.jwks.clone())
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| Failure::Unavailable)?
            .json()
            .await
            .map_err(|_| Failure::Unavailable)?;
        let mut result = HashMap::new();
        for SigningKey { jwk, issuer } in keys.keys {
            if !matches!(jwk.algorithm, AlgorithmParameters::RSA(_))
                || jwk
                    .common
                    .key_algorithm
                    .is_some_and(|alg| alg != KeyAlgorithm::RS256)
                || jwk
                    .common
                    .public_key_use
                    .as_ref()
                    .is_some_and(|usage| *usage != PublicKeyUse::Signature)
                || jwk
                    .common
                    .key_operations
                    .as_ref()
                    .is_some_and(|ops| !ops.contains(&KeyOperations::Verify))
                || issuer.as_ref().is_some_and(|issuer| {
                    issuer != &self.issuer
                        && issuer != "https://login.microsoftonline.com/{tenantid}/v2.0"
                })
            {
                continue;
            }
            if let Some(kid) = &jwk.common.key_id
                && let Ok(key) = DecodingKey::from_jwk(&jwk)
                && result.insert(kid.clone(), key).is_some()
            {
                return Err(Failure::Unavailable);
            }
        }
        if result.is_empty() {
            return Err(Failure::Unavailable);
        }
        Ok(result)
    }
}

#[derive(Deserialize, Default)]
pub struct Start {
    #[serde(default)]
    pilot: bool,
    #[serde(default)]
    panel_tab: bool,
    #[serde(default)]
    return_to: String,
}

fn safe_return_to(path: &str) -> &str {
    let lower = path.to_ascii_lowercase();
    let pathname = lower.split(['?', '#']).next().unwrap_or_default();
    if path.len() <= 2048
        && path.starts_with('/')
        && !path.starts_with("//")
        && !pathname.contains(['\\', '%'])
        && !pathname.split('/').any(|part| matches!(part, "." | ".."))
        && !pathname.starts_with("/api/")
        && pathname != "/api"
        && path.bytes().all(|b| b.is_ascii_graphic() && b != b'\\')
    {
        path
    } else {
        "/"
    }
}

struct Flow {
    nonce_hash: String,
    verifier: String,
    return_to: String,
    panel_tab: bool,
    pilot: bool,
}

fn flow_cookie(value: &str) -> Cookie<'static> {
    Cookie::build((BROWSER_COOKIE, value.to_owned()))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .max_age(time::Duration::seconds(FLOW_SECS))
        .build()
}

fn private(response: impl IntoResponse) -> Response {
    (
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        response,
    )
        .into_response()
}

pub async fn start(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    Json(request): Json<Start>,
) -> Response {
    let Some(settings) = state.auth.microsoft.as_ref().filter(|m| m.enabled) else {
        return private(StatusCode::NOT_FOUND);
    };
    let pilot = !settings.pilot.is_empty();
    if pilot && !request.pilot {
        return private(StatusCode::NOT_FOUND);
    }
    let Some(microsoft) = &state.microsoft else {
        return private(StatusCode::SERVICE_UNAVAILABLE);
    };
    if microsoft.secret.is_none()
        || !tokio::time::timeout(TIMEOUT, microsoft.cached_keys(None))
            .await
            .is_ok_and(|keys| keys.is_ok())
    {
        return private(StatusCode::SERVICE_UNAVAILABLE);
    }
    let oauth_state = auth::new_token();
    let nonce = auth::new_token();
    let verifier = auth::new_token();
    let browser = auth::new_token();
    let old_browser = jar.get(BROWSER_COOKIE).map(|c| auth::token_hash(c.value()));
    let url = microsoft.authorization_url(&oauth_state, &nonce, &verifier);
    let state_hash = auth::token_hash(&oauth_state);
    let browser_hash = auth::token_hash(&browser);
    let nonce_hash = auth::token_hash(&nonce);
    let registration = settings.registration();
    let saved = state.db.call(move |db| {
        let tx = db.transaction()?;
        let now = now();
        tx.execute("DELETE FROM microsoft_flows WHERE expires_at <= ?1", [now])?;
        if let Some(old_browser) = old_browser {
            tx.execute("DELETE FROM microsoft_flows WHERE browser_hash = ?1", [&old_browser])?;
        }
        tx.execute(
            "INSERT INTO microsoft_flows
             (state_hash, browser_hash, registration, nonce_hash, verifier, return_to, panel_tab, pilot, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![state_hash, browser_hash, registration, nonce_hash, verifier,
                safe_return_to(&request.return_to), request.panel_tab, pilot, now + FLOW_SECS],
        )?;
        tx.commit()?;
        Ok(())
    }).await;
    if saved.is_err() {
        tracing::warn!("Penyimpanan percobaan masuk Microsoft gagal.");
        return private(StatusCode::SERVICE_UNAVAILABLE);
    }
    private((
        jar.add(flow_cookie(&browser)),
        Json(serde_json::json!({ "url": url.as_str() })),
    ))
}

#[derive(Deserialize)]
pub struct Callback {
    state: String,
    code: Option<String>,
    error: Option<String>,
}

fn result_location(failure: Failure, flow: Option<&Flow>) -> String {
    let mut location = format!("/?ms={}", failure.slug());
    if let Some(flow) = flow {
        if flow.pilot {
            location.push_str("&microsoft=pilot");
        }
        if flow.panel_tab {
            location.push_str("&ms_tab=1");
        }
    }
    location
}

pub async fn callback(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    query: Result<Query<Callback>, QueryRejection>,
) -> Response {
    let expired = || private(Redirect::to(&result_location(Failure::Expired, None)));
    let (Some(settings), Ok(Query(query)), Some(browser)) = (
        state.auth.microsoft.as_ref().filter(|m| m.enabled),
        query,
        jar.get(BROWSER_COOKIE),
    ) else {
        return expired();
    };
    let state_hash = auth::token_hash(&query.state);
    let browser_hash = auth::token_hash(browser.value());
    let registration = settings.registration();
    let flow = state
        .db
        .call(move |db| {
            Ok(db
                .query_row(
                    "DELETE FROM microsoft_flows
             WHERE state_hash = ?1 AND browser_hash = ?2 AND registration = ?3 AND expires_at > ?4
             RETURNING nonce_hash, verifier, return_to, panel_tab, pilot",
                    params![state_hash, browser_hash, registration, now()],
                    |r| {
                        Ok(Flow {
                            nonce_hash: r.get(0)?,
                            verifier: r.get(1)?,
                            return_to: r.get(2)?,
                            panel_tab: r.get(3)?,
                            pilot: r.get(4)?,
                        })
                    },
                )
                .optional()?)
        })
        .await;
    let flow = match flow {
        Ok(Some(flow)) => flow,
        Ok(None) => return expired(),
        Err(_) => return private(Redirect::to(&result_location(Failure::Unavailable, None))),
    };
    let jar = jar.remove(flow_cookie(""));
    match finish(&state, settings, query, &flow).await {
        Ok(token) => {
            let destination = if flow.panel_tab {
                "/?ms_tab=1&ms=done"
            } else {
                safe_return_to(&flow.return_to)
            };
            private((
                jar.add(auth::session_cookie(token, &state)),
                Redirect::to(destination),
            ))
        }
        Err(failure) => {
            tracing::info!(reason = failure.slug(), "Login Microsoft tidak selesai.");
            private((jar, Redirect::to(&result_location(failure, Some(&flow)))))
        }
    }
}

async fn finish(
    state: &AppState,
    settings: &Settings,
    query: Callback,
    flow: &Flow,
) -> Result<String, Failure> {
    if let Some(error) = query.error.as_deref() {
        return Err(match error {
            "access_denied" => Failure::Canceled,
            "server_error" | "temporarily_unavailable" => Failure::Unavailable,
            _ => Failure::Wrong,
        });
    }
    let code = query
        .code
        .as_deref()
        .filter(|code| !code.is_empty())
        .ok_or(Failure::Invalid)?;
    let microsoft = state.microsoft.as_ref().ok_or(Failure::Unavailable)?;
    let claims = tokio::time::timeout(TIMEOUT * 2, microsoft.claims(code, flow))
        .await
        .map_err(|_| Failure::Unavailable)??;
    if claims.tid != settings.tenant_id || claims.oid.is_nil() {
        return Err(Failure::Invalid);
    }
    // Klaim opsional: kalau Entra mengirimnya, nilainya harus anggota tenant. Tanpa klaim ini tamu
    // tetap tertolak, karena nama masuknya bukan alamat kantor.
    if !claims.acct.is_null()
        && claims.acct.as_u64() != Some(0)
        && claims.acct.as_str() != Some("0")
    {
        return Err(Failure::Wrong);
    }
    // Nama masuk Entra bisa berbeda dari alamat email, sedangkan daftar akses memakai alamat email.
    // Keduanya berasal dari token tenant yang sudah diperiksa dan harus berdomain kantor.
    let work = |value: &Option<String>| {
        value
            .as_deref()
            .and_then(|email| auth::normalize_email(email, &state.auth.email_domain).ok())
    };
    let sign_in = work(&claims.preferred_username).ok_or(Failure::Wrong)?;
    let mail = work(&claims.email);
    let mut emails: Vec<String> = mail.iter().cloned().collect();
    if !emails.contains(&sign_in) {
        emails.push(sign_in);
    }
    if !settings.pilot.is_empty()
        && (!flow.pilot || !emails.iter().any(|email| settings.pilot.contains(email)))
    {
        return Err(Failure::Pilot);
    }
    let tenant = claims.tid.to_string();
    let oid = claims.oid.to_string();
    let name: String = claims
        .name
        .as_deref()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(100)
        .collect();
    let name = if name.is_empty() {
        emails[0].clone()
    } else {
        name
    };
    let token = auth::new_token();
    let hash = auth::token_hash(&token);
    let ttl = state.auth.session_hours * 3600;
    let pilot = settings.pilot.clone();
    // Staf kantor yang tidak mendapat departemen dari impor ARCHIVE memakai akses Umum.
    let default_access: Vec<String> = state
        .auth
        .departments
        .iter()
        .find(|name| name.eq_ignore_ascii_case("Umum"))
        .cloned()
        .into_iter()
        .collect();
    let result = state
        .db
        .call(move |db| {
            let tx = db.transaction()?;
            let result = link_account(
                &tx,
                mail.as_ref(),
                &emails,
                &name,
                &tenant,
                &oid,
                &pilot,
                &default_access,
            )?;
            let result = match result {
                Ok(id) if auth::insert_session(&tx, &hash, id, ttl, None, false)? => Ok(()),
                Ok(_) => Err(Failure::Pending),
                Err(failure) => Err(failure),
            };
            tx.commit()?;
            Ok(result)
        })
        .await
        .map_err(|_| Failure::Unavailable)?;
    result.map(|()| token)
}

/// `emails` berisi alamat email lalu nama masuk, tanpa duplikat.
#[allow(clippy::too_many_arguments)]
fn link_account(
    tx: &Transaction<'_>,
    mail: Option<&String>,
    emails: &[String],
    name: &str,
    tenant: &str,
    oid: &str,
    pilot: &[String],
    default_access: &[String],
) -> Result<Result<i64, Failure>> {
    if let Some((id, status)) = tx
        .query_row(
            "SELECT u.id, u.status FROM microsoft_identities i JOIN users u ON u.id = i.user_id
             WHERE tenant_id = ?1 AND object_id = ?2",
            params![tenant, oid],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    {
        if status == "disabled" {
            return Ok(Err(Failure::Disabled));
        }
        update_addresses(tx, id, mail, emails)?;
        apply_import(tx, id, None, default_access)?;
        return Ok(Ok(id));
    }
    let mut existing = None;
    for email in emails {
        let found = tx
            .query_row(
                "SELECT u.id, u.status, u.role, i.user_id IS NOT NULL, u.approved_at IS NULL
             FROM users u LEFT JOIN microsoft_identities i ON i.user_id = u.id
             WHERE u.email = ?1",
                [email],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, bool>(3)?,
                        row.get::<_, bool>(4)?,
                    ))
                },
            )
            .optional()?;
        if found.is_some() && existing.is_some() {
            // Dua akun Bakase untuk satu orang; admin harus menentukan yang benar.
            return Ok(Err(Failure::Linked));
        }
        existing = existing.or(found.map(|user| (user, email)));
    }
    let (id, fallback) =
        if let Some(((id, status, role, linked, never_approved), matched)) = existing {
            if id == archive::SYSTEM_ID || status == "disabled" {
                return Ok(Err(Failure::Disabled));
            }
            // Alamat email saja tidak cukup untuk mewarisi akun admin, kecuali alamat itu sedang
            // didaftarkan untuk uji coba.
            let by_mail_only = emails.last() != Some(matched);
            if linked || (role == "admin" && by_mail_only && !pilot.contains(matched)) {
                return Ok(Err(Failure::Linked));
            }
            if status == "pending" && role == "user" && never_approved {
                // Email pendaftar lokal tidak pernah diverifikasi. Password buatannya tidak boleh
                // ikut ke akun pemilik email yang sebenarnya.
                tx.execute("UPDATE users SET password_hash = '' WHERE id = ?1", [id])?;
            }
            (id, Some(matched))
        } else {
            let reserved: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM microsoft_addresses WHERE email IN (?1, ?2))",
                params![emails[0], emails.last()],
                |r| r.get(0),
            )?;
            if reserved {
                return Ok(Err(Failure::Linked));
            }
            tx.execute(
                "INSERT INTO users (email, name, password_hash, status, created_at)
             VALUES (?1, ?2, '', 'pending', ?3)",
                params![emails[0], name, now()],
            )?;
            (tx.last_insert_rowid(), emails.last())
        };
    tx.execute(
        "INSERT INTO microsoft_identities (tenant_id, object_id, user_id) VALUES (?1, ?2, ?3)",
        params![tenant, oid, id],
    )?;
    update_addresses(tx, id, mail, emails)?;
    apply_import(tx, id, fallback, default_access)?;
    Ok(Ok(id))
}

fn update_addresses(
    tx: &Transaction<'_>,
    id: i64,
    mail: Option<&String>,
    emails: &[String],
) -> rusqlite::Result<()> {
    tx.execute("DELETE FROM microsoft_addresses WHERE user_id = ?1", [id])?;
    for email in emails {
        tx.execute(
            "INSERT INTO microsoft_addresses (email, user_id, is_mail)
             SELECT ?1, ?2, ?3 WHERE NOT EXISTS (
                 SELECT 1 FROM users WHERE email = ?1 AND id <> ?2
             ) ON CONFLICT(email) DO NOTHING",
            params![email, id, mail == Some(email)],
        )?;
    }
    Ok(())
}

/// Alamat email menjadi acuan akses impor. Saat pertama tertaut, baris untuk email akun lama atau
/// nama masuk akun baru (`fallback`) menjadi cadangannya. Baris lain dibuang tanpa diterapkan,
/// supaya impor lama lewat alamat kedua tidak mengembalikan akses yang sudah dicabut. Akun yang
/// belum pernah disetujui menjadi aktif kalau hasilnya punya akses. Kalau impor tidak memberinya
/// departemen, akun itu mendapat `default_access` (Umum), kecuali alamatnya dilarang di ekspor
/// terakhir. Semua alamat di sini sudah dipastikan berdomain kantor.
fn apply_import(
    tx: &Transaction<'_>,
    id: i64,
    fallback: Option<&String>,
    default_access: &[String],
) -> Result<()> {
    let current = tx.query_row(
        "SELECT departments FROM users_with_access WHERE id = ?1",
        [id],
        |row| access::from_row(row, 0),
    )?;
    let mut staged = tx.prepare(
        "SELECT a.email, a.is_mail, s.departments
         FROM microsoft_addresses a JOIN imported_access s ON s.email = a.email
         WHERE a.user_id = ?1 ORDER BY a.is_mail DESC",
    )?;
    let rows = staged
        .query_map([id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, bool>(1)?,
                access::from_row(r, 2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut imported = None;
    let banned = !rows.is_empty() && is_banned(tx, id)?;
    for (email, is_mail, departments) in rows {
        if is_mail || fallback == Some(&email) {
            imported = imported.or(Some(departments));
        }
        tx.execute("DELETE FROM imported_access WHERE email = ?1", [&email])?;
    }
    if banned {
        imported = Some(Vec::new());
    }
    let mut departments = imported.unwrap_or_else(|| current.clone());
    if departments.is_empty()
        && !default_access.is_empty()
        && never_approved(tx, id)?
        && !is_banned(tx, id)?
    {
        departments = default_access.to_vec();
    }
    if departments != current {
        access::replace(tx, id, &departments)?;
    }
    if !departments.is_empty() {
        tx.execute(
            "UPDATE users SET status = 'active', approved_at = ?1
             WHERE id = ?2 AND status = 'pending' AND role = 'user' AND approved_at IS NULL",
            params![now(), id],
        )?;
    }
    Ok(())
}

fn is_banned(tx: &Transaction<'_>, id: i64) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM users u JOIN imported_access_bans b ON b.email = u.email WHERE u.id = ?1
             UNION ALL
             SELECT 1 FROM microsoft_addresses a JOIN imported_access_bans b ON b.email = a.email
             WHERE a.user_id = ?1
         )",
        [id],
        |r| r.get(0),
    )
}

/// User biasa berstatus Menunggu yang belum pernah disetujui admin.
fn never_approved(tx: &Transaction<'_>, id: i64) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT status = 'pending' AND role = 'user' AND approved_at IS NULL FROM users WHERE id = ?1",
        [id],
        |r| r.get(0),
    )
}

#[cfg(test)]
mod tests;
