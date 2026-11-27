//! Error API: pesan untuk user dikirim sebagai JSON `{ "message": ... }`.
//! Detail error internal hanya masuk log.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "Silakan masuk dulu.")
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        tracing::error!("internal error: {e:#}");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Terjadi kesalahan di server. Coba lagi. Kalau tetap gagal, hubungi IT.",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let note = crate::request_log::ErrorNote(self.message.clone());
        let mut res = (
            self.status,
            Json(serde_json::json!({ "message": self.message })),
        )
            .into_response();
        res.extensions_mut().insert(note);
        res
    }
}
