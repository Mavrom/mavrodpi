use serde::Serialize;
use sha2::{Digest, Sha256};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use tauri::Manager;
use windows_service::service::{
    Service, ServiceAccess, ServiceAction, ServiceActionType, ServiceConfig, ServiceDependency,
    ServiceErrorControl, ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo,
    ServiceStartType, ServiceState, ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

const SERVICE_NAME: &str = "MavroDPI";
const SERVICE_ROOT_DIRECTORY: &str = r"C:\ProgramData\MavroDPI";
const INSTALL_DIRECTORY: &str = r"C:\ProgramData\MavroDPI\service";
const STAGING_DIRECTORY: &str = r"C:\ProgramData\MavroDPI\service.staging";
const BACKUP_DIRECTORY: &str = r"C:\ProgramData\MavroDPI\service.backup";
const SERVICE_BINARY: &str = "mavrodpi-svc.exe";
const EXPECTED_SERVICE_SHA256: &str = env!("MAVRODPI_SERVICE_SHA256");
const MIN_SERVICE_BINARY_BYTES: u64 = 64 * 1024;
const SERVICE_OPERATION_TIMEOUT: Duration = Duration::from_secs(20);
const FILE_OPERATION_RETRIES: usize = 20;
const FILE_OPERATION_RETRY_DELAY: Duration = Duration::from_millis(250);
const BUNDLED_FILE_HASHES: [(&str, &str); 3] = [
    (
        "goodbyedpi.exe",
        "331ac6c1d22ba5a0a217f3f27d0d823051869cafc8b8ef7f2002fa2accebc74e",
    ),
    (
        "WinDivert.dll",
        "a97859785a2df1d4462e7d48d33ccbd89fedd40dac4970f4afd89e63f59ee1ec",
    ),
    (
        "WinDivert64.sys",
        "53ab28ec00be6e6f8aefa9ee76fc2735e94d7f3f9dbc06eb2b7ac8cd3084a6af",
    ),
];

const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;
const ERROR_SERVICE_CANNOT_ACCEPT_CTRL: i32 = 1061;
const ERROR_SERVICE_NOT_ACTIVE: i32 = 1062;
const ERROR_SERVICE_ALREADY_RUNNING: i32 = 1056;
const ERROR_SERVICE_MARKED_FOR_DELETE: i32 = 1072;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreservedServiceState {
    Stopped,
    Running,
    Paused,
}

#[derive(Debug)]
struct ActivationFailure {
    message: String,
    previous_service_state_restorable: bool,
}

impl ActivationFailure {
    fn new(message: impl Into<String>, previous_service_state_restorable: bool) -> Self {
        Self {
            message: message.into(),
            previous_service_state_restorable,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatus {
    pub installed: bool,
    pub running: bool,
    pub state: String,
    pub binary_path_current: bool,
    pub helper_hash_current: bool,
    pub payload_current: bool,
    pub needs_repair: bool,
}

fn install_dir() -> PathBuf {
    PathBuf::from(INSTALL_DIRECTORY)
}

fn service_root_dir() -> PathBuf {
    PathBuf::from(SERVICE_ROOT_DIRECTORY)
}

fn staging_dir() -> PathBuf {
    PathBuf::from(STAGING_DIRECTORY)
}

fn backup_dir() -> PathBuf {
    PathBuf::from(BACKUP_DIRECTORY)
}

fn find_goodbyedpi_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(res) = app.path().resource_dir() {
        candidates.push(res.join("resources/goodbyedpi/x86_64"));
        candidates.push(res.join("goodbyedpi/x86_64"));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/goodbyedpi/x86_64"));

    for candidate in &candidates {
        if ["goodbyedpi.exe", "WinDivert.dll", "WinDivert64.sys"]
            .iter()
            .all(|name| candidate.join(name).is_file())
        {
            return Ok(candidate.clone());
        }
    }

    Err(format!(
        "GoodbyeDPI dosyaları bulunamadı. Aranan konumlar: {candidates:?}"
    ))
}

fn find_service_host(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(res) = app.path().resource_dir() {
        candidates.push(res.join("resources").join(SERVICE_BINARY));
        candidates.push(res.join(SERVICE_BINARY));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/mavrodpi-svc.exe"));

    // Geliştirme derlemesinde servis yardımcısı ana uygulamanın yanında bulunur.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(SERVICE_BINARY));
        }
    }

    let mut invalid = Vec::new();
    for candidate in &candidates {
        if !candidate.exists() {
            continue;
        }
        match validate_service_binary(candidate) {
            Ok(()) => return Ok(candidate.clone()),
            Err(reason) => invalid.push(format!("{} ({reason})", candidate.display())),
        }
    }

    let invalid_note = if invalid.is_empty() {
        String::new()
    } else {
        format!(" Geçersiz dosyalar: {}", invalid.join(", "))
    };
    Err(format!(
        "Geçerli {SERVICE_BINARY} bulunamadı. Aranan konumlar: {candidates:?}.{invalid_note}"
    ))
}

fn validate_service_binary(path: &Path) -> Result<(), String> {
    let metadata = std::fs::metadata(path).map_err(|e| format!("dosya bilgisi okunamadı: {e}"))?;
    if !metadata.is_file() {
        return Err("normal bir dosya değil".into());
    }
    if metadata.len() < MIN_SERVICE_BINARY_BYTES {
        return Err(format!(
            "dosya çok küçük ({} bayt; en az {MIN_SERVICE_BINARY_BYTES} bayt gerekli)",
            metadata.len()
        ));
    }

    let mut file = File::open(path).map_err(|e| format!("dosya açılamadı: {e}"))?;
    let mut dos_header = [0_u8; 64];
    file.read_exact(&mut dos_header)
        .map_err(|e| format!("PE başlığı okunamadı: {e}"))?;
    if &dos_header[..2] != b"MZ" {
        return Err("MZ başlığı yok".into());
    }

    let pe_offset = u32::from_le_bytes(
        dos_header[0x3c..0x40]
            .try_into()
            .expect("PE ofset alanı dört bayttır"),
    ) as u64;
    if pe_offset < 64 || pe_offset.saturating_add(4) > metadata.len() {
        return Err("PE başlığı ofseti geçersiz".into());
    }

    file.seek(SeekFrom::Start(pe_offset))
        .map_err(|e| format!("PE başlığına gidilemedi: {e}"))?;
    let mut signature = [0_u8; 4];
    file.read_exact(&mut signature)
        .map_err(|e| format!("PE imzası okunamadı: {e}"))?;
    if signature != *b"PE\0\0" {
        return Err("PE imzası geçersiz".into());
    }

    let actual_hash = sha256_file(path)?;
    let expected_hash = EXPECTED_SERVICE_SHA256;
    if actual_hash != expected_hash {
        return Err(format!(
            "uygulama paketine ait SHA-256 değeriyle eşleşmiyor (beklenen: {expected_hash}, bulunan: {actual_hash})"
        ));
    }

    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("{} açılamadı: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("{} okunamadı: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn verify_file_hash(path: &Path, name: &str, expected_hash: &str) -> Result<(), String> {
    let actual_hash = sha256_file(path)?;
    if actual_hash != expected_hash {
        return Err(format!(
            "{name} bütünlük denetimini geçemedi (beklenen SHA-256: {expected_hash}, bulunan: {actual_hash})."
        ));
    }
    Ok(())
}

fn verify_goodbyedpi_files(directory: &Path) -> Result<(), String> {
    for (name, expected_hash) in BUNDLED_FILE_HASHES {
        verify_file_hash(&directory.join(name), name, expected_hash)?;
    }
    Ok(())
}

fn service_error_code(error: &windows_service::Error) -> Option<i32> {
    match error {
        windows_service::Error::Winapi(error) => error.raw_os_error(),
        _ => None,
    }
}

fn describe_service_error(error: &windows_service::Error) -> String {
    match error {
        windows_service::Error::Winapi(error) => error.to_string(),
        _ => error.to_string(),
    }
}

fn manager(access: ServiceManagerAccess) -> Result<ServiceManager, String> {
    ServiceManager::local_computer(None::<&str>, access).map_err(|e| {
        format!(
            "Windows Servis Yöneticisi açılamadı: {}",
            describe_service_error(&e)
        )
    })
}

fn state_text(state: ServiceState) -> &'static str {
    match state {
        ServiceState::Stopped => "stopped",
        ServiceState::StartPending => "start_pending",
        ServiceState::StopPending => "stop_pending",
        ServiceState::Running => "running",
        ServiceState::ContinuePending => "continue_pending",
        ServiceState::PausePending => "pause_pending",
        ServiceState::Paused => "paused",
    }
}

fn preserved_service_state(state: ServiceState) -> Option<PreservedServiceState> {
    match state {
        ServiceState::Stopped => Some(PreservedServiceState::Stopped),
        ServiceState::Running => Some(PreservedServiceState::Running),
        ServiceState::Paused => Some(PreservedServiceState::Paused),
        ServiceState::StartPending
        | ServiceState::StopPending
        | ServiceState::ContinuePending
        | ServiceState::PausePending => None,
    }
}

fn query_stable_service_state(service: &Service) -> Result<PreservedServiceState, String> {
    let deadline = Instant::now() + SERVICE_OPERATION_TIMEOUT;
    loop {
        let status = service
            .query_status()
            .map_err(|e| format!("Servis durumu okunamadı: {}", describe_service_error(&e)))?;
        if let Some(state) = preserved_service_state(status.current_state) {
            return Ok(state);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "MavroDPI servisinin kararlı duruma geçmesi beklenirken zaman aşımı oluştu (durum: {}).",
                state_text(status.current_state)
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn wait_until_deleted(manager: &ServiceManager) -> Result<(), String> {
    let deadline = Instant::now() + SERVICE_OPERATION_TIMEOUT;
    loop {
        match manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS) {
            Ok(service) => drop(service),
            Err(error)
                if matches!(
                    service_error_code(&error),
                    Some(ERROR_SERVICE_DOES_NOT_EXIST)
                ) =>
            {
                return Ok(());
            }
            Err(error)
                if matches!(
                    service_error_code(&error),
                    Some(ERROR_SERVICE_MARKED_FOR_DELETE)
                ) => {}
            Err(error) => {
                return Err(format!(
                    "Servisin silinme durumu doğrulanamadı: {}",
                    describe_service_error(&error)
                ));
            }
        }

        if Instant::now() >= deadline {
            return Err("MavroDPI servisi silinirken zaman aşımı oluştu.".into());
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn stop_service_and_wait(service: &Service) -> Result<(), String> {
    let deadline = Instant::now() + SERVICE_OPERATION_TIMEOUT;
    let mut last_control_error: Option<String> = None;

    loop {
        let status = service
            .query_status()
            .map_err(|e| format!("Servis durumu okunamadı: {}", describe_service_error(&e)))?;
        if status.current_state == ServiceState::Stopped {
            return Ok(());
        }

        if status.current_state != ServiceState::StopPending {
            match service.stop() {
                Ok(_) => {}
                Err(error)
                    if matches!(service_error_code(&error), Some(ERROR_SERVICE_NOT_ACTIVE)) =>
                {
                    return Ok(());
                }
                Err(error)
                    if matches!(
                        service_error_code(&error),
                        Some(ERROR_SERVICE_CANNOT_ACCEPT_CTRL)
                    ) =>
                {
                    last_control_error = Some(describe_service_error(&error));
                }
                Err(error) => {
                    return Err(format!(
                        "MavroDPI servisi durdurulamadı: {}",
                        describe_service_error(&error)
                    ));
                }
            }
        }

        if Instant::now() >= deadline {
            let detail = last_control_error
                .map(|error| format!(" Son Windows hatası: {error}"))
                .unwrap_or_default();
            return Err(format!(
                "MavroDPI servisi durdurulurken zaman aşımı oluştu.{detail}"
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn start_service_and_wait(service: &Service) -> Result<(), String> {
    let deadline = Instant::now() + SERVICE_OPERATION_TIMEOUT;
    let mut start_requested = false;
    let mut resume_requested = false;

    loop {
        let status = service
            .query_status()
            .map_err(|e| format!("Servis durumu okunamadı: {}", describe_service_error(&e)))?;

        match status.current_state {
            ServiceState::Running => return Ok(()),
            ServiceState::Stopped if !start_requested => match service.start(&[] as &[&OsStr]) {
                Ok(()) => start_requested = true,
                Err(error)
                    if matches!(
                        service_error_code(&error),
                        Some(ERROR_SERVICE_ALREADY_RUNNING)
                    ) =>
                {
                    start_requested = true;
                }
                Err(error) => {
                    return Err(format!(
                        "MavroDPI servisi başlatılamadı: {}",
                        describe_service_error(&error)
                    ));
                }
            },
            ServiceState::Stopped => {
                return Err("MavroDPI servisi başladıktan hemen sonra durdu.".into());
            }
            ServiceState::Paused if !resume_requested => {
                service.resume().map_err(|e| {
                    format!(
                        "MavroDPI servisi devam ettirilemedi: {}",
                        describe_service_error(&e)
                    )
                })?;
                resume_requested = true;
            }
            ServiceState::Paused => {
                return Err("MavroDPI servisi duraklatılmış durumda kaldı.".into());
            }
            ServiceState::StartPending
            | ServiceState::StopPending
            | ServiceState::ContinuePending
            | ServiceState::PausePending => {}
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "MavroDPI servisi başlatılırken zaman aşımı oluştu (durum: {}).",
                state_text(status.current_state)
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn pause_service_and_wait(service: &Service) -> Result<(), String> {
    let deadline = Instant::now() + SERVICE_OPERATION_TIMEOUT;
    let mut start_requested = false;
    let mut pause_requested = false;

    loop {
        let status = service
            .query_status()
            .map_err(|e| format!("Servis durumu okunamadı: {}", describe_service_error(&e)))?;

        match status.current_state {
            ServiceState::Paused => return Ok(()),
            ServiceState::Stopped if !start_requested => match service.start(&[] as &[&OsStr]) {
                Ok(()) => start_requested = true,
                Err(error)
                    if matches!(
                        service_error_code(&error),
                        Some(ERROR_SERVICE_ALREADY_RUNNING)
                    ) =>
                {
                    start_requested = true;
                }
                Err(error) => {
                    return Err(format!(
                        "MavroDPI servisi duraklatılmak üzere başlatılamadı: {}",
                        describe_service_error(&error)
                    ));
                }
            },
            ServiceState::Stopped => {
                return Err("MavroDPI servisi duraklatılmadan önce yeniden durdu.".into());
            }
            ServiceState::Running if !pause_requested => {
                service.pause().map_err(|error| {
                    format!(
                        "MavroDPI servisi duraklatılamadı: {}",
                        describe_service_error(&error)
                    )
                })?;
                pause_requested = true;
            }
            ServiceState::Running => {}
            ServiceState::StartPending
            | ServiceState::StopPending
            | ServiceState::ContinuePending
            | ServiceState::PausePending => {}
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "MavroDPI servisi duraklatılırken zaman aşımı oluştu (durum: {}).",
                state_text(status.current_state)
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn restore_service_state(
    service: &Service,
    previous_state: PreservedServiceState,
) -> Result<(), String> {
    match previous_state {
        PreservedServiceState::Stopped => stop_service_and_wait(service),
        PreservedServiceState::Running => start_service_and_wait(service),
        PreservedServiceState::Paused => pause_service_and_wait(service),
    }
}

fn remove_existing_service(manager: &ServiceManager) -> Result<(), String> {
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE;
    let service = match manager.open_service(SERVICE_NAME, access) {
        Ok(service) => service,
        Err(error)
            if matches!(
                service_error_code(&error),
                Some(ERROR_SERVICE_DOES_NOT_EXIST)
            ) =>
        {
            return Ok(());
        }
        Err(error)
            if matches!(
                service_error_code(&error),
                Some(ERROR_SERVICE_MARKED_FOR_DELETE)
            ) =>
        {
            return wait_until_deleted(manager);
        }
        Err(error) => {
            return Err(format!(
                "Mevcut MavroDPI servisi açılamadı: {}",
                describe_service_error(&error)
            ));
        }
    };

    stop_service_and_wait(&service)?;
    match service.delete() {
        Ok(()) => {}
        Err(error)
            if matches!(
                service_error_code(&error),
                Some(ERROR_SERVICE_DOES_NOT_EXIST | ERROR_SERVICE_MARKED_FOR_DELETE)
            ) => {}
        Err(error) => {
            return Err(format!(
                "MavroDPI servisi silinemedi: {}",
                describe_service_error(&error)
            ));
        }
    }
    drop(service);
    wait_until_deleted(manager)
}

fn copy_with_retry(src: &Path, dst: &Path) -> Result<(), String> {
    let mut last_error = String::new();
    for _ in 0..FILE_OPERATION_RETRIES {
        match std::fs::copy(src, dst) {
            Ok(_) => return Ok(()),
            Err(error) => {
                last_error = error.to_string();
                thread::sleep(FILE_OPERATION_RETRY_DELAY);
            }
        }
    }
    Err(last_error)
}

fn is_managed_directory(path: &Path) -> bool {
    [
        Path::new(INSTALL_DIRECTORY),
        Path::new(STAGING_DIRECTORY),
        Path::new(BACKUP_DIRECTORY),
    ]
    .contains(&path)
}

fn is_service_storage_directory(path: &Path) -> bool {
    path == Path::new(SERVICE_ROOT_DIRECTORY) || is_managed_directory(path)
}

fn secure_known_payload_files(directory: &Path, require_all: bool) -> Result<(), String> {
    for name in [
        "goodbyedpi.exe",
        "WinDivert.dll",
        "WinDivert64.sys",
        SERVICE_BINARY,
    ] {
        let path = directory.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(_) => crate::windows_acl::secure_file(&path).map_err(|error| {
                format!("{} güvenli hale getirilemedi: {error}", path.display())
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !require_all => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(format!("Gerekli servis dosyası eksik: {}", path.display()));
            }
            Err(error) => {
                return Err(format!("{} denetlenemedi: {error}", path.display()));
            }
        }
    }
    Ok(())
}

fn secure_service_storage_directory(directory: &Path, require_all: bool) -> Result<(), String> {
    if !is_service_storage_directory(directory) {
        return Err("Güvenlik denetimi: yönetilmeyen servis depolama dizini.".into());
    }
    crate::windows_acl::ensure_secure_directory(directory)
        .map_err(|error| format!("{} güvenli hale getirilemedi: {error}", directory.display()))?;
    secure_known_payload_files(directory, require_all)
}

fn secure_existing_service_storage() -> Result<(), String> {
    let root = service_root_dir();
    secure_service_storage_directory(&root, false)?;

    for directory in [install_dir(), staging_dir(), backup_dir()] {
        if managed_path_exists(&directory)? {
            secure_service_storage_directory(&directory, false)?;
        }
    }
    Ok(())
}

fn managed_path_exists(path: &Path) -> Result<bool, String> {
    if !is_managed_directory(path) {
        return Err("Güvenlik denetimi: yönetilmeyen servis dizini.".into());
    }

    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("{} denetlenemedi: {error}", path.display())),
    }
}

fn cleanup_managed_dir(target: &Path) -> Result<(), String> {
    if !is_managed_directory(target) {
        return Err("Güvenlik denetimi: geçersiz servis dizini.".into());
    }

    let mut last_error = String::new();
    for _ in 0..FILE_OPERATION_RETRIES {
        match std::fs::symlink_metadata(target) {
            Ok(metadata) => {
                let file_type = metadata.file_type();
                let result = if metadata.is_dir() || file_type.is_symlink() {
                    // remove_dir_all Windows'ta yeniden ayrıştırma noktalarını takip etmez;
                    // yalnızca sabit servis, staging veya backup alt dizinini kaldırır.
                    // Üst dizindeki DNS geri yükleme günlüğüne (state) dokunulmaz.
                    std::fs::remove_dir_all(target).or_else(|dir_error| {
                        if file_type.is_symlink() {
                            std::fs::remove_file(target)
                        } else {
                            Err(dir_error)
                        }
                    })
                } else {
                    std::fs::remove_file(target)
                };
                match result {
                    Ok(()) => return Ok(()),
                    Err(error) => last_error = error.to_string(),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => last_error = error.to_string(),
        }
        thread::sleep(FILE_OPERATION_RETRY_DELAY);
    }

    Err(format!("{} temizlenemedi: {last_error}", target.display()))
}

fn cleanup_install_dir() -> Result<(), String> {
    cleanup_managed_dir(&install_dir())
}

fn move_managed_dir(source: &Path, destination: &Path) -> Result<(), String> {
    if !is_managed_directory(source) || !is_managed_directory(destination) || source == destination
    {
        return Err("Güvenlik denetimi: geçersiz servis dizini taşıma isteği.".into());
    }
    secure_service_storage_directory(&service_root_dir(), false)?;
    if !managed_path_exists(source)? {
        return Err(format!("{} taşınmak için bulunamadı.", source.display()));
    }
    secure_service_storage_directory(source, false)?;
    if managed_path_exists(destination)? {
        return Err(format!(
            "{} zaten var; mevcut veri ezilmeyecek.",
            destination.display()
        ));
    }

    let mut last_error = String::new();
    for _ in 0..FILE_OPERATION_RETRIES {
        match std::fs::rename(source, destination) {
            Ok(()) => {
                return match secure_service_storage_directory(destination, false) {
                    Ok(()) => Ok(()),
                    Err(security_error) => match std::fs::rename(destination, source) {
                        Ok(()) => Err(format!(
                            "{} taşındıktan sonra güvenli hale getirilemedi; taşıma geri alındı: {security_error}",
                            destination.display()
                        )),
                        Err(restore_error) => Err(format!(
                            "{} taşındıktan sonra güvenli hale getirilemedi ({security_error}) ve {} konumuna geri alınamadı: {restore_error}",
                            destination.display(),
                            source.display()
                        )),
                    },
                };
            }
            Err(error) => {
                last_error = error.to_string();
                thread::sleep(FILE_OPERATION_RETRY_DELAY);
            }
        }
    }

    Err(format!(
        "{} -> {} taşınamadı: {last_error}",
        source.display(),
        destination.display()
    ))
}

fn secure_and_verify_complete_service_directory(directory: &Path) -> Result<(), String> {
    secure_service_storage_directory(directory, true)?;
    validate_service_binary(&directory.join(SERVICE_BINARY))
        .map_err(|error| format!("{SERVICE_BINARY} doğrulanamadı: {error}"))?;
    verify_goodbyedpi_files(directory)
}

fn prepare_staging(source: &Path, service_host: &Path) -> Result<(), String> {
    secure_existing_service_storage()?;
    let staging = staging_dir();
    cleanup_managed_dir(&staging)?;
    secure_service_storage_directory(&staging, false)?;

    let result = (|| {
        for (file, expected_hash) in BUNDLED_FILE_HASHES {
            let staged_file = staging.join(file);
            copy_with_retry(&source.join(file), &staged_file)
                .map_err(|error| format!("{file} hazırlık alanına kopyalanamadı: {error}"))?;
            crate::windows_acl::secure_file(&staged_file)
                .map_err(|error| format!("{file} güvenli hale getirilemedi: {error}"))?;
            verify_file_hash(&staged_file, file, expected_hash)
                .map_err(|error| format!("Hazırlanan {file} doğrulanamadı: {error}"))?;
        }

        let staged_service_host = staging.join(SERVICE_BINARY);
        copy_with_retry(service_host, &staged_service_host)
            .map_err(|error| format!("{SERVICE_BINARY} hazırlık alanına kopyalanamadı: {error}"))?;
        crate::windows_acl::secure_file(&staged_service_host)
            .map_err(|error| format!("{SERVICE_BINARY} güvenli hale getirilemedi: {error}"))?;
        validate_service_binary(&staged_service_host)
            .map_err(|error| format!("Hazırlanan {SERVICE_BINARY} doğrulanamadı: {error}"))?;
        secure_and_verify_complete_service_directory(&staging)
            .map_err(|error| format!("Hazırlık alanı doğrulanamadı: {error}"))
    })();

    if result.is_err() {
        let _ = cleanup_managed_dir(&staging);
    }
    result
}

fn activate_staged_directory() -> Result<bool, ActivationFailure> {
    secure_existing_service_storage().map_err(|error| ActivationFailure::new(error, false))?;
    let installed = install_dir();
    let staging = staging_dir();
    let backup = backup_dir();
    secure_and_verify_complete_service_directory(&staging).map_err(|error| {
        ActivationFailure::new(
            format!("Hazırlık alanı etkinleştirmeden önce doğrulanamadı: {error}"),
            true,
        )
    })?;
    if managed_path_exists(&backup).map_err(|error| ActivationFailure::new(error, false))? {
        return Err(ActivationFailure::new(
            format!(
                "{} içinde korunmuş bir önceki kurulum var. Mevcut servis ve yedek değiştirilmedi.",
                backup.display()
            ),
            true,
        ));
    }

    let had_previous_directory =
        managed_path_exists(&installed).map_err(|error| ActivationFailure::new(error, false))?;
    if had_previous_directory {
        move_managed_dir(&installed, &backup)
            .map_err(|error| ActivationFailure::new(error, false))?;
    }

    if let Err(error) = move_managed_dir(&staging, &installed) {
        if had_previous_directory {
            if let Err(restore_error) = move_managed_dir(&backup, &installed) {
                return Err(ActivationFailure::new(
                    format!("{error} Önceki servis dizini de geri taşınamadı: {restore_error}"),
                    false,
                ));
            }
        }
        return Err(ActivationFailure::new(error, true));
    }

    if let Err(error) = secure_and_verify_complete_service_directory(&installed) {
        let moved_new_directory_back = move_managed_dir(&installed, &staging);
        let restored_previous_directory = if had_previous_directory {
            move_managed_dir(&backup, &installed)
        } else {
            Ok(())
        };
        return match (moved_new_directory_back, restored_previous_directory) {
            (Ok(()), Ok(())) => Err(ActivationFailure::new(
                format!(
                    "Yeni servis dizini etkinleştirme sonrasında doğrulanamadı ve dosya taşıması geri alındı: {error}"
                ),
                true,
            )),
            (new_result, previous_result) => Err(ActivationFailure::new(
                format!(
                    "Yeni servis dizini etkinleştirme sonrasında doğrulanamadı: {error}. Yeni dizini geri taşıma: {}. Önceki dizini geri yükleme: {}.",
                    new_result
                        .err()
                        .unwrap_or_else(|| "başarılı".into()),
                    previous_result
                        .err()
                        .unwrap_or_else(|| "başarılı".into())
                ),
                false,
            )),
        };
    }

    Ok(had_previous_directory)
}

fn restore_previous_directory(had_previous_directory: bool) -> Result<(), String> {
    let installed = install_dir();
    let staging = staging_dir();
    let backup = backup_dir();

    cleanup_managed_dir(&staging)?;
    let moved_new_directory = if managed_path_exists(&installed)? {
        move_managed_dir(&installed, &staging)?;
        true
    } else {
        false
    };

    if had_previous_directory {
        if let Err(error) = move_managed_dir(&backup, &installed) {
            if moved_new_directory {
                let _ = move_managed_dir(&staging, &installed);
            }
            return Err(format!("Önceki servis dosyaları geri yüklenemedi: {error}"));
        }
    }

    cleanup_managed_dir(&staging)
}

fn desired_service_info(executable_path: PathBuf) -> ServiceInfo {
    ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from("MavroDPI"),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path,
        launch_arguments: Vec::new(),
        dependencies: Vec::<ServiceDependency>::new(),
        account_name: None,
        account_password: None,
    }
}

fn service_info_from_config(config: &ServiceConfig) -> ServiceInfo {
    ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: config.display_name.clone(),
        service_type: config.service_type,
        start_type: config.start_type,
        error_control: config.error_control,
        executable_path: configured_executable_path(&config.executable_path),
        launch_arguments: Vec::new(),
        dependencies: config.dependencies.clone(),
        // None tells ChangeServiceConfigW to preserve the existing account and password.
        account_name: None,
        account_password: None,
    }
}

fn desired_failure_actions() -> ServiceFailureActions {
    ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86_400)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(5),
            },
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(10),
            },
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(30),
            },
        ]),
    }
}

fn configure_failure_actions(service: &Service) -> Result<(), String> {
    service
        .update_failure_actions(desired_failure_actions())
        .map_err(|error| {
            format!(
                "Servis kurtarma ayarları yapılandırılamadı: {}",
                describe_service_error(&error)
            )
        })?;
    service
        .set_failure_actions_on_non_crash_failures(true)
        .map_err(|error| {
            format!(
                "Servis sıfır olmayan çıkış kodlarında kurtarma çalıştıracak şekilde yapılandırılamadı: {}",
                describe_service_error(&error)
            )
        })
}

fn failure_actions_for_restore(
    mut failure_actions: ServiceFailureActions,
) -> ServiceFailureActions {
    // ChangeServiceConfig2 ignores reset_period/actions when lpsaActions is null.
    // A non-null pointer with cActions=0 explicitly deletes an action array, so
    // represent a previously absent array as Some(empty) during rollback.
    if failure_actions.actions.is_none() {
        failure_actions.actions = Some(Vec::new());
    }
    failure_actions
}

fn windows_paths_equal(left: &Path, right: &Path) -> bool {
    left.as_os_str()
        .to_string_lossy()
        .trim_matches('"')
        .eq_ignore_ascii_case(right.as_os_str().to_string_lossy().trim_matches('"'))
}

fn configured_executable_path(path: &Path) -> PathBuf {
    let command = path.as_os_str().to_string_lossy();
    let trimmed = command.trim();
    if let Some(quoted) = trimmed.strip_prefix('"') {
        if let Some(end_quote) = quoted.find('"') {
            return PathBuf::from(&quoted[..end_quote]);
        }
    }
    PathBuf::from(trimmed)
}

fn service_integrity(configured_path: &Path) -> (bool, bool, bool) {
    let executable_path = configured_executable_path(configured_path);
    let expected_path = install_dir().join(SERVICE_BINARY);
    let binary_path_current = windows_paths_equal(&executable_path, &expected_path);
    let helper_hash_current = sha256_file(&executable_path)
        .map(|hash| hash == EXPECTED_SERVICE_SHA256)
        .unwrap_or(false);
    let payload_current = verify_goodbyedpi_files(&install_dir()).is_ok();
    (binary_path_current, helper_hash_current, payload_current)
}

#[tauri::command]
pub fn service_status() -> Result<ServiceStatus, String> {
    let manager = manager(ServiceManagerAccess::CONNECT)?;
    match manager.open_service(
        SERVICE_NAME,
        ServiceAccess::QUERY_STATUS | ServiceAccess::QUERY_CONFIG,
    ) {
        Ok(service) => {
            let status = service
                .query_status()
                .map_err(|e| format!("Servis durumu okunamadı: {}", describe_service_error(&e)))?;
            let (binary_path_current, helper_hash_current, payload_current) = service
                .query_config()
                .map(|config| service_integrity(&config.executable_path))
                .unwrap_or((false, false, false));
            Ok(ServiceStatus {
                installed: true,
                running: status.current_state == ServiceState::Running,
                state: state_text(status.current_state).into(),
                binary_path_current,
                helper_hash_current,
                payload_current,
                needs_repair: !(binary_path_current && helper_hash_current && payload_current),
            })
        }
        Err(error)
            if matches!(
                service_error_code(&error),
                Some(ERROR_SERVICE_DOES_NOT_EXIST)
            ) =>
        {
            Ok(ServiceStatus {
                installed: false,
                running: false,
                state: "not_installed".into(),
                binary_path_current: false,
                helper_hash_current: false,
                payload_current: false,
                needs_repair: false,
            })
        }
        Err(error)
            if matches!(
                service_error_code(&error),
                Some(ERROR_SERVICE_MARKED_FOR_DELETE)
            ) =>
        {
            Ok(ServiceStatus {
                installed: true,
                running: false,
                state: "marked_for_delete".into(),
                binary_path_current: false,
                helper_hash_current: false,
                payload_current: false,
                needs_repair: true,
            })
        }
        Err(error) => Err(format!(
            "MavroDPI servis durumu alınamadı: {}",
            describe_service_error(&error)
        )),
    }
}

#[tauri::command]
pub fn service_installed() -> bool {
    service_status()
        .map(|status| status.installed)
        .unwrap_or(false)
}

struct ExistingService {
    service: Service,
    config: ServiceConfig,
    failure_actions: ServiceFailureActions,
    failure_actions_on_non_crash_failures: bool,
    previous_state: PreservedServiceState,
}

fn open_existing_service(
    service_manager: &ServiceManager,
) -> Result<Option<ExistingService>, String> {
    let access = ServiceAccess::QUERY_STATUS
        | ServiceAccess::QUERY_CONFIG
        | ServiceAccess::START
        | ServiceAccess::STOP
        | ServiceAccess::PAUSE_CONTINUE
        | ServiceAccess::CHANGE_CONFIG;
    let service = match service_manager.open_service(SERVICE_NAME, access) {
        Ok(service) => service,
        Err(error)
            if matches!(
                service_error_code(&error),
                Some(ERROR_SERVICE_DOES_NOT_EXIST)
            ) =>
        {
            return Ok(None);
        }
        Err(error)
            if matches!(
                service_error_code(&error),
                Some(ERROR_SERVICE_MARKED_FOR_DELETE)
            ) =>
        {
            wait_until_deleted(service_manager)?;
            return Ok(None);
        }
        Err(error) => {
            return Err(format!(
                "Mevcut MavroDPI servisi güncelleme için açılamadı: {}",
                describe_service_error(&error)
            ));
        }
    };

    let previous_state = query_stable_service_state(&service)?;
    let config = service.query_config().map_err(|error| {
        format!(
            "Servis yapılandırması okunamadı: {}",
            describe_service_error(&error)
        )
    })?;
    let failure_actions = service.get_failure_actions().map_err(|error| {
        format!(
            "Servis kurtarma eylemleri okunamadı: {}",
            describe_service_error(&error)
        )
    })?;
    let failure_actions_on_non_crash_failures = service
        .get_failure_actions_on_non_crash_failures()
        .map_err(|error| {
            format!(
                "Servis kurtarma kapsamı okunamadı: {}",
                describe_service_error(&error)
            )
        })?;
    Ok(Some(ExistingService {
        service,
        config,
        failure_actions,
        failure_actions_on_non_crash_failures,
        previous_state,
    }))
}

fn rollback_message(primary_error: String, rollback: Result<(), String>) -> String {
    match rollback {
        Ok(()) => format!("{primary_error} Önceki servis kurulumu geri yüklendi."),
        Err(rollback_error) => format!(
            "{primary_error} Önceki servis kurulumu geri yüklenirken ayrıca hata oluştu: {rollback_error}"
        ),
    }
}

fn rollback_prerequisites_complete(
    config_restored: bool,
    failure_actions_restored: bool,
    failure_scope_restored: bool,
    directory_restored: bool,
) -> bool {
    config_restored && failure_actions_restored && failure_scope_restored && directory_restored
}

fn rollback_existing_update(
    existing: &ExistingService,
    had_previous_directory: bool,
) -> Result<(), String> {
    stop_service_and_wait(&existing.service)?;

    let mut errors = Vec::new();
    let previous_info = service_info_from_config(&existing.config);
    let config_restored = match existing.service.change_config(&previous_info) {
        Ok(()) => true,
        Err(error) => {
            errors.push(format!(
                "önceki servis kaydı geri yüklenemedi: {}",
                describe_service_error(&error)
            ));
            false
        }
    };
    let failure_actions_restored =
        match existing
            .service
            .update_failure_actions(failure_actions_for_restore(
                existing.failure_actions.clone(),
            )) {
            Ok(()) => true,
            Err(error) => {
                errors.push(format!(
                    "önceki servis kurtarma eylemleri geri yüklenemedi: {}",
                    describe_service_error(&error)
                ));
                false
            }
        };
    let failure_scope_restored = match existing
        .service
        .set_failure_actions_on_non_crash_failures(existing.failure_actions_on_non_crash_failures)
    {
        Ok(()) => true,
        Err(error) => {
            errors.push(format!(
                "önceki servis kurtarma kapsamı geri yüklenemedi: {}",
                describe_service_error(&error)
            ));
            false
        }
    };
    let directory_restored = match restore_previous_directory(had_previous_directory) {
        Ok(()) => true,
        Err(error) => {
            errors.push(error);
            false
        }
    };

    if rollback_prerequisites_complete(
        config_restored,
        failure_actions_restored,
        failure_scope_restored,
        directory_restored,
    ) {
        if let Err(error) = restore_service_state(&existing.service, existing.previous_state) {
            errors.push(format!(
                "önceki servis çalışma durumu geri yüklenemedi: {error}"
            ));
        }
    } else {
        errors.push(
            "Geri alma eksiksiz doğrulanamadığı için servis güvenli biçimde durdurulmuş bırakıldı."
                .into(),
        );
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(" "))
    }
}

fn update_existing_service(
    existing: &ExistingService,
    had_previous_directory: bool,
) -> Result<(), String> {
    let desired_info = desired_service_info(install_dir().join(SERVICE_BINARY));
    if let Err(error) = existing.service.change_config(&desired_info) {
        return Err(rollback_message(
            format!(
                "MavroDPI servis kaydı güncellenemedi: {}",
                describe_service_error(&error)
            ),
            rollback_existing_update(existing, had_previous_directory),
        ));
    }
    if let Err(error) = configure_failure_actions(&existing.service) {
        return Err(rollback_message(
            error,
            rollback_existing_update(existing, had_previous_directory),
        ));
    }
    if let Err(error) = restore_service_state(&existing.service, existing.previous_state) {
        return Err(rollback_message(
            format!("Servisin önceki çalışma durumu geri yüklenemedi: {error}"),
            rollback_existing_update(existing, had_previous_directory),
        ));
    }
    Ok(())
}

fn rollback_new_service(
    service_manager: &ServiceManager,
    had_previous_directory: bool,
) -> Result<(), String> {
    remove_existing_service(service_manager)?;
    restore_previous_directory(had_previous_directory)
}

#[tauri::command]
pub fn install_service(app: tauri::AppHandle) -> Result<(), String> {
    // Çalışan bir kuruluma dokunmadan önce yeni paketin tamamını ayrı alanda doğrula.
    let src = find_goodbyedpi_dir(&app)?;
    verify_goodbyedpi_files(&src)?;
    let service_host = find_service_host(&app)?;
    prepare_staging(&src, &service_host)?;

    let manager =
        match manager(ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE) {
            Ok(manager) => manager,
            Err(error) => {
                let _ = cleanup_managed_dir(&staging_dir());
                return Err(error);
            }
        };
    match managed_path_exists(&backup_dir()) {
        Ok(false) => {}
        Ok(true) => {
            let _ = cleanup_managed_dir(&staging_dir());
            return Err(format!(
                "{} içinde korunmuş bir önceki kurulum var. Çalışan servis ve yedek değiştirilmedi.",
                backup_dir().display()
            ));
        }
        Err(error) => {
            let _ = cleanup_managed_dir(&staging_dir());
            return Err(error);
        }
    }

    let existing = match open_existing_service(&manager) {
        Ok(existing) => existing,
        Err(error) => {
            let _ = cleanup_managed_dir(&staging_dir());
            return Err(error);
        }
    };

    if let Some(existing) = existing {
        if let Err(error) = stop_service_and_wait(&existing.service) {
            let restart_note = restore_service_state(&existing.service, existing.previous_state)
                .err()
                .map(|restore_error| {
                    format!(
                        " Önceki servis çalışma durumu geri yüklenirken de hata oluştu: {restore_error}"
                    )
                })
                .unwrap_or_default();
            let _ = cleanup_managed_dir(&staging_dir());
            return Err(format!(
                "Mevcut servis güvenli güncelleme için durdurulamadı: {error}{restart_note}"
            ));
        }

        let had_previous_directory = match activate_staged_directory() {
            Ok(had_previous_directory) => had_previous_directory,
            Err(failure) => {
                let restart_note = if failure.previous_service_state_restorable {
                    restore_service_state(&existing.service, existing.previous_state)
                        .err()
                        .map(|restore_error| {
                            format!(
                                " Önceki servis çalışma durumu geri yüklenirken de hata oluştu: {restore_error}"
                            )
                        })
                        .unwrap_or_default()
                } else {
                    " Dosya geri alımı doğrulanamadığı için servis güvenli biçimde durdurulmuş bırakıldı."
                        .into()
                };
                let _ = cleanup_managed_dir(&staging_dir());
                return Err(format!("{}{restart_note}", failure.message));
            }
        };

        update_existing_service(&existing, had_previous_directory)?;
        if let Err(error) = cleanup_managed_dir(&backup_dir()) {
            return Err(format!(
                "Servis güncellendi ve önceki çalışma durumuna döndürüldü; ancak güvenlik yedeği temizlenemedi: {error}"
            ));
        }
        return Ok(());
    }

    let had_previous_directory = match activate_staged_directory() {
        Ok(had_previous_directory) => had_previous_directory,
        Err(failure) => {
            let _ = cleanup_managed_dir(&staging_dir());
            return Err(failure.message);
        }
    };

    let service_info = desired_service_info(install_dir().join(SERVICE_BINARY));
    let service_access = ServiceAccess::QUERY_STATUS
        | ServiceAccess::QUERY_CONFIG
        | ServiceAccess::START
        | ServiceAccess::STOP
        | ServiceAccess::PAUSE_CONTINUE
        | ServiceAccess::DELETE
        | ServiceAccess::CHANGE_CONFIG;
    let service = match manager.create_service(&service_info, service_access) {
        Ok(service) => service,
        Err(error) => {
            return Err(rollback_message(
                format!(
                    "MavroDPI servisi oluşturulamadı: {}",
                    describe_service_error(&error)
                ),
                restore_previous_directory(had_previous_directory),
            ));
        }
    };

    if let Err(error) = configure_failure_actions(&service) {
        drop(service);
        return Err(rollback_message(
            error,
            rollback_new_service(&manager, had_previous_directory),
        ));
    }

    if let Err(error) = start_service_and_wait(&service) {
        drop(service);
        return Err(rollback_message(
            error,
            rollback_new_service(&manager, had_previous_directory),
        ));
    }

    if let Err(error) = cleanup_managed_dir(&backup_dir()) {
        return Err(format!(
            "Servis kuruldu ve çalışıyor; ancak güvenlik yedeği temizlenemedi: {error}"
        ));
    }

    Ok(())
}

#[tauri::command]
pub fn uninstall_service() -> Result<(), String> {
    secure_existing_service_storage()?;
    let manager = manager(ServiceManagerAccess::CONNECT)?;
    remove_existing_service(&manager)?;
    let mut errors = Vec::new();
    if let Err(error) = cleanup_install_dir() {
        errors.push(error);
    }
    if let Err(error) = cleanup_managed_dir(&staging_dir()) {
        errors.push(error);
    }
    if let Err(error) = cleanup_managed_dir(&backup_dir()) {
        errors.push(error);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(" "))
    }
}

#[tauri::command]
pub fn start_service_now() -> Result<(), String> {
    secure_existing_service_storage()?;
    let manager = manager(ServiceManagerAccess::CONNECT)?;
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::PAUSE_CONTINUE;
    let service = manager
        .open_service(SERVICE_NAME, access)
        .map_err(|e| format!("MavroDPI servisi açılamadı: {}", describe_service_error(&e)))?;
    start_service_and_wait(&service)
}

#[cfg(test)]
mod tests {
    use super::{
        configured_executable_path, desired_failure_actions, failure_actions_for_restore,
        is_service_storage_directory, preserved_service_state, rollback_prerequisites_complete,
        windows_paths_equal, PreservedServiceState,
    };
    use std::path::Path;
    use windows_service::service::{
        ServiceActionType, ServiceFailureActions, ServiceFailureResetPeriod, ServiceState,
    };

    #[test]
    fn extracts_quoted_service_executable_without_arguments() {
        assert_eq!(
            configured_executable_path(Path::new(
                r#""C:\Program Files\MavroDPI\mavrodpi-svc.exe" --ignored"#
            )),
            Path::new(r"C:\Program Files\MavroDPI\mavrodpi-svc.exe")
        );
    }

    #[test]
    fn compares_windows_paths_case_insensitively() {
        assert!(windows_paths_equal(
            Path::new(r"C:\ProgramData\MavroDPI\service\mavrodpi-svc.exe"),
            Path::new(r"c:\programdata\mavrodpi\SERVICE\MAVRODPI-SVC.EXE")
        ));
    }

    #[test]
    fn desired_recovery_restarts_after_each_failure() {
        let recovery = desired_failure_actions();
        assert_eq!(
            recovery.reset_period,
            ServiceFailureResetPeriod::After(std::time::Duration::from_secs(86_400))
        );
        let actions = recovery.actions.expect("restart actions must be present");
        assert_eq!(actions.len(), 3);
        assert!(actions
            .iter()
            .all(|action| action.action_type == ServiceActionType::Restart));
        assert_eq!(
            actions
                .iter()
                .map(|action| action.delay.as_secs())
                .collect::<Vec<_>>(),
            vec![5, 10, 30]
        );
    }

    #[test]
    fn rollback_explicitly_clears_a_previously_absent_action_array() {
        let restored = failure_actions_for_restore(ServiceFailureActions {
            reset_period: ServiceFailureResetPeriod::Never,
            reboot_msg: None,
            command: None,
            actions: None,
        });
        assert_eq!(restored.actions, Some(Vec::new()));
    }

    #[test]
    fn captures_only_exact_stable_service_states() {
        assert_eq!(
            preserved_service_state(ServiceState::Stopped),
            Some(PreservedServiceState::Stopped)
        );
        assert_eq!(
            preserved_service_state(ServiceState::Running),
            Some(PreservedServiceState::Running)
        );
        assert_eq!(
            preserved_service_state(ServiceState::Paused),
            Some(PreservedServiceState::Paused)
        );
        assert_eq!(preserved_service_state(ServiceState::StartPending), None);
        assert_eq!(preserved_service_state(ServiceState::StopPending), None);
        assert_eq!(preserved_service_state(ServiceState::PausePending), None);
        assert_eq!(preserved_service_state(ServiceState::ContinuePending), None);
    }

    #[test]
    fn acl_scope_accepts_only_the_service_root_and_managed_directories() {
        assert!(is_service_storage_directory(Path::new(
            r"C:\ProgramData\MavroDPI"
        )));
        assert!(is_service_storage_directory(Path::new(
            r"C:\ProgramData\MavroDPI\service"
        )));
        assert!(!is_service_storage_directory(Path::new(
            r"C:\ProgramData\MavroDPI\state"
        )));
        assert!(!is_service_storage_directory(Path::new(
            r"C:\ProgramData\Other"
        )));
    }

    #[test]
    fn rollback_restores_service_state_only_after_every_prerequisite_succeeds() {
        assert!(rollback_prerequisites_complete(true, true, true, true));
        for failed_step in 0..4 {
            let mut steps = [true; 4];
            steps[failed_step] = false;
            assert!(!rollback_prerequisites_complete(
                steps[0], steps[1], steps[2], steps[3]
            ));
        }
    }
}
