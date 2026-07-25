$ErrorActionPreference = 'Stop'

$cimModule = [Environment]::GetEnvironmentVariable(
  'MAVRODPI_CIM_MODULE',
  'Process'
)
if (
  [string]::IsNullOrWhiteSpace($cimModule) -or
  -not [IO.Path]::IsPathRooted($cimModule) -or
  -not (Test-Path -LiteralPath $cimModule -PathType Leaf)
) {
  throw 'Trusted CIM module path is invalid.'
}
Import-Module -Name $cimModule -Force -ErrorAction Stop

$expectedRaw = [Environment]::GetEnvironmentVariable(
  'MAVRODPI_LEGACY_ENGINE_PATH',
  'Process'
)
if ([string]::IsNullOrWhiteSpace($expectedRaw)) {
  throw 'Legacy engine path was not provided.'
}

$expected = [IO.Path]::GetFullPath($expectedRaw)
if ([IO.Path]::GetFileName($expected) -ine 'goodbyedpi.exe') {
  throw 'Legacy engine file name is invalid.'
}

$programFilesRaw = [Environment]::GetEnvironmentVariable('ProgramW6432', 'Process')
if ([string]::IsNullOrWhiteSpace($programFilesRaw)) {
  $programFilesRaw = [Environment]::GetFolderPath('ProgramFiles')
}
$programFiles = [IO.Path]::GetFullPath($programFilesRaw).TrimEnd('\') + '\'
if (-not $expected.StartsWith($programFiles, [StringComparison]::OrdinalIgnoreCase)) {
  throw 'Legacy engine is outside Program Files.'
}

function Get-ExactLegacyProcess {
  @(
    Get-CimInstance Win32_Process -Filter "Name='goodbyedpi.exe'" -ErrorAction Stop |
      Where-Object {
        if (-not $_.ExecutablePath) {
          return $false
        }
        try {
          return [IO.Path]::GetFullPath([string]$_.ExecutablePath).Equals(
            $expected,
            [StringComparison]::OrdinalIgnoreCase
          )
        } catch {
          return $false
        }
      }
  )
}

foreach ($process in (Get-ExactLegacyProcess)) {
  $result = Invoke-CimMethod `
    -InputObject $process `
    -MethodName Terminate `
    -ErrorAction Stop
  if ([int]$result.ReturnValue -ne 0) {
    throw "Legacy engine could not be stopped: $($result.ReturnValue)"
  }
}

$deadline = [DateTime]::UtcNow.AddSeconds(10)
do {
  $remaining = @(Get-ExactLegacyProcess)
  if ($remaining.Count -eq 0) {
    break
  }
  Start-Sleep -Milliseconds 200
} while ([DateTime]::UtcNow -lt $deadline)

if (@(Get-ExactLegacyProcess).Count -ne 0) {
  throw 'Legacy engine is still running.'
}
