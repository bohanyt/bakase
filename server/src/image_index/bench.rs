use super::{Access, Live, live};
use crate::{
    acl::{AuthUser, Role},
    config::Config,
    db::Db,
    documents,
    embed::{Embedder, Model, OrtEmbedder, cosine},
    search::SearchFilters,
};
use anyhow::{Result, ensure};
use image::{ImageFormat, Rgb, RgbImage};
use std::{io::Cursor, path::PathBuf, sync::Arc, time::Instant};

fn drawing(kind: usize, shifted: bool) -> Result<Vec<u8>> {
    let shade = [[195, 50, 50], [35, 86, 178], [35, 136, 73], [208, 129, 33]][kind];
    let image = RgbImage::from_fn(448, 336, |x, y| {
        let x = x as i32 - i32::from(shifted) * 8;
        let y = y as i32;
        let painted = match kind {
            0 => (x - 224).pow(2) + (y - 164).pow(2) <= 113 * 113,
            1 => (111..=337).contains(&x) && (51..=277).contains(&y),
            2 => (46..=282).contains(&y) && (x - 224).abs() <= (y - 46) * 131 / 236,
            _ => (72..=391).contains(&x) && (y - (308 - x * 3 / 5)).abs() <= 7,
        };
        Rgb(if painted {
            shade
        } else if kind == 3
            && ((72..=391).contains(&x) && (273..=277).contains(&y)
                || (70..=74).contains(&x) && (64..=277).contains(&y))
        {
            [48, 70, 88]
        } else {
            [246, 246, 240]
        })
    });
    let mut buffer = Cursor::new(Vec::new());
    image.write_to(&mut buffer, ImageFormat::Png)?;
    Ok(buffer.into_inner())
}

fn percentile(values: &mut [f64], quantile: usize) -> f64 {
    values.sort_by(f64::total_cmp);
    values[(values.len() * quantile).div_ceil(100) - 1]
}

#[tokio::test]
#[ignore = "memerlukan ekspor EG2 dan snapshot evaluasi lokal"]
async fn eg2_search_stay_fast() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(
            "off,app_hub_server::image_index::eg2=debug",
        ))
        .without_time()
        .with_target(false)
        .with_ansi(false)
        .try_init();
    let path = PathBuf::from(std::env::var_os("APP_GAMBAR_BENCH").expect("isi APP_GAMBAR_BENCH"));
    let cfg =
        Config::load(&path).map_err(|_| anyhow::anyhow!("Config evaluasi tidak dapat dibaca."))?;
    let mut image_cfg = cfg
        .gambar
        .clone()
        .expect("isi model gambar di config evaluasi");
    image_cfg.gpu = std::env::var_os("APP_GAMBAR_GPU").is_some();
    let live = Live::new(image_cfg.clone(), cfg.data_dir.clone());
    let (_lock, index) = crate::eval::open_index(&cfg.data_dir)?;
    let index = Arc::new(index);
    let db = Db::open_read_only(&cfg.data_dir)?;
    let user = AuthUser {
        id: 0,
        email: String::new(),
        name: String::new(),
        role: Role::Admin,
        departments: vec![],
    };
    let queue_deadline = || tokio::time::Instant::now() + live::SEARCH_DEADLINE;
    let mut normalized = Vec::new();
    let cold = Instant::now();
    for kind in 0..4 {
        normalized.push(live.vision(drawing(kind, false)?, queue_deadline()).await?);
    }
    let vision_ms = cold.elapsed().as_secs_f64() * 1000.0;
    let queries = [
        "lingkaran merah",
        "kotak biru",
        "segitiga hijau",
        "grafik garis dengan tren naik",
        "a red circle",
        "a blue square",
        "a green triangle",
        "an upward trending line chart",
        "jadwal rapat besok",
        "kebijakan cuti tahunan",
        "pengaturan kata sandi",
        "bunga yang sedang mekar",
    ];
    let mut positive = 1.0_f32;
    let mut negative = -1.0_f32;
    let mut kept = 0;
    let mut rejected = 0;
    let mut cold_text = 0.0;
    for (n, query) in queries.iter().enumerate() {
        let now = Instant::now();
        let vector = live.text((*query).into(), queue_deadline()).await?;
        if n == 0 {
            cold_text = now.elapsed().as_secs_f64() * 1000.0;
        }
        for (k, row) in normalized.iter().enumerate() {
            let score = cosine(row, &vector);
            if n < 8 && n % 4 == k {
                positive = positive.min(score);
                kept += u32::from(score >= live::TEXT_MIN);
            } else {
                negative = negative.max(score);
                rejected += u32::from(score < live::TEXT_MIN);
            }
        }
    }
    let visual = live.vision(drawing(0, true)?, queue_deadline()).await?;
    let visual_positive = cosine(&normalized[0], &visual);
    let visual_negative = normalized[1..]
        .iter()
        .map(|row| cosine(row, &visual))
        .max_by(f32::total_cmp)
        .unwrap();
    println!(
        "kalibrasi teks_positif={kept}/8 teks_negatif={rejected}/40 minimum_positif={positive:.6} maksimum_negatif={negative:.6} visual_positif={visual_positive:.6} visual_negatif={visual_negative:.6} visi_empat_ms={vision_ms:.2} teks_dingin_ms={cold_text:.2}"
    );
    ensure!(
        kept == 8
            && rejected == 40
            && visual_positive >= live::IMAGE_MIN
            && visual_negative < live::IMAGE_MIN,
        "Ambang pencarian gagal pada gambar sintetis."
    );

    if !live.available(queue_deadline()).await {
        return Ok(());
    }
    let granite = std::env::var_os("APP_GRANITE_MODEL").map(PathBuf::from);
    let mut granite = granite
        .map(|folder| -> Result<_> {
            let model = Model::load(&folder)?;
            let encoder = OrtEmbedder::open(
                &folder,
                &model,
                image_cfg.ort_dylib.as_deref(),
                2,
                cfg!(feature = "cuda"),
            )?;
            Ok((encoder, model.query_prefix))
        })
        .transpose()?;
    let access = Access {
        db: &db,
        index: &index,
        user: &user,
        sources: &cfg.sources,
    };
    let mut base = Vec::new();
    let mut extra = Vec::new();
    let mut added = Vec::new();
    let mut hits = 0;
    let mut timeouts = 0;
    let filters = SearchFilters::default();
    for query in queries.iter().take(8).cycle().take(24) {
        let mut baseline = || {
            if let Some((encoder, prefix)) = &mut granite {
                encoder.embed(&[format!("{prefix}{query}")])?;
            }
            Ok::<_, anyhow::Error>(documents::search(
                index.clone(),
                &db,
                &user,
                &cfg.sources,
                (*query).into(),
                6,
                2,
                false,
                filters.clone(),
            ))
        };
        let start = Instant::now();
        baseline()?.await?;
        let before = start.elapsed().as_secs_f64() * 1000.0;
        live.clear_thumbnail_cache();
        let start = Instant::now();
        let deadline = tokio::time::Instant::now() + live::SEARCH_DEADLINE;
        let images = async {
            let search = async {
                if !live.available(deadline).await {
                    return Ok(Vec::new());
                }
                let vector = live.text((*query).into(), deadline).await?;
                live.search(access, vector, false, &filters, deadline).await
            };
            match tokio::time::timeout_at(deadline, search).await {
                Ok(hits) => Ok(live.thumbnails(access, hits?, deadline).await),
                Err(_) => Ok::<_, anyhow::Error>((Vec::new(), true)),
            }
        };
        let (text, images) = tokio::join!(async { baseline()?.await }, images);
        text?;
        let (images, expired) = images?;
        hits += images.len();
        timeouts += usize::from(expired);
        let after = start.elapsed().as_secs_f64() * 1000.0;
        base.push(before);
        extra.push(after);
        added.push(after - before);
    }
    println!(
        "latensi kueri={} thumbnail_dingin={hits} tenggat={timeouts} dasar_p50_ms={:.2} dasar_p90_ms={:.2} gambar_p50_ms={:.2} gambar_p90_ms={:.2} tambahan_p50_ms={:.2} tambahan_p90_ms={:.2}",
        base.len(),
        percentile(&mut base, 50),
        percentile(&mut base, 90),
        percentile(&mut extra, 50),
        percentile(&mut extra, 90),
        percentile(&mut added, 50),
        percentile(&mut added, 90)
    );
    Ok(())
}
