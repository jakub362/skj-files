# Build the Windows installer.
#
#   powershell -ExecutionPolicy Bypass -File scripts\package-windows.ps1
#
# Builds the release .exe if it is missing or stale, then runs Inno Setup over
# installer\skj-files.iss. The version comes from Cargo.toml, so it is never typed twice.
#
# Needs Inno Setup 6:  winget install -e --id JRSoftware.InnoSetup

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# --- version, straight from Cargo.toml (the single source of truth) ---
$cargo = Get-Content 'Cargo.toml' -Raw
if ($cargo -notmatch '(?m)^\s*version\s*=\s*"([^"]+)"') { throw 'Could not read the version from Cargo.toml' }
$version = $Matches[1]
# the project URL lives in Cargo.toml too, so moving the repo is one line there
if ($cargo -match '(?m)^\s*repository\s*=\s*"([^"]+)"') { $repoUrl = $Matches[1].TrimEnd('/') }
else { $repoUrl = 'https://github.com/jakub362/skj-files' }
Write-Host "SKJ Files $version  ($repoUrl)" -ForegroundColor Cyan

# --- the binary ---
$exe = 'target\release\skj-files.exe'
if (-not (Test-Path $exe)) {
    Write-Host 'Building the release binary...' -ForegroundColor Yellow
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
}
if (-not (Test-Path $exe)) { throw "Missing $exe" }

# refuse to package a binary whose version does not match Cargo.toml
$exeVer = (Get-Item $exe).VersionInfo.FileVersion
if ($exeVer -and $exeVer.Split('.')[0..2] -join '.' -ne $version) {
    throw "$exe reports version $exeVer but Cargo.toml says $version - run 'cargo build --release' again"
}

# --- Inno Setup ---
$iscc = (Get-Command iscc -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    # winget installs per-user under LOCALAPPDATA; the .exe installer and CI runners use Program Files
    $candidates = @(
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
    )
    foreach ($c in $candidates) { if (Test-Path $c) { $iscc = $c; break } }
}
if (-not $iscc) {
    throw "Inno Setup 6 not found. Install it with:  winget install -e --id JRSoftware.InnoSetup"
}

New-Item -ItemType Directory -Force -Path 'dist' | Out-Null
& $iscc "/DAppVersion=$version" "/DExePath=..\$exe" "/DAppUrl=$repoUrl" 'installer\skj-files.iss'
if ($LASTEXITCODE -ne 0) { throw 'Inno Setup failed' }

$out = "dist\SKJ-Files-$version-windows-x64-setup.exe"
if (-not (Test-Path $out)) { throw "Inno Setup reported success but $out is missing" }
$mb = [math]::Round((Get-Item $out).Length / 1MB, 1)
Write-Host "`nBuilt $out ($mb MB)" -ForegroundColor Green
