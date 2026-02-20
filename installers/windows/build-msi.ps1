param(
  [string]$Version = "",
  [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

function Info([string]$Message) {
  Write-Host "[info] $Message" -ForegroundColor Cyan
}

function Ok([string]$Message) {
  Write-Host "[ok] $Message" -ForegroundColor Green
}

function Fail([string]$Message) {
  Write-Host "[error] $Message" -ForegroundColor Red
  exit 1
}

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Resolve-Path (Join-Path $ScriptDir "..\..")
Set-Location $RepoRoot

if ([string]::IsNullOrWhiteSpace($Version)) {
  $cargoToml = Join-Path $RepoRoot "Cargo.toml"
  $versionLine = Select-String -Path $cargoToml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
  if (-not $versionLine) {
    Fail "Failed to read version from Cargo.toml"
  }
  $Version = $versionLine.Matches[0].Groups[1].Value
}

Info "Version: $Version"

if (-not $SkipBuild) {
  Info "Building app with Tauri..."
  cargo tauri build
}

$msiDir = Join-Path $RepoRoot "target\release\bundle\msi"
if (-not (Test-Path $msiDir)) {
  Fail "MSI output directory not found: $msiDir"
}

$expectedToken = "_$Version`_"
$matched = Get-ChildItem -Path $msiDir -Filter "*.msi" |
  Where-Object { $_.Name -like "*$expectedToken*" } |
  Sort-Object LastWriteTime -Descending |
  Select-Object -First 1

if (-not $matched) {
  $available = Get-ChildItem -Path $msiDir -Filter "*.msi" | Select-Object -ExpandProperty Name
  if (-not $available) {
    Fail "No MSI files found under $msiDir"
  }
  Fail "No MSI matched version $Version in $msiDir. Found: $($available -join ', ')"
}

$stableDir = Join-Path $RepoRoot "target\release\bundle\windows"
New-Item -ItemType Directory -Path $stableDir -Force | Out-Null
$stablePath = Join-Path $stableDir "gwt-windows-x86_64.msi"
Copy-Item -Path $matched.FullName -Destination $stablePath -Force

$size = (Get-Item $stablePath).Length
Ok "Using MSI: $($matched.FullName)"
Ok "Copied MSI: $stablePath ($size bytes)"
