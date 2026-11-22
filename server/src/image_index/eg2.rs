use crate::config::GambarConfig;
use anyhow::{Result, anyhow, ensure};
use image::{RgbImage, imageops::FilterType};
use ort::{
    session::{OutputSelector, RunOptions, Session},
    value::Tensor,
};
use std::{path::Path, sync::Arc};

pub(super) const DIM: usize = 768;
pub(super) const PREPROCESS_VERSION: u32 = 1;
const PATCH: u32 = 16;
const GRID: u32 = PATCH * 3;
const SOFT: u32 = 280;
const PATCHES: usize = (SOFT * 9) as usize;
const RGB_PATCH: usize = 3 * PATCH as usize * PATCH as usize;
#[derive(Clone, Copy)]
pub(super) struct Tokens(pub(super) [i64; 4]);

impl Tokens {
    pub(super) fn load(folder: &Path) -> Result<Self> {
        fn read(folder: &Path, name: &str) -> Result<serde_json::Value> {
            std::fs::read(folder.join(name))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .ok_or_else(|| anyhow!("Artefak EG2 {name} tidak tersedia atau tidak valid."))
        }
        let config = read(folder, "config.json")?;
        let processor = read(folder, "processor_config.json")?;
        let processor = &processor["image_processor"];
        ensure!(
            processor["do_convert_rgb"] == true
                && processor["do_resize"] == true
                && processor["do_rescale"] == true
                && processor["do_normalize"] == false
                && processor["resample"] == 3
                && processor["patch_size"] == PATCH
                && processor["pooling_kernel_size"] == GRID / PATCH
                && processor["max_soft_tokens"] == SOFT
                && processor["rescale_factor"]
                    .as_f64()
                    .is_some_and(|scale| (scale - 1.0 / 255.0).abs() < 1e-12)
                && config["text_config"]["embedding_dim"] == DIM,
            "Konfigurasi praproses EG2 tidak cocok dengan ekspor FP32 yang didukung."
        );
        let tokenizer = tokenizers::Tokenizer::from_file(folder.join("tokenizer.json"))
            .map_err(|_| anyhow!("Tokenizer EG2 gagal dimuat."))?;
        let fields = [
            (&config["text_config"]["bos_token_id"], "<bos>"),
            (&config["boi_token_id"], "<|image>"),
            (&config["image_token_id"], "<|image|>"),
            (&config["eoi_token_id"], "<image|>"),
        ];
        let mut ids = [0; 4];
        for ((field, name), output) in fields.into_iter().zip(&mut ids) {
            let id = field.as_u64().and_then(|id| u32::try_from(id).ok());
            ensure!(
                id.is_some() && id == tokenizer.token_to_id(name),
                "ID token khusus EG2 berbeda antara config dan tokenizer."
            );
            *output = i64::from(id.expect("ID sudah diperiksa"));
        }
        Ok(Self(ids))
    }
}

pub(super) struct Prepared {
    pub(super) pixels: Vec<f32>,
    pub(super) positions: Vec<i64>,
    pub(super) soft_tokens: usize,
}

fn dimensions(width: u32, height: u32) -> (u32, u32) {
    let scale = (f64::from(SOFT * GRID * GRID) / (f64::from(width) * f64::from(height))).sqrt();
    let side = |size| (scale * f64::from(size) / f64::from(GRID)).floor() as u32 * GRID;
    (side(width), side(height))
}

pub(super) fn prepare(image: &RgbImage) -> Prepared {
    let (width, height) = dimensions(image.width(), image.height());
    let resized = (image.dimensions() != (width, height))
        .then(|| image::imageops::resize(image, width, height, FilterType::CatmullRom));
    let image = resized.as_ref().unwrap_or(image);
    let mut pixels = vec![0.0; PATCHES * RGB_PATCH];
    let mut positions = vec![-1; PATCHES * 2];
    let grid_width = width / PATCH;
    let grid_height = height / PATCH;
    let mut index = 0;
    for y in 0..grid_height {
        for x in 0..grid_width {
            positions[index * 2] = i64::from(x);
            positions[index * 2 + 1] = i64::from(y);
            let mut write = index * RGB_PATCH;
            for dy in 0..PATCH {
                let start = ((y * PATCH + dy) * width + x * PATCH) as usize * 3;
                for &channel in &image.as_raw()[start..start + PATCH as usize * 3] {
                    pixels[write] = f32::from(channel) / 255.0;
                    write += 1;
                }
            }
            index += 1;
        }
    }
    Prepared {
        pixels,
        positions,
        soft_tokens: (grid_width * grid_height / 9) as usize,
    }
}

pub(super) struct Encoder {
    vision: Session,
    backbone: Session,
    tokens: Tokens,
}

impl Encoder {
    pub(super) fn open(
        config: &GambarConfig,
        dll: &Path,
        gpu: bool,
        tokens: Tokens,
    ) -> Result<Self> {
        ensure!(
            !gpu || cfg!(feature = "cuda"),
            "Build CUDA diperlukan untuk --gpu."
        );
        let dll = dll
            .canonicalize()
            .map_err(|_| anyhow!("DLL ONNX Runtime tidak ditemukan."))?;
        ort::init_from(dll)
            .map_err(|_| anyhow!("DLL ONNX Runtime gagal dimuat."))?
            .with_logger(Arc::new(|_, _, _, _, _| {}))
            .commit();
        ensure!(
            !gpu || (config.device_id >= 0
                && config.vision_arena_mib > 0
                && config.backbone_arena_mib > 0),
            "Periksa nomor perangkat dan batas arena CUDA untuk gambar."
        );
        let session = |name: &str, _arena_mib: u32| -> Result<Session> {
            let setup = || anyhow!("Sesi ONNX gambar gagal disiapkan.");
            let builder = Session::builder()
                .map_err(|_| setup())?
                .with_intra_threads(2)
                .map_err(|_| setup())?
                .with_inter_threads(1)
                .map_err(|_| setup())?
                .with_intra_op_spinning(false)
                .map_err(|_| setup())?
                .with_memory_pattern(!gpu)
                .map_err(|_| setup())?;
            #[cfg(feature = "cuda")]
            let placed = Arc::new(std::sync::Mutex::new(None));
            #[cfg(feature = "cuda")]
            let builder = if gpu {
                use ort::ep::{ArenaExtendStrategy, CUDA};
                let count = placed.clone();
                builder
                    .with_logger(Arc::new(move |_, _, _, location, message| {
                        if location.contains("VerifyEachNodeIsAssignedToAnEp")
                            && let Some(nodes) = cpu_node_count(message)
                        {
                            let mut total = count.lock().unwrap();
                            *total = Some(total.unwrap_or(0) + nodes);
                        }
                    }))
                    .map_err(|_| setup())?
                    .with_log_level(ort::logging::LogLevel::Verbose)
                    .map_err(|_| setup())?
                    .with_log_verbosity(1)
                    .map_err(|_| setup())?
                    .with_execution_providers([CUDA::default()
                        .with_device_id(config.device_id)
                        .with_arena_extend_strategy(ArenaExtendStrategy::SameAsRequested)
                        .with_memory_limit(_arena_mib as usize * 1024 * 1024)
                        .build()
                        .error_on_failure()])
                    .map_err(|_| {
                        anyhow!(
                            "CUDA gagal diaktifkan untuk {name}. Periksa perangkat dan DLL CUDA."
                        )
                    })?
            } else {
                builder
            };
            let mut builder = builder;
            let session = builder
                .commit_from_file(config.model.join(name))
                .map_err(|_| {
                    anyhow!("Model {name} gagal dimuat. Periksa ekspor EG2 FP32 dan bobotnya.")
                })?;
            #[cfg(feature = "cuda")]
            if gpu {
                if let Some(cpu_nodes) = *placed.lock().unwrap() {
                    tracing::info!(graph = name, cpu_nodes, "Penempatan operator EG2");
                } else {
                    tracing::debug!(
                        graph = name,
                        "ONNX Runtime tidak melaporkan jumlah operator CPU"
                    );
                }
            }
            Ok(session)
        };
        Ok(Self {
            vision: session("vision_encoder.onnx", config.vision_arena_mib)?,
            backbone: session("model.onnx", config.backbone_arena_mib)?,
            tokens,
        })
    }

    pub(super) fn embed(&mut self, images: Vec<Prepared>) -> Result<Vec<Vec<f32>>> {
        let batch = images.len();
        let longest = images
            .iter()
            .map(|image| image.soft_tokens)
            .max()
            .unwrap_or(0);
        ensure!(longest > 0, "Masukan gambar kosong.");
        let width = longest + 3;
        let mut ids = vec![0_i64; batch * width];
        let mut mask = vec![0_i64; batch * width];
        let mut pixels = Vec::with_capacity(batch * PATCHES * RGB_PATCH);
        let mut positions = Vec::with_capacity(batch * PATCHES * 2);
        let mut total = 0;
        let [bos, start_image, image_token, end_image] = self.tokens.0;
        for (row, image) in images.into_iter().enumerate() {
            let start = row * width;
            let end = start + 2 + image.soft_tokens;
            ids[start] = bos;
            ids[start + 1] = start_image;
            ids[start + 2..end].fill(image_token);
            ids[end] = end_image;
            mask[start..=end].fill(1);
            pixels.extend(image.pixels);
            positions.extend(image.positions);
            total += image.soft_tokens;
        }
        let failed = || anyhow!("Inferensi gambar gagal. Periksa model FP32 atau kecilkan batch.");
        let encoded = self.vision.run(ort::inputs![
            "pixel_values" => Tensor::from_array(([batch, PATCHES, RGB_PATCH], pixels)).map_err(|_| failed())?,
            "pixel_position_ids" => Tensor::from_array(([batch, PATCHES, 2], positions)).map_err(|_| failed())?,
        ]).map_err(|_| failed())?;
        let features = encoded.get("image_features").ok_or_else(failed)?;
        let (shape, values) = features.try_extract_tensor::<f32>().map_err(|_| failed())?;
        ensure!(
            shape.as_ref() == [total as i64, 512] && values.iter().all(|value| value.is_finite()),
            "Bentuk atau nilai fitur visi FP32 tidak valid."
        );
        let empty = Tensor::from_array(([0, 512], Vec::<f32>::new())).map_err(|_| failed())?;
        let output = "sentence_embedding";
        let options = RunOptions::new()
            .map_err(|_| failed())?
            .with_outputs(OutputSelector::no_default().with(output));
        let results = self.backbone.run_with_options(ort::inputs![
            "input_ids" => Tensor::from_array(([batch, width], ids)).map_err(|_| failed())?,
            "attention_mask" => Tensor::from_array(([batch, width], mask)).map_err(|_| failed())?,
            "image_features" => features,
            "audio_features" => &empty,
            "video_features" => &empty,
        ], &options).map_err(|_| failed())?;
        let (shape, values) = results
            .get(output)
            .ok_or_else(failed)?
            .try_extract_tensor::<f32>()
            .map_err(|_| failed())?;
        ensure!(
            shape.as_ref() == [batch as i64, DIM as i64],
            "Dimensi vektor gambar tidak valid."
        );
        let rows = values
            .chunks_exact(DIM)
            .map(<[f32]>::to_vec)
            .collect::<Vec<_>>();
        ensure!(
            rows.iter().all(
                |row| (row.iter().map(|value| value * value).sum::<f32>() - 1.0).abs() < 0.002
            ),
            "Vektor gambar FP32 tidak ternormalisasi."
        );
        Ok(rows)
    }
}

#[cfg(any(feature = "cuda", test))]
fn cpu_node_count(message: &str) -> Option<usize> {
    let message = message.trim();
    if let Some(summary) = message.strip_prefix("All nodes placed on [") {
        let (provider, count) = summary.split_once("]. Number of nodes:")?;
        return Some(if provider == "CPUExecutionProvider" {
            count.trim().parse().ok()?
        } else {
            0
        });
    }
    message
        .strip_prefix("Node(s) placed on [CPUExecutionProvider]. Number of nodes:")?
        .trim()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    #[test]
    fn ort_placement_counts_only_cpu_nodes() {
        assert_eq!(
            cpu_node_count("  Node(s) placed on [CPUExecutionProvider]. Number of nodes: 16"),
            Some(16)
        );
        assert_eq!(
            cpu_node_count("All nodes placed on [CUDAExecutionProvider]. Number of nodes: 540"),
            Some(0)
        );
        assert_eq!(
            cpu_node_count("All nodes placed on [CPUExecutionProvider]. Number of nodes: 3"),
            Some(3)
        );
        assert_eq!(
            cpu_node_count("Node(s) placed on [CUDAExecutionProvider]. Number of nodes: 540"),
            None
        );
    }

    #[test]
    #[ignore = "memerlukan ekspor EG2 FP32 dan DLL ONNX Runtime lokal"]
    fn eg2_matches_official_fp32_reference() {
        let folder = std::env::var_os("APP_EG2_MODEL").expect("isi APP_EG2_MODEL");
        let dll = std::env::var_os("ORT_DYLIB_PATH").expect("isi ORT_DYLIB_PATH");
        let config = GambarConfig {
            model: folder.into(),
            ort_dylib: None,
            gpu: false,
            device_id: 0,
            vision_arena_mib: 4096,
            backbone_arena_mib: 2048,
        };
        let tokens = Tokens::load(&config.model).unwrap();
        let mut encoder = Encoder::open(&config, Path::new(&dll), false, tokens).unwrap();
        // Gemma4ImageProcessorPil (Transformers 5.19.0), lalu kedua graf ONNX FP32
        // pada ekspor onnx-community/embeddinggemma-2-ONNX @ daa72c51.
        let reference: Vec<Vec<f32>> =
            serde_json::from_str(include_str!("referensi-eg2-sintetis.json")).unwrap();
        let images = [(768, 768), (353, 257), (1103, 752), (1500, 67)].map(|(w, h)| {
            RgbImage::from_fn(w, h, |x, y| {
                Rgb([
                    ((x * 241 / w + (y / 19) % 3 * 17) % 256) as u8,
                    ((y * 251 / h + (x / 19) % 5 * 11) % 256) as u8,
                    ((x * 31 / w + y * 150 / h + 40 * u32::from(x > w / 2)) % 256) as u8,
                ])
            })
        });
        let mut official_min = 1.0_f32;
        let mut batch_min = 1.0_f32;
        for (images, reference) in images.chunks_exact(2).zip(reference.chunks_exact(2)) {
            let batch = encoder.embed(images.iter().map(prepare).collect()).unwrap();
            for ((image, expected), row) in images.iter().zip(reference).zip(batch) {
                let single = encoder.embed(vec![prepare(image)]).unwrap().remove(0);
                let official = crate::embed::cosine(&single, expected);
                let batch_official = crate::embed::cosine(&row, expected);
                let batch_single = crate::embed::cosine(&single, &row);
                official_min = official_min.min(official).min(batch_official);
                batch_min = batch_min.min(batch_single);
                assert!(official >= 0.999 && batch_official >= 0.999 && batch_single >= 0.999);
            }
        }
        println!("cosine_referensi_min={official_min:.8} cosine_batch_min={batch_min:.8}");
    }

    #[test]
    fn patches_follow_rgb_xy_and_padding() {
        assert_eq!(dimensions(1200, 800), (960, 624));
        let image = RgbImage::from_fn(768, 768, |x, y| {
            Rgb([(x % 256) as u8, (y % 256) as u8, 255])
        });
        let patches = prepare(&image);
        assert_eq!(patches.soft_tokens, 256);
        assert_eq!(&patches.positions[..6], &[0, 0, 1, 0, 2, 0]);
        assert_eq!(
            &patches.pixels[..6],
            &[0.0, 0.0, 1.0, 1.0 / 255.0, 0.0, 1.0]
        );
        assert_eq!(
            &patches.pixels[16 * 3..16 * 3 + 3],
            &[0.0, 1.0 / 255.0, 1.0]
        );
        assert!(
            patches.positions[2304 * 2..]
                .iter()
                .all(|&position| position == -1)
        );
        assert!(
            patches.pixels[2304 * RGB_PATCH..]
                .iter()
                .all(|&pixel| pixel == 0.0)
        );
    }
}
