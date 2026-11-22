//! Kurs USD ke IDR untuk laporan biaya di Admin. Diambil sehari sekali dari layanan kurs
//! publik (bawaannya Frankfurter, kurs referensi ECB). Permintaannya hanya URL kurs, tanpa
//! data kerja. Kalau gagal, kurs terakhir yang tersimpan tetap dipakai.

use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;

use crate::db::{Db, now};

/// Kurs diambil lagi kalau pengambilan terakhir lebih dari 12 jam lalu; dicek tiap jam.
const STALE_SECS: i64 = 12 * 3600;
const CHECK_EVERY: Duration = Duration::from_secs(3600);
const TIMEOUT: Duration = Duration::from_secs(15);

/// Kurs yang dipakai laporan.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Rate {
    pub usd_idr: f64,
    /// Tanggal kurs dari sumbernya (YYYY-MM-DD); kosong untuk kurs cadangan dari config.
    pub date: Option<String>,
    /// `otomatis` (diambil dari layanan kurs) atau `config` (`fallback_usd_idr`).
    pub source: &'static str,
}

/// Kurs tersimpan terakhir, atau `fallback` dari config kalau belum pernah berhasil diambil.
pub async fn current(db: &Db, fallback: f64) -> Result<Option<Rate>> {
    let stored = db
        .call(|c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row(
                "SELECT date, usd_idr FROM fx_rates ORDER BY date DESC LIMIT 1",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)),
            )
            .optional()?)
        })
        .await?;
    Ok(match stored {
        Some((date, usd_idr)) => Some(Rate {
            usd_idr,
            date: Some(date),
            source: "otomatis",
        }),
        None if fallback > 0.0 => Some(Rate {
            usd_idr: fallback,
            date: None,
            source: "config",
        }),
        None => None,
    })
}

/// Jalan terus di latar: ambil kurs saat server start, lalu setiap kali sudah basi.
pub async fn refresh_loop(db: Db, url: String) {
    let client = match reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent("rag-hub")
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("klien kurs tidak bisa dibuat: {e:#}");
            return;
        }
    };
    loop {
        if let Err(e) = refresh_if_stale(&db, &client, &url).await {
            tracing::warn!("kurs USD-IDR belum bisa diambil, kurs terakhir tetap dipakai: {e:#}");
        }
        tokio::time::sleep(CHECK_EVERY).await;
    }
}

async fn refresh_if_stale(db: &Db, client: &reqwest::Client, url: &str) -> Result<()> {
    let last: Option<i64> = db
        .call(|c| Ok(c.query_row("SELECT max(fetched_at) FROM fx_rates", [], |r| r.get(0))?))
        .await?;
    if last.is_some_and(|t| now() - t < STALE_SECS) {
        return Ok(());
    }
    let body: Value = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let (date, rate) = parse(&body)?;
    save(db, date.clone(), rate).await?;
    tracing::info!("kurs USD-IDR {date}: {rate}");
    Ok(())
}

pub async fn save(db: &Db, date: String, usd_idr: f64) -> Result<()> {
    db.call(move |c| {
        c.execute(
            "INSERT INTO fx_rates (date, usd_idr, fetched_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(date) DO UPDATE SET usd_idr = excluded.usd_idr,
                                             fetched_at = excluded.fetched_at",
            rusqlite::params![date, usd_idr, now()],
        )?;
        Ok(())
    })
    .await
}

/// Jawaban Frankfurter: `{"base":"USD","date":"2026-11-16","rates":{"IDR":18259}}`.
fn parse(body: &Value) -> Result<(String, f64)> {
    let rate = body["rates"]["IDR"]
        .as_f64()
        .context("jawaban kurs tanpa rates.IDR")?;
    anyhow::ensure!(
        rate.is_finite() && (1_000.0..=100_000.0).contains(&rate),
        "kurs IDR tidak masuk akal: {rate}"
    );
    let date = body["date"].as_str().context("jawaban kurs tanpa date")?;
    let b = date.as_bytes();
    anyhow::ensure!(
        b.len() == 10
            && b[4] == b'-'
            && b[7] == b'-'
            && b.iter()
                .enumerate()
                .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit()),
        "tanggal kurs tidak dikenal: {date:?}"
    );
    Ok((date.to_string(), rate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frankfurter_answer_is_parsed_and_checked() {
        assert_eq!(
            parse(&json!({ "base": "USD", "date": "2026-11-16", "rates": { "IDR": 18259 } }))
                .unwrap(),
            ("2026-11-16".to_string(), 18259.0)
        );
        for bad in [
            json!({ "date": "2026-11-16", "rates": {} }),
            json!({ "date": "2026-11-16", "rates": { "IDR": 0.06 } }),
            json!({ "date": "16/11/2026", "rates": { "IDR": 18259 } }),
            json!({ "rates": { "IDR": 18259 } }),
        ] {
            assert!(parse(&bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn latest_stored_rate_wins_over_fallback() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(current(&db, 0.0).await.unwrap(), None);
        assert_eq!(
            current(&db, 18000.0).await.unwrap().unwrap().source,
            "config"
        );
        save(&db, "2026-11-13".into(), 18100.0).await.unwrap();
        save(&db, "2026-11-16".into(), 18259.0).await.unwrap();
        save(&db, "2026-11-16".into(), 18260.0).await.unwrap();
        assert_eq!(
            current(&db, 18000.0).await.unwrap(),
            Some(Rate {
                usd_idr: 18260.0,
                date: Some("2026-11-16".into()),
                source: "otomatis"
            })
        );
    }
}
