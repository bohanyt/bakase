use std::{
    io::{Cursor, Write},
    sync::LazyLock,
};

use axum::{
    http::header,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use crate::{acl::AuthUser, error::ApiError};

const MANIFEST: &[u8] = include_bytes!("../../extension/manifest.json");
const FILES: &[(&str, &[u8])] = &[
    ("manifest.json", MANIFEST),
    (
        "background.js",
        include_bytes!("../../extension/background.js"),
    ),
    ("panel.html", include_bytes!("../../extension/panel.html")),
    ("panel.js", include_bytes!("../../extension/panel.js")),
    (
        "lib/snapshot.js",
        include_bytes!("../../extension/lib/snapshot.js"),
    ),
    (
        "icons/icon-16.png",
        include_bytes!("../../extension/icons/icon-16.png"),
    ),
    (
        "icons/icon-32.png",
        include_bytes!("../../extension/icons/icon-32.png"),
    ),
    (
        "icons/icon-48.png",
        include_bytes!("../../extension/icons/icon-48.png"),
    ),
    (
        "icons/icon-128.png",
        include_bytes!("../../extension/icons/icon-128.png"),
    ),
    (
        "CARA PASANG.txt",
        include_bytes!("../../extension/CARA PASANG.txt"),
    ),
];

#[derive(Deserialize)]
struct Manifest<'a> {
    version: &'a str,
}

pub fn version() -> &'static str {
    static MANIFEST_VERSION: LazyLock<Manifest<'static>> =
        LazyLock::new(|| serde_json::from_slice(MANIFEST).expect("manifest extension tidak valid"));
    MANIFEST_VERSION.version
}

/// `manifest.json` harus di akar ZIP: Edge dan Chrome menolak ZIP yang diseret ke halaman
/// extension kalau isinya dibungkus folder.
fn archive() -> anyhow::Result<Vec<u8>> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, bytes) in FILES {
        zip.start_file(*name, options)?;
        zip.write_all(bytes)?;
    }
    Ok(zip.finish()?.into_inner())
}

pub async fn download(_user: AuthUser) -> Result<Response, ApiError> {
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"Bakase-{}.zip\"", version()),
            ),
            (header::CACHE_CONTROL, "no-store".to_owned()),
        ],
        archive()?,
    )
        .into_response())
}
