use std::path::PathBuf;
use tauri::Manager;

fn install_dir() -> PathBuf {
    PathBuf::from(r"C:\ProgramData\MavroDPI")
}

fn find_goodbyedpi_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(res) = app.path().resource_dir() {
        candidates.push(res.join("resources/goodbyedpi/x86_64"));
        candidates.push(res.join("goodbyedpi/x86_64"));
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/goodbyedpi/x86_64"),
    );
    for c in &candidates {
        if c.join("goodbyedpi.exe").exists() {
            return Ok(c.clone());
        }
    }
    Err(format!("goodbyedpi.exe bulunamadı. Aranan: {:?}", candidates))
}

fn find_service_host(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(res) = app.path().resource_dir() {
        candidates.push(res.join("resources/mavrodpi-svc.exe"));
        candidates.push(res.join("mavrodpi-svc.exe"));
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/mavrodpi-svc.exe"),
    );
    // dev: binary next to the running exe
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("mavrodpi-svc.exe"));
        }
    }
    for c in &candidates {
        if c.exists() {
            return Ok(c.clone());
        }
    }
    Err(format!("mavrodpi-svc.exe bulunamadı. Aranan: {:?}", candidates))
}

fn sc(args: &[&str]) -> std::io::Result<std::process::Output> {
    std::process::Command::new("sc.exe").args(args).output()
}

#[tauri::command]
pub fn service_installed() -> bool {
    sc(&["query", "MavroDPI"])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[tauri::command]
pub fn install_service(app: tauri::AppHandle) -> Result<(), String> {
    let src = find_goodbyedpi_dir(&app)?;
    let svc_bin = find_service_host(&app)?;
    let dst = install_dir();

    // Tüm çalışan goodbyedpi örneklerini durdur.
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/IM", "goodbyedpi.exe"])
        .output();

    // Varsa eski servisi durdur ve sil.
    let _ = sc(&["stop", "MavroDPI"]);
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let _ = sc(&["delete", "MavroDPI"]);
    std::thread::sleep(std::time::Duration::from_millis(800));

    std::fs::create_dir_all(&dst).map_err(|e| format!("Klasör oluşturulamadı: {e}"))?;

    for file in &["goodbyedpi.exe", "WinDivert.dll", "WinDivert64.sys"] {
        std::fs::copy(src.join(file), dst.join(file))
            .map_err(|e| format!("{file} kopyalanamadı: {e}"))?;
    }
    std::fs::copy(&svc_bin, dst.join("mavrodpi-svc.exe"))
        .map_err(|e| format!("mavrodpi-svc.exe kopyalanamadı: {e}"))?;

    let svc_exe = dst.join("mavrodpi-svc.exe");
    let svc_path = svc_exe.to_string_lossy().to_string();

    // Windows Service olarak kaydet.
    let out = sc(&[
        "create", "MavroDPI",
        "binPath=", &svc_path,
        "start=", "auto",
        "DisplayName=", "MavroDPI",
        "type=", "own",
    ]).map_err(|e| e.to_string())?;

    if !out.status.success() {
        return Err(format!(
            "Servis oluşturulamadı: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        ));
    }

    // Çökerse otomatik yeniden başlat.
    let _ = sc(&[
        "failure", "MavroDPI",
        "reset=", "86400",
        "actions=", "restart/5000/restart/10000/restart/30000",
    ]);

    // Hemen başlat.
    let _ = sc(&["start", "MavroDPI"]);

    Ok(())
}

#[tauri::command]
pub fn uninstall_service() -> Result<(), String> {
    let _ = sc(&["stop", "MavroDPI"]);
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let out = sc(&["delete", "MavroDPI"]).map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[tauri::command]
pub fn start_service_now() -> Result<(), String> {
    let out = sc(&["start", "MavroDPI"]).map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}
