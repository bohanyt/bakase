//! Catatan request yang gagal, dan pembungkus error untuk web Bakase.
//!
//! Setiap request dengan status 400 ke atas dicatat satu baris di log: method, path tanpa
//! query, kode, user, ukuran body, lama, dan pesan error untuk user. Isi request tidak dicatat.
//!
//! Proxy jaringan kantor di depan hub mengganti isi balasan 400, 404, 405, 413, 415, 422
//! dengan halaman errornya sendiri, jadi pesan Bakase tidak sampai ke user. Web Bakase
//! mengirim header `X-app-Error-Mode: 200`; untuk request API seperti itu, error dikirim
//! dengan status 200 dan kode aslinya di `X-app-Status`. Klien lain tetap menerima kode asli.

use axum::{
    extract::Request,
    http::{HeaderValue, StatusCode, header},
    middleware::Next,
    response::Response,
};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};
use std::time::Instant;

pub const ERROR_MODE_HEADER: &str = "x-app-error-mode";
pub const STATUS_HEADER: &str = "x-app-status";
/// Panjang pesan error yang ikut dicatat.
const NOTE_CHARS: usize = 300;

/// Pesan error untuk user, dititipkan `ApiError` di respons supaya ikut tercatat.
#[derive(Clone)]
pub struct ErrorNote(pub String);

/// User yang dikenali extractor `AuthUser` di request ini (0 = belum dikenali).
#[derive(Clone, Default)]
pub struct SeenUser(Arc<AtomicI64>);

impl SeenUser {
    pub fn set(&self, id: i64) {
        self.0.store(id, Ordering::Relaxed);
    }

    fn get(&self) -> Option<i64> {
        Some(self.0.load(Ordering::Relaxed)).filter(|id| *id != 0)
    }
}

pub async fn middleware(mut req: Request, next: Next) -> Response {
    let seen = SeenUser::default();
    req.extensions_mut().insert(seen.clone());
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let size = req
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let wrap = path.starts_with("/api/")
        && req
            .headers()
            .get(ERROR_MODE_HEADER)
            .is_some_and(|v| v.as_bytes() == b"200");
    let started = Instant::now();
    let mut res = next.run(req).await;
    let status = res.status();
    if status.as_u16() < 400 {
        return res;
    }
    let note: String = res
        .extensions()
        .get::<ErrorNote>()
        .map(|n| n.0.chars().take(NOTE_CHARS).collect())
        .unwrap_or_default();
    let user = seen
        .get()
        .map_or_else(|| "-".to_string(), |id| id.to_string());
    let size = size.map_or_else(String::new, |n| format!(" {n} byte"));
    let ms = started.elapsed().as_millis();
    let code = status.as_u16();
    if status.is_server_error() {
        tracing::error!("request gagal {method} {path} {code} user={user}{size} {ms} ms: {note}");
    } else if status == StatusCode::UNAUTHORIZED {
        tracing::debug!("request gagal {method} {path} {code} user={user}{size} {ms} ms: {note}");
    } else {
        tracing::warn!("request gagal {method} {path} {code} user={user}{size} {ms} ms: {note}");
    }
    if wrap {
        let headers = res.headers_mut();
        headers.insert(STATUS_HEADER, HeaderValue::from(code));
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        *res.status_mut() = StatusCode::OK;
    }
    res
}
