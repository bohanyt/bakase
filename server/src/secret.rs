//! API key Azure di Windows Credential Manager: terenkripsi per user Windows, jadi
//! tidak perlu diketik ulang setiap server dijalankan dan tidak ikut terbawa ke
//! environment program lain seperti env var permanen.

/// Nama credential yang tampil di Credential Manager.
#[cfg(windows)]
const TARGET: &str = "rag-hub:azure-api-key";

#[cfg(windows)]
fn entry() -> anyhow::Result<keyring_core::Entry> {
    use std::collections::HashMap;
    keyring_core::set_default_store(windows_native_keyring_store::Store::new()?);
    // Local: hanya di mesin ini, tidak ikut roaming profile domain ke komputer lain.
    let modifiers = HashMap::from([("target", TARGET), ("persistence", "Local")]);
    Ok(keyring_core::Entry::new_with_modifiers(
        "rag-hub",
        "azure-api-key",
        &modifiers,
    )?)
}

/// Key yang tersimpan, atau `None` kalau belum pernah disimpan.
pub fn load_azure_key() -> anyhow::Result<Option<String>> {
    #[cfg(windows)]
    {
        match entry()?.get_password() {
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
        let entry = entry()?;
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
        match entry()?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring_core::Error::NoEntry) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
    #[cfg(not(windows))]
    Ok(false)
}
