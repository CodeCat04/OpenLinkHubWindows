<#
.SYNOPSIS
    Builds OpenLinkHub.exe and the OpenLinkHub-Setup installer into .\dist

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\build.ps1
#>
param(
    # Build only OpenLinkHub.exe, skip the installer.
    [switch]$NoInstaller,
    # Skip the unit tests.
    [switch]$SkipTests,
    # Fail if Inno Setup is missing instead of skipping the installer (used by CI).
    [switch]$RequireInstaller
)

$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

function Fail($message) {
    Write-Host ""
    Write-Host $message -ForegroundColor Red
    exit 1
}

function Find-Iscc {
    $cmd = Get-Command iscc -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    foreach ($dir in @("${env:ProgramFiles(x86)}\Inno Setup 6", "$env:ProgramFiles\Inno Setup 6", "$env:LOCALAPPDATA\Programs\Inno Setup 6")) {
        $exe = Join-Path $dir 'ISCC.exe'
        if (Test-Path $exe) { return $exe }
    }
    return $null
}

# --- Prerequisites -----------------------------------------------------------
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    $cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
    if (Test-Path (Join-Path $cargoBin 'cargo.exe')) {
        $env:Path = "$cargoBin;$env:Path"
    } else {
        Fail "Rust is not installed. Install it with:`n    winget install Rustlang.Rustup`nthen open a new terminal and run this script again."
    }
}

$iscc = $null
if (-not $NoInstaller) {
    $iscc = Find-Iscc
    if (-not $iscc) {
        if ($RequireInstaller) { Fail "Inno Setup 6 not found." }
        Write-Host "Inno Setup not found, so only OpenLinkHub.exe will be built." -ForegroundColor Yellow
        Write-Host "To also build the installer:  winget install JRSoftware.InnoSetup" -ForegroundColor Yellow
    }
}

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "Building OpenLinkHub $version" -ForegroundColor Cyan

# --- Build -------------------------------------------------------------------
if (-not $SkipTests) {
    cargo test --locked
    if ($LASTEXITCODE -ne 0) { Fail "Tests failed." }
}

cargo build --locked --release
if ($LASTEXITCODE -ne 0) {
    Fail ("Build failed. If the error mentions link.exe or rc.exe, install the C++ build tools:`n" +
          "    winget install Microsoft.VisualStudio.2022.BuildTools --override `"--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended`"")
}

New-Item -ItemType Directory -Force dist | Out-Null
Copy-Item target\release\OpenLinkHub.exe dist\OpenLinkHub.exe -Force
Copy-Item target\release\OpenLinkHubSensors.exe dist\OpenLinkHubSensors.exe -Force

if ($iscc) {
    & $iscc /Qp "/DAppVersion=$version" installer\OpenLinkHub.iss
    if ($LASTEXITCODE -ne 0) { Fail "Installer build failed." }
}

# --- Summary -----------------------------------------------------------------
Write-Host ""
Write-Host "Done:" -ForegroundColor Green
Get-ChildItem dist -File | ForEach-Object {
    $hash = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower()
    Write-Host ("  dist\{0}  ({1:N1} MB, sha256 {2})" -f $_.Name, ($_.Length / 1MB), $hash)
}
if ($iscc) {
    Write-Host ""
    Write-Host "Run dist\OpenLinkHub-Setup-$version.exe to install." -ForegroundColor Green
}
