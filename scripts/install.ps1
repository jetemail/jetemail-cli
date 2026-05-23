# Installer for jetemail-cli on Windows.
#
# Usage:
#   irm https://github.com/jetemail/jetemail-cli/releases/latest/download/install.ps1 | iex
#
# Honored env vars:
#   $env:JETEMAIL_INSTALL_DIR  install location (default: $env:LOCALAPPDATA\jetemail\bin)
#   $env:JETEMAIL_VERSION      pin to a specific version, e.g. v0.1.2 (default: latest)

$ErrorActionPreference = 'Stop'

$Repo = 'jetemail/jetemail-cli'
$Bin = 'jetemail.exe'
$InstallDir = if ($env:JETEMAIL_INSTALL_DIR) { $env:JETEMAIL_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'jetemail\bin' }

function Info($msg) { Write-Host "==> $msg" -ForegroundColor Green }
function Fail($msg) { Write-Host "error: $msg" -ForegroundColor Red; exit 1 }

if ([Environment]::Is64BitOperatingSystem -eq $false) {
    Fail "32-bit Windows is not supported"
}

if ($env:JETEMAIL_VERSION) {
    $tag = $env:JETEMAIL_VERSION
} else {
    Info "Looking up latest release"
    try {
        $tag = (Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest").tag_name
    } catch {
        Fail "could not determine latest release: $_"
    }
}
# Validate the tag is a plain semver before interpolating it into URLs.
if ($tag -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+([.+-][0-9A-Za-z.-]+)?$') {
    Fail "unexpected version tag: $tag"
}
$version = $tag.TrimStart('v')

$asset = "jetemail-$version-x86_64-pc-windows-msvc.exe"
$url = "https://github.com/$Repo/releases/download/$tag/$asset"
$sumsUrl = "https://github.com/$Repo/releases/download/$tag/SHA256SUMS"

Info "Downloading $asset"
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$dest = Join-Path $InstallDir $Bin

# Download to a temp file and verify its SHA-256 against the release's published
# SHA256SUMS before moving it into place — fail closed on any mismatch so a
# tampered binary is never installed.
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("jetemail-" + [System.Guid]::NewGuid().ToString() + ".exe")
$sumsTmp = "$tmp.SHA256SUMS"
try {
    Invoke-WebRequest -Uri $url -OutFile $tmp -UseBasicParsing
} catch {
    Fail "download failed: $url`n$_"
}
try {
    Invoke-WebRequest -Uri $sumsUrl -OutFile $sumsTmp -UseBasicParsing
} catch {
    Remove-Item -Force $tmp -ErrorAction SilentlyContinue
    Fail "could not fetch checksums: $sumsUrl`n$_"
}
$expected = Get-Content $sumsTmp | ForEach-Object {
    $p = $_ -split '\s+', 2
    if ($p.Count -eq 2 -and ($p[1].TrimStart('*') -eq $asset)) { $p[0] }
} | Select-Object -First 1
$actual = (Get-FileHash -Algorithm SHA256 -Path $tmp).Hash
Remove-Item -Force $sumsTmp -ErrorAction SilentlyContinue
if (-not $expected) {
    Remove-Item -Force $tmp -ErrorAction SilentlyContinue
    Fail "no checksum listed for $asset in SHA256SUMS"
}
# PowerShell string comparison is case-insensitive by default (sha256sum is
# lowercase, Get-FileHash uppercase).
if ($actual -ne $expected.Trim()) {
    Remove-Item -Force $tmp -ErrorAction SilentlyContinue
    Fail "checksum mismatch for $asset (expected $($expected.Trim()), got $actual)"
}
Info "Verified SHA-256 checksum"

Move-Item -Force $tmp $dest
Info "Installed to $dest"

# Persist InstallDir on the user PATH if missing.
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$paths = ($userPath -split ';') | Where-Object { $_ }
if ($paths -notcontains $InstallDir) {
    $newPath = (($paths + $InstallDir) -join ';')
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    Info "Added $InstallDir to your user PATH (open a new shell to pick it up)"
}

try { & $dest --version } catch { }
