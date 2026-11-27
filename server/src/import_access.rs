use crate::{access, archive, auth, config::Config, db::Db, microsoft};
use anyhow::{Result, anyhow, bail, ensure};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Deserialize)]
struct Export {
    users: Vec<ExportUser>,
}

#[derive(Deserialize)]
struct ExportUser {
    email: String,
    apps: Vec<String>,
    banned: bool,
}

#[derive(Default)]
struct Entry {
    spellings: BTreeSet<String>,
    departments: Vec<String>,
    banned: bool,
}

#[derive(Default, Clone, Copy)]
struct Options {
    apply: bool,
    revoke_absent: bool,
    confirm: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Counts {
    matched: usize,
    unmatched: usize,
    changed: usize,
    case_duplicates: usize,
    ignored_emails: usize,
    unknown_apps: usize,
    banned: usize,
    linked: usize,
    absent: usize,
    absent_with_access: usize,
    losing_access: usize,
    stale: usize,
}

impl Counts {
    fn needs_confirmation(&self) -> bool {
        self.matched + self.unmatched == 0 || self.losing_access > self.linked / 2
    }
}

const HELP: &str = "Pakai impor-akses <file.json> --peta APLIKASI=DEPARTEMEN [--peta ...] [--terapkan] [--cabut-yang-absen] [--konfirmasi-impor] [--config <file>].";

pub(crate) async fn cli(args: &[String]) -> Result<()> {
    let Some((file, flags)) = args.split_first() else {
        bail!("{HELP}");
    };
    if matches!(file.as_str(), "--help" | "-h") {
        println!("{HELP}");
        return Ok(());
    }
    ensure!(!file.starts_with('-'), "{HELP}");
    let mut args = flags.iter();
    let mut config = None;
    let mut pairs = Vec::new();
    let mut options = Options::default();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--terapkan" => options.apply = true,
            "--cabut-yang-absen" => options.revoke_absent = true,
            "--konfirmasi-impor" => options.confirm = true,
            "--config" if config.is_none() => {
                config = Some(PathBuf::from(args.next().ok_or_else(|| anyhow!("{HELP}"))?));
            }
            "--peta" => pairs.push(args.next().ok_or_else(|| anyhow!("{HELP}"))?),
            _ => bail!("{HELP}"),
        }
    }
    ensure!(!pairs.is_empty(), "{HELP}");
    let config = config.unwrap_or_else(|| "config.toml".into());
    let cfg = Config::load(&config)
        .map_err(|_| anyhow!("Config impor gagal dibaca atau tidak valid."))?;
    let mut mapping = BTreeMap::new();
    for pair in pairs {
        let (app, department) = pair.split_once('=').ok_or_else(|| anyhow!("{HELP}"))?;
        let app = app.trim();
        ensure!(!app.is_empty(), "{HELP}");
        let department = auth::canonical_department(department, &cfg.auth.departments)
            .map_err(|_| anyhow!("Departemen pada peta harus tercantum di config."))?;
        if let Some(previous) = mapping.insert(app.to_string(), department.clone()) {
            ensure!(
                previous == department,
                "Satu aplikasi dipetakan ke dua departemen."
            );
        }
    }
    let bytes = std::fs::read(file).map_err(|_| anyhow!("File impor tidak bisa dibaca."))?;
    let export: Export =
        serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes))
            .map_err(|_| anyhow!("Format JSON akses tidak valid."))?;
    ensure!(
        cfg.data_dir.join("rag-hub.sqlite").is_file(),
        "Database impor belum ada."
    );
    let db = if options.apply {
        Db::open_existing(&cfg.data_dir)
    } else {
        Db::open_read_only(&cfg.data_dir)
    }
    .map_err(|_| anyhow!("Database impor gagal dibuka."))?;
    let counts = db
        .call(move |c| {
            let tx = c.transaction()?;
            let counts = import(&tx, export, &cfg.auth.email_domain, &mapping, options)?;
            if options.apply && (options.confirm || !counts.needs_confirmation()) {
                tx.commit()?;
            }
            Ok(counts)
        })
        .await
        .map_err(|_| anyhow!("Impor akses gagal. Periksa database dan coba lagi."))?;

    println!(
        "cocok={} tidak_cocok={} berubah={} email_dobel_beda_huruf={} email_diabaikan={} aplikasi_tidak_dipetakan={} banned={} tertaut={} absen={} absen_berakses={} kehilangan_akses={} impor_usang={}",
        counts.matched,
        counts.unmatched,
        counts.changed,
        counts.case_duplicates,
        counts.ignored_emails,
        counts.unknown_apps,
        counts.banned,
        counts.linked,
        counts.absent,
        counts.absent_with_access,
        counts.losing_access,
        counts.stale,
    );
    ensure!(
        !options.apply || options.confirm || !counts.needs_confirmation(),
        "Impor ditolak. Periksa ringkasan sebelum memakai --konfirmasi-impor."
    );
    Ok(())
}

#[derive(Default)]
struct Desired {
    priority: u8,
    departments: Vec<String>,
    banned: bool,
}

fn import(
    tx: &Transaction<'_>,
    export: Export,
    domain: &str,
    mapping: &BTreeMap<String, String>,
    options: Options,
) -> Result<Counts> {
    let mut counts = Counts::default();
    let mut entries = BTreeMap::<String, Entry>::new();
    for row in export.users {
        let Ok(email) = auth::normalize_email(&row.email, domain) else {
            counts.ignored_emails += 1;
            continue;
        };
        if email == archive::SYSTEM_EMAIL {
            counts.ignored_emails += 1;
            continue;
        }
        let entry = entries.entry(email).or_default();
        if entry.spellings.insert(row.email.trim().to_string()) && entry.spellings.len() > 1 {
            counts.case_duplicates += 1;
        }
        entry.banned |= row.banned;
        for app in row.apps {
            if let Some(department) = mapping.get(app.trim()) {
                entry.departments.push(department.clone());
            } else {
                counts.unknown_apps += 1;
            }
        }
    }

    let initialized = access::initialized(tx)?;
    let aliases = Db::table_exists(tx, "microsoft_addresses")?;
    let mut primary = tx.prepare("SELECT id FROM users WHERE email = ?1")?;
    let mut alias = aliases
        .then(|| tx.prepare("SELECT user_id, is_mail FROM microsoft_addresses WHERE email = ?1"))
        .transpose()?;
    let mut desired = BTreeMap::<i64, Desired>::new();
    let mut pending = BTreeMap::new();
    let mut bans = BTreeSet::new();
    let mut valid = BTreeSet::new();
    for (email, mut entry) in entries {
        let by_primary: Option<i64> = primary.query_row([&email], |r| r.get(0)).optional()?;
        if by_primary == Some(archive::SYSTEM_ID) {
            counts.ignored_emails += 1;
            continue;
        }
        let by_alias: Option<(i64, bool)> = match &mut alias {
            Some(alias) => alias
                .query_row([&email], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?,
            None => None,
        };
        valid.insert(email.clone());
        if entry.banned {
            counts.banned += 1;
            bans.insert(email.clone());
            entry.departments.clear();
        }
        access::sort_departments(&mut entry.departments);
        let id = by_primary.or(by_alias.map(|(id, _)| id));
        if let Some(id) = id {
            counts.matched += 1;
            let priority = if by_alias == Some((id, true)) {
                3
            } else if by_primary.is_some() {
                2
            } else {
                1
            };
            let account = desired.entry(id).or_default();
            account.banned |= entry.banned;
            if priority > account.priority {
                account.priority = priority;
                account.departments = entry.departments;
            }
        } else {
            counts.unmatched += 1;
            pending.insert(email, entry.departments);
        }
    }

    let linked = if Db::table_exists(tx, "microsoft_identities")? {
        let mut rows = tx.prepare("SELECT user_id FROM microsoft_identities")?;
        rows.query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<BTreeSet<i64>>>()?
    } else {
        BTreeSet::new()
    };
    counts.linked = linked.len();
    let select = if initialized {
        "SELECT departments FROM users_with_access WHERE id = ?1"
    } else {
        "SELECT CASE WHEN department IS NULL THEN '[]' ELSE json_array(department) END
         FROM users WHERE id = ?1"
    };
    let mut current = tx.prepare(select)?;
    let mut changes = Vec::new();
    for id in linked
        .iter()
        .copied()
        .chain(desired.keys().copied().filter(|id| !linked.contains(id)))
    {
        let old = current.query_row([id], |r| access::from_row(r, 0))?;
        let new = match desired.get(&id) {
            Some(entry) if !entry.banned => &entry.departments[..],
            Some(_) => &[],
            None => {
                counts.absent += 1;
                counts.absent_with_access += usize::from(!old.is_empty());
                if !options.revoke_absent {
                    continue;
                }
                &[]
            }
        };
        if old != new {
            counts.changed += 1;
            if linked.contains(&id)
                && old
                    .iter()
                    .any(|department| !new.iter().any(|d| d.eq_ignore_ascii_case(department)))
            {
                counts.losing_access += 1;
            }
            changes.push((id, new));
        }
    }

    // Siapkan daftar perubahan sebelum menyentuh skema maupun data.
    let mut remove = Vec::new();
    if initialized {
        let mut rows = tx.prepare("SELECT email, departments FROM imported_access")?;
        for row in rows.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, access::from_row(r, 1)?))
        })? {
            let (email, old) = row?;
            let email_key = email.to_ascii_lowercase();
            if let Some(new) = pending.get(&email_key) {
                if *new == old {
                    pending.remove(&email_key);
                }
            } else {
                counts.stale += usize::from(!valid.contains(&email_key));
                remove.push(email);
            }
        }
    }
    counts.changed += pending.len() + remove.len();
    if options.apply && (options.confirm || !counts.needs_confirmation()) {
        access::init(tx)?;
        tx.execute_batch(microsoft::SCHEMA)?;
        tx.execute_batch(microsoft::ADDRESS_SCHEMA)?;
        for (id, departments) in changes {
            access::replace(tx, id, departments)?;
        }
        for email in remove {
            tx.execute("DELETE FROM imported_access WHERE email = ?1", [email])?;
        }
        for (email, departments) in pending {
            tx.execute(
                "INSERT INTO imported_access (email, departments) VALUES (?1, ?2)
                 ON CONFLICT(email) DO UPDATE SET departments = excluded.departments",
                params![email, serde_json::to_string(&departments)?],
            )?;
        }
        tx.execute("DELETE FROM imported_access_bans", [])?;
        for email in bans {
            tx.execute("INSERT INTO imported_access_bans VALUES (?1)", [email])?;
        }
    }
    Ok(counts)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn aliases_snapshot_revocation_and_guard() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            c.execute_batch(
                "INSERT INTO users (id,email,name,password_hash,status,department,created_at) VALUES
                    (1,'masuk@example.test','A','','active','Review',0),
                    (2,'lama@example.test','B','','active','Review',0),
                    (3,'lokal@example.test','C','','active','Review',0);
                 INSERT INTO microsoft_identities VALUES ('t','a',1), ('t','b',2);
                 INSERT INTO microsoft_addresses VALUES
                    ('masuk@example.test',1,0), ('surat@example.test',1,1);
                 INSERT INTO imported_access VALUES ('usang@example.test','[\"Review\"]');",
            )?;
            let run = |c: &mut rusqlite::Connection, json, options| -> Result<Counts> {
                let tx = c.transaction()?;
                let counts = import(
                    &tx,
                    serde_json::from_str(json)?,
                    "example.test",
                    &BTreeMap::from([
                        ("Q".into(), "Review".into()),
                        ("P".into(), "Operasional".into()),
                    ]),
                    options,
                )?;
                // Deliberately commit even the dry and blocked cases so stray writes cannot hide.
                tx.commit()?;
                Ok(counts)
            };
            let current = |c: &rusqlite::Connection, id| {
                c.query_row(
                    "SELECT departments FROM users_with_access WHERE id=?1",
                    [id],
                    |r| access::from_row(r, 0),
                )
            };
            let apply = Options {
                apply: true,
                ..Options::default()
            };
            let two = r#"{"users":[
                {"email":"masuk@example.test","apps":["Q"],"banned":false},
                {"email":"surat@example.test","apps":["P","Q"],"banned":false}
            ]}"#;
            let dry = run(c, two, Options::default())?;
            assert_eq!(
                (
                    dry.matched,
                    dry.changed,
                    dry.linked,
                    dry.absent,
                    dry.absent_with_access,
                    dry.stale
                ),
                (2, 2, 2, 1, 1, 1)
            );
            assert_eq!(current(c, 1)?, ["Review"]);
            assert_eq!(run(c, two, apply)?, dry);
            assert_eq!(current(c, 1)?, ["Operasional", "Review"]);
            assert_eq!(current(c, 2)?, ["Review"]);
            let blocked = r#"{"users":[
                {"email":"masuk@example.test","apps":[],"banned":true},
                {"email":"surat@example.test","apps":["P","Q"],"banned":false}
            ]}"#;
            let revoke = Options {
                revoke_absent: true,
                ..apply
            };
            let rejected = run(c, blocked, revoke)?;
            assert!(rejected.needs_confirmation());
            assert_eq!(rejected.losing_access, 2);
            assert_eq!(current(c, 1)?, ["Operasional", "Review"]);
            assert_eq!(current(c, 2)?, ["Review"]);
            let confirmed = Options {
                confirm: true,
                ..revoke
            };
            assert_eq!(run(c, blocked, confirmed)?, rejected);
            assert!(current(c, 1)?.is_empty());
            assert!(current(c, 2)?.is_empty());
            assert_eq!(current(c, 3)?, ["Review"]);
            assert_eq!(
                c.query_row("SELECT count(*) FROM imported_access", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );

            let empty = r#"{"users":[{"email":"invalid","apps":["Q"],"banned":false}]}"#;
            assert!(run(c, empty, apply)?.needs_confirmation());
            assert_eq!(
                c.query_row("SELECT count(*) FROM imported_access_bans", [], |r| r
                    .get::<_, i64>(0))?,
                1
            );
            run(c, empty, confirmed)?;
            assert_eq!(
                c.query_row("SELECT count(*) FROM imported_access_bans", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
            Ok(())
        })
        .await
        .unwrap();
    }

    #[test]
    fn dry_run_without_access_or_identity_tables() -> Result<()> {
        let mut c = rusqlite::Connection::open_in_memory()?;
        c.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT COLLATE NOCASE, department TEXT);
             INSERT INTO users VALUES (1,'uji@example.test','Review');",
        )?;
        c.pragma_update(None, "query_only", true)?;
        let tx = c.transaction()?;
        let export = serde_json::from_str(
            r#"{"users":[
            {"email":"uji@example.test","apps":[],"banned":false},
            {"email":"baru@example.test","apps":[],"banned":false}
        ]}"#,
        )?;
        let counts = import(
            &tx,
            export,
            "example.test",
            &BTreeMap::new(),
            Options::default(),
        )?;
        assert_eq!(
            (counts.matched, counts.unmatched, counts.changed),
            (1, 1, 2)
        );
        assert_eq!(
            tx.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r
                .get::<_, i64>(0))?,
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn dry_run_and_apply_share_counts_without_touching_roles_or_status() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            c.execute_batch(
                "INSERT INTO users (id,email,name,password_hash,status,role,department,created_at) VALUES
                 (1,'multi@example.test','Multi','x','active','user','Review',0),
                 (2,'pending@example.test','Pending','x','pending','user',NULL,0),
                 (3,'disabled@example.test','Disabled','x','disabled','user','Operasional',0),
                 (4,'banned@example.test','Banned','x','active','admin','Review',0),
                 (-1,'arsip@example.test','Arsip','','disabled','user',NULL,0);
                 INSERT INTO sessions VALUES ('multi',1,0,100), ('banned',4,0,100);
                 INSERT INTO imported_access VALUES ('staged-banned@example.test','[\"Review\"]');",
            )?;
            let mapping = BTreeMap::from([
                ("Review_APP".to_string(), "Review".to_string()),
                ("PRODUCTION_APP".to_string(), "Operasional".to_string()),
                ("EXAMPLE_AI".to_string(), "Contoh".to_string()),
            ]);
            let export = || serde_json::from_str(r#"{"users":[
                {"email":"multi@example.test","apps":["Review_APP","REPORTS_APP"],"banned":false},
                {"email":"Multi@EXAMPLE.TEST","apps":["PRODUCTION_APP","EXAMPLE_AI"],"banned":false},
                {"email":"multi@example.test","apps":["Review_APP"],"banned":false},
                {"email":"pending@example.test","apps":["EXAMPLE_AI"],"banned":false},
                {"email":"disabled@example.test","apps":["PRODUCTION_APP"],"banned":false},
                {"email":"banned@example.test","apps":["Review_APP"],"banned":false},
                {"email":"BANNED@example.test","apps":["PRODUCTION_APP"],"banned":true},
                {"email":"new@example.test","apps":["EXAMPLE_AI"],"banned":false},
                {"email":"staged-banned@example.test","apps":["Review_APP"],"banned":true},
                {"email":"arsip@example.test","apps":[],"banned":false},
                {"email":"archive@app.invalid","apps":["Review_APP"],"banned":false},
                {"email":"lain@example.test.evil","apps":["Review_APP"],"banned":false}
            ]}"#).unwrap();
            let expected = Counts {
                matched: 4,
                unmatched: 2,
                changed: 5,
                case_duplicates: 2,
                ignored_emails: 3,
                unknown_apps: 1,
                banned: 2,
                ..Counts::default()
            };
            let read = |c: &rusqlite::Connection, id| {
                c.query_row("SELECT status, role, departments FROM users_with_access WHERE id = ?1", [id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, access::from_row(r, 2)?))
                }).unwrap()
            };
            let staged = |c: &rusqlite::Connection, email| {
                c.query_row("SELECT departments FROM imported_access WHERE email = ?1", [email], |r| {
                    access::from_row(r, 0)
                }).optional().unwrap()
            };
            {
                let dry = c.transaction()?;
                assert_eq!(import(&dry, export(), "example.test", &mapping, Options::default())?, expected);
                // Commit the fixture transaction so this also catches accidental dry-run writes.
                dry.commit()?;
            }
            assert_eq!(read(c, 1).2, ["Review"]);
            assert!(staged(c, "new@example.test").is_none());
            assert_eq!(staged(c, "staged-banned@example.test").unwrap(), ["Review"]);
            assert_eq!(c.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))?, 2);
            let tx = c.transaction()?;
            let apply = Options { apply: true, ..Options::default() };
            assert_eq!(import(&tx, export(), "example.test", &mapping, apply)?, expected);
            tx.commit()?;
            // Urutannya mengikuti nama, yang diganti di ekspor publik.
            let departments = read(c, 1).2;
            assert_eq!(departments.len(), 3);
            for name in ["Operasional", "Review", "Contoh"] {
                assert!(departments.iter().any(|department| department == name), "{name}");
            }
            assert_eq!(read(c, 2), ("pending".into(), "user".into(), vec!["Contoh".into()]));
            assert_eq!(read(c, 3), ("disabled".into(), "user".into(), vec!["Operasional".into()]));
            assert_eq!(read(c, 4), ("active".into(), "admin".into(), vec![]));
            assert_eq!(read(c, -1), ("disabled".into(), "user".into(), vec![]));
            assert_eq!(staged(c, "new@example.test").unwrap(), ["Contoh"]);
            assert!(staged(c, "staged-banned@example.test").unwrap().is_empty());
            assert!(staged(c, "lain@example.test.evil").is_none());
            assert_eq!(c.query_row("SELECT count(*) FROM users", [], |r| r.get::<_, i64>(0))?, 5);
            assert_eq!(c.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))?, 0);
            let same = c.transaction()?;
            assert_eq!(import(&same, export(), "example.test", &mapping, apply)?.changed, 0);
            same.commit()?;
            Ok(())
        })
        .await
        .unwrap();
    }
}
