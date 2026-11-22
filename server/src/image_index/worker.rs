use super::extract::{self, Accept, Counts, Kind};
use anyhow::{Result, anyhow, ensure};
use image::RgbImage;
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) const COMMAND: &str = "__baca-gambar";
const MEMORY_BYTES: usize = 768 * 1024 * 1024;
const MAX_HEADER: usize = 256 * 1024;
const TIME_LIMIT: Duration = Duration::from_secs(120);

#[derive(Serialize, Deserialize)]
pub(super) struct Request {
    pub(super) path: PathBuf,
    pub(super) selection: Selection,
}

#[derive(Serialize, Deserialize)]
pub(super) enum Selection {
    All(Kind),
    One(String),
    Place { kind: String, media: String },
}

impl Request {
    fn extract(self, emit: &mut Accept<'_>) -> Result<Counts> {
        match self.selection {
            Selection::All(kind) => extract::visit(&self.path, kind, emit),
            Selection::One(location) => {
                if let Some(image) = extract::at(&self.path, &location) {
                    emit(location, image)?;
                }
                Ok(Counts::default())
            }
            Selection::Place { .. } => {
                anyhow::bail!("Permintaan lokasi tidak menghasilkan piksel.")
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
enum Frame {
    Image(String, u32, u32),
    Done(Counts),
    Place(Option<(String, Option<u32>)>),
}

pub(crate) fn run() -> Result<()> {
    std::panic::set_hook(Box::new(|_| {}));
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(read)?
        .join()
        .map_err(|_| anyhow!("Pembaca gambar berhenti."))?
}

fn read() -> Result<()> {
    let mut line = Vec::new();
    let mut input = BufReader::new(std::io::stdin().lock()).take(MAX_HEADER as u64);
    input.read_until(b'\n', &mut line)?;
    ensure!(line.ends_with(b"\n"), "Permintaan pembaca terlalu panjang.");
    let request: Request = serde_json::from_slice(&line)?;
    crate::attachments::ensure_limited(MEMORY_BYTES)?;
    let mut out = BufWriter::new(std::io::stdout());
    if let Selection::Place { kind, media } = request.selection {
        send(
            &mut out,
            &Frame::Place(extract::office_place(&request.path, &kind, &media)),
        )?;
        out.flush()?;
        return Ok(());
    }
    let counts = request.extract(&mut |location, image| {
        send(
            &mut out,
            &Frame::Image(location, image.width(), image.height()),
        )?;
        out.write_all(image.as_raw())?;
        out.flush()?;
        Ok(false)
    })?;
    send(&mut out, &Frame::Done(counts))?;
    out.flush()?;
    Ok(())
}

fn send(out: &mut impl Write, frame: &Frame) -> Result<()> {
    let bytes = serde_json::to_vec(frame)?;
    ensure!(
        bytes.len() <= MAX_HEADER,
        "Kepala keluaran gambar terlalu panjang."
    );
    out.write_all(&(bytes.len() as u32).to_le_bytes())?;
    out.write_all(&bytes)?;
    Ok(())
}

#[cfg(not(test))]
pub(super) async fn visit(request: Request, emit: &mut Accept<'_>) -> Result<Counts> {
    if matches!(
        request.selection,
        Selection::All(Kind::Unsupported | Kind::Unreadable)
    ) {
        return request.extract(emit);
    }
    visit_process(&std::env::current_exe()?, request, emit).await
}

async fn visit_process(exe: &Path, request: Request, emit: &mut Accept<'_>) -> Result<Counts> {
    let result = tokio::time::timeout(TIME_LIMIT, visit_inner(exe, request, emit))
        .await
        .map_err(|_| anyhow!("Waktu pembaca gambar habis."))??;
    match result {
        Frame::Done(counts) => Ok(counts),
        _ => anyhow::bail!("Keluaran pembaca gambar tidak sesuai."),
    }
}

pub(super) async fn place(
    path: PathBuf,
    kind: String,
    media: String,
) -> Option<(String, Option<u32>)> {
    #[cfg(test)]
    let exe = match std::env::var_os("APP_IMAGE_WORKER_EXE") {
        Some(exe) => PathBuf::from(exe),
        None => return extract::office_place(&path, &kind, &media),
    };
    #[cfg(not(test))]
    let exe = std::env::current_exe().ok()?;
    let request = Request {
        path,
        selection: Selection::Place { kind, media },
    };
    match visit_inner(&exe, request, &mut |_, _| {
        anyhow::bail!("Lokasi tidak menghasilkan piksel.")
    })
    .await
    .ok()?
    {
        Frame::Place(place) => place,
        _ => None,
    }
}

async fn visit_inner(exe: &Path, request: Request, emit: &mut Accept<'_>) -> Result<Frame> {
    use std::process::Stdio;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let started = Instant::now();
    let mut command = tokio::process::Command::new(exe);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = command
        .arg(COMMAND)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let job = crate::worker_job::WorkerJob::attach(&child, MEMORY_BYTES)?;
    #[cfg(windows)]
    let _job = job.deadline(TIME_LIMIT.saturating_sub(started.elapsed()))?;
    #[cfg(not(windows))]
    let _job = job;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("Kanal masukan tidak tersedia."))?;
    let mut bytes = serde_json::to_vec(&request)?;
    ensure!(
        bytes.len() < MAX_HEADER,
        "Permintaan pembaca terlalu panjang."
    );
    bytes.push(b'\n');
    stdin.write_all(&bytes).await?;
    drop(stdin);
    let mut out = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Kanal keluaran tidak tersedia."))?;
    let mut duplicate = 0;
    loop {
        ensure!(
            started.elapsed() < TIME_LIMIT,
            "Waktu pembaca gambar habis."
        );
        let length = out.read_u32_le().await? as usize;
        ensure!(
            length <= MAX_HEADER,
            "Kepala keluaran gambar terlalu panjang."
        );
        let mut header = vec![0; length];
        out.read_exact(&mut header).await?;
        match serde_json::from_slice(&header)? {
            Frame::Image(location, width, height) => {
                ensure!(
                    extract::allowed_size(width, height),
                    "Dimensi keluaran gambar terlalu besar."
                );
                let mut pixels = vec![0; width as usize * height as usize * 3];
                out.read_exact(&mut pixels).await?;
                let image = RgbImage::from_raw(width, height, pixels)
                    .ok_or_else(|| anyhow!("Piksel keluaran tidak valid."))?;
                duplicate += u64::from(emit(location, image)?);
            }
            mut result @ (Frame::Done(_) | Frame::Place(_)) => {
                ensure!(child.wait().await?.success(), "Pembaca gambar gagal.");
                ensure!(
                    started.elapsed() < TIME_LIMIT,
                    "Waktu pembaca gambar habis."
                );
                if let Frame::Done(counts) = &mut result {
                    counts.duplicate = duplicate;
                }
                return Ok(result);
            }
        }
    }
}

#[cfg(test)]
pub(super) async fn visit(request: Request, emit: &mut Accept<'_>) -> Result<Counts> {
    if let Some(exe) = std::env::var_os("APP_IMAGE_WORKER_EXE") {
        return visit_process(Path::new(&exe), request, emit).await;
    }
    request.extract(emit)
}
