//! Service Windows untuk server Hub.
//!
//! - `service install [--akun DOMAIN\user] [--nama NAMA] [config.toml]` memasang service "Bakase"
//!   yang jalan otomatis saat Windows menyala dan dinyalakan ulang kalau berhenti karena error.
//!   `--nama` dipakai untuk slot deploy tanpa downtime (misalnya APPHub-A dan APPHub-B),
//!   masing-masing dengan config dan port sendiri.
//!   Service memakai akun Windows (bawaan: user yang menjalankan perintah) supaya key Azure
//!   di Credential Manager akun itu terbaca. Password akun diminta sekali dan hanya
//!   diteruskan ke Windows.
//! - `service uninstall [--nama NAMA]` menghentikan dan menghapus service.
//! - `service run <config> --nama NAMA` dipanggil oleh Windows Service Manager, bukan oleh manusia.
use std::ffi::{OsStr, OsString};
use std::io::{BufRead, IsTerminal};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
    ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{
    self, ServiceControlHandlerResult, ServiceStatusHandle,
};
use windows_service::service_dispatcher;
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

/// Nama service bawaan kalau `--nama` tidak diisi.
pub const DEFAULT_NAME: &str = "APPHub";
const DESCRIPTION: &str =
    "Asisten kantor Example Corp (Bakase). Dipasang dengan `rag-hub-server service install`.";
/// Log yang lebih besar dari ini dipindah ke `rag-hub.1.log` saat service mulai.
const MAX_LOG_BYTES: u64 = 20 * 1024 * 1024;
/// Batas waktu mulai (migrasi database, membuka index) yang dilaporkan ke Windows dan
/// ditunggu oleh `install`.
const START_WAIT: Duration = Duration::from_secs(120);

/// Handle status service, supaya control handler bisa melapor StopPending.
static STATUS: std::sync::OnceLock<ServiceStatusHandle> = std::sync::OnceLock::new();
/// Lama menunggu jawaban yang masih mengalir saat berhenti, dari config (`shutdown_grace_secs`).
static STOP_GRACE: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();

/// Nama service: huruf, angka, `-`, dan `_`, paling panjang 64.
pub fn check_name(name: &str) -> Result<()> {
    anyhow::ensure!(
        !name.is_empty()
            && name.len() <= 64
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "nama service {name:?} tidak sah; pakai huruf, angka, - atau _"
    );
    Ok(())
}

fn display_name(name: &str) -> String {
    if name == DEFAULT_NAME {
        "Bakase".into()
    } else {
        format!("Bakase ({name})")
    }
}

/// Nama service yang sedang dijalankan Service Manager, dari argumen `--nama`.
fn running_name() -> String {
    let args: Vec<OsString> = std::env::args_os().collect();
    args.iter()
        .position(|a| a == "--nama")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.to_str())
        .unwrap_or(DEFAULT_NAME)
        .to_string()
}

pub fn install(config: &Path, account: Option<&str>, name: &str) -> Result<()> {
    check_name(name)?;
    // Path absolut tanpa awalan `\\?\`, supaya path relatif di config (`../data`) tetap
    // bisa dipakai dari folder config.
    let config = std::path::absolute(config)?;
    anyhow::ensure!(config.is_file(), "config {} tidak ada", config.display());
    let account = match account {
        Some(a) => normalize_account(a)?,
        None => current_account()?,
    };
    let password = read_password(&account)?;
    grant_service_logon(&account)
        .with_context(|| format!("gagal memberi hak \"Log on as a service\" ke {account}"))?;

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .context("tidak bisa membuka Service Manager; jalankan PowerShell sebagai Administrator")?;
    let info = ServiceInfo {
        name: name.into(),
        display_name: display_name(name).into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: std::env::current_exe()?,
        launch_arguments: vec![
            "service".into(),
            "run".into(),
            config.clone().into(),
            "--nama".into(),
            name.into(),
        ],
        dependencies: vec![],
        account_name: Some(account.clone().into()),
        account_password: Some(password.into()),
    };
    let service = manager
        .create_service(
            &info,
            ServiceAccess::CHANGE_CONFIG | ServiceAccess::START | ServiceAccess::QUERY_STATUS,
        )
        .context("gagal membuat service; kalau sudah ada, jalankan `service uninstall` dulu")?;
    service.set_description(DESCRIPTION)?;
    // Menunggu jaringan dan Credential Manager siap setelah boot.
    service.set_delayed_auto_start(true)?;
    let restart = ServiceAction {
        action_type: ServiceActionType::Restart,
        delay: Duration::from_secs(10),
    };
    service.update_failure_actions(ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 3600)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![restart.clone(), restart.clone(), restart]),
    })?;
    // Server yang berhenti dengan error (bukan crash) juga dinyalakan ulang.
    service.set_failure_actions_on_non_crash_failures(true)?;
    service.start::<&OsStr>(&[]).context(
        "service terpasang tapi gagal dinyalakan; cek password akun, lalu `service uninstall` dan pasang lagi",
    )?;

    // Service baru melapor Running setelah database terbuka dan port berhasil dipakai.
    let log = log_dir(&config);
    let deadline = Instant::now() + START_WAIT;
    loop {
        let state = service.query_status()?.current_state;
        if state == ServiceState::Running {
            break;
        }
        anyhow::ensure!(
            state != ServiceState::Stopped,
            "service berhenti lagi setelah dinyalakan; lihat {}",
            log.display()
        );
        anyhow::ensure!(
            Instant::now() < deadline,
            "service belum siap setelah {} detik; lihat {}",
            START_WAIT.as_secs(),
            log.display()
        );
        std::thread::sleep(Duration::from_millis(500));
    }
    println!(
        "Service \"{}\" ({name}) jalan dengan akun {account}.",
        display_name(name)
    );
    println!("Config: {}", config.display());
    println!("Log: {}", log.display());
    println!("Cek status: sc.exe query {name}");
    Ok(())
}

pub fn uninstall(name: &str) -> Result<()> {
    check_name(name)?;
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("tidak bisa membuka Service Manager; jalankan PowerShell sebagai Administrator")?;
    let service = manager
        .open_service(
            name,
            ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
        )
        .with_context(|| format!("service {name} tidak ditemukan"))?;
    if service.query_status()?.current_state != ServiceState::Stopped {
        service.stop()?;
        // Jawaban yang masih mengalir ditunggu sampai batas `shutdown_grace_secs` service itu.
        let deadline = Instant::now() + Duration::from_secs(300);
        while service.query_status()?.current_state != ServiceState::Stopped {
            anyhow::ensure!(
                Instant::now() < deadline,
                "service belum berhenti setelah 5 menit"
            );
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    service.delete()?;
    println!("Service {name} dihentikan dan dihapus.");
    Ok(())
}

windows_service::define_windows_service!(ffi_service_main, service_main);

/// Dipanggil dari `main` untuk `service run`, sebelum runtime tokio dibuat.
pub fn run() -> Result<()> {
    service_dispatcher::start(running_name(), ffi_service_main)
        .context("`service run` hanya dijalankan oleh Windows Service Manager")?;
    Ok(())
}

fn report(state: ServiceState, exit_code: ServiceExitCode, wait_hint: Duration) {
    let Some(status) = STATUS.get() else {
        return;
    };
    let controls_accepted = if state == ServiceState::Running {
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
    } else {
        ServiceControlAccept::empty()
    };
    let _ = status.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted,
        exit_code,
        checkpoint: 0,
        wait_hint,
        process_id: None,
    });
}

fn service_main(_args: Vec<OsString>) {
    // Path config dan nama ada di argumen proses (`service run <config> --nama N`), bukan
    // argumen start.
    let config = std::env::args_os().nth(3).map(PathBuf::from);
    let name = running_name();
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let handler = move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            report(
                ServiceState::StopPending,
                ServiceExitCode::Win32(0),
                STOP_GRACE.get().copied().unwrap_or(Duration::from_secs(10))
                    + Duration::from_secs(5),
            );
            let _ = stop_tx.send(true);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let Ok(status) = service_control_handler::register(&name, handler) else {
        return;
    };
    let _ = STATUS.set(status);
    report(
        ServiceState::StartPending,
        ServiceExitCode::Win32(0),
        START_WAIT,
    );

    let result = run_server(config, &name, stop_rx);
    if let Err(e) = &result {
        tracing::error!("server berhenti karena error: {e:#}");
    }
    // Exit code bukan nol membuat Windows menyalakan ulang service (failure actions).
    let exit_code = match result {
        Ok(()) => ServiceExitCode::Win32(0),
        Err(_) => ServiceExitCode::ServiceSpecific(1),
    };
    report(ServiceState::Stopped, exit_code, Duration::ZERO);
}

fn run_server(
    config: Option<PathBuf>,
    name: &str,
    stop: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let config = config.context("path config tidak ada di argumen service")?;
    // Log dibuka sebelum config dibaca, supaya config yang hilang atau rusak ikut tercatat.
    init_file_log(&log_dir(&config))?;
    tracing::info!("service {name} mulai");
    if let Some(dir) = config.parent() {
        std::env::set_current_dir(dir)?;
    }
    let cfg = crate::config::Config::load(&config)?;
    let _ = STOP_GRACE.set(Duration::from_secs(cfg.shutdown_grace_secs));
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(crate::serve(cfg, stop, || {
            report(
                ServiceState::Running,
                ServiceExitCode::Win32(0),
                Duration::ZERO,
            )
        }))
}

/// Folder log service: `logs` di samping config.toml.
fn log_dir(config: &Path) -> PathBuf {
    config
        .parent()
        .map(|dir| dir.join("logs"))
        .unwrap_or_else(|| PathBuf::from("logs"))
}

/// Service tidak punya konsol, jadi log ditulis ke `<folder config>/logs/rag-hub.log`.
fn init_file_log(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("rag-hub.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = std::fs::rename(&path, dir.join("rag-hub.1.log"));
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("tidak bisa menulis log {}", path.display()))?;
    use tracing_subscriber::{Layer, layer::SubscriberExt, util::SubscriberInitExt};
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(file))
                .with_filter(crate::log_filter(crate::SERVER_LOG_FILTER)),
        )
        .init();
    Ok(())
}

/// Akun yang menjalankan perintah ini (`DOMAIN\user`, akun lokal `KOMPUTER\user`), dibaca dari
/// token Windows. `USERDOMAIN` tidak dipakai: lewat SSH di komputer workgroup isinya `WORKGROUP`.
fn current_account() -> Result<String> {
    use windows_sys::Win32::Security::Authentication::Identity::{
        GetUserNameExW, NameSamCompatible,
    };
    let mut len = 0u32;
    // Panggilan pertama hanya menanyakan panjang (termasuk nol penutup).
    unsafe { GetUserNameExW(NameSamCompatible, std::ptr::null_mut(), &mut len) };
    anyhow::ensure!(len > 0, "nama akun Windows tidak bisa dibaca");
    let mut buf = vec![0u16; len as usize];
    let ok = unsafe { GetUserNameExW(NameSamCompatible, buf.as_mut_ptr(), &mut len) };
    anyhow::ensure!(
        ok,
        "nama akun Windows tidak bisa dibaca: {}",
        std::io::Error::last_os_error()
    );
    Ok(String::from_utf16_lossy(&buf[..len as usize]))
}

/// `LookupAccountNameW` tidak menerima `.\user`, jadi akun lokal ditulis sebagai
/// `KOMPUTER\user`. Bentuk `DOMAIN\user` dan `user@domain` dipakai apa adanya; nama tanpa
/// domain dianggap akun lokal.
fn normalize_account(account: &str) -> Result<String> {
    let account = account.trim();
    anyhow::ensure!(!account.is_empty(), "nama akun kosong");
    let local = |user: &str| -> Result<String> {
        let computer = std::env::var("COMPUTERNAME").context("COMPUTERNAME tidak ada")?;
        Ok(format!("{computer}\\{user}"))
    };
    match account.strip_prefix(".\\") {
        Some(user) => local(user),
        None if account.contains(['\\', '@']) => Ok(account.to_string()),
        None => local(account),
    }
}

/// Password diketik tanpa tampil; dari skrip, dibaca dari baris pertama stdin.
fn read_password(account: &str) -> Result<String> {
    let password = if std::io::stdin().is_terminal() {
        rpassword::prompt_password(format!(
            "Password Windows untuk {account} (dipakai service, tidak disimpan Hub): "
        ))?
    } else {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        line.trim_end_matches(['\r', '\n']).to_string()
    };
    anyhow::ensure!(!password.is_empty(), "password kosong");
    Ok(password)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Windows tidak otomatis memberi hak "Log on as a service" ke akun yang dipasang lewat
/// API (services.msc melakukannya diam-diam). Tanpa hak ini service gagal start (1069).
fn grant_service_logon(account: &str) -> Result<()> {
    use windows_sys::Win32::Foundation::NTSTATUS;
    use windows_sys::Win32::Security::Authentication::Identity::{
        LSA_HANDLE, LSA_OBJECT_ATTRIBUTES, LSA_UNICODE_STRING, LsaAddAccountRights, LsaClose,
        LsaNtStatusToWinError, LsaOpenPolicy, POLICY_CREATE_ACCOUNT, POLICY_LOOKUP_NAMES,
    };
    use windows_sys::Win32::Security::{LookupAccountNameW, SID_NAME_USE};

    fn check(status: NTSTATUS) -> Result<()> {
        if status == 0 {
            return Ok(());
        }
        // SAFETY: fungsi murni yang menerjemahkan kode status.
        let code = unsafe { LsaNtStatusToWinError(status) };
        Err(std::io::Error::from_raw_os_error(code as i32).into())
    }

    let name = wide(account);
    let (mut sid_len, mut domain_len, mut kind) = (0u32, 0u32, 0 as SID_NAME_USE);
    // SAFETY: panggilan pertama hanya menanyakan ukuran buffer (buffer null, ukuran 0).
    unsafe {
        LookupAccountNameW(
            std::ptr::null(),
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut sid_len,
            std::ptr::null_mut(),
            &mut domain_len,
            &mut kind,
        );
    }
    anyhow::ensure!(sid_len > 0, "akun {account} tidak ditemukan");
    // u32 supaya SID ter-align seperti yang diharapkan Windows.
    let mut sid = vec![0u32; (sid_len as usize).div_ceil(4)];
    let mut domain = vec![0u16; domain_len as usize];
    // SAFETY: buffer sesuai ukuran yang dilaporkan panggilan pertama.
    let ok = unsafe {
        LookupAccountNameW(
            std::ptr::null(),
            name.as_ptr(),
            sid.as_mut_ptr().cast(),
            &mut sid_len,
            domain.as_mut_ptr(),
            &mut domain_len,
            &mut kind,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("akun {account} tidak ditemukan"));
    }

    let attributes = LSA_OBJECT_ATTRIBUTES {
        Length: std::mem::size_of::<LSA_OBJECT_ATTRIBUTES>() as u32,
        ..Default::default()
    };
    // SAFETY: handle diisi oleh LsaOpenPolicy dan ditutup di bawah.
    let mut policy: LSA_HANDLE = unsafe { std::mem::zeroed() };
    check(unsafe {
        LsaOpenPolicy(
            std::ptr::null(),
            &attributes,
            (POLICY_CREATE_ACCOUNT | POLICY_LOOKUP_NAMES) as u32,
            &mut policy,
        )
    })?;
    let mut right_name = wide("SeServiceLogonRight");
    let bytes = ((right_name.len() - 1) * 2) as u16;
    let right = LSA_UNICODE_STRING {
        Length: bytes,
        MaximumLength: bytes + 2,
        Buffer: right_name.as_mut_ptr(),
    };
    // SAFETY: policy terbuka, SID valid dari LookupAccountNameW, satu hak.
    let status = unsafe { LsaAddAccountRights(policy, sid.as_mut_ptr().cast(), &right, 1) };
    // SAFETY: menutup handle yang dibuka di atas.
    unsafe { LsaClose(policy) };
    check(status)
}

#[cfg(test)]
mod tests {
    use super::{check_name, current_account, display_name, normalize_account};

    #[test]
    fn current_account_has_a_domain() {
        let account = current_account().unwrap();
        let (domain, user) = account.split_once('\\').expect(&account);
        assert!(!domain.is_empty() && !user.is_empty(), "{account}");
        assert!(!domain.eq_ignore_ascii_case("WORKGROUP"), "{account}");
    }

    #[test]
    fn slot_names_are_checked() {
        for ok in ["APPHub", "APPHub-A", "APPHub_B2"] {
            assert!(check_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "Demo Service", "a/b", "x;y", &"n".repeat(65)] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
        assert_eq!(display_name("APPHub"), "Bakase");
        assert_eq!(display_name("APPHub-A"), "Bakase (APPHub-A)");
    }

    #[test]
    fn local_accounts_get_the_computer_name() {
        let computer = std::env::var("COMPUTERNAME").unwrap();
        assert_eq!(
            normalize_account(r".\hubsvc").unwrap(),
            format!(r"{computer}\hubsvc")
        );
        assert_eq!(
            normalize_account("hubsvc").unwrap(),
            format!(r"{computer}\hubsvc")
        );
        assert_eq!(
            normalize_account(r"EXAMPLE\maintainer").unwrap(),
            r"EXAMPLE\maintainer"
        );
        assert_eq!(
            normalize_account("maintainer@example.com").unwrap(),
            "maintainer@example.com"
        );
        assert!(normalize_account("  ").is_err());
    }
}
