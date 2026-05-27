// GoodbyeDPI motorunu bir alt-süreç olarak yönetir:
// başlat / durdur / durum, ve stdout'u ön yüze olay olarak akıtır.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, State};

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Default)]
pub struct EngineState {
    child: Mutex<Option<Child>>,
}

// GoodbyeDPI yürütülebilir dosyasını hem geliştirme hem paketlenmiş modda bulur.
fn goodbyedpi_path(app: &AppHandle) -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(res) = app.path().resource_dir() {
        candidates.push(res.join("resources/goodbyedpi/x86_64/goodbyedpi.exe"));
        candidates.push(res.join("goodbyedpi/x86_64/goodbyedpi.exe"));
    }
    // Geliştirme modu: derleme zamanı proje dizini.
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/goodbyedpi/x86_64/goodbyedpi.exe"),
    );

    for c in &candidates {
        if c.exists() {
            return Ok(c.clone());
        }
    }
    Err(format!(
        "goodbyedpi.exe bulunamadı. Aranan yerler: {:?}",
        candidates
    ))
}

#[tauri::command]
pub fn start_protection(
    app: AppHandle,
    state: State<EngineState>,
    args: Vec<String>,
) -> Result<(), String> {
    let mut guard = state.child.lock().map_err(|e| e.to_string())?;
    if guard.is_some() {
        return Err("Koruma zaten çalışıyor.".into());
    }

    let exe = goodbyedpi_path(&app)?;
    let workdir = exe
        .parent()
        .ok_or("Geçersiz yürütülebilir yolu.")?
        .to_path_buf();

    let mut cmd = Command::new(&exe);
    cmd.args(&args)
        .current_dir(&workdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("GoodbyeDPI başlatılamadı: {e}"))?;

    if let Some(out) = child.stdout.take() {
        let app2 = app.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = app2.emit("gdpi-log", line);
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        let app3 = app.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                let _ = app3.emit("gdpi-log", format!("[hata] {line}"));
            }
        });
    }

    *guard = Some(child);
    let _ = app.emit("gdpi-status", true);
    let _ = app.emit("gdpi-log", format!("Koruma başlatıldı: goodbyedpi {}", args.join(" ")));
    Ok(())
}

#[tauri::command]
pub fn stop_protection(app: AppHandle, state: State<EngineState>) -> Result<(), String> {
    stop_internal(state.inner());
    let _ = app.emit("gdpi-status", false);
    let _ = app.emit("gdpi-log", "Koruma durduruldu.".to_string());
    Ok(())
}

#[tauri::command]
pub fn get_status(state: State<EngineState>) -> bool {
    let mut guard = match state.child.lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    if let Some(child) = guard.as_mut() {
        match child.try_wait() {
            Ok(Some(_)) => {
                *guard = None;
                false
            }
            Ok(None) => true,
            Err(_) => true,
        }
    } else {
        false
    }
}

// Olay/AppHandle gerektirmeden süreci öldürür (tepsi menüsü "Çıkış" için).
pub fn stop_internal(state: &EngineState) {
    if let Ok(mut guard) = state.child.lock() {
        if let Some(mut child) = guard.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
