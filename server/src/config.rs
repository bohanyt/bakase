//! Konfigurasi server, dibaca dari `config.toml` lalu ditimpa variabel lingkungan.

use crate::acl::Visibility;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_bind")]
    pub bind: String,
    /// Folder hasil build frontend (`web/dist`).
    #[serde(default = "default_web_dir")]
    pub web_dir: PathBuf,
    /// Folder data runtime (database, index). Tidak masuk git.
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    pub azure: AzureConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    /// Folder dokumen untuk pencarian, dibaca oleh `rag-hub-server ingest`.
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
    pub makna: Option<MaknaConfig>,
    pub gambar: Option<GambarConfig>,
    /// Pertanyaan tanpa login di halaman masuk.
    #[serde(default)]
    pub guest: GuestConfig,
    /// Kontak IT untuk tombol "Ada masalah? Hubungi IT".
    #[serde(default)]
    pub support: SupportConfig,
    /// Tarif model dan kurs untuk laporan Pemakaian di Admin.
    #[serde(default)]
    pub pricing: PricingConfig,
    /// Lama menunggu jawaban yang masih mengalir saat server diminta berhenti. Untuk pergantian
    /// instans tanpa jeda layanan, gunakan sekitar 120 detik.
    #[serde(default = "default_shutdown_grace")]
    pub shutdown_grace_secs: u64,
    /// Kalau file ini ada, `/api/health` menjawab 503 supaya reverse proxy berhenti mengirim
    /// permintaan baru ke slot ini sebelum dimatikan. Diisi `Config::load`: `draining` di
    /// folder config.
    #[serde(skip)]
    pub drain_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MaknaConfig {
    pub model: PathBuf,
    pub ort_dylib: Option<PathBuf>,
    #[serde(default = "query_threads")]
    pub threads: usize,
    #[serde(default)]
    pub gpu: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GambarConfig {
    #[serde(default)]
    pub model: PathBuf,
    pub ort_dylib: Option<PathBuf>,
    #[serde(default)]
    pub gpu: bool,
    #[serde(default)]
    pub device_id: i32,
    #[serde(default = "vision_arena_mib")]
    pub vision_arena_mib: u32,
    #[serde(default = "backbone_arena_mib")]
    pub backbone_arena_mib: u32,
}

fn vision_arena_mib() -> u32 {
    4096
}

fn backbone_arena_mib() -> u32 {
    2048
}

fn query_threads() -> usize {
    2
}

fn default_shutdown_grace() -> u64 {
    10
}

/// Kontak IT. Nomor WhatsApp hanya angka dengan kode negara, misalnya 6281234567890.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SupportConfig {
    #[serde(default)]
    pub whatsapp: String,
}

impl SupportConfig {
    /// Nomor yang sah (8 sampai 15 angka setelah tanda selain angka dibuang), atau `None`.
    pub fn whatsapp_number(&self) -> Option<String> {
        let digits: String = self.whatsapp.chars().filter(char::is_ascii_digit).collect();
        (8..=15).contains(&digits.len()).then_some(digits)
    }
}

/// Pertanyaan tanpa login: tanpa dokumen, tanpa tool, tidak disimpan. Mati kalau tidak
/// diaktifkan, karena setiap pertanyaan memakai kuota Azure.
#[derive(Debug, Clone, Deserialize)]
pub struct GuestConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Batas per perangkat (alamat IP) per jam.
    #[serde(default = "default_guest_per_device")]
    pub per_device_per_hour: u32,
    /// Batas semua tamu per jam.
    #[serde(default = "default_guest_total")]
    pub total_per_hour: u32,
}

impl Default for GuestConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            per_device_per_hour: default_guest_per_device(),
            total_per_hour: default_guest_total(),
        }
    }
}

/// Tarif dalam USD per 1 juta token. Bawaannya contoh tarif API (Standard, prompt
/// sampai 272K token) per 17 November 2026: https://developers.openai.com/api/docs/pricing
#[derive(Debug, Clone, Deserialize)]
pub struct PricingConfig {
    #[serde(default = "default_input_usd")]
    pub input_usd: f64,
    /// Token input yang diambil dari cache prompt.
    #[serde(default = "default_cached_input_usd")]
    pub cached_input_usd: f64,
    /// Termasuk token reasoning.
    #[serde(default = "default_output_usd")]
    pub output_usd: f64,
    /// Kurs USD ke IDR, diambil sehari sekali. Bawaannya Frankfurter (kurs referensi ECB),
    /// tanpa key dan tanpa mengirim data kerja. Kosongkan untuk mematikan.
    #[serde(default = "default_fx_url")]
    pub fx_url: String,
    /// Kurs yang dipakai kalau belum pernah berhasil mengambil kurs. 0 berarti tidak ada.
    #[serde(default)]
    pub fallback_usd_idr: f64,
}

impl Default for PricingConfig {
    fn default() -> Self {
        Self {
            input_usd: default_input_usd(),
            cached_input_usd: default_cached_input_usd(),
            output_usd: default_output_usd(),
            fx_url: default_fx_url(),
            fallback_usd_idr: 0.0,
        }
    }
}

fn default_input_usd() -> f64 {
    0.10
}
fn default_cached_input_usd() -> f64 {
    0.01
}
fn default_output_usd() -> f64 {
    0.50
}
fn default_fx_url() -> String {
    "https://api.frankfurter.dev/v1/latest?base=USD&symbols=IDR".into()
}

fn default_guest_per_device() -> u32 {
    3
}
fn default_guest_total() -> u32 {
    60
}

/// Satu folder dokumen. Semua file di dalamnya memakai hak akses yang sama.
#[derive(Debug, Clone, Deserialize)]
pub struct SourceConfig {
    /// Nama pendek yang tampil di sitasi, misalnya `Review`. Harus unik.
    pub name: String,
    pub path: PathBuf,
    /// Siapa yang boleh membaca isi folder ini. Contoh:
    /// `{ type = "departemen", departments = ["Review"] }`, `{ type = "umum" }`, `{ type = "admin" }`.
    pub visibility: Visibility,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AzureConfig {
    /// Contoh: `https://nama-resource.openai.azure.com`
    pub endpoint: String,
    /// Nama deployment model di Azure, misalnya deployment model.
    pub deployment: String,
    #[serde(default = "default_api_version")]
    pub api_version: String,
    /// Kosongkan di file. Key dibaca dari env `APP_AZURE_API_KEY`, atau dari
    /// Windows Credential Manager setelah `rag-hub-server set-azure-key`.
    #[serde(default)]
    pub api_key: String,
    /// Batas konteks model dalam token, untuk anggaran riwayat chat.
    #[serde(default = "default_context_tokens")]
    pub context_tokens: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    /// Hanya email dengan domain ini yang boleh mendaftar.
    #[serde(default = "default_email_domain")]
    pub email_domain: String,
    /// Lama sesi tanpa aktivitas sebelum harus masuk lagi.
    #[serde(default = "default_session_hours")]
    pub session_hours: i64,
    /// Aktifkan kalau server diakses lewat HTTPS.
    #[serde(default)]
    pub secure_cookie: bool,
    /// Daftar departemen yang bisa dipilih saat daftar dan ditetapkan admin.
    #[serde(default = "default_departments")]
    pub departments: Vec<String>,
    /// ID extension Edge Bakase (folder `extension/`) yang boleh menampilkan Bakase di panel
    /// samping. Kosong berarti Bakase tidak bisa ditampilkan di frame milik situs lain.
    #[serde(default)]
    pub extension_ids: Vec<String>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            email_domain: default_email_domain(),
            session_hours: default_session_hours(),
            secure_cookie: false,
            departments: default_departments(),
            extension_ids: Vec::new(),
        }
    }
}

fn default_bind() -> String {
    "127.0.0.1:8100".into()
}
fn default_web_dir() -> PathBuf {
    PathBuf::from("../web/dist")
}
fn default_data_dir() -> PathBuf {
    PathBuf::from("../data")
}
fn default_api_version() -> String {
    "2024-10-21".into()
}
fn default_context_tokens() -> usize {
    128_000
}
fn default_email_domain() -> String {
    "example.com".into()
}
fn default_session_hours() -> i64 {
    24
}
fn default_departments() -> Vec<String> {
    vec!["Umum".into()]
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("tidak bisa membaca {}", path.display()))?;
        let mut cfg: Config = toml::from_str(&text).context("config.toml tidak valid")?;
        if let Ok(key) = std::env::var("APP_AZURE_API_KEY") {
            cfg.azure.api_key = key;
        }
        if let Ok(bind) = std::env::var("APP_BIND") {
            cfg.bind = bind;
        }
        cfg.gambar = cfg
            .gambar
            .filter(|gambar| !gambar.model.as_os_str().is_empty());
        // Path relatif dihitung dari folder config, bukan dari folder tempat perintah
        // dijalankan, supaya server, service, dan `ingest` selalu memakai data yang sama.
        if let Some(base) = std::path::absolute(path)?.parent() {
            cfg.drain_file = Some(base.join("draining"));
            cfg.web_dir = base.join(&cfg.web_dir);
            cfg.data_dir = base.join(&cfg.data_dir);
            if let Some(makna) = &mut cfg.makna {
                makna.model = base.join(&makna.model);
                makna.ort_dylib = makna.ort_dylib.as_ref().map(|path| base.join(path));
            }
            if let Some(gambar) = &mut cfg.gambar {
                gambar.model = base.join(&gambar.model);
                gambar.ort_dylib = gambar.ort_dylib.as_ref().map(|path| base.join(path));
            }
            for source in &mut cfg.sources {
                source.path = base.join(&source.path);
            }
        }
        cfg.check_sources()?;
        Ok(cfg)
    }

    fn check_sources(&self) -> Result<()> {
        let mut names = std::collections::HashSet::new();
        for source in &self.sources {
            let name = source.name.trim();
            anyhow::ensure!(!name.is_empty(), "ada [[sources]] tanpa nama");
            anyhow::ensure!(
                names.insert(name.to_lowercase()),
                "nama sumber {name:?} dipakai lebih dari sekali"
            );
        }
        Ok(())
    }

    pub fn require_api_key(&self) -> Result<()> {
        anyhow::ensure!(
            !self.azure.api_key.is_empty(),
            "API key Azure belum ada. Jalankan `rag-hub-server set-azure-key` sekali, atau isi env APP_AZURE_API_KEY."
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_follow_the_config_folder() {
        let dir = std::env::temp_dir().join(format!("rag-hub-config-{}", rand::random::<u32>()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let absolute = std::env::temp_dir().join("sumber-absolut");
        std::fs::write(
            &path,
            format!(
                r#"
bind = "127.0.0.1:0"
web_dir = "web/dist"
data_dir = "../data"

[azure]
endpoint = "https://contoh.openai.azure.com"
deployment = "vision"

[makna]
model = "models/uji"
ort_dylib = "runtime/onnxruntime.dll"

[[sources]]
name = "Review"
path = "sumber/Review"
visibility = {{ type = "admin" }}

[[sources]]
name = "Umum"
path = '{}'
visibility = {{ type = "umum" }}
"#,
                absolute.display()
            ),
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.web_dir, dir.join("web/dist"));
        assert_eq!(cfg.data_dir, dir.join("../data"));
        assert_eq!(cfg.sources[0].path, dir.join("sumber/Review"));
        assert_eq!(cfg.sources[1].path, absolute, "path absolut tidak diubah");
        let makna = cfg.makna.unwrap();
        assert_eq!(makna.model, dir.join("models/uji"));
        assert_eq!(
            makna.ort_dylib.unwrap(),
            dir.join("runtime/onnxruntime.dll")
        );
        assert_eq!(makna.threads, 2);
        assert!(!makna.gpu);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
