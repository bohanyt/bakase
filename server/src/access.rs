use rusqlite::{Connection, Row, Transaction, params, types::Type};

const SCHEMA: &str = r#"
CREATE TABLE user_departments (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE CHECK (user_id <> -1),
    department TEXT NOT NULL COLLATE NOCASE,
    PRIMARY KEY (user_id, department)
);
CREATE TABLE imported_access (
    email TEXT PRIMARY KEY COLLATE NOCASE,
    departments TEXT NOT NULL
);
INSERT INTO user_departments (user_id, department)
    SELECT id, department FROM users WHERE id <> -1 AND department IS NOT NULL;
CREATE VIEW users_with_access AS
    SELECT u.*, (SELECT json_group_array(department ORDER BY department) FROM user_departments
                 WHERE user_id = u.id) AS departments
    FROM users u;
CREATE TRIGGER user_departments_legacy_insert AFTER INSERT ON users
WHEN NEW.id <> -1 AND NEW.department IS NOT NULL
BEGIN
    INSERT INTO user_departments VALUES (NEW.id, NEW.department);
END;
CREATE TRIGGER user_departments_legacy_update AFTER UPDATE OF department ON users
WHEN NEW.id <> -1 AND NEW.department IS NOT OLD.department
BEGIN
    DELETE FROM user_departments WHERE user_id = NEW.id;
    INSERT INTO user_departments SELECT NEW.id, NEW.department WHERE NEW.department IS NOT NULL;
    DELETE FROM sessions WHERE user_id = NEW.id;
END;
"#;

pub(crate) fn initialized(c: &Connection) -> rusqlite::Result<bool> {
    c.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_schema WHERE name = 'user_departments' AND type = 'table')",
        [],
        |row| row.get(0),
    )
}

pub(crate) fn init(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    // Slot 6f7e8db menolak kenaikan user_version. Skema tambahan ini atomik.
    if !initialized(tx)? {
        tx.execute_batch(SCHEMA)?;
    }
    Ok(())
}

pub(crate) fn from_row(row: &Row<'_>, column: usize) -> rusqlite::Result<Vec<String>> {
    serde_json::from_str(row.get_ref(column)?.as_str()?)
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(err)))
}

pub(crate) fn sort_departments(departments: &mut Vec<String>) {
    departments.sort_by_cached_key(|d| d.to_ascii_lowercase());
    departments.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
}

/// Pemanggil membandingkan akses lama dan baru dalam transaksi tulis yang sama.
pub(crate) fn replace(
    tx: &Transaction<'_>,
    user_id: i64,
    departments: &[String],
) -> rusqlite::Result<()> {
    // Ubah kolom lama dulu agar trigger hanya berjalan sebelum daftar lengkap ditulis.
    tx.execute(
        "UPDATE users SET department = ?1 WHERE id = ?2",
        params![departments.first(), user_id],
    )?;
    tx.execute("DELETE FROM user_departments WHERE user_id = ?1", [user_id])?;
    let mut insert = tx.prepare("INSERT INTO user_departments VALUES (?1, ?2)")?;
    for department in departments {
        insert.execute(params![user_id, department])?;
    }
    tx.execute("DELETE FROM sessions WHERE user_id = ?1", [user_id])?;
    Ok(())
}
