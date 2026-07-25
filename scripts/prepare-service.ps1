[CmdletBinding()]
param(
    [string] $OutputPath
)

$ErrorActionPreference = "Stop"

function Assert-RealPortableExecutable {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Path
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Service helper was not produced: $Path"
    }

    $bytes = [System.IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -lt 4096) {
        throw "Service helper is too small ($($bytes.Length) bytes); refusing a placeholder."
    }
    if ($bytes[0] -ne 0x4D -or $bytes[1] -ne 0x5A) {
        throw "Service helper does not have an MZ executable header."
    }

    $peOffset = [System.BitConverter]::ToInt32($bytes, 0x3C)
    if (
        $peOffset -lt 0x40 -or
        $peOffset + 4 -gt $bytes.Length -or
        $bytes[$peOffset] -ne 0x50 -or
        $bytes[$peOffset + 1] -ne 0x45 -or
        $bytes[$peOffset + 2] -ne 0x00 -or
        $bytes[$peOffset + 3] -ne 0x00
    ) {
        throw "Service helper does not have a valid PE signature."
    }
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$cargoManifest = Join-Path $repositoryRoot "src-tauri\service-helper\Cargo.toml"
$cargoLock = Join-Path $repositoryRoot "src-tauri\service-helper\Cargo.lock"
$resourceTarget = if ($OutputPath) {
    [System.IO.Path]::GetFullPath($OutputPath)
}
else {
    Join-Path $repositoryRoot "src-tauri\resources\mavrodpi-svc.exe"
}
$resourceDirectory = Split-Path -Parent $resourceTarget
$temporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) (
    "mavrodpi-svc-build-" + [System.Guid]::NewGuid().ToString("N")
)

if (
    -not (Test-Path -LiteralPath $cargoManifest -PathType Leaf) -or
    -not (Test-Path -LiteralPath $cargoLock -PathType Leaf)
) {
    throw "Locked Rust service project was not found."
}

$previousCargoTarget = [Environment]::GetEnvironmentVariable(
    "CARGO_TARGET_DIR",
    "Process"
)
try {
    $temporaryTarget = Join-Path $temporaryRoot "target"
    New-Item -ItemType Directory -Path $temporaryTarget -Force | Out-Null
    [Environment]::SetEnvironmentVariable(
        "CARGO_TARGET_DIR",
        $temporaryTarget,
        "Process"
    )

    Push-Location $repositoryRoot
    try {
        cargo build `
            --manifest-path $cargoManifest `
            --release `
            --locked `
            --bin mavrodpi-svc
        if ($LASTEXITCODE -ne 0) {
            throw "Locked service helper build failed with exit code $LASTEXITCODE."
        }
    }
    finally {
        Pop-Location
    }

    $builtHelper = Join-Path $temporaryTarget "release\mavrodpi-svc.exe"
    Assert-RealPortableExecutable -Path $builtHelper

    New-Item -ItemType Directory -Path $resourceDirectory -Force | Out-Null
    Copy-Item -LiteralPath $builtHelper -Destination $resourceTarget -Force
    Assert-RealPortableExecutable -Path $resourceTarget

    $hash = (Get-FileHash -LiteralPath $resourceTarget -Algorithm SHA256).Hash
    Write-Host "Verified service helper: $resourceTarget"
    Write-Host "SHA256: $hash"
}
finally {
    [Environment]::SetEnvironmentVariable(
        "CARGO_TARGET_DIR",
        $previousCargoTarget,
        "Process"
    )
    if (Test-Path -LiteralPath $temporaryRoot) {
        $resolvedTemporaryRoot = [System.IO.Path]::GetFullPath($temporaryRoot)
        $resolvedSystemTemp = [System.IO.Path]::GetFullPath(
            [System.IO.Path]::GetTempPath()
        )
        if (
            -not $resolvedTemporaryRoot.StartsWith(
                $resolvedSystemTemp,
                [System.StringComparison]::OrdinalIgnoreCase
            ) -or
            (Split-Path -Leaf $resolvedTemporaryRoot) -notlike "mavrodpi-svc-build-*"
        ) {
            throw "Refusing to clean an unexpected build path: $resolvedTemporaryRoot"
        }
        Remove-Item -LiteralPath $temporaryRoot -Recurse -Force
    }
}
