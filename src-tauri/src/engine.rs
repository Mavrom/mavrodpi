// GoodbyeDPI motorunu kontrollü bir alt süreç olarak yönetir.
// Renderer yalnızca izin verilen profil argümanlarını gönderebilir.

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, State};

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const ENGINE_FILES: &[(&str, &str)] = &[
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

struct ManagedChild {
    child: Child,
    #[cfg(windows)]
    _job: crate::windows_job::KillOnCloseJob,
    profile: String,
    started_at: u64,
}

#[derive(Default)]
pub struct EngineState {
    child: Mutex<Option<ManagedChild>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineInfo {
    running: bool,
    profile: Option<String>,
    pid: Option<u32>,
    started_at: Option<u64>,
}

fn profile_for_args(args: &[String]) -> Option<&'static str> {
    match args {
        [mode] if mode == "-5" => Some("balanced"),
        [mode] if mode == "-6" => Some("compatibility"),
        _ => None,
    }
}

fn verify_file_hash(path: &std::path::Path, expected: &str) -> Result<(), String> {
    let mut file =
        File::open(path).map_err(|error| format!("{} okunamadı: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("{} doğrulanamadı: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    let actual = format!("{:x}", hasher.finalize());
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "{} güvenlik doğrulamasını geçemedi. Uygulamayı resmi paketten yeniden kurun.",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Motor dosyası")
        ))
    }
}

fn verify_engine_bundle(directory: &std::path::Path) -> Result<(), String> {
    for (name, expected) in ENGINE_FILES {
        verify_file_hash(&directory.join(name), expected)?;
    }
    Ok(())
}

// GoodbyeDPI yürütülebilir dosyasını hem geliştirme hem paketlenmiş modda bulur.
fn goodbyedpi_path(app: &AppHandle) -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(res) = app.path().resource_dir() {
        candidates.push(res.join("resources/goodbyedpi/x86_64/goodbyedpi.exe"));
        candidates.push(res.join("goodbyedpi/x86_64/goodbyedpi.exe"));
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/goodbyedpi/x86_64/goodbyedpi.exe"),
    );

    for candidate in &candidates {
        if candidate.exists() {
            let directory = candidate
                .parent()
                .ok_or("GoodbyeDPI kaynak klasörü geçersiz.")?;
            verify_engine_bundle(directory)?;
            return Ok(candidate.clone());
        }
    }

    Err(format!(
        "goodbyedpi.exe bulunamadı. Aranan yerler: {:?}",
        candidates
    ))
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[tauri::command]
pub fn start_protection(
    app: AppHandle,
    state: State<EngineState>,
    args: Vec<String>,
) -> Result<(), String> {
    let profile = profile_for_args(&args).ok_or_else(|| {
        "Geçersiz koruma profili. Yalnızca Dengeli ve Uyumluluk profilleri desteklenir.".to_string()
    })?;

    if crate::service::service_status()?.running {
        return Err(
            "MavroDPI Windows servisi zaten çalışıyor. İkinci bir yerel motor başlatılmadı.".into(),
        );
    }

    let mut guard = state.child.lock().map_err(|e| e.to_string())?;
    if let Some(managed) = guard.as_mut() {
        match managed.child.try_wait() {
            Ok(None) => return Err("Koruma zaten çalışıyor.".into()),
            Ok(Some(_)) | Err(_) => {
                *guard = None;
            }
        }
    }

    let exe = goodbyedpi_path(&app)?;
    let workdir = exe
        .parent()
        .ok_or("Geçersiz yürütülebilir yolu.")?
        .to_path_buf();

    let mut cmd = Command::new(&exe);
    cmd.args(&args)
        .current_dir(&workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("GoodbyeDPI başlatılamadı: {e}"))?;

    #[cfg(windows)]
    let job = match crate::windows_job::KillOnCloseJob::assign(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "GoodbyeDPI güvenli süreç grubuna alınamadı: {error}"
            ));
        }
    };

    if let Some(out) = child.stdout.take() {
        let app_for_stdout = app.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = app_for_stdout.emit("gdpi-log", line);
            }
        });
    }

    if let Some(err) = child.stderr.take() {
        let app_for_stderr = app.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                let _ = app_for_stderr.emit("gdpi-log", format!("[hata] {line}"));
            }
        });
    }

    let pid = child.id();
    *guard = Some(ManagedChild {
        child,
        #[cfg(windows)]
        _job: job,
        profile: profile.to_string(),
        started_at: now_unix_seconds(),
    });

    let _ = app.emit("gdpi-status", true);
    let _ = app.emit(
        "gdpi-log",
        format!("Koruma başlatıldı: {profile} profil · PID {pid}"),
    );
    Ok(())
}

#[tauri::command]
pub fn stop_protection(app: AppHandle, state: State<EngineState>) -> Result<(), String> {
    stop_internal(state.inner());
    let _ = app.emit("gdpi-log", "Koruma durduruldu.".to_string());

    let restore_result = crate::dns::disable_doh_internal(&app);
    let actual_doh_status = crate::dns::doh_status();
    let _ = app.emit("doh-status", actual_doh_status);
    // Başlat düğmesi DNS geri yüklemesi tamamlanana kadar yeniden açılmamalı.
    let _ = app.emit("gdpi-status", false);

    match restore_result {
        Ok(()) => {
            let _ = app.emit(
                "gdpi-log",
                "Önceki Windows DNS ayarı geri yüklendi.".to_string(),
            );
            Ok(())
        }
        Err(error) => {
            let _ = app.emit(
                "gdpi-log",
                format!("[hata] Windows DNS ayarı geri yüklenemedi: {error}"),
            );
            Err(error)
        }
    }
}

#[tauri::command]
pub fn get_status(state: State<EngineState>) -> bool {
    let mut guard = match state.child.lock() {
        Ok(guard) => guard,
        Err(_) => return false,
    };

    if let Some(managed) = guard.as_mut() {
        match managed.child.try_wait() {
            Ok(Some(_)) => {
                *guard = None;
                false
            }
            Ok(None) | Err(_) => true,
        }
    } else {
        false
    }
}

#[tauri::command]
pub fn get_engine_info(state: State<EngineState>) -> EngineInfo {
    let mut guard = match state.child.lock() {
        Ok(guard) => guard,
        Err(_) => {
            return EngineInfo {
                running: false,
                profile: None,
                pid: None,
                started_at: None,
            }
        }
    };

    let exited = guard
        .as_mut()
        .and_then(|managed| managed.child.try_wait().ok())
        .flatten()
        .is_some();
    if exited {
        *guard = None;
    }

    match guard.as_ref() {
        Some(managed) => EngineInfo {
            running: true,
            profile: Some(managed.profile.clone()),
            pid: Some(managed.child.id()),
            started_at: Some(managed.started_at),
        },
        None => EngineInfo {
            running: false,
            profile: None,
            pid: None,
            started_at: None,
        },
    }
}

// Motor beklenmedik biçimde kapanırsa arayüze gerçek durumu yayınlar.
pub fn start_monitor(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));

        let state = app.state::<EngineState>();
        let exit_message = {
            let mut guard = match state.child.lock() {
                Ok(guard) => guard,
                Err(_) => continue,
            };

            let status = guard
                .as_mut()
                .and_then(|managed| managed.child.try_wait().ok())
                .flatten();

            status.map(|status| {
                *guard = None;
                format!("Koruma motoru kapandı: {status}")
            })
        };

        if let Some(message) = exit_message {
            let _ = app.emit("gdpi-log", message);
            match crate::dns::disable_doh_internal(&app) {
                Ok(()) => {
                    let _ = app.emit(
                        "gdpi-log",
                        "Motor kapandığı için önceki Windows DNS ayarı geri yüklendi.".to_string(),
                    );
                }
                Err(error) => {
                    let _ = app.emit(
                        "gdpi-log",
                        format!("[hata] Motor kapandıktan sonra DNS geri alınamadı: {error}"),
                    );
                }
            }
            let _ = app.emit("doh-status", crate::dns::doh_status());
            // DNS geri yüklenmeden yeni bir başlatma akışına izin verme.
            let _ = app.emit("gdpi-status", false);
        }
    });
}

// Olay/AppHandle gerektirmeden yalnızca bu uygulamanın başlattığı süreci öldürür.
pub fn stop_internal(state: &EngineState) {
    if let Ok(mut guard) = state.child.lock() {
        if let Some(mut managed) = guard.take() {
            let _ = managed.child.kill();
            let _ = managed.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::profile_for_args;

    #[test]
    fn only_exact_supported_profiles_are_accepted() {
        assert_eq!(profile_for_args(&["-5".into()]), Some("balanced"));
        assert_eq!(profile_for_args(&["-6".into()]), Some("compatibility"));
        assert_eq!(profile_for_args(&["-5".into(), "--extra".into()]), None);
        assert_eq!(profile_for_args(&["--help".into()]), None);
        assert_eq!(profile_for_args(&[]), None);
    }
}
