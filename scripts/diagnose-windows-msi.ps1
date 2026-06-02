[CmdletBinding()]
param(
  [string]$MsiPath = (Join-Path $env:USERPROFILE "Downloads\gwt-windows-x86_64.msi"),
  [string]$ExpectedSha256 = "",
  [string]$OutputDir = "",
  [switch]$SkipInstall,
  [switch]$CheckHeadless
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function New-DefaultOutputDir {
  $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
  return Join-Path $env:TEMP "gwt-msi-diagnostics-$stamp"
}

if ([string]::IsNullOrWhiteSpace($OutputDir)) {
  $OutputDir = New-DefaultOutputDir
}

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

$SummaryPath = Join-Path $OutputDir "summary.txt"
$TranscriptPath = Join-Path $OutputDir "transcript.txt"
$InstallerLogPath = Join-Path $OutputDir "msiexec-install.log"
$InstallerMatchesPath = Join-Path $OutputDir "msiexec-interesting-lines.txt"

Set-Content -Path $SummaryPath -Value "GWT Windows MSI diagnostics" -Encoding UTF8

function Add-Summary {
  param([string]$Message)

  $Message | Tee-Object -FilePath $SummaryPath -Append
}

function Add-Section {
  param([string]$Title)

  Add-Summary ""
  Add-Summary "== $Title =="
}

function Save-Object {
  param(
    [Parameter(Mandatory = $true)]$InputObject,
    [Parameter(Mandatory = $true)][string]$Path
  )

  $InputObject | Format-List * | Out-File -FilePath $Path -Encoding UTF8
}

function Capture-InstalledLayout {
  param([string]$InstallRoot)

  Add-Section "Installed layout"
  Add-Summary "InstallRoot=$InstallRoot"

  if (-not (Test-Path -LiteralPath $InstallRoot)) {
    Add-Summary "Install root not found."
    return
  }

  $layoutPath = Join-Path $OutputDir "installed-layout.txt"
  Get-ChildItem -LiteralPath $InstallRoot -Force |
    Select-Object Mode, LastWriteTime, Length, Name |
    Format-Table -AutoSize |
    Out-String |
    Tee-Object -FilePath $layoutPath |
    Write-Host

  Add-Summary "Installed layout written to $layoutPath"
}

function Capture-GwtVersion {
  param([string]$GwtExe)

  Add-Section "gwt.exe version"
  Add-Summary "GwtExe=$GwtExe"

  if (-not (Test-Path -LiteralPath $GwtExe)) {
    Add-Summary "gwt.exe not found."
    return
  }

  $versionPath = Join-Path $OutputDir "gwt-version.txt"
  try {
    & $GwtExe --version 2>&1 |
      Tee-Object -FilePath $versionPath |
      Write-Host
    Add-Summary "gwt.exe --version output written to $versionPath"
  } catch {
    Add-Summary "gwt.exe --version failed: $($_.Exception.Message)"
  }
}

function Capture-HeadlessLaunch {
  param([string]$GwtExe)

  if (-not $CheckHeadless) {
    return
  }

  Add-Section "gwt browser server smoke"
  Add-Summary "GwtExe=$GwtExe"

  if (-not (Test-Path -LiteralPath $GwtExe)) {
    Add-Summary "gwt.exe not found; skipping headless smoke."
    return
  }

  $stdoutPath = Join-Path $OutputDir "gwt-server-stdout.txt"
  $stderrPath = Join-Path $OutputDir "gwt-server-stderr.txt"
  $args = "--no-open --port 0"
  Add-Summary "Starting: $GwtExe $args"

  $process = Start-Process -FilePath $GwtExe `
    -ArgumentList $args `
    -RedirectStandardOutput $stdoutPath `
    -RedirectStandardError $stderrPath `
    -PassThru

  Start-Sleep -Seconds 5

  if ($process.HasExited) {
    Add-Summary "gwt browser server exited early with code $($process.ExitCode)."
  } else {
    Add-Summary "gwt browser server stayed alive for 5 seconds; stopping diagnostic process."
    Stop-Process -Id $process.Id -Force
  }

  Add-Summary "stdout=$stdoutPath"
  Add-Summary "stderr=$stderrPath"
}

Start-Transcript -Path $TranscriptPath -Force | Out-Null

try {
  Add-Section "Environment"
  Add-Summary "OutputDir=$OutputDir"
  Add-Summary "ComputerName=$env:COMPUTERNAME"
  Add-Summary "UserName=$env:USERNAME"
  Add-Summary "OS=$([System.Environment]::OSVersion.VersionString)"
  Add-Summary "PowerShell=$($PSVersionTable.PSVersion)"

  Add-Section "MSI file"
  $resolvedMsi = Resolve-Path -LiteralPath $MsiPath
  $msiFullPath = $resolvedMsi.Path
  $msiItem = Get-Item -LiteralPath $msiFullPath
  Add-Summary "MsiPath=$msiFullPath"
  Add-Summary "Length=$($msiItem.Length)"
  Add-Summary "LastWriteTime=$($msiItem.LastWriteTime.ToString('o'))"

  $hash = Get-FileHash -LiteralPath $msiFullPath -Algorithm SHA256
  Save-Object -InputObject $hash -Path (Join-Path $OutputDir "msi-sha256.txt")
  Add-Summary "SHA256=$($hash.Hash.ToLowerInvariant())"
  if (-not [string]::IsNullOrWhiteSpace($ExpectedSha256)) {
    $expected = $ExpectedSha256.ToLowerInvariant()
    if ($hash.Hash.ToLowerInvariant() -eq $expected) {
      Add-Summary "ExpectedSha256=match"
    } else {
      Add-Summary "ExpectedSha256=mismatch expected=$expected"
    }
  }

  Add-Section "Authenticode signature"
  $signature = Get-AuthenticodeSignature -LiteralPath $msiFullPath
  Save-Object -InputObject $signature -Path (Join-Path $OutputDir "authenticode-signature.txt")
  Add-Summary "SignatureStatus=$($signature.Status)"
  Add-Summary "SignatureStatusMessage=$($signature.StatusMessage)"
  if ($null -ne $signature.SignerCertificate) {
    Add-Summary "SignerSubject=$($signature.SignerCertificate.Subject)"
    Add-Summary "SignerThumbprint=$($signature.SignerCertificate.Thumbprint)"
  } else {
    Add-Summary "SignerCertificate=<none>"
  }

  Add-Section "Zone.Identifier"
  $zonePath = Join-Path $OutputDir "zone-identifier.txt"
  try {
    Get-Content -LiteralPath $msiFullPath -Stream Zone.Identifier -ErrorAction Stop |
      Tee-Object -FilePath $zonePath |
      Write-Host
    Add-Summary "Zone.Identifier written to $zonePath"
  } catch {
    Add-Summary "Zone.Identifier not found or unreadable: $($_.Exception.Message)"
  }

  Add-Section "Windows Installer"
  if ($SkipInstall) {
    Add-Summary "SkipInstall=true; msiexec was not run."
  } else {
    $msiexecArgs = "/i `"$msiFullPath`" /L*V `"$InstallerLogPath`""
    Add-Summary "Starting: msiexec.exe $msiexecArgs"
    $installer = Start-Process -FilePath "msiexec.exe" `
      -ArgumentList $msiexecArgs `
      -Wait `
      -PassThru
    Add-Summary "msiexec exit code=$($installer.ExitCode)"
    Add-Summary "msiexec log=$InstallerLogPath"

    if (Test-Path -LiteralPath $InstallerLogPath) {
      Select-String -Path $InstallerLogPath `
        -Pattern "Return value 3", "Error ", "LaunchCondition", "GWT_LEGACY", "MainEngineThread", "Product: GWT" `
        -Context 2, 2 |
        Out-File -FilePath $InstallerMatchesPath -Encoding UTF8
      Add-Summary "Interesting msiexec lines=$InstallerMatchesPath"
    } else {
      Add-Summary "msiexec log was not created."
    }
  }

  $installRoot = Join-Path $env:LOCALAPPDATA "Programs\GWT"
  $gwtExe = Join-Path $installRoot "gwt.exe"
  Capture-InstalledLayout -InstallRoot $installRoot
  Capture-GwtVersion -GwtExe $gwtExe
  Capture-HeadlessLaunch -GwtExe $gwtExe

  Add-Section "Result"
  Add-Summary "Diagnostics complete."
  Add-Summary "Attach this directory when reporting the issue: $OutputDir"
} finally {
  Stop-Transcript | Out-Null
  Write-Host ""
  Write-Host "Diagnostics written to $OutputDir"
}
