use super::{Failure, Model, Pooling, model_path};
use anyhow::{Result, anyhow, ensure};
use ort::{
    session::{OutputSelector, RunOptions, Session},
    value::{Outlet, Tensor, TensorElementType, ValueType},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokenizers::{Tokenizer, TruncationParams};

pub(crate) trait Embedder: Send {
    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

pub(crate) fn default_threads() -> usize {
    (std::thread::available_parallelism().map_or(1, usize::from) / 2).max(1)
}

fn runtime_error(message: &'static str) -> anyhow::Error {
    anyhow::Error::new(Failure::Runtime).context(message)
}

pub(crate) struct OrtEmbedder {
    session: Session,
    tokenizer: Tokenizer,
    pad_id: u32,
    output: String,
    prepooled: bool,
    dim: usize,
    pooling: Pooling,
}

impl OrtEmbedder {
    pub(crate) fn open(
        folder: &Path,
        model: &Model,
        dll: Option<&Path>,
        threads: usize,
        gpu: bool,
    ) -> Result<Self> {
        Self::open_model(folder, model, dll, threads, gpu, false)
    }

    pub(crate) fn image_query(folder: &Path, dll: Option<&Path>) -> Result<Self> {
        let model = Model {
            name: "embeddinggemma-2".into(),
            dim: 768,
            pooling: Pooling::Mean,
            max_tokens: 512,
            query_prefix: String::new(),
            document_prefix: String::new(),
            normalize: true,
            hash: String::new(),
            artifacts: None,
        };
        Self::open_model(folder, &model, dll, 2, false, true)
    }

    fn open_model(
        folder: &Path,
        model: &Model,
        dll: Option<&Path>,
        threads: usize,
        gpu: bool,
        multimodal: bool,
    ) -> Result<Self> {
        if gpu && !cfg!(feature = "cuda") {
            return Err(runtime_error(
                "Build ini belum mendukung CUDA. Build dengan --features cuda.",
            ));
        }
        let dll = dll
            .map(Path::to_path_buf)
            .or_else(|| std::env::var_os("ORT_DYLIB_PATH").map(PathBuf::from))
            .ok_or_else(|| {
                runtime_error("Isi ORT_DYLIB_PATH atau --ort-dylib dengan DLL ONNX Runtime lokal.")
            })?;
        let dll = dll
            .canonicalize()
            .map_err(|_| runtime_error("DLL ONNX Runtime tidak ditemukan."))?;
        ort::init_from(dll)
            .map_err(|_| {
                runtime_error("DLL ONNX Runtime gagal dimuat. Periksa versi dan DLL pendukung.")
            })?
            .with_logger(Arc::new(|_, _, _, _, _| {}))
            .commit();

        let mut tokenizer = Tokenizer::from_file(folder.join("tokenizer.json"))
            .map_err(|_| anyhow!("tokenizer.json gagal dibaca."))?;
        let pad_id = tokenizer
            .get_padding()
            .map(|pad| pad.pad_id)
            .or_else(|| tokenizer.token_to_id("<pad>"))
            .or_else(|| tokenizer.token_to_id("<|endoftext|>"))
            .ok_or_else(|| anyhow!("Token padding tidak ditemukan."))?;
        tokenizer.with_padding(None);
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: model.max_tokens,
                ..Default::default()
            }))
            .map_err(|_| anyhow!("Batas token model tidak valid."))?;

        let setup = || runtime_error("Sesi ONNX gagal disiapkan. Periksa DLL dan jumlah thread.");
        let builder = Session::builder()
            .map_err(|_| setup())?
            .with_intra_threads(threads)
            .map_err(|_| setup())?
            .with_intra_op_spinning(false)
            .map_err(|_| setup())?
            .with_inter_threads(1)
            .map_err(|_| setup())?;
        #[cfg(feature = "cuda")]
        let builder = if gpu {
            builder
                .with_execution_providers([ort::ep::CUDA::default().build().error_on_failure()])
                .map_err(|_| {
                    runtime_error(
                        "CUDA gagal diaktifkan. Periksa DLL CUDA dan cuDNN di PATH proses.",
                    )
                })?
        } else {
            builder
        };
        let mut builder = builder;
        let session = builder.commit_from_file(model_path(folder))
            .map_err(|_| runtime_error("Model ONNX gagal dimuat. Periksa file model, bobot eksternal dan versi ONNX Runtime."))?;
        for input in session.inputs() {
            ensure!(
                (multimodal && empty_features(input))
                    || past_cache_shape(input, 1).is_some()
                    || (matches!(
                        input.name(),
                        "input_ids" | "attention_mask" | "position_ids" | "token_type_ids"
                    ) && matches!(input.dtype(), ValueType::Tensor { ty: TensorElementType::Int64, shape, .. } if shape.len() == 2)),
                "Input ONNX tidak didukung."
            );
        }
        let mut outputs = session.outputs().iter().filter_map(|output| {
            if multimodal && output.name() != "sentence_embedding" {
                return None;
            }
            let ValueType::Tensor {
                ty: TensorElementType::Float32,
                shape,
                ..
            } = output.dtype()
            else {
                return None;
            };
            match shape.as_ref() {
                [_, _, dim] if *dim == model.dim as i64 => Some((output.name(), false)),
                [_, dim] if *dim == model.dim as i64 && output.name() == "sentence_embedding" => {
                    Some((output.name(), true))
                }
                _ => None,
            }
        });
        let (output, prepooled) = outputs
            .next()
            .ok_or_else(|| anyhow!("Output ONNX dengan dimensi model tidak ditemukan."))?;
        ensure!(outputs.next().is_none(), "Output token ONNX ambigu.");
        let output = output.to_owned();
        Ok(Self {
            session,
            tokenizer,
            pad_id,
            output,
            prepooled,
            dim: model.dim,
            pooling: model.pooling,
        })
    }
}

fn empty_features(input: &Outlet) -> bool {
    matches!(
        input.name(),
        "image_features" | "audio_features" | "video_features"
    ) && matches!(input.dtype(), ValueType::Tensor { ty: TensorElementType::Float32, shape, .. }
            if matches!(shape.as_ref(), [_, 512]))
}

fn past_cache_shape(input: &Outlet, batch: usize) -> Option<[i64; 4]> {
    let (layer, item) = input
        .name()
        .strip_prefix("past_key_values.")?
        .split_once('.')?;
    if layer.parse::<usize>().is_err() || !matches!(item, "key" | "value") {
        return None;
    }
    match input.dtype() {
        ValueType::Tensor {
            ty: TensorElementType::Float32,
            shape,
            ..
        } => match shape.as_ref() {
            [_, heads, past, size] if *heads > 0 && *past <= 0 && *size > 0 => {
                Some([batch as i64, *heads, 0, *size])
            }
            _ => None,
        },
        _ => None,
    }
}

impl Embedder for OrtEmbedder {
    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(self
            .embed_counted(texts)?
            .into_iter()
            .map(|(vector, _)| vector)
            .collect())
    }
}

impl OrtEmbedder {
    pub(super) fn embed_counted(&mut self, texts: &[String]) -> Result<Vec<(Vec<f32>, bool)>> {
        let encodings = texts
            .iter()
            .map(|text| self.tokenizer.encode(text.as_str(), true))
            .collect::<tokenizers::Result<Vec<_>>>()
            .map_err(|_| anyhow!("Tokenisasi gagal."))?;
        let width = encodings.iter().map(|e| e.len()).max().unwrap_or(0);
        ensure!(width > 0, "Tokenisasi menghasilkan masukan kosong.");
        let batch = encodings.len();
        let mut mask = Vec::with_capacity(batch * width);
        for e in &encodings {
            mask.extend((0..width).map(|i| i < e.len()));
        }
        let failed = || anyhow!("Inferensi ONNX gagal.");
        let mut inputs = Vec::new();
        for input in self.session.inputs() {
            if empty_features(input) {
                let empty =
                    Tensor::from_array(([0, 512], Vec::<f32>::new())).map_err(|_| failed())?;
                inputs.push((input.name().to_owned(), empty.into_dyn()));
                continue;
            }
            // Embedding memakai satu forward pass, jadi ekspor dengan KV cache menerima past kosong.
            if let Some(shape) = past_cache_shape(input, batch) {
                let empty = Tensor::from_array((shape, Vec::<f32>::new())).map_err(|_| failed())?;
                inputs.push((input.name().to_owned(), empty.into_dyn()));
                continue;
            }
            let values: Vec<i64> = match input.name() {
                "input_ids" => encodings
                    .iter()
                    .flat_map(|e| {
                        (0..width)
                            .map(|i| i64::from(e.get_ids().get(i).copied().unwrap_or(self.pad_id)))
                    })
                    .collect(),
                "attention_mask" => mask.iter().map(|&m| i64::from(m)).collect(),
                "position_ids" => encodings
                    .iter()
                    .flat_map(|e| (0..width).map(|i| if i < e.len() { i as i64 } else { 0 }))
                    .collect(),
                "token_type_ids" => encodings
                    .iter()
                    .flat_map(|e| {
                        (0..width).map(|i| i64::from(e.get_type_ids().get(i).copied().unwrap_or(0)))
                    })
                    .collect(),
                _ => unreachable!(),
            };
            inputs.push((
                input.name().to_owned(),
                Tensor::from_array(([batch, width], values))
                    .map_err(|_| failed())?
                    .into_dyn(),
            ));
        }
        let options = RunOptions::new()
            .map_err(|_| failed())?
            .with_outputs(OutputSelector::no_default().with(&self.output));
        let outputs = self
            .session
            .run_with_options(inputs, &options)
            .map_err(|_| failed())?;
        let (shape, values) = outputs
            .get(&self.output)
            .ok_or_else(failed)?
            .try_extract_tensor::<f32>()
            .map_err(|_| failed())?;
        let rows = if self.prepooled {
            ensure!(
                shape.as_ref() == [batch as i64, self.dim as i64],
                "Ukuran output ONNX tidak sesuai."
            );
            values.chunks_exact(self.dim).map(<[f32]>::to_vec).collect()
        } else {
            ensure!(
                shape.as_ref() == [batch as i64, width as i64, self.dim as i64],
                "Ukuran output ONNX tidak sesuai."
            );
            pool(values, &mask, width, self.dim, self.pooling)?
        };
        rows.into_iter()
            .zip(encodings)
            .map(|(row, encoding)| Ok((normalize(row)?, !encoding.get_overflowing().is_empty())))
            .collect()
    }
}

fn pool(
    values: &[f32],
    mask: &[bool],
    width: usize,
    dim: usize,
    pooling: Pooling,
) -> Result<Vec<Vec<f32>>> {
    values
        .chunks_exact(width * dim)
        .zip(mask.chunks_exact(width))
        .map(|(row, mask)| {
            let token = |i| row[i * dim..(i + 1) * dim].to_vec();
            match pooling {
                Pooling::Cls => Ok(token(0)),
                Pooling::Last => mask
                    .iter()
                    .rposition(|&present| present)
                    .map(token)
                    .ok_or_else(|| anyhow!("Tidak ada token untuk pooling.")),
                Pooling::Mean => {
                    let mut mean = vec![0.0; dim];
                    let mut count = 0;
                    for (token, &present) in row.chunks_exact(dim).zip(mask) {
                        if present {
                            for (sum, value) in mean.iter_mut().zip(token) {
                                *sum += value;
                            }
                            count += 1;
                        }
                    }
                    ensure!(count > 0, "Tidak ada token untuk pooling.");
                    mean.iter_mut().for_each(|value| *value /= count as f32);
                    Ok(mean)
                }
            }
        })
        .collect()
}

fn normalize(mut row: Vec<f32>) -> Result<Vec<f32>> {
    let norm = cosine(&row, &row).sqrt();
    ensure!(
        norm.is_finite() && norm > 0.0,
        "Norma vektor model tidak valid."
    );
    row.iter_mut().for_each(|value| *value /= norm);
    Ok(row)
}

// Dot product is cosine similarity for L2-normalized vectors.
pub(crate) fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let mut lanes = [0.0; 8];
    let (a_blocks, a_tail) = a.as_chunks::<8>();
    let (b_blocks, b_tail) = b.as_chunks::<8>();
    for (a, b) in a_blocks.iter().zip(b_blocks) {
        for i in 0..lanes.len() {
            lanes[i] += a[i] * b[i];
        }
    }
    lanes.into_iter().sum::<f32>() + a_tail.iter().zip(b_tail).map(|(a, b)| a * b).sum::<f32>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pooling_uses_only_requested_tokens() {
        let values = [2., 4., 6., 8., 80., 90., 3., 5., 7., 9., 11., 13.];
        let mask = [true, true, false, true, true, true];
        for (pooling, expected) in [
            (Pooling::Cls, [[2., 4.], [3., 5.]]),
            (Pooling::Mean, [[4., 6.], [7., 9.]]),
            (Pooling::Last, [[6., 8.], [11., 13.]]),
        ] {
            assert_eq!(pool(&values, &mask, 3, 2, pooling).unwrap(), expected);
        }
    }

    #[test]
    fn l2_normalizes_and_rejects_invalid_norms() {
        assert_eq!(normalize(vec![3., 4.]).unwrap(), [0.6, 0.8]);
        assert!(normalize(vec![0., 0.]).is_err());
        assert!(normalize(vec![f32::NAN]).is_err());
    }

    #[test]
    #[ignore = "needs public model files and a local CPU ONNX Runtime DLL"]
    fn real_model_smoke() {
        let folder = PathBuf::from(std::env::var_os("app_SMOKE_MODEL").unwrap());
        let model = Model::load(&folder).unwrap();
        let mut embedder = OrtEmbedder::open(&folder, &model, None, 2, false).unwrap();
        println!(
            "{} | input {:?} | output {:?} | dim {} | threads {}",
            model.name,
            embedder
                .session
                .inputs()
                .iter()
                .map(|x| x.name())
                .collect::<Vec<_>>(),
            embedder
                .session
                .outputs()
                .iter()
                .map(|x| x.name())
                .collect::<Vec<_>>(),
            model.dim,
            2
        );
        let started = std::time::Instant::now();
        let docs = embedder
            .embed(&[
                format!(
                    "{}Bibit tomat sebaiknya disiram setiap pagi sebelum matahari terik.",
                    model.document_prefix
                ),
                format!(
                    "{}A violin has four strings and is usually played with a bow.",
                    model.document_prefix
                ),
                format!(
                    "{}Kereta malam itu berhenti di tiga stasiun sebelum mencapai pantai.",
                    model.document_prefix
                ),
            ])
            .unwrap();
        let document_time = started.elapsed().as_secs_f64();
        let started = std::time::Instant::now();
        let queries = embedder
            .embed(&[
                format!(
                    "{}Kapan waktu terbaik untuk menyiram tanaman tomat?",
                    model.query_prefix
                ),
                format!("{}How many strings does a violin have?", model.query_prefix),
            ])
            .unwrap();
        println!(
            "batch dokumen (3) {document_time:.3}s | batch pertanyaan (2) {:.3}s",
            started.elapsed().as_secs_f64()
        );
        let query = [format!(
            "{}Kapan waktu terbaik untuk menyiram tanaman tomat?",
            model.query_prefix
        )];
        let mut times = Vec::new();
        for _ in 0..11 {
            let started = std::time::Instant::now();
            std::hint::black_box(embedder.embed(&query).unwrap());
            times.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_unstable_by(f64::total_cmp);
        println!(
            "pertanyaan tunggal CPU 2 thread (11 kali) | median {:.2} ms | maks {:.2} ms",
            times[5], times[10]
        );
        for (i, query) in queries.iter().enumerate() {
            let matching = cosine(query, &docs[i]);
            let unrelated = docs
                .iter()
                .enumerate()
                .filter(|(j, _)| i != *j)
                .map(|(_, doc)| cosine(query, doc))
                .fold(f32::NEG_INFINITY, f32::max);
            println!(
                "pasangan {} | cocok {matching:.4} | lainnya (maks) {unrelated:.4}",
                i + 1
            );
            assert!(matching > unrelated);
        }
    }
}
