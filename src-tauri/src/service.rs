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
    Err(format!(
        "goodbyedpi.exe bulunamadı. Aranan: {:?}",
        candidates
    ))
}

#[tauri::command]
pub fn service_installed() -> bool {
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-ScheduledTask -TaskName 'MavroDPI' -ErrorAction SilentlyContinue | Select-Object -ExpandProperty TaskName",
        ])
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim() == "MavroDPI",
        Err(_) => false,
    }
}

#[tauri::command]
pub fn install_service(app: tauri::AppHandle) -> Result<(), String> {
    let src = find_goodbyedpi_dir(&app)?;
    let dst = install_dir();

    // Eğer servis zaten çalışıyorsa dosyalar kilitli olur — önce durdur.
    let _ = std::process::Command::new("powershell")
        .args([
            "-NoProfile", "-NonInteractive", "-Command",
            "Stop-ScheduledTask -TaskName 'MavroDPI' -ErrorAction SilentlyContinue",
        ])
        .output();

    std::fs::create_dir_all(&dst).map_err(|e| format!("Klasör oluşturulamadı: {e}"))?;

    for file in &["goodbyedpi.exe", "WinDivert.dll", "WinDivert64.sys"] {
        std::fs::copy(src.join(file), dst.join(file))
            .map_err(|e| format!("{file} kopyalanamadı: {e}"))?;
    }

    let exe = dst.join("goodbyedpi.exe");
    let exe_str = exe.to_string_lossy();
    let dir_str = dst.to_string_lossy();

    let script = format!(
        r#"
$action  = New-ScheduledTaskAction -Execute '{exe}' -Argument '-5' -WorkingDirectory '{dir}'
$trigger = New-ScheduledTaskTrigger -AtStartup
$set     = New-ScheduledTaskSettingsSet -StartWhenAvailable -ExecutionTimeLimit 0
$prin    = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
Unregister-ScheduledTask -TaskName 'MavroDPI' -Confirm:$false -ErrorAction SilentlyContinue
Register-ScheduledTask -TaskName 'MavroDPI' -Action $action -Trigger $trigger -Settings $set -Principal $prin | Out-Null
Start-ScheduledTask -TaskName 'MavroDPI'
"#,
        exe = exe_str,
        dir = dir_str,
    );

    run_ps(&script)
}

#[tauri::command]
pub fn uninstall_service() -> Result<(), String> {
    run_ps(
        r"
Stop-ScheduledTask  -TaskName 'MavroDPI' -ErrorAction SilentlyContinue
Unregister-ScheduledTask -TaskName 'MavroDPI' -Confirm:$false -ErrorAction SilentlyContinue
",
    )
}

#[tauri::command]
pub fn start_service_now() -> Result<(), String> {
    run_ps("Start-ScheduledTask -TaskName 'MavroDPI'")
}

fn run_ps(script: &str) -> Result<(), String> {
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .map_err(|e| e.to_string())?;

    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        Err(format!("{}{}", stderr, stdout).trim().to_string())
    }
}
