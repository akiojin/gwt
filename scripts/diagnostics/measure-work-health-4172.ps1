# Local diagnostic driver for Issue #4172 AC-4.
#
# Builds the measurement program against this checkout's normal (not
# test-support) gwt rlib, prepares an isolated fixture HOME, and runs the
# per_row / shared_snapshot comparison against an empty, a half-size, and a
# full copy of the host error ledger. It never writes to the original ledger,
# the real HOME, or any real worktree.
#
# Usage: powershell -NoProfile -File scripts/diagnostics/measure-work-health-4172.ps1 -OutDir <dir>
param(
    [Parameter(Mandatory = $true)][string]$OutDir,
    [string]$Counts = '1,32,254',
    [int]$Repeats = 3
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$rlib = Join-Path $root 'target/debug/libgwt.rlib'
if (-not (Test-Path $rlib)) {
    throw "missing normal gwt rlib: $rlib (run: cargo build -p gwt --lib)"
}
$nativeLib = Get-ChildItem -Directory -Path (Join-Path $HOME '.cargo/registry/src/*/windows_x86_64_msvc-0.53.1/lib') |
    Select-Object -First 1
if (-not $nativeLib) { throw 'missing windows_x86_64_msvc native library search path' }

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path
$exe = Join-Path $OutDir 'measure-work-health-4172.exe'
& rustc --edition=2021 (Join-Path $root 'scripts/diagnostics/measure-work-health-4172.rs') `
    --extern "gwt=$rlib" `
    -L "dependency=$(Join-Path $root 'target/debug/deps')" `
    -L "native=$($nativeLib.FullName)" `
    -o $exe
if ($LASTEXITCODE -ne 0) { throw "rustc failed with $LASTEXITCODE" }

# Isolated fixture home: matching HOME / USERPROFILE, marker, empty worktree.
$fixtureHome = Join-Path $OutDir 'fixture-home'
if (Test-Path $fixtureHome) { Remove-Item -Recurse -Force $fixtureHome }
$worktree = Join-Path $fixtureHome 'worktree'
New-Item -ItemType Directory -Force -Path $worktree | Out-Null
Set-Content -Path (Join-Path $fixtureHome '.issue-4172-measurement-fixture') -Value 'issue-4172' -NoNewline
$ledgerDir = Join-Path $fixtureHome '.gwt/logs/errors'

$sourceLedger = Join-Path $HOME '.gwt/logs/errors'
$sourceFiles = @()
if (Test-Path $sourceLedger) {
    $sourceFiles = @(Get-ChildItem -File -Path $sourceLedger -Filter 'errors.*.jsonl')
}

function Set-Fixture-Ledger([string]$Condition) {
    if (Test-Path $ledgerDir) { Remove-Item -Recurse -Force $ledgerDir }
    if ($Condition -eq 'empty') { return }
    New-Item -ItemType Directory -Force -Path $ledgerDir | Out-Null
    foreach ($file in $sourceFiles) {
        $lines = [System.IO.File]::ReadAllLines($file.FullName)
        if ($Condition -eq 'half') {
            # Every other line from the head, so the sample keeps whole records.
            $lines = @(for ($i = 0; $i -lt $lines.Length; $i += 2) { $lines[$i] })
        }
        [System.IO.File]::WriteAllLines((Join-Path $ledgerDir $file.Name), $lines)
    }
}

foreach ($condition in @('empty', 'half', 'full')) {
    Set-Fixture-Ledger $condition
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    # Windows PowerShell 5.1 has no ProcessStartInfo.ArgumentList / .Environment.
    $psi.Arguments = ('"{0}" "{1}" "{2}"' -f $worktree, $Counts, $Repeats)
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    # Ambient gwt / git environment must not leak into the measurement.
    foreach ($name in @($psi.EnvironmentVariables.Keys)) {
        if ($name -like 'GWT_*' -or $name -like 'GIT_*') {
            $psi.EnvironmentVariables.Remove($name) | Out-Null
        }
    }
    $psi.EnvironmentVariables['HOME'] = $fixtureHome
    $psi.EnvironmentVariables['USERPROFILE'] = $fixtureHome
    $process = [System.Diagnostics.Process]::Start($psi)
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    if ($process.ExitCode -ne 0) { throw "measurement ($condition) failed: $stderr" }
    Set-Content -Path (Join-Path $OutDir "health-4172-$condition.jsonl") -Value $stdout -NoNewline
    Write-Output "== $condition =="
    Write-Output $stdout.Trim()
}
