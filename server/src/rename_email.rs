use crate::{archive, auth, config::Config, db::Db};
use anyhow::{Result, anyhow, bail, ensure};
use rusqlite::{OptionalExtension, Transaction, params};
use std::{collections::HashSet, path::Path};

const HELP: &str = "Pakai ganti-email <file.csv> [--terapkan] [--config <file>].";

pub async fn cli(args: &[String]) -> Result<()> {
    let Some((file, options)) = args.split_first() else {
        bail!("{HELP}");
    };
    ensure!(!file.starts_with('-'), "{HELP}");
    let mut cfg_path = None;
    let mut apply = false;
    let mut options = options.iter();
    while let Some(flag) = options.next() {
        match flag.as_str() {
            "--terapkan" if !apply => apply = true,
            "--config" if cfg_path.is_none() => {
                cfg_path = Some(options.next().ok_or_else(|| anyhow!("{HELP}"))?)
            }
            _ => bail!("{HELP}"),
        }
    }
    let cfg = Config::load(Path::new(cfg_path.map_or("config.toml", String::as_str)))
        .map_err(|_| anyhow!("Config gagal dibaca atau tidak valid."))?;
    let file = std::fs::File::open(file).map_err(|_| anyhow!("File CSV gagal dibuka."))?;
    let mut reader = csv::Reader::from_reader(file);
    let headers = reader
        .headers()
        .map_err(|_| anyhow!("Format CSV tidak valid."))?;
    ensure!(
        headers.iter().eq(["lama", "baru"]),
        "Header CSV harus lama,baru."
    );
    let rows = reader
        .records()
        .collect::<csv::Result<Vec<_>>>()
        .map_err(|_| anyhow!("Format CSV tidak valid."))?;
    ensure!(
        cfg.data_dir.join("rag-hub.sqlite").is_file(),
        "Database belum ada."
    );
    let db = if apply {
        Db::open(&cfg.data_dir)
    } else {
        Db::open_read_only(&cfg.data_dir)
    }
    .map_err(|_| anyhow!("Database gagal dibuka."))?;
    let total = rows.len();
    let (planned, rejected) = db
        .call(move |db| {
            let tx = db.transaction()?;
            let counts = rename(&tx, &rows, &cfg.auth.email_domain, apply)?;
            if apply {
                tx.commit()?;
            }
            Ok(counts)
        })
        .await
        .map_err(|_| anyhow!("Perubahan email gagal. Periksa database dan coba lagi."))?;
    println!(
        "baris={total} cocok={planned} ditolak={rejected} diubah={}",
        if apply && rejected == 0 { planned } else { 0 }
    );
    ensure!(rejected == 0, "Berkas ditolak.");
    Ok(())
}

fn rename(
    tx: &Transaction<'_>,
    rows: &[csv::StringRecord],
    domain: &str,
    apply: bool,
) -> rusqlite::Result<(usize, usize)> {
    let mut planned = Vec::new();
    let aliases = Db::table_exists(tx, "microsoft_addresses")?;
    let mut old_seen = HashSet::new();
    let mut new_seen = HashSet::new();
    for row in rows {
        let parsed = auth::normalize_email(&row[0], domain)
            .and_then(|old| auth::normalize_email(&row[1], domain).map(|new| (old, new)));
        let Ok((old, new)) = parsed else { continue };
        if old == new
            || new == archive::SYSTEM_EMAIL
            || !old_seen.insert(old.clone())
            || !new_seen.insert(new.clone())
        {
            continue;
        }
        let id = tx
            .query_row(
                "SELECT id FROM users WHERE email = ?1 AND id <> ?2",
                params![old, archive::SYSTEM_ID],
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        let occupied = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM users WHERE email = ?1 UNION ALL
                            SELECT 1 FROM imported_access WHERE email = ?1)",
            [&new],
            |r| r.get::<_, bool>(0),
        )?;
        let occupied = occupied
            || (aliases
                && tx.query_row(
                    "SELECT EXISTS (
                SELECT 1 FROM microsoft_addresses WHERE email = ?1 AND user_id IS NOT ?2
                UNION ALL
                SELECT 1 FROM imported_access_bans WHERE email = ?1
             )",
                    params![new, id],
                    |r| r.get(0),
                )?);
        if let Some(id) = id.filter(|_| !occupied) {
            planned.push((id, old, new));
        }
    }
    let rejected = rows.len() - planned.len();
    if apply && rejected == 0 {
        for (id, old, new) in &planned {
            tx.execute(
                "UPDATE users SET email = ?1 WHERE id = ?2",
                params![new, id],
            )?;
            tx.execute(
                "UPDATE imported_access SET email = ?1 WHERE email = ?2",
                params![new, old],
            )?;
            if aliases {
                tx.execute(
                    "UPDATE imported_access_bans SET email = ?1 WHERE email = ?2",
                    params![new, old],
                )?;
            }
            tx.execute("DELETE FROM sessions WHERE user_id = ?1", [id])?;
        }
    }
    Ok((planned.len(), rejected))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dry_run_rejections_and_atomic_change_keep_history_and_identity() {
        Db::open_in_memory().unwrap().call(|db| {
            db.execute_batch(
                "INSERT INTO users (id,email,name,password_hash,status,role,department,created_at) VALUES
                 (1,'old@example.test','Tes','hash','active','admin','Review',0),
                 (2,'taken@example.test','Tes','hash','active','user',NULL,0);
                 INSERT INTO microsoft_identities VALUES ('tenant','oid',1);
                 INSERT INTO sessions VALUES ('hash',1,0,100);
                 INSERT INTO conversations VALUES ('history',1,'Judul',0,0);
                 INSERT INTO imported_access VALUES ('old@example.test','[]'),('import@example.test','[]');"
            )?;
            let tx = db.transaction()?;
            let valid = csv::StringRecord::from(vec!["OLD@example.test", "new@example.test"]);
            assert_eq!(rename(&tx, std::slice::from_ref(&valid), "example.test", false)?, (1, 0));
            let attempted = [
                valid.clone(),
                csv::StringRecord::from(vec!["old@example.test", "other@example.test"]),
                csv::StringRecord::from(vec!["taken@example.test", "new@example.test"]),
                csv::StringRecord::from(vec!["taken@example.test", "old@example.test"]),
                csv::StringRecord::from(vec!["old@example.test", "new@example.org"]),
                csv::StringRecord::from(vec!["old@example.test", "import@example.test"]),
            ];
            assert_eq!(rename(&tx, &attempted, "example.test", true)?, (1, 5));
            let archive = csv::StringRecord::from(vec![archive::SYSTEM_EMAIL, "new@app.invalid"]);
            assert_eq!(rename(&tx, &[archive], "app.invalid", true)?, (0, 1));
            assert_eq!(tx.query_row("SELECT email FROM users WHERE id=1", [], |r| r.get::<_, String>(0))?, "old@example.test");
            assert_eq!(rename(&tx, &[valid], "example.test", true)?, (1, 0));
            let snapshot: (String, String, String, i64, i64, i64, i64) = tx.query_row(
                "SELECT email,role,department,(SELECT user_id FROM conversations),
                  (SELECT user_id FROM microsoft_identities), (SELECT COUNT(*) FROM sessions),
                  (SELECT COUNT(*) FROM imported_access WHERE email = 'new@example.test')
                 FROM users WHERE id=1", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)),
            )?;
            assert_eq!(snapshot, ("new@example.test".into(), "admin".into(), "Review".into(), 1, 1, 0, 1));
            Ok(())
        }).await.unwrap();
    }
}
