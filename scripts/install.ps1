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
$version = $tag.TrimStart('v')

$asset = "jetemail-$version-x86_64-pc-windows-msvc.exe"
$url = "https://github.com/$Repo/releases/download/$tag/$asset"

Info "Downloading $asset"
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$dest = Join-Path $InstallDir $Bin
try {
    Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing
} catch {
    Fail "download failed: $url`n$_"
}
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
