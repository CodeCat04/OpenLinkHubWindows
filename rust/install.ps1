# OpenLinkHub for Windows: one-line installer.
#
#   irm https://github.com/CodeCat04/OpenLinkHubWindows/releases/latest/download/install.ps1 | iex
#
# Downloads the latest OpenLinkHub.exe from this repository's GitHub Releases, verifies its
# SHA-256 checksum, and installs it for the current user (no administrator rights needed).

$ErrorActionPreference = 'Stop'
$base = 'https://github.com/CodeCat04/OpenLinkHubWindows/releases/latest/download'
$tmp  = Join-Path ([IO.Path]::GetTempPath()) ("OpenLinkHub-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null

try {
    Write-Host 'Downloading OpenLinkHub...'
    $exe = Join-Path $tmp 'OpenLinkHub.exe'
    Invoke-WebRequest -UseBasicParsing "$base/OpenLinkHub.exe" -OutFile $exe
    $expected = ((Invoke-WebRequest -UseBasicParsing "$base/OpenLinkHub.exe.sha256").Content -split '\s+')[0].Trim().ToLower()
    $actual = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLower()
    if ($expected -ne $actual) { throw "Checksum mismatch (expected $expected, got $actual). Aborting." }

    if (Get-Process -Name 'iCUE' -ErrorAction SilentlyContinue) {
        Write-Warning 'iCUE is running. Quit iCUE (or uninstall it) so OpenLinkHub can control your hubs.'
    }

    Write-Host 'Installing...'
    Unblock-File $exe
    $p = Start-Process -FilePath $exe -ArgumentList '--install', '--quiet' -PassThru -Wait
    if ($p.ExitCode -ne 0) { throw "Installer exited with code $($p.ExitCode)." }

    Start-Sleep -Seconds 3
    Start-Process 'http://127.0.0.1:27003/'
    Write-Host 'OpenLinkHub is installed and running. Find it in the Start menu or the system tray.' -ForegroundColor Green
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
