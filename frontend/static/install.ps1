# ololo CLI installer — Windows.
#
#   powershell -c "irm ololo.dev/install.ps1 | iex"
#
# Install a specific version (default: latest release):
#   powershell -c "$env:OLOLO_VERSION='0.1.0'; irm ololo.dev/install.ps1 | iex"
#
# Install directory (default: %LOCALAPPDATA%\Programs\ololo):
#   powershell -c "$env:OLOLO_INSTALL_DIR='C:\tools\ololo'; irm ololo.dev/install.ps1 | iex"

$ErrorActionPreference = "Stop"

$Repo = "ololo-dev/ololo"
$Artifact = "ololo-windows-x86_64.zip"

function Say($msg) { Write-Host "==> $msg" -ForegroundColor Blue }

if (-not [Environment]::Is64BitOperatingSystem) {
  throw "ololo requires a 64-bit Windows system"
}

# --- Resolve download URL ----------------------------------------------------
$Version = $env:OLOLO_VERSION
if ($Version) {
  $Version = $Version.TrimStart("v")
  $Url = "https://github.com/$Repo/releases/download/v$Version/$Artifact"
  Say "Installing ololo v$Version (windows/x86_64)"
} else {
  $Url = "https://github.com/$Repo/releases/latest/download/$Artifact"
  Say "Installing ololo latest release (windows/x86_64)"
}

# --- Pick install dir --------------------------------------------------------
$InstallDir = if ($env:OLOLO_INSTALL_DIR) { $env:OLOLO_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Programs\ololo" }
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

# --- Download & install ------------------------------------------------------
$Tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("ololo-install-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $Tmp | Out-Null
try {
  $Zip = Join-Path $Tmp $Artifact
  Say "Downloading $Url"
  Invoke-WebRequest -Uri $Url -OutFile $Zip -UseBasicParsing

  Expand-Archive -Path $Zip -DestinationPath $Tmp -Force
  $Exe = Join-Path $Tmp "ololo.exe"
  if (-not (Test-Path $Exe)) { throw "archive did not contain ololo.exe" }

  Copy-Item -Path $Exe -Destination (Join-Path $InstallDir "ololo.exe") -Force
  Say "Installed to $InstallDir\ololo.exe"
} finally {
  Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}

# --- Ensure install dir is on the user PATH ----------------------------------
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (($UserPath -split ";") -notcontains $InstallDir) {
  [Environment]::SetEnvironmentVariable("Path", "$UserPath;$InstallDir", "User")
  $env:Path = "$env:Path;$InstallDir"
  Say "Added $InstallDir to your user PATH (restart other terminals to pick it up)"
}

Say "Done. Run 'ololo login' to get started."
