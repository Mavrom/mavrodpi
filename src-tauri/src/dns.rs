// Windows 11 yerel DNS-over-HTTPS yönetimi.
// Etkinleştirmeden önce kullanıcının gerçek statik/DHCP DNS yapılandırması
// diske kaydedilir; durdurma, çökme sonrası açılış ve kaldırma akışında aynen
// geri yüklenir.

use std::net::IpAddr;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use std::{fs::OpenOptions, io::Read};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
use std::{ffi::OsString, os::windows::ffi::OsStringExt};
#[cfg(windows)]
use windows_sys::Win32::System::Com::CoTaskMemFree;
#[cfg(windows)]
use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;
#[cfg(windows)]
use windows_sys::Win32::UI::Shell::{FOLDERID_ProgramData, SHGetKnownFolderPath};
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static DNS_OPERATION_LOCK: Mutex<()> = Mutex::new(());
const TRUST_MARKER: &str = "state-format-v1.trusted";
const SNAPSHOT_FILE: &str = "dns-snapshot.json";
const TRUST_MARKER_CONTENTS: &[u8] = b"MavroDPI DNS state v1\r\n";

fn lock_dns_operations() -> Result<MutexGuard<'static, ()>, String> {
    DNS_OPERATION_LOCK
        .lock()
        .map_err(|_| "DNS işlem kilidi beklenmedik biçimde bozuldu.".into())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DnsSnapshot {
    interface_index: u32,
    interface_guid: String,
    name_server_v4: Option<String>,
    name_server_v6: Option<String>,
    #[serde(default)]
    doh_servers: Vec<DohServerSnapshot>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DohServerSnapshot {
    server_address: String,
    existed: bool,
    doh_template: Option<String>,
    allow_fallback_to_udp: bool,
    auto_upgrade: bool,
}

#[cfg(windows)]
fn windows_directory() -> Result<PathBuf, String> {
    let mut buffer = vec![0_u16; 32_768];
    let length = unsafe { GetWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    if length == 0 || length as usize >= buffer.len() {
        return Err("Windows sistem klasörü bulunamadı.".into());
    }

    Ok(PathBuf::from(OsString::from_wide(
        &buffer[..length as usize],
    )))
}

#[cfg(windows)]
fn powershell_path() -> Result<PathBuf, String> {
    let windows_dir = windows_directory()?;
    let powershell = windows_dir.join("System32/WindowsPowerShell/v1.0/powershell.exe");
    if !powershell.is_file() {
        return Err("Windows PowerShell bulunamadı.".into());
    }
    Ok(powershell)
}

#[cfg(not(windows))]
fn powershell_path() -> Result<PathBuf, String> {
    Ok(PathBuf::from("powershell"))
}

#[cfg(windows)]
fn configure_trusted_powershell(cmd: &mut Command) -> Result<String, String> {
    let windows_dir = windows_directory()?;
    let system32 = windows_dir.join("System32");
    let module_root = system32.join("WindowsPowerShell/v1.0/Modules");
    let net_tcpip = module_root.join("NetTCPIP/NetTCPIP.psd1");
    let dns_client = module_root.join("DnsClient/DnsClient.psd1");
    let ipconfig = system32.join("ipconfig.exe");

    for (name, path) in [
        ("NetTCPIP", &net_tcpip),
        ("DnsClient", &dns_client),
        ("ipconfig.exe", &ipconfig),
    ] {
        if !path.is_file() {
            return Err(format!("Güvenilir Windows bileşeni bulunamadı: {name}"));
        }
    }

    cmd.env_clear()
        .env("SystemRoot", &windows_dir)
        .env("WINDIR", &windows_dir)
        .env("PATH", &system32)
        .env("PSModulePath", &module_root)
        .env("MAVRODPI_NETTCPIP_MODULE", &net_tcpip)
        .env("MAVRODPI_DNSCLIENT_MODULE", &dns_client)
        .env("MAVRODPI_IPCONFIG_PATH", &ipconfig)
        .current_dir(&system32);

    Ok(r#"$ErrorActionPreference = 'Stop'
$netTcpipModule = [Environment]::GetEnvironmentVariable('MAVRODPI_NETTCPIP_MODULE', 'Process')
$dnsClientModule = [Environment]::GetEnvironmentVariable('MAVRODPI_DNSCLIENT_MODULE', 'Process')
$MavroDpiIpconfig = [Environment]::GetEnvironmentVariable('MAVRODPI_IPCONFIG_PATH', 'Process')
foreach ($trustedPath in @($netTcpipModule, $dnsClientModule, $MavroDpiIpconfig)) {
  if ([string]::IsNullOrWhiteSpace($trustedPath) -or -not [IO.Path]::IsPathRooted($trustedPath)) {
    throw 'Güvenilir Windows bileşeni yolu geçersiz.'
  }
}
Import-Module -Name $netTcpipModule -Force -ErrorAction Stop
Import-Module -Name $dnsClientModule -Force -ErrorAction Stop
"#
    .to_string())
}

fn run_ps(script: &str) -> Result<String, String> {
    let mut cmd = Command::new(powershell_path()?);
    #[cfg(windows)]
    let trusted_bootstrap = configure_trusted_powershell(&mut cmd)?;
    #[cfg(windows)]
    let command_script = format!("{trusted_bootstrap}\n{script}");
    #[cfg(not(windows))]
    let command_script = script.to_string();

    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        &command_script,
    ]);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let output = cmd
        .output()
        .map_err(|error| format!("PowerShell çalıştırılamadı: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if detail.is_empty() {
            "DNS işlemi başarısız oldu.".into()
        } else {
            detail
        })
    }
}

fn validated_guid(snapshot: &DnsSnapshot) -> Result<&str, String> {
    let guid = snapshot.interface_guid.trim();
    let bytes = guid.as_bytes();
    let valid = bytes.len() == 38
        && bytes[0] == b'{'
        && bytes[37] == b'}'
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            0 => *byte == b'{',
            9 | 14 | 19 | 24 => *byte == b'-',
            37 => *byte == b'}',
            _ => byte.is_ascii_hexdigit(),
        });

    if valid {
        Ok(guid)
    } else {
        Err("DNS geri yükleme kaydındaki ağ bağdaştırıcısı kimliği geçersiz.".into())
    }
}

#[cfg(windows)]
fn state_directory() -> Result<PathBuf, String> {
    let mut raw_path = std::ptr::null_mut();
    let result = unsafe {
        SHGetKnownFolderPath(
            &FOLDERID_ProgramData,
            0,
            std::ptr::null_mut(),
            &mut raw_path,
        )
    };
    if result < 0 || raw_path.is_null() {
        return Err("Windows ortak uygulama verisi klasörü bulunamadı.".into());
    }

    let length = unsafe {
        let mut length = 0_usize;
        while *raw_path.add(length) != 0 {
            length += 1;
        }
        length
    };
    let directory = PathBuf::from(OsString::from_wide(unsafe {
        std::slice::from_raw_parts(raw_path, length)
    }));
    unsafe {
        CoTaskMemFree(raw_path.cast());
    }
    Ok(directory.join("MavroDPI").join("state"))
}

#[cfg(not(windows))]
fn state_directory() -> Result<PathBuf, String> {
    Err("Şifreli DNS yalnızca Windows üzerinde desteklenir.".into())
}

fn secure_state_storage() -> Result<PathBuf, String> {
    let state = state_directory()?;
    let root = state
        .parent()
        .ok_or("DNS durum klasörünün üst yolu geçersiz.")?;

    #[cfg(windows)]
    {
        crate::windows_acl::ensure_secure_directory(root)?;
        crate::windows_acl::ensure_secure_directory(&state)?;
    }
    #[cfg(not(windows))]
    {
        return Err("Şifreli DNS yalnızca Windows üzerinde desteklenir.".into());
    }

    let marker = state.join(TRUST_MARKER);
    let snapshot = state.join(SNAPSHOT_FILE);

    match std::fs::symlink_metadata(&marker) {
        Ok(_) => {
            #[cfg(windows)]
            crate::windows_acl::verify_secure_file(&marker)?;
            let contents = std::fs::read(&marker)
                .map_err(|error| format!("DNS güven işareti okunamadı: {error}"))?;
            if contents != TRUST_MARKER_CONTENTS {
                return Err("DNS durum klasörünün güven işareti geçersiz.".into());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if std::fs::symlink_metadata(&snapshot).is_ok() {
                return Err("Güven işareti olmayan DNS geri yükleme kaydı reddedildi.".into());
            }
            #[cfg(windows)]
            crate::windows_acl::create_secure_file(&marker, TRUST_MARKER_CONTENTS)?;
        }
        Err(error) => {
            return Err(format!("DNS güven işareti doğrulanamadı: {error}"));
        }
    }

    if std::fs::symlink_metadata(&snapshot).is_ok() {
        #[cfg(windows)]
        crate::windows_acl::verify_secure_file(&snapshot)?;
    }

    Ok(snapshot)
}

fn existing_trusted_snapshot() -> Result<Option<PathBuf>, String> {
    let state = state_directory()?;
    match std::fs::symlink_metadata(&state) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!("DNS geri yükleme klasörü doğrulanamadı: {error}"));
        }
        Ok(_) => {}
    }

    let snapshot = secure_state_storage()?;
    match std::fs::symlink_metadata(&snapshot) {
        Ok(_) => {
            #[cfg(windows)]
            crate::windows_acl::verify_secure_file(&snapshot)?;
            Ok(Some(snapshot))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("DNS geri yükleme kaydı doğrulanamadı: {error}")),
    }
}

fn read_snapshot(path: &std::path::Path) -> Result<DnsSnapshot, String> {
    #[cfg(windows)]
    crate::windows_acl::verify_secure_file(path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|error| format!("DNS geri yükleme kaydı okunamadı: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("DNS geri yükleme kaydı doğrulanamadı: {error}"))?;
    if metadata.len() > 65_536 {
        return Err("DNS geri yükleme kaydı beklenenden büyük.".into());
    }
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|error| format!("DNS geri yükleme kaydı okunamadı: {error}"))?;
    serde_json::from_str(&contents)
        .map_err(|error| format!("DNS geri yükleme kaydı geçersiz: {error}"))
}

fn write_snapshot(path: &std::path::Path, snapshot: &DnsSnapshot) -> Result<(), String> {
    let serialized = serde_json::to_vec_pretty(snapshot)
        .map_err(|error| format!("DNS geri yükleme kaydı hazırlanamadı: {error}"))?;
    #[cfg(windows)]
    {
        crate::windows_acl::create_secure_file(path, &serialized)
    }
    #[cfg(not(windows))]
    {
        let _ = serialized;
        Err("Şifreli DNS yalnızca Windows üzerinde desteklenir.".into())
    }
}

const CAPTURE_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
if (-not (Get-Command Add-DnsClientDohServerAddress -ErrorAction SilentlyContinue)) {
  throw 'Bu Windows sürümü yerel DNS-over-HTTPS yapılandırmasını desteklemiyor.'
}
$route = Get-NetRoute -DestinationPrefix '0.0.0.0/0' |
  Sort-Object RouteMetric |
  Select-Object -First 1
if (-not $route) { throw 'Etkin varsayılan ağ rotası bulunamadı.' }
$idx = [int]$route.InterfaceIndex
$adapter = Get-NetAdapter -InterfaceIndex $idx -ErrorAction Stop
$guid = ([Guid]$adapter.InterfaceGuid).ToString('B')
$v4Path = "HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces\$guid"
$v6Path = "HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip6\Parameters\Interfaces\$guid"
$v4Raw = (Get-ItemProperty -Path $v4Path -Name NameServer -ErrorAction SilentlyContinue).NameServer
$v6Raw = (Get-ItemProperty -Path $v6Path -Name NameServer -ErrorAction SilentlyContinue).NameServer
$v4 = if ($null -ne $v4Raw) { ([string]$v4Raw).Trim([char]0).Trim() } else { $null }
$v6 = if ($null -ne $v6Raw) { ([string]$v6Raw).Trim([char]0).Trim() } else { $null }
$dohServers = @(
  foreach ($address in @('1.1.1.1', '8.8.8.8')) {
    $entry = Get-DnsClientDohServerAddress -ServerAddress $address -ErrorAction SilentlyContinue |
      Select-Object -First 1
    if ($entry) {
      [pscustomobject]@{
        serverAddress = $address
        existed = $true
        dohTemplate = [string]$entry.DohTemplate
        allowFallbackToUdp = [bool]$entry.AllowFallbackToUdp
        autoUpgrade = [bool]$entry.AutoUpgrade
      }
    } else {
      [pscustomobject]@{
        serverAddress = $address
        existed = $false
        dohTemplate = $null
        allowFallbackToUdp = $false
        autoUpgrade = $false
      }
    }
  }
)
[pscustomobject]@{
  interfaceIndex = $idx
  interfaceGuid = $guid
  nameServerV4 = if ($v4) { [string]$v4 } else { $null }
  nameServerV6 = if ($v6) { [string]$v6 } else { $null }
  dohServers = $dohServers
} | ConvertTo-Json -Compress
"#;

fn enable_script(snapshot: &DnsSnapshot) -> Result<String, String> {
    let interface_guid = validated_guid(snapshot)?;
    Ok(format!(
        r#"
$ErrorActionPreference = 'Stop'
$adapter = Get-NetAdapter | Where-Object {{ $_.InterfaceGuid -eq [Guid]'{interface_guid}' }} | Select-Object -First 1
if (-not $adapter) {{ throw 'Kaydedilen ağ bağdaştırıcısı artık etkin değil.' }}
$idx = [int]$adapter.InterfaceIndex
if (-not (Get-Command Add-DnsClientDohServerAddress -ErrorAction SilentlyContinue)) {{
  throw 'Bu Windows sürümü yerel DNS-over-HTTPS yapılandırmasını desteklemiyor.'
}}
Get-NetAdapter -InterfaceIndex $idx -ErrorAction Stop | Out-Null
$servers = @(
  @{{ Address='1.1.1.1'; Template='https://cloudflare-dns.com/dns-query' }},
  @{{ Address='8.8.8.8'; Template='https://dns.google/dns-query' }}
)
foreach ($server in $servers) {{
  try {{
    Add-DnsClientDohServerAddress -ServerAddress $server.Address -DohTemplate $server.Template -AllowFallbackToUdp $false -AutoUpgrade $true -ErrorAction Stop
  }} catch {{
    Set-DnsClientDohServerAddress -ServerAddress $server.Address -DohTemplate $server.Template -AllowFallbackToUdp $false -AutoUpgrade $true -ErrorAction Stop
  }}
}}
Set-DnsClientServerAddress -InterfaceIndex $idx -ServerAddresses ($servers.Address)
Clear-DnsClientCache
& $MavroDpiIpconfig /flushdns | Out-Null
$active = (Get-DnsClientServerAddress -InterfaceIndex $idx -AddressFamily IPv4).ServerAddresses
$verified = Get-DnsClientDohServerAddress | Where-Object {{
  $active -contains $_.ServerAddress -and $_.AutoUpgrade -eq $true -and $_.AllowFallbackToUdp -eq $false
}}
if (-not $verified) {{ throw 'Şifreli DNS doğrulanamadı.' }}
'ok'
"#
    ))
}

fn parse_static_servers(snapshot: &DnsSnapshot) -> Vec<IpAddr> {
    snapshot
        .name_server_v4
        .iter()
        .chain(snapshot.name_server_v6.iter())
        .flat_map(|value| value.split([',', ';', ' ', '\0']))
        .filter_map(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                trimmed.parse::<IpAddr>().ok()
            }
        })
        .collect()
}

fn powershell_single_quoted(value: &str) -> Result<String, String> {
    if value.len() > 2_048 || !value.starts_with("https://") || value.chars().any(char::is_control)
    {
        return Err("DNS geri yükleme kaydındaki DoH şablonu geçersiz.".into());
    }
    Ok(format!("'{}'", value.replace('\'', "''")))
}

fn restore_doh_commands(snapshot: &DnsSnapshot) -> Result<String, String> {
    let mut commands = String::new();
    for server in &snapshot.doh_servers {
        if server.server_address != "1.1.1.1" && server.server_address != "8.8.8.8" {
            return Err("DNS geri yükleme kaydındaki DoH sunucusu geçersiz.".into());
        }

        let address = &server.server_address;
        if server.existed {
            let template = server
                .doh_template
                .as_deref()
                .ok_or("DNS geri yükleme kaydındaki DoH şablonu eksik.")?;
            let template = powershell_single_quoted(template)?;
            let fallback = if server.allow_fallback_to_udp {
                "$true"
            } else {
                "$false"
            };
            let auto_upgrade = if server.auto_upgrade {
                "$true"
            } else {
                "$false"
            };
            commands.push_str(&format!(
                r#"
$existingDoh = Get-DnsClientDohServerAddress -ServerAddress '{address}' -ErrorAction SilentlyContinue | Select-Object -First 1
if ($existingDoh) {{
  Set-DnsClientDohServerAddress -ServerAddress '{address}' -DohTemplate {template} -AllowFallbackToUdp {fallback} -AutoUpgrade {auto_upgrade} -ErrorAction Stop
}} else {{
  Add-DnsClientDohServerAddress -ServerAddress '{address}' -DohTemplate {template} -AllowFallbackToUdp {fallback} -AutoUpgrade {auto_upgrade} -ErrorAction Stop
}}
"#
            ));
        } else {
            commands.push_str(&format!(
                r#"
$existingDoh = Get-DnsClientDohServerAddress -ServerAddress '{address}' -ErrorAction SilentlyContinue | Select-Object -First 1
if ($existingDoh) {{
  Remove-DnsClientDohServerAddress -ServerAddress '{address}' -Confirm:$false -ErrorAction Stop
}}
"#
            ));
        }
    }
    Ok(commands)
}

fn restore_script(snapshot: &DnsSnapshot) -> Result<String, String> {
    let interface_guid = validated_guid(snapshot)?;
    let addresses = parse_static_servers(snapshot);
    let doh_restore = restore_doh_commands(snapshot)?;
    if addresses.is_empty() {
        Ok(format!(
            r#"
$ErrorActionPreference = 'Stop'
$adapter = Get-NetAdapter | Where-Object {{ $_.InterfaceGuid -eq [Guid]'{}' }} | Select-Object -First 1
if (-not $adapter) {{ throw 'Kaydedilen ağ bağdaştırıcısı bulunamadı.' }}
$idx = [int]$adapter.InterfaceIndex
Set-DnsClientServerAddress -InterfaceIndex $idx -ResetServerAddresses
{doh_restore}
Clear-DnsClientCache
& $MavroDpiIpconfig /flushdns | Out-Null
'ok'
"#,
            interface_guid
        ))
    } else {
        let quoted = addresses
            .iter()
            .map(|address| format!("'{address}'"))
            .collect::<Vec<_>>()
            .join(",");
        Ok(format!(
            r#"
$ErrorActionPreference = 'Stop'
$adapter = Get-NetAdapter | Where-Object {{ $_.InterfaceGuid -eq [Guid]'{}' }} | Select-Object -First 1
if (-not $adapter) {{ throw 'Kaydedilen ağ bağdaştırıcısı bulunamadı.' }}
$idx = [int]$adapter.InterfaceIndex
Set-DnsClientServerAddress -InterfaceIndex $idx -ServerAddresses @({})
{doh_restore}
Clear-DnsClientCache
& $MavroDpiIpconfig /flushdns | Out-Null
'ok'
"#,
            interface_guid, quoted
        ))
    }
}

const STATUS_SCRIPT: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
if (-not (Get-Command Get-DnsClientDohServerAddress -ErrorAction SilentlyContinue)) { 'off'; exit 0 }
$route = Get-NetRoute -DestinationPrefix '0.0.0.0/0' |
  Sort-Object RouteMetric |
  Select-Object -First 1
if (-not $route) { 'off'; exit 0 }
$active = (Get-DnsClientServerAddress -InterfaceIndex $route.InterfaceIndex -AddressFamily IPv4).ServerAddresses
$verified = Get-DnsClientDohServerAddress | Where-Object {
  $active -contains $_.ServerAddress -and $_.AutoUpgrade -eq $true -and $_.AllowFallbackToUdp -eq $false
}
if ($verified) { 'on' } else { 'off' }
"#;

const RESET_UNMANAGED_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$route = Get-NetRoute -DestinationPrefix '0.0.0.0/0' |
  Sort-Object RouteMetric |
  Select-Object -First 1
if (-not $route) { throw 'Etkin varsayılan ağ rotası bulunamadı.' }
Get-NetAdapter -InterfaceIndex $route.InterfaceIndex -ErrorAction Stop | Out-Null
Set-DnsClientServerAddress -InterfaceIndex $route.InterfaceIndex -ResetServerAddresses
Clear-DnsClientCache
& $MavroDpiIpconfig /flushdns | Out-Null
'ok'
"#;

fn restore_from_path(path: &std::path::Path) -> Result<(), String> {
    let snapshot = read_snapshot(path)?;
    run_ps(&restore_script(&snapshot)?)?;
    std::fs::remove_file(path)
        .map_err(|error| format!("DNS geri yükleme kaydı temizlenemedi: {error}"))
}

#[tauri::command]
pub fn enable_doh(app: AppHandle) -> Result<(), String> {
    let _operation = lock_dns_operations()?;
    enable_doh_locked(&app)
}

fn enable_doh_locked(_app: &AppHandle) -> Result<(), String> {
    let path = secure_state_storage()?;
    let snapshot = if path.exists() {
        read_snapshot(&path)?
    } else {
        let captured = run_ps(CAPTURE_SCRIPT)?;
        let snapshot: DnsSnapshot = serde_json::from_str(&captured)
            .map_err(|error| format!("Mevcut DNS ayarları kaydedilemedi: {error}"))?;
        write_snapshot(&path, &snapshot)?;
        snapshot
    };

    match run_ps(&enable_script(&snapshot)?) {
        Ok(_) => Ok(()),
        Err(error) => match restore_from_path(&path) {
            Ok(()) => Err(format!("Şifreli DNS etkinleştirilemedi: {error}")),
            Err(restore_error) => Err(format!(
                "Şifreli DNS etkinleştirilemedi: {error}. DNS geri alma işlemi de tamamlanamadı: {restore_error}. Kurtarma kaydı korunuyor."
            )),
        },
    }
}

#[tauri::command]
pub fn disable_doh(app: AppHandle) -> Result<(), String> {
    disable_doh_internal(&app)
}

pub fn disable_doh_internal(app: &AppHandle) -> Result<(), String> {
    let _operation = lock_dns_operations()?;
    disable_doh_locked(app)
}

fn disable_doh_locked(_app: &AppHandle) -> Result<(), String> {
    match existing_trusted_snapshot()? {
        Some(path) => restore_from_path(&path),
        None => Ok(()),
    }
}

pub fn recover_stale_snapshot(_app: &AppHandle) {
    if let Ok(_operation) = lock_dns_operations() {
        if let Ok(Some(path)) = existing_trusted_snapshot() {
            let _ = restore_from_path(&path);
        }
    }
}

pub fn restore_persisted_snapshot() -> Result<(), String> {
    let _operation = lock_dns_operations()?;
    match existing_trusted_snapshot()? {
        Some(path) => restore_from_path(&path),
        None => Ok(()),
    }
}

#[tauri::command]
pub fn doh_managed() -> bool {
    let Ok(_operation) = lock_dns_operations() else {
        return false;
    };
    existing_trusted_snapshot()
        .map(|snapshot| snapshot.is_some())
        .unwrap_or(false)
}

#[tauri::command]
pub fn reset_unmanaged_dns() -> Result<(), String> {
    let _operation = lock_dns_operations()?;
    if existing_trusted_snapshot()?.is_some() {
        return Err(
            "Bu DNS ayarı MavroDPI tarafından yönetiliyor; normal durdurma işlemini kullanın."
                .into(),
        );
    }
    run_ps(RESET_UNMANAGED_SCRIPT).map(|_| ())
}

#[tauri::command]
pub fn doh_status() -> bool {
    run_ps(STATUS_SCRIPT)
        .map(|status| status.trim() == "on")
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{
        parse_static_servers, powershell_single_quoted, restore_doh_commands, DnsSnapshot,
        DohServerSnapshot,
    };
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    fn snapshot() -> DnsSnapshot {
        DnsSnapshot {
            interface_index: 13,
            interface_guid: "{66cbb323-e5e4-4c3f-83b6-b8b833de3168}".into(),
            name_server_v4: Some("1.1.1.1,8.8.8.8\0\0".into()),
            name_server_v6: Some("2606:4700:4700::1111,2001:4860:4860::8888\0".into()),
            doh_servers: vec![],
        }
    }

    #[test]
    fn parses_static_dns_without_registry_padding() {
        assert_eq!(
            parse_static_servers(&snapshot()),
            vec![
                IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
                IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
                IpAddr::V6("2606:4700:4700::1111".parse::<Ipv6Addr>().unwrap()),
                IpAddr::V6("2001:4860:4860::8888".parse::<Ipv6Addr>().unwrap()),
            ]
        );
    }

    #[test]
    fn rejects_unsafe_doh_snapshot_values() {
        assert!(powershell_single_quoted("http://resolver.test/dns-query").is_err());
        assert!(powershell_single_quoted("https://resolver.test/\nquery").is_err());

        let mut value = snapshot();
        value.doh_servers.push(DohServerSnapshot {
            server_address: "9.9.9.9".into(),
            existed: false,
            doh_template: None,
            allow_fallback_to_udp: false,
            auto_upgrade: false,
        });
        assert!(restore_doh_commands(&value).is_err());
    }

    #[test]
    fn escapes_single_quotes_in_https_templates() {
        assert_eq!(
            powershell_single_quoted("https://resolver.test/a'b").unwrap(),
            "'https://resolver.test/a''b'"
        );
    }
}
