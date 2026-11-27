//! Pemakaian token chat dan ingest untuk laporan biaya di Admin. Catatan memuat angka token
//! dan jumlah panggilan tanpa menyimpan isi.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AppState, auth::AdminUser, config::PricingConfig, db::Db, error::ApiError};

pub const INGEST_VISION: &str = "ingest_vision";
pub const LABEL_SCAN: &str = "label_scan";
/// Perintah terminal yang memanggil model: `tanya` dan `cek-vision`.
pub const TERMINAL: &str = "terminal";
/// Token yang diproses model embedding lokal. Tanpa biaya, jadi tidak ikut total.
pub const EMBED: &str = "embed";

/// Jumlah token dari satu atau beberapa respons Responses API.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Usage {
    pub input: i64,
    /// Bagian dari `input` yang diambil dari cache prompt (lebih murah).
    pub cached: i64,
    pub output: i64,
    /// Bagian dari `output` yang dipakai untuk berpikir.
    pub reasoning: i64,
    /// Jumlah panggilan ke model dari Hub. Retry HTTP otomatis di dalam klien (untuk 429, 5xx,
    /// atau koneksi gagal) tidak dihitung terpisah: Azure tidak menagih respons error itu, dan
    /// biaya dihitung dari token yang dilaporkan.
    pub requests: i64,
    /// Permintaan yang jumlah tokennya dilaporkan Azure. Selisih dengan `requests` adalah
    /// permintaan yang terputus sebelum selesai (timeout, browser pergi), yang tetap berbiaya
    /// tapi tokennya tidak diketahui.
    pub reported: i64,
}

impl Usage {
    pub fn add(&mut self, other: Self) {
        self.input += other.input;
        self.cached += other.cached;
        self.output += other.output;
        self.reasoning += other.reasoning;
        self.requests += other.requests;
        self.reported += other.reported;
    }

    /// Dipanggil setiap kali permintaan ke model berhasil dikirim.
    pub fn start_request(&mut self) {
        self.requests += 1;
    }

    /// Menambahkan `usage` dari objek respons (`response.completed`, juga `response.incomplete`
    /// dan `response.failed` yang tetap memakai token). Respons tanpa `usage` diabaikan.
    pub fn add_response(&mut self, response: &Value) {
        let usage = &response["usage"];
        if !usage.is_object() {
            return;
        }
        let n = |v: &Value| v.as_i64().unwrap_or(0);
        self.input += n(&usage["input_tokens"]);
        self.cached += n(&usage["input_tokens_details"]["cached_tokens"]);
        self.output += n(&usage["output_tokens"]);
        self.reasoning += n(&usage["output_tokens_details"]["reasoning_tokens"]);
        self.reported += 1;
    }
}

/// Mencatat pemakaian satu giliran chat/tamu atau satu permintaan ingest.
/// Kegagalan mencatat hanya ditulis ke log.
pub async fn record(
    db: &Db,
    kind: &'static str,
    user_id: Option<i64>,
    message_id: Option<i64>,
    usage: Usage,
) {
    if usage.requests == 0 {
        return;
    }
    let result = db
        .call(move |c| Ok(insert(c, kind, user_id, message_id, usage)?))
        .await;
    if let Err(e) = result {
        tracing::warn!("gagal mencatat pemakaian token: {e:#}");
    }
}

/// Untuk perintah terminal. Database tidak dibuat kalau server belum pernah jalan di sini.
pub fn record_command(data_dir: &std::path::Path, kind: &'static str, usage: Usage) {
    if usage.requests == 0 || !Db::exists(data_dir) {
        return;
    }
    let result = Db::open(data_dir)
        .and_then(|db| db.call_blocking(|c| Ok(insert(c, kind, None, None, usage)?)));
    if result.is_err() {
        println!("Pemakaian token gagal dicatat.");
    }
}

fn insert(
    c: &rusqlite::Connection,
    kind: &str,
    user_id: Option<i64>,
    message_id: Option<i64>,
    usage: Usage,
) -> rusqlite::Result<usize> {
    c.execute(
        "INSERT INTO usage_log (created_at, kind, user_id, message_id, input_tokens,
         cached_tokens, output_tokens, reasoning_tokens, requests, reported_requests)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            crate::db::now(),
            kind,
            user_id,
            message_id,
            usage.input,
            usage.cached,
            usage.output,
            usage.reasoning,
            usage.requests,
            usage.reported
        ],
    )
}

/// Hari dan bulan laporan mengikuti WIB.
const WIB_SECS: i64 = 7 * 3600;

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize)]
pub struct Totals {
    /// Jumlah giliran (satu pertanyaan user atau tamu).
    pub turns: i64,
    pub input: i64,
    pub cached: i64,
    pub output: i64,
    pub reasoning: i64,
    pub requests: i64,
    pub reported: i64,
    pub cost_usd: f64,
}

impl Totals {
    /// Baris SQL berisi jumlah giliran, token, dan permintaan, mulai kolom `at`.
    fn from_row(r: &rusqlite::Row, at: usize) -> rusqlite::Result<Self> {
        Ok(Self {
            turns: r.get(at)?,
            input: r.get(at + 1)?,
            cached: r.get(at + 2)?,
            output: r.get(at + 3)?,
            reasoning: r.get(at + 4)?,
            requests: r.get(at + 5)?,
            reported: r.get(at + 6)?,
            cost_usd: 0.0,
        })
    }

    fn priced(mut self, p: &PricingConfig) -> Self {
        let fresh = (self.input - self.cached).max(0) as f64;
        self.cost_usd = (fresh * p.input_usd
            + self.cached as f64 * p.cached_input_usd
            + self.output as f64 * p.output_usd)
            / 1_000_000.0;
        self
    }

    fn add(&mut self, o: &Totals) {
        self.turns += o.turns;
        self.input += o.input;
        self.cached += o.cached;
        self.output += o.output;
        self.reasoning += o.reasoning;
        self.requests += o.requests;
        self.reported += o.reported;
        self.cost_usd += o.cost_usd;
    }
}

const SUMS: &str = "count(CASE WHEN kind IN ('chat', 'tamu') THEN 1 END),
     coalesce(sum(input_tokens), 0), coalesce(sum(cached_tokens), 0), coalesce(sum(output_tokens), 0),
     coalesce(sum(reasoning_tokens), 0), coalesce(sum(requests), 0), coalesce(sum(reported_requests), 0)";

#[derive(Serialize)]
pub struct Period {
    /// `YYYY-MM` untuk bulan, `YYYY-MM-DD` untuk hari.
    pub period: String,
    #[serde(flatten)]
    pub totals: Totals,
}

#[derive(Serialize)]
pub struct UserUsage {
    /// Kosong untuk tamu (tanpa login).
    pub user_id: Option<i64>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub department: Option<String>,
    pub departments: Vec<String>,
    pub kind: String,
    #[serde(flatten)]
    pub totals: Totals,
}

#[derive(Serialize)]
pub struct Pricing {
    pub input_usd: f64,
    pub cached_input_usd: f64,
    pub output_usd: f64,
}

#[derive(Serialize)]
pub struct Report {
    pub month: String,
    /// Sampai 12 bulan terakhir yang ada pemakaiannya, terbaru dulu.
    pub months: Vec<Period>,
    pub total: Totals,
    pub days: Vec<Period>,
    /// Biaya terbesar dulu.
    pub users: Vec<UserUsage>,
    pub ingest: Option<Totals>,
    pub label_scan: Option<Totals>,
    pub terminal: Option<Totals>,
    /// Tanpa biaya dan di luar `total`.
    pub embed: Option<Totals>,
    pub pricing: Pricing,
    /// Kosong kalau kurs belum pernah berhasil diambil dan tidak ada kurs cadangan.
    pub rate: Option<crate::fx::Rate>,
}

#[derive(Deserialize)]
pub struct ReportQuery {
    /// `YYYY-MM`; bawaannya bulan ini (WIB).
    month: Option<String>,
}

/// Awal dan akhir (eksklusif) bulan `YYYY-MM` dalam WIB, sebagai detik Unix.
fn month_range(month: &str) -> Option<(i64, i64)> {
    let (y, m) = month.split_once('-')?;
    if y.len() != 4 || m.len() != 2 {
        return None;
    }
    let (y, m): (i32, u8) = (y.parse().ok()?, m.parse().ok()?);
    let start = |y: i32, m: u8| -> Option<i64> {
        let date = time::Date::from_calendar_date(y, time::Month::try_from(m).ok()?, 1).ok()?;
        Some(date.midnight().assume_utc().unix_timestamp() - WIB_SECS)
    };
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    Some((start(y, m)?, start(ny, nm)?))
}

fn this_month() -> String {
    let t = time::OffsetDateTime::from_unix_timestamp(crate::db::now() + WIB_SECS)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    format!("{:04}-{:02}", t.year(), u8::from(t.month()))
}

/// Laporan pemakaian dan biaya untuk tab Admin > Pemakaian. Biaya dihitung dari tarif di
/// config saat laporan dibuka; Rupiah memakai kurs terakhir.
pub async fn admin_report(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
    Query(q): Query<ReportQuery>,
) -> Result<Json<Report>, ApiError> {
    let month = q.month.unwrap_or_else(this_month);
    let (from, to) =
        month_range(&month).ok_or_else(|| ApiError::bad_request("Bulan harus YYYY-MM."))?;
    let (months, days, users) = state
        .db
        .call(move |c| {
            // Satu transaksi baca: slot lain bisa mencatat pemakaian di tengah laporan, dan
            // tanpa snapshot yang sama total per user bisa melebihi total bulan.
            let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)?;
            let months = tx
                .prepare(&format!(
                    "SELECT strftime('%Y-%m', created_at + {WIB_SECS}, 'unixepoch') AS m, {SUMS}
                     FROM usage_log WHERE kind <> '{EMBED}' GROUP BY m ORDER BY m DESC LIMIT 12"
                ))?
                .query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, Totals::from_row(r, 1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let days = tx
                .prepare(&format!(
                    "SELECT strftime('%Y-%m-%d', created_at + {WIB_SECS}, 'unixepoch') AS d, {SUMS}
                     FROM usage_log WHERE created_at >= ?1 AND created_at < ?2 AND kind <> '{EMBED}'
                     GROUP BY d ORDER BY d"
                ))?
                .query_map([from, to], |r| {
                    Ok((r.get::<_, String>(0)?, Totals::from_row(r, 1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let users = tx
                .prepare(&format!(
                    "SELECT l.user_id, u.name, u.email, u.department, l.kind, COALESCE(u.departments, '[]'), {SUMS}
                     FROM usage_log l LEFT JOIN users_with_access u ON u.id = l.user_id
                     WHERE l.created_at >= ?1 AND l.created_at < ?2
                     GROUP BY l.kind, l.user_id"
                ))?
                .query_map([from, to], |r| {
                    Ok(UserUsage {
                        user_id: r.get(0)?,
                        name: r.get(1)?,
                        email: r.get(2)?,
                        department: r.get(3)?,
                        departments: crate::access::from_row(r, 5)?,
                        kind: r.get(4)?,
                        totals: Totals::from_row(r, 6)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            tx.commit()?;
            Ok((months, days, users))
        })
        .await?;
    let p = &state.pricing;
    let period = |(period, t): (String, Totals)| Period {
        period,
        totals: t.priced(p),
    };
    let days: Vec<Period> = days.into_iter().map(period).collect();
    let mut total = Totals::default();
    for d in &days {
        total.add(&d.totals);
    }
    let mut ingest = Totals::default();
    let mut label_scan = Totals::default();
    let mut terminal = Totals::default();
    let mut embed = Totals::default();
    let mut users: Vec<UserUsage> = users
        .into_iter()
        .filter_map(|mut u| {
            let shared = match u.kind.as_str() {
                INGEST_VISION => Some(&mut ingest),
                LABEL_SCAN => Some(&mut label_scan),
                TERMINAL => Some(&mut terminal),
                EMBED => Some(&mut embed),
                _ => None,
            };
            if let Some(shared) = shared {
                shared.add(&u.totals);
                return None;
            }
            u.totals = u.totals.priced(p);
            Some(u)
        })
        .collect();
    users.sort_by(|a, b| b.totals.cost_usd.total_cmp(&a.totals.cost_usd));
    Ok(Json(Report {
        month,
        months: months.into_iter().map(period).collect(),
        total,
        days,
        users,
        ingest: (ingest.requests > 0).then(|| ingest.priced(p)),
        label_scan: (label_scan.requests > 0).then(|| label_scan.priced(p)),
        terminal: (terminal.requests > 0).then(|| terminal.priced(p)),
        embed: (embed.requests > 0).then_some(embed),
        pricing: Pricing {
            input_usd: p.input_usd,
            cached_input_usd: p.cached_input_usd,
            output_usd: p.output_usd,
        },
        rate: crate::fx::current(&state.db, p.fallback_usd_idr).await?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn months_follow_wib() {
        // 1 Nov 2026 00:00 WIB = 31 Okt 2026 17:00 UTC.
        assert_eq!(month_range("2026-11"), Some((1_793_466_000, 1_796_058_000)));
        assert_eq!(month_range("2026-12").unwrap().1, 1_798_736_400);
        for bad in ["2026-13", "2026-1", "26-11", "2026/11", "", "abcd-ef"] {
            assert_eq!(month_range(bad), None, "{bad}");
        }
    }

    #[test]
    fn cost_uses_cheaper_rate_for_cached_input() {
        let p = PricingConfig::default();
        let t = Totals {
            input: 1_000_000,
            cached: 800_000,
            output: 100_000,
            ..Totals::default()
        }
        .priced(&p);
        // 200K input baru × 0,10 + 800K cached × 0,01 + 100K output × 0,50 per juta.
        assert!(
            (t.cost_usd - (0.02 + 0.008 + 0.05)).abs() < 1e-12,
            "{}",
            t.cost_usd
        );
    }

    #[test]
    fn usage_adds_up_across_responses() {
        let mut usage = Usage::default();
        for _ in 0..3 {
            usage.start_request();
        }
        usage.add_response(&json!({ "usage": {
            "input_tokens": 1200,
            "input_tokens_details": { "cached_tokens": 1000 },
            "output_tokens": 300,
            "output_tokens_details": { "reasoning_tokens": 120 },
        }}));
        usage.add_response(&json!({ "usage": { "input_tokens": 50, "output_tokens": 5 } }));
        usage.add_response(&json!({ "status": "failed" }));
        assert_eq!(
            usage,
            Usage {
                input: 1250,
                cached: 1000,
                output: 305,
                reasoning: 120,
                requests: 3,
                reported: 2
            }
        );
    }

    #[test]
    fn commands_record_only_into_an_existing_database() {
        let dir = std::env::temp_dir().join(format!("rag-hub-usage-{}", rand::random::<u64>()));
        let mut usage = Usage::default();
        usage.start_request();
        record_command(&dir, TERMINAL, usage);
        assert!(
            !dir.exists(),
            "database tidak dibuat oleh perintah terminal"
        );
        let db = Db::open(&dir).unwrap();
        record_command(&dir, TERMINAL, usage);
        record_command(&dir, EMBED, Usage::default());
        let kinds = db
            .call_blocking(|c| {
                let mut stmt = c.prepare("SELECT kind FROM usage_log")?;
                let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .unwrap();
        assert_eq!(kinds, [TERMINAL]);
        drop(db);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn record_writes_one_row_and_skips_empty_turns() {
        let db = Db::open_in_memory().unwrap();
        record(&db, "chat", Some(1), Some(10), Usage::default()).await;
        let mut usage = Usage::default();
        usage.start_request();
        usage.add_response(&json!({ "usage": { "input_tokens": 7, "output_tokens": 3 } }));
        record(&db, "tamu", None, None, usage).await;
        // Permintaan yang terputus tetap tercatat walau tokennya tidak diketahui.
        let mut cut = Usage::default();
        cut.start_request();
        record(&db, "tamu", None, None, cut).await;
        let rows: Vec<(String, Option<i64>, i64, i64, i64)> = db
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT kind, user_id, input_tokens, output_tokens, requests - reported_requests
                     FROM usage_log ORDER BY id",
                )?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .unwrap();
        assert_eq!(
            rows,
            vec![
                ("tamu".to_string(), None, 7, 3, 0),
                ("tamu".to_string(), None, 0, 0, 1)
            ]
        );
    }
}
