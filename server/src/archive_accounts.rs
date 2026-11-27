use crate::{archive, auth, config::Config, db::Db};
use anyhow::{Result, anyhow, bail, ensure};
use rusqlite::{OptionalExtension, Transaction, params};
use std::{collections::BTreeSet, path::Path};

const HELP: &str = "Pakai arsipkan-akun <file.csv>|--semua-lokal [--terapkan] [--config <file>].";

enum Selection {
    All,
    Emails(Vec<String>),
}

#[derive(Default, Debug, PartialEq, Eq)]
struct Counts {
    accounts: usize,
    conversations: usize,
    messages: usize,
    files: usize,
    feedback: usize,
    rejected: usize,
}

pub(crate) async fn cli(args: &[String]) -> Result<()> {
    let Some((selection, flags)) = args.split_first() else {
        bail!("{HELP}");
    };
    if matches!(selection.as_str(), "--help" | "-h") {
        println!("{HELP}");
        return Ok(());
    }
    let mut args = flags.iter();
    let mut config = None;
    let mut apply = false;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--terapkan" => apply = true,
            "--config" if config.is_none() => {
                config = Some(args.next().ok_or_else(|| anyhow!("{HELP}"))?);
            }
            _ => bail!("{HELP}"),
        }
    }
    let cfg = Config::load(Path::new(config.map_or("config.toml", String::as_str)))
        .map_err(|_| anyhow!("Config arsip gagal dibaca atau tidak valid."))?;
    let selection = if selection == "--semua-lokal" {
        Selection::All
    } else {
        ensure!(!selection.starts_with('-'), "{HELP}");
        let mut csv =
            csv::Reader::from_path(selection).map_err(|_| anyhow!("File CSV gagal dibuka."))?;
        let headers = csv
            .headers()
            .map_err(|_| anyhow!("Format CSV tidak valid."))?;
        ensure!(headers.iter().eq(["email"]), "Header CSV harus email.");
        let rows = csv
            .records()
            .map(|r| r.map(|r| r[0].to_owned()))
            .collect::<csv::Result<Vec<_>>>()
            .map_err(|_| anyhow!("Format CSV tidak valid."))?;
        Selection::Emails(rows)
    };
    let dir = cfg.data_dir;
    let db = if apply {
        Db::open_existing(&dir)
    } else {
        Db::open_read_only(&dir)
    }
    .map_err(|_| anyhow!("Database arsip gagal dibuka."))?;
    if apply {
        ensure!(
            db.call(|c| Ok(Db::preserves_user_ids(c)?))
                .await
                .map_err(|_| anyhow!("Skema database arsip gagal diperiksa."))?,
            "Jalankan server versi baru sekali agar database siap diarsipkan."
        );
    }
    let (counts, outcome) = db
        .call(move |c| {
            let tx = c.transaction()?;
            let (accounts, counts) = plan(&tx, selection, &cfg.auth.email_domain)?;
            let outcome = if apply && counts.rejected == 0 && !accounts.is_empty() {
                let dest = dir.join("backup").join(format!(
                    "arsip-akun-{}-{}.sqlite",
                    crate::db::now(),
                    crate::files::new_id()
                ));
                // Kunci tulis SQLite menahan perubahan dari slot lain sejak hitungan sampai commit.
                crate::backup_db(&dir, &dest)
                    .map_err(|_| {
                        anyhow!("Cadangan database gagal dibuat. Tidak ada akun yang dihapus.")
                    })
                    .and_then(|()| {
                        archive_and_delete(&tx, &accounts, &counts)
                            .and_then(|()| Ok(tx.commit()?))
                            .map_err(|_| anyhow!("Pengarsipan gagal. Tidak ada akun yang dihapus."))
                    })
            } else {
                Ok(())
            };
            Ok((counts, outcome))
        })
        .await
        .map_err(|_| anyhow!("Akun untuk arsip gagal diperiksa."))?;
    println!(
        "akun={} percakapan={} pesan={} file={} feedback={} ditolak={}",
        counts.accounts,
        counts.conversations,
        counts.messages,
        counts.files,
        counts.feedback,
        counts.rejected
    );
    ensure!(counts.rejected == 0, "Pilihan akun ditolak.");
    outcome
}

fn plan(
    tx: &Transaction<'_>,
    selection: Selection,
    domain: &str,
) -> Result<(BTreeSet<i64>, Counts)> {
    let linked = if Db::table_exists(tx, "microsoft_identities")? {
        "EXISTS(SELECT 1 FROM microsoft_identities m WHERE m.user_id=u.id)"
    } else {
        "0"
    };
    let eligible = format!("u.id<>?1 AND u.role<>'admin' AND NOT {linked}");
    let mut counts = Counts::default();
    let accounts = match selection {
        Selection::All => tx
            .prepare(&format!("SELECT u.id FROM users u WHERE {eligible}"))?
            .query_map([archive::SYSTEM_ID], |r| r.get(0))?
            .collect::<rusqlite::Result<BTreeSet<_>>>()?,
        Selection::Emails(emails) => {
            let mut accounts = BTreeSet::new();
            let mut lookup = tx.prepare(&format!(
                "SELECT u.id FROM users u WHERE {eligible} AND u.email=?2"
            ))?;
            for email in emails {
                let Ok(email) = auth::normalize_email(&email, domain) else {
                    counts.rejected += 1;
                    continue;
                };
                match lookup
                    .query_row(params![archive::SYSTEM_ID, email], |r| r.get(0))
                    .optional()?
                {
                    Some(id) => {
                        accounts.insert(id);
                    }
                    None => counts.rejected += 1,
                }
            }
            accounts
        }
    };
    counts.accounts = accounts.len();
    let mut count = tx.prepare(
        "SELECT
            (SELECT count(*) FROM conversations WHERE user_id=?1),
            (SELECT count(*) FROM messages m JOIN conversations c ON c.id=m.conversation_id WHERE c.user_id=?1),
            (SELECT count(*) FROM files WHERE user_id=?1 AND (conversation_id IS NOT NULL OR message_id IS NOT NULL)),
            (SELECT count(*) FROM feedback WHERE user_id=?1)",
    )?;
    for id in &accounts {
        let (conversations, messages, files, feedback): (i64, i64, i64, i64) =
            count.query_row([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        counts.conversations += usize::try_from(conversations)?;
        counts.messages += usize::try_from(messages)?;
        counts.files += usize::try_from(files)?;
        counts.feedback += usize::try_from(feedback)?;
    }
    Ok((accounts, counts))
}

fn archive_and_delete(
    tx: &Transaction<'_>,
    accounts: &BTreeSet<i64>,
    counts: &Counts,
) -> Result<()> {
    let mut archived = 0;
    let mut files = 0;
    for id in accounts {
        let mismatched: bool = tx.query_row(
            "SELECT EXISTS (
                SELECT 1 FROM files f JOIN conversations c ON c.id=f.conversation_id
                WHERE (c.user_id=?1 OR f.user_id=?1) AND c.user_id<>f.user_id
            )",
            [id],
            |r| r.get(0),
        )?;
        ensure!(!mismatched, "kepemilikan file dan percakapan berbeda");
    }
    for id in accounts {
        let (moved, moved_files) = archive::archive_conversations(tx, *id, None)?;
        archived += moved;
        files += moved_files;
    }
    ensure!(
        archived == counts.conversations && files == counts.files,
        "jumlah arsip tidak cocok"
    );
    if counts.feedback != 0 {
        archive::ensure_system(tx)?;
    }
    for id in accounts {
        let unfinished: bool = tx.query_row(
            "SELECT EXISTS (
                SELECT 1 FROM conversations WHERE user_id=?1
                UNION ALL
                SELECT 1 FROM files WHERE user_id=?1 AND (conversation_id IS NOT NULL OR message_id IS NOT NULL)
                UNION ALL
                SELECT 1 FROM feedback f JOIN messages m ON m.id=f.message_id
                WHERE f.user_id=?1 AND NOT EXISTS (
                    SELECT 1 FROM archived_conversations a JOIN conversations c ON c.id=a.conversation_id
                    WHERE a.conversation_id=m.conversation_id AND a.original_user_id=?1 AND c.user_id=?2
                )
            )",
            params![id, archive::SYSTEM_ID],
            |r| r.get(0),
        )?;
        ensure!(!unfinished, "riwayat belum seluruhnya diarsipkan");
        tx.execute(
            "UPDATE feedback SET user_id=?2 WHERE user_id=?1 AND message_id IS NULL",
            params![id, archive::SYSTEM_ID],
        )?;
        tx.execute(
            "UPDATE users SET approved_by=NULL WHERE approved_by=?1",
            [id],
        )?;
        // Trigger archive_keep_feedback mengambil feedback yang masih tertaut ke pesan.
        ensure!(
            tx.execute("DELETE FROM users WHERE id=?1", [id])? == 1,
            "akun belum dihapus"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files;
    use rusqlite::Connection;
    use std::fs;

    #[tokio::test]
    async fn dry_run_backup_failure_and_transactional_archive() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("app-arsip-akun-{}", files::new_id()));
        let db = Db::open(&dir)?;
        let config = dir.join("uji.toml");
        fs::write(
            &config,
            r#"data_dir = "."
            [azure]
            endpoint = "https://example.test"
            deployment = "uji"
            api_version = "2024-10-21"
            [auth]
            email_domain = "example.test"
        "#,
        )?;
        let db_path = dir.join("rag-hub.sqlite");
        let legacy = Connection::open(&db_path)?;
        legacy.pragma_update(None, "foreign_keys", true)?;
        legacy.execute_batch(
            "INSERT INTO users (id,email,name,password_hash,role,created_at) VALUES
                (1,'admin@example.test','Admin','','admin',0),
                (2,'tertaut@example.test','B','','user',0),
                (3,'lokal@example.test','C','','user',0),
                (4,'lokal2@example.test','D','','user',0);
             INSERT INTO microsoft_identities VALUES ('tid','oid',2);
             INSERT INTO conversations VALUES ('chat',3,'',0,0), ('kosong',4,'',0,0);
             INSERT INTO messages (id,conversation_id,role,content,created_at) VALUES
                (1,'chat','user','uji',0),(2,'chat','assistant','uji',0);
             INSERT INTO conversation_compactions (conversation_id,through_message_id,content)
                VALUES ('chat',1,'uji');
             INSERT INTO turn_traces (id,question_id,owner,deployment,server_version,started_at_ms,status)
                VALUES ('turn',1,'test','test','test',0,'ok');
             INSERT INTO trace_events VALUES (1,'turn',0,0,'uji',NULL,'{}');
             INSERT INTO feedback (message_id,user_id,rating,shared_answer,created_at) VALUES
                (2,3,'down','uji',0), (NULL,3,'down','uji lawas',0);
             INSERT INTO files VALUES
                ('file',3,'chat',1,'uji.txt','txt',3,'{}',0),
                ('draft',3,NULL,NULL,'draf.txt','txt',3,'{}',0);
             INSERT INTO usage_log (created_at,kind,user_id,message_id) VALUES (0,'chat',3,2);
             INSERT INTO sessions VALUES ('biner-lama',3,0,100);",
        )?;
        db.call(|c| {
            let tx = c.transaction()?;
            archive::ensure_system(&tx)?;
            let (_, counts) = plan(&tx, Selection::All, "example.test")?;
            assert_eq!(
                counts,
                Counts {
                    accounts: 2,
                    conversations: 2,
                    messages: 2,
                    files: 1,
                    feedback: 2,
                    rejected: 0
                }
            );
            tx.commit()?;
            Ok(())
        })
        .await?;
        let files_dir = dir.join("files");
        let file = files::path_of(&files_dir, 3, "file", "txt");
        let draft = files::path_of(&files_dir, 3, "draft", "txt");
        fs::create_dir_all(file.parent().unwrap())?;
        fs::write(&file, b"uji")?;
        fs::write(&draft, b"uji")?;
        let count = |sql| legacy.query_row(sql, [], |r| r.get::<_, i64>(0));
        let run = async |selection: &str, apply: bool| {
            let mut args = vec![
                selection.into(),
                "--config".into(),
                config.to_string_lossy().into_owned(),
            ];
            if apply {
                args.push("--terapkan".into());
            }
            cli(&args).await
        };

        run("--semua-lokal", false).await?;
        assert_eq!(count("SELECT count(*) FROM archived_conversations")?, 0);
        let csv = dir.join("uji.csv");
        fs::write(
            &csv,
            "email\nlokal@example.test\narchive@app.invalid\nadmin@example.test\ntertaut@example.test\nhilang@example.test\n",
        )?;
        assert!(run(&csv.to_string_lossy(), true).await.is_err());
        assert!(!dir.join("backup").exists());

        fs::write(dir.join("backup"), b"jalan buntu")?;
        assert!(run("--semua-lokal", true).await.is_err());
        assert_eq!(count("SELECT count(*) FROM users WHERE id IN (3,4)")?, 2);
        fs::remove_file(dir.join("backup"))?;

        legacy.execute_batch(
            "CREATE TRIGGER gagal BEFORE DELETE ON users WHEN OLD.id=4
             BEGIN SELECT RAISE(ABORT,'uji batal'); END;",
        )?;
        assert!(run("--semua-lokal", true).await.is_err());
        assert_eq!(count("SELECT count(*) FROM users WHERE id IN (3,4)")?, 2);
        assert_eq!(count("SELECT count(*) FROM archived_conversations")?, 0);
        assert_eq!(count("SELECT count(*) FROM files WHERE user_id=3")?, 2);
        legacy.execute_batch("DROP TRIGGER gagal;")?;

        run("--semua-lokal", true).await?;
        assert_eq!(count("PRAGMA user_version")?, 10);
        assert_eq!(count("SELECT count(*) FROM users WHERE id IN (3,4)")?, 0);
        assert_eq!(count("SELECT count(*) FROM users WHERE id IN (1,2)")?, 2);
        assert_eq!(
            count("SELECT count(*) FROM conversations WHERE user_id=-1")?,
            2
        );
        assert_eq!(
            count("SELECT count(*) FROM archived_conversations WHERE original_user_id IN (3,4)")?,
            2
        );
        assert_eq!(count("SELECT count(*) FROM messages")?, 2);
        assert_eq!(
            count("SELECT count(*) FROM files WHERE user_id=-1 AND id='file'")?,
            1
        );
        assert_eq!(count("SELECT count(*) FROM files WHERE id='draft'")?, 0);
        assert_eq!(count("SELECT count(*) FROM trace_events")?, 1);
        assert_eq!(count("SELECT count(*) FROM conversation_compactions")?, 1);
        assert_eq!(
            count("SELECT count(*) FROM feedback WHERE user_id=-1 AND shared_answer IS NOT NULL")?,
            2
        );
        assert_eq!(
            count("SELECT count(*) FROM usage_log WHERE user_id=3 AND message_id=2")?,
            1
        );
        assert_eq!(count("SELECT count(*) FROM sessions WHERE user_id=3")?, 0);
        assert_eq!(fs::read(file)?, b"uji");
        assert_eq!(fs::read(draft)?, b"uji");

        let backups = fs::read_dir(dir.join("backup"))?.collect::<std::io::Result<Vec<_>>>()?;
        assert_eq!(backups.len(), 2);
        for backup in backups {
            let snapshot = Connection::open(backup.path())?;
            assert_eq!(
                snapshot.query_row("SELECT count(*) FROM users WHERE id IN (3,4)", [], |r| r
                    .get::<_, i64>(0))?,
                2
            );
            assert_eq!(
                snapshot.query_row("SELECT count(*) FROM archived_conversations", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
        }
        // INSERT dari bd00a2c tetap memakai ID implicit dan tidak memakai ulang ID arsip.
        legacy.execute(
            "INSERT INTO users (email,name,password_hash,status,created_at)
                         VALUES ('lokal@example.test','Akun baru','','pending',0)",
            [],
        )?;
        assert!(legacy.last_insert_rowid() > 4);
        assert_eq!(
            count(
                "SELECT count(*) FROM conversations WHERE user_id=(SELECT id FROM users WHERE email='lokal@example.test')"
            )?,
            0
        );
        assert_eq!(count("SELECT count(*) FROM pragma_foreign_key_check")?, 0);
        drop((legacy, db));
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
