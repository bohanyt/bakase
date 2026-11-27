//! API key Azure di Windows Credential Manager: terenkripsi per user Windows, jadi
//! tidak perlu diketik ulang setiap server dijalankan dan tidak ikut terbawa ke
//! environment program lain seperti env var permanen.

use crate::{config::Config, microsoft::Settings};
use anyhow::{Result, anyhow, ensure};

/// Nama credential yang tampil di Credential Manager.
#[cfg(windows)]
const TARGET: &str = "rag-hub:azure-api-key";

#[cfg(windows)]
fn entry(name: &str, target: &str) -> Result<keyring_core::Entry> {
    use std::collections::HashMap;
    keyring_core::set_default_store(windows_native_keyring_store::Store::new()?);
    // Local: hanya di mesin ini, tidak ikut roaming profile domain ke komputer lain.
    let modifiers = HashMap::from([("target", target), ("persistence", "Local")]);
    Ok(keyring_core::Entry::new_with_modifiers(
        "rag-hub", name, &modifiers,
    )?)
}

#[cfg(windows)]
fn azure_entry() -> Result<keyring_core::Entry> {
    entry("azure-api-key", TARGET)
}

#[cfg(windows)]
fn microsoft_entry(settings: &Settings) -> Result<keyring_core::Entry> {
    let name = format!(
        "microsoft-client-secret:{}:{}",
        settings.tenant_id, settings.client_id
    );
    entry(&name, &format!("rag-hub:{name}"))
}

/// Key yang tersimpan, atau `None` kalau belum pernah disimpan.
pub fn load_azure_key() -> anyhow::Result<Option<String>> {
    #[cfg(windows)]
    {
        match azure_entry()?.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    #[cfg(not(windows))]
    Ok(None)
}

pub fn save_azure_key(key: &str) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        let entry = azure_entry()?;
        entry.set_password(key)?;
        anyhow::ensure!(
            entry.get_password()? == key,
            "Key tersimpan tidak sama dengan yang dimasukkan."
        );
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = key;
        anyhow::bail!("Penyimpanan key hanya ada di Windows. Pakai env APP_AZURE_API_KEY.")
    }
}

/// `true` kalau ada key yang dihapus.
pub fn delete_azure_key() -> anyhow::Result<bool> {
    #[cfg(windows)]
    {
        match azure_entry()?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring_core::Error::NoEntry) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
    #[cfg(not(windows))]
    Ok(false)
}

pub fn load_microsoft_secret(settings: &Settings) -> Result<Option<String>> {
    #[cfg(windows)]
    {
        match microsoft_entry(settings)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = settings;
        Ok(None)
    }
}

pub fn set_microsoft_secret(args: &[String]) -> Result<()> {
    let help = "Pakai set-microsoft-secret <file.env> [--config <file>].";
    let (file, path) = match args {
        [file] if !file.starts_with('-') => (file, "config.toml"),
        [file, flag, path] if !file.starts_with('-') && flag == "--config" => (file, path.as_str()),
        _ => anyhow::bail!("{help}"),
    };
    let cfg = Config::load(std::path::Path::new(path))
        .map_err(|_| anyhow!("Config Microsoft gagal dibaca atau tidak valid."))?;
    let settings = cfg
        .auth
        .microsoft
        .ok_or_else(|| anyhow!("Isi bagian auth.microsoft."))?;
    settings.check_registration()?;
    let contents = std::fs::read_to_string(file)
        .map_err(|_| anyhow!("File env tidak bisa dibaca sebagai UTF-8."))?;
    let secret = microsoft_secret_from_env(&contents, &settings)?;
    store_microsoft_secret(&settings, secret)?;
    println!("Secret Microsoft tersimpan di Windows Credential Manager.");
    Ok(())
}

fn microsoft_secret_from_env<'a>(text: &'a str, settings: &Settings) -> Result<&'a str> {
    use std::collections::HashMap;
    const ID: &str = "MICROSOFT_CLIENT_ID";
    const TENANT: &str = "MICROSOFT_TENANT_ID";
    const CALLBACK: &str = "MICROSOFT_CALLBACK_URI";
    const SECRET: &str = "MICROSOFT_CLIENT_SECRET";
    let mut values = HashMap::new();
    for line in text.trim_start_matches('\u{feff}').lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, value) = line
            .split_once('=')
            .ok_or_else(|| anyhow!("Format file env tidak valid."))?;
        let name = name.trim();
        if !matches!(name, ID | TENANT | CALLBACK | SECRET) {
            continue;
        }
        let value = value.trim();
        let value = ['"', '\'']
            .into_iter()
            .find_map(|quote| {
                value
                    .strip_prefix(quote)
                    .and_then(|value| value.strip_suffix(quote))
            })
            .unwrap_or(value);
        ensure!(
            !value.is_empty() && !value.contains(['"', '\'']),
            "Nilai Microsoft di file env kosong atau tidak valid."
        );
        ensure!(
            values.insert(name, value).is_none(),
            "Ada nama konfigurasi Microsoft ganda di file env."
        );
    }
    let get = |key| {
        values
            .get(key)
            .copied()
            .ok_or_else(|| anyhow!("{key} belum ada di file env."))
    };
    ensure!(
        get(ID)?.parse::<uuid::Uuid>().ok() == Some(settings.client_id),
        "{ID} berbeda dari config."
    );
    ensure!(
        get(TENANT)?.parse::<uuid::Uuid>().ok() == Some(settings.tenant_id),
        "{TENANT} berbeda dari config."
    );
    ensure!(
        get(CALLBACK)? == settings.redirect_uri,
        "{CALLBACK} berbeda dari config."
    );
    get(SECRET)
}

fn store_microsoft_secret(settings: &Settings, secret: &str) -> Result<()> {
    #[cfg(windows)]
    {
        let failed = || {
            anyhow!(
                "Credential Manager gagal. Jalankan dalam sesi konsol atau RDP dengan akun Windows milik service."
            )
        };
        let entry = microsoft_entry(settings).map_err(|_| failed())?;
        entry.set_password(secret).map_err(|_| failed())?;
        let stored = entry.get_password().map_err(|_| failed())?;
        ensure!(
            stored == secret,
            "Credential Manager tidak mengembalikan secret yang disimpan."
        );
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (settings, secret);
        anyhow::bail!("Penyimpanan secret Microsoft memerlukan Windows Credential Manager.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_checks_registration_before_returning_secret() {
        let settings = Settings {
            tenant_id: uuid::Uuid::from_u128(1),
            client_id: uuid::Uuid::from_u128(2),
            redirect_uri: "https://example.test/api/auth/callback/microsoft".into(),
            ..Settings::default()
        };
        let file = format!(
            "\u{feff}# Fixture\r\nMICROSOFT_TENANT_ID={}\r\nMICROSOFT_CLIENT_ID={}\r\nMICROSOFT_CALLBACK_URI={}\r\nMICROSOFT_CLIENT_SECRET='uji=rahasia'\r\n",
            settings.tenant_id, settings.client_id, settings.redirect_uri
        );
        assert!(
            microsoft_secret_from_env(&file, &settings).is_ok_and(|secret| secret == "uji=rahasia")
        );
        for (from, to) in [
            (
                settings.tenant_id.to_string(),
                uuid::Uuid::from_u128(3).to_string(),
            ),
            (
                settings.client_id.to_string(),
                uuid::Uuid::from_u128(4).to_string(),
            ),
            (
                settings.redirect_uri.clone(),
                "https://elsewhere.test/callback".into(),
            ),
            ("'uji=rahasia'".into(), "".into()),
        ] {
            let error = microsoft_secret_from_env(&file.replace(&from, &to), &settings)
                .unwrap_err()
                .to_string();
            assert!(!error.contains("uji=rahasia"));
        }
    }
}
