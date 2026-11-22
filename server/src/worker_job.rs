/// Job Object Windows milik server untuk satu proses pembaca: batas memori, mati kalau
/// handle ini tertutup (drop, atau server mati), dan mati kalau ada exception.
#[cfg(windows)]
pub(crate) struct WorkerJob(windows_sys::Win32::Foundation::HANDLE);

// SAFETY: handle job berlaku di seluruh proses dan hanya ditutup sekali di `Drop`.
#[cfg(windows)]
unsafe impl Send for WorkerJob {}

#[cfg(windows)]
impl WorkerJob {
    pub(crate) fn attach(child: &tokio::process::Child, memory: usize) -> anyhow::Result<Self> {
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW,
            JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectExtendedLimitInformation, SetInformationJobObject,
        };
        let process = child
            .raw_handle()
            .ok_or_else(|| anyhow::anyhow!("proses pembaca sudah selesai"))?;
        // SAFETY: panggilan Win32 dengan struct yang diisi nol lalu diisi field-nya. Kalau
        // gagal di tengah, `Self` sudah memegang handle-nya dan menutupnya di `Drop`.
        unsafe {
            let job = Self(CreateJobObjectW(std::ptr::null(), std::ptr::null()));
            anyhow::ensure!(!job.0.is_null(), "job object tidak bisa dibuat");
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY
                | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
            info.ProcessMemoryLimit = memory;
            let set = SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            anyhow::ensure!(set != 0, "batas memori job object tidak bisa dipasang");
            anyhow::ensure!(
                AssignProcessToJobObject(job.0, process as _) != 0,
                "proses pembaca tidak bisa masuk job object"
            );
            Ok(job)
        }
    }
}

#[cfg(windows)]
impl Drop for WorkerJob {
    fn drop(&mut self) {
        // SAFETY: handle dari CreateJobObjectW, ditutup sekali.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(not(windows))]
pub(crate) struct WorkerJob;

#[cfg(not(windows))]
impl WorkerJob {
    pub(crate) fn attach(_child: &tokio::process::Child, _memory: usize) -> anyhow::Result<Self> {
        Ok(Self)
    }
}

#[cfg(windows)]
pub(crate) struct Deadline {
    cancel: Option<std::sync::mpsc::Sender<()>>,
    timer: Option<std::thread::JoinHandle<()>>,
}

#[cfg(windows)]
impl WorkerJob {
    pub(crate) fn deadline(self, duration: std::time::Duration) -> std::io::Result<Deadline> {
        let (cancel, wait) = std::sync::mpsc::channel();
        // Callback inferensi dapat memblokir runtime yang membaca kanal anak.
        let timer = std::thread::Builder::new().spawn(move || {
            if matches!(
                wait.recv_timeout(duration),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ) {
                // SAFETY: job ini hanya berisi proses anak yang dipasang oleh pembaca.
                unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1) };
            }
            drop(self);
        })?;
        Ok(Deadline {
            cancel: Some(cancel),
            timer: Some(timer),
        })
    }
}

#[cfg(windows)]
impl Drop for Deadline {
    fn drop(&mut self) {
        drop(self.cancel.take());
        if let Some(timer) = self.timer.take() {
            let _ = timer.join();
        }
    }
}
