// Şifreli DNS (DNS-over-HTTPS) yönetimi.
// Türkiye'deki erişim engellerinin çoğu DNS zehirlemesiyle yapıldığı için
// asıl çözüm budur: sistem DNS'ini DoH'a çevirip ISP'nin sahte cevap
// dönmesini engelleriz. Uygulama yönetici olarak çalıştığı için PowerShell
// cmdlet'lerini doğrudan çağırırız.

use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn run_ps(script: &str) -> Result<String, String> {
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
    ]);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let out = cmd.output().map_err(|e| format!("PowerShell çalıştırılamadı: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

const ENABLE_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$idx = (Get-NetRoute -DestinationPrefix '0.0.0.0/0' | Sort-Object RouteMetric | Select-Object -First 1).InterfaceIndex
$guid = (Get-NetAdapter -InterfaceIndex $idx).InterfaceGuid
$v4 = @{ '1.1.1.1'='https://cloudflare-dns.com/dns-query'; '8.8.8.8'='https://dns.google/dns-query' }
$v6 = @{ '2606:4700:4700::1111'='https://cloudflare-dns.com/dns-query'; '2001:4860:4860::8888'='https://dns.google/dns-query' }
$all = $v4 + $v6
foreach ($s in $all.Keys) {
  try { Add-DnsClientDohServerAddress -ServerAddress $s -DohTemplate $all[$s] -AllowFallbackToUdp $false -AutoUpgrade $true -ErrorAction Stop }
  catch { try { Set-DnsClientDohServerAddress -ServerAddress $s -DohTemplate $all[$s] -AllowFallbackToUdp $false -AutoUpgrade $true -ErrorAction Stop } catch {} }
}
Set-DnsClientServerAddress -InterfaceIndex $idx -ServerAddresses ('1.1.1.1','8.8.8.8','2606:4700:4700::1111','2001:4860:4860::8888')
foreach ($s in $all.Keys) {
  $b = "HKLM:\SYSTEM\CurrentControlSet\Services\Dnscache\InterfaceSpecificParameters\$guid\DohInterfaceSettings\Doh\$s"
  New-Item -Path $b -Force | Out-Null
  New-ItemProperty -Path $b -Name 'DohFlags' -Value 1 -PropertyType QWord -Force | Out-Null
}
Clear-DnsClientCache
ipconfig /flushdns | Out-Null
'ok'
"#;

const DISABLE_SCRIPT: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
$idx = (Get-NetRoute -DestinationPrefix '0.0.0.0/0' | Sort-Object RouteMetric | Select-Object -First 1).InterfaceIndex
Set-DnsClientServerAddress -InterfaceIndex $idx -ResetServerAddresses
Clear-DnsClientCache
ipconfig /flushdns | Out-Null
'ok'
"#;

const STATUS_SCRIPT: &str = r#"
$idx = (Get-NetRoute -DestinationPrefix '0.0.0.0/0' | Sort-Object RouteMetric | Select-Object -First 1).InterfaceIndex
$dns = (Get-DnsClientServerAddress -InterfaceIndex $idx -AddressFamily IPv4).ServerAddresses
if ($dns -contains '1.1.1.1' -or $dns -contains '8.8.8.8') { 'on' } else { 'off' }
"#;

#[tauri::command]
pub fn enable_doh() -> Result<(), String> {
    run_ps(ENABLE_SCRIPT).map(|_| ())
}

#[tauri::command]
pub fn disable_doh() -> Result<(), String> {
    run_ps(DISABLE_SCRIPT).map(|_| ())
}

#[tauri::command]
pub fn doh_status() -> bool {
    run_ps(STATUS_SCRIPT)
        .map(|s| s.trim() == "on")
        .unwrap_or(false)
}
