# Issue #5510: actual eight-PTY Windows update handoff, using this checkout only.
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArgs)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This fixture requires Windows and PowerShell 7.' }
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$checkoutGwt = Join-Path $repoRoot 'target/debug/gwt.exe'
$checkoutGwtd = Join-Path $repoRoot 'target/debug/gwtd.exe'
foreach ($binary in @($checkoutGwt, $checkoutGwtd)) {
  if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
    throw 'Build this checkout first: cargo build -p gwt --bin gwt --bin gwtd'
  }
}
# Keep all evidence outside verify.run's command-private TEMP, which is removed.
$evidence = Join-Path $repoRoot ('target/issue-5510-' + [guid]::NewGuid())
$checkHome = Join-Path $evidence 'home'
foreach ($directory in @($checkHome, "$checkHome/.gwt", "$checkHome/.codex", "$checkHome/tmp")) {
  [void](New-Item -ItemType Directory -Path $directory -Force)
}
[IO.File]::WriteAllText("$checkHome/.codex/config.toml", '')
$runtime = Join-Path $env:USERPROFILE '.gwt/runtime'
if (Test-Path -LiteralPath $runtime -PathType Container) {
  [void](New-Item -ItemType Junction -Path "$checkHome/.gwt/runtime" -Target $runtime)
}
$localHookPattern = '(?i)(^|[/\\])target[/\\](?:[^/\\]+[/\\])*(debug|release)[/\\]gwtd(\.exe)?$'
$hookBin = $env:GWT_HOOK_BIN
if (-not $hookBin -or $hookBin -match $localHookPattern -or
    ($hookBin -ne 'gwtd' -and -not (Test-Path -LiteralPath $hookBin -PathType Leaf))) {
  $previousBin = $env:GWT_BIN_PATH
  $previousHook = $env:GWT_HOOK_BIN
  try {
    $env:GWT_BIN_PATH = $null
    $env:GWT_HOOK_BIN = $null
    $doctor = '{"schema_version":1,"operation":"hook.doctor","params":{}}' | & $checkoutGwtd | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0 -or -not $doctor.ok) { throw 'Cannot resolve managed hook fallback.' }
    $hookBin = ($doctor.output | ConvertFrom-Json).managed_hook_bin
  } finally { $env:GWT_BIN_PATH = $previousBin; $env:GWT_HOOK_BIN = $previousHook }
}
if (-not $hookBin -or $hookBin -match $localHookPattern) { throw 'Expected a stable managed hook fallback.' }
$childEnvironment = @{}
foreach ($entry in [Environment]::GetEnvironmentVariables().GetEnumerator()) {
  if ($entry.Key -notmatch '^(GWT_|CODEX_|CLAUDE_|GH_TOKEN$|GITHUB_TOKEN$|OPENAI_API_KEY$|ANTHROPIC_API_KEY$)') {
    $childEnvironment[$entry.Key] = [string]$entry.Value
  }
}
$childEnvironment.HOME = $checkHome
$childEnvironment.USERPROFILE = $checkHome
$childEnvironment.CODEX_HOME = "$checkHome/.codex"
$childEnvironment.TEMP = "$checkHome/tmp"
$childEnvironment.TMP = "$checkHome/tmp"
$childEnvironment.GIT_TERMINAL_PROMPT = '0'
$childEnvironment.GH_PROMPT_DISABLED = '1'
$childEnvironment.GWT_PROJECT_ROOT = $repoRoot
$childEnvironment.GWT_HOOK_BIN = $hookBin
$childEnvironment.GWT_DISABLE_BACKGROUND_INDEX = '1'
# Existing CI mode suppresses unrelated release discovery. This fixture owns a
# same-version staged payload and exercises explicit HTTP apply/helper restart;
# a newly published GitHub release must not supersede that controlled payload.
$childEnvironment.CI = 'true'
$urlFile = Join-Path $checkHome 'url.txt'
$childEnvironment.GWT_BROWSER_URL_FILE = $urlFile
$providerLog = Join-Path $checkHome 'shell-processes.jsonl'
[IO.File]::WriteAllText($providerLog, '')
$childEnvironment.GWT_UPDATE_HANDOFF_RECORDER = $providerLog
# Each real shell runs a local Node process; no provider or network is involved.
[IO.File]::WriteAllText((Join-Path $checkHome 'shell-load.cjs'), @'
const fs = require('node:fs');
fs.appendFileSync(process.env.GWT_UPDATE_HANDOFF_RECORDER, JSON.stringify({pid:process.pid,
  started_at:Date.now()-process.uptime()*1000})+'\n');
let n = 0;
setInterval(() => console.log('GWT_HANDOFF_PTY_'+(++n)), 200);
'@)

function New-FixtureStartInfo([string]$FileName) {
  $info = [Diagnostics.ProcessStartInfo]::new($FileName)
  $info.WorkingDirectory = $repoRoot
  $info.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
  $info.UseShellExecute = $false
  $info.CreateNoWindow = $true
  $info.Environment.Clear()
  foreach ($key in $childEnvironment.Keys) { $info.Environment[$key] = $childEnvironment[$key] }
  return $info
}
function Invoke-FixtureGwtd([string]$Operation, [hashtable]$Parameters) {
  $info = New-FixtureStartInfo $checkoutGwtd
  $info.RedirectStandardInput = $true
  $info.RedirectStandardOutput = $true
  $info.RedirectStandardError = $true
  $info.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
  $process = [Diagnostics.Process]::Start($info)
  try {
    $process.StandardInput.WriteLine((@{schema_version=1;operation=$Operation;params=$Parameters} | ConvertTo-Json -Depth 10 -Compress))
    $process.StandardInput.Close()
    $output = $process.StandardOutput.ReadToEndAsync()
    $errors = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(60000)) { $process.Kill($true); throw "$Operation timed out." }
    $result = $output.GetAwaiter().GetResult() | ConvertFrom-Json
    if ($process.ExitCode -ne 0 -or -not $result.ok) { throw "$Operation failed: $($errors.GetAwaiter().GetResult())" }
    return $result
  } finally { $process.Dispose() }
}
function Assert-HookHealth($health) {
  $publicGwtd = Get-Command gwtd -CommandType Application -ErrorAction SilentlyContinue
  $blocking = @($health.issues | Where-Object {
    $_ -notmatch '^managed hook failure: .* state=fail-open( |$)' -and
    -not ($hookBin -eq 'gwtd' -and -not $publicGwtd -and $_ -match '^managed hook binary missing: .* uses gwtd$')
  })
  if ($health.status -eq 'inactive' -or $blocking.Count) { throw "Hook convergence failed: $($blocking -join '; ')" }
}
function Wait-FixtureReady($Process) {
  $deadline = [DateTime]::UtcNow.AddSeconds(90)
  do {
    if ($Process.HasExited) { throw 'Fresh gwt exited during startup.' }
    if (Test-Path -LiteralPath $urlFile -PathType Leaf) {
      $candidate = [IO.File]::ReadAllText($urlFile).Trim()
      if ($candidate) {
        try { if ((Invoke-WebRequest -Uri $candidate -Method Head -TimeoutSec 2).StatusCode -eq 200) { return $candidate } } catch { }
      }
    }
    Start-Sleep -Milliseconds 200
  } while ([DateTime]::UtcNow -lt $deadline)
  throw 'Fresh gwt readiness timed out.'
}
function Read-FixtureLock([string]$Path) {
  # The resident keeps its payload open for writes. .NET's default ReadAllText
  # sharing would reject that live handle on Windows.
  $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read,
    ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
  $reader = [IO.StreamReader]::new($stream)
  try { return ($reader.ReadToEnd() | ConvertFrom-Json) } finally { $reader.Dispose() }
}
$status = Invoke-FixtureGwtd 'issue.monitor.status' @{}
$repoHash = $status.project_store.hash
if ($repoHash -notmatch '^[a-zA-Z0-9_-]+$') { throw 'Missing canonical project_store.hash.' }
$preferences = "$checkHome/.gwt/projects/$repoHash/project-state"
[void](New-Item -ItemType Directory -Path $preferences -Force)
[IO.File]::WriteAllText("$preferences/pm.json", '{"settings":{"auto_start":false}}')
[IO.File]::WriteAllText("$preferences/issue-monitor.json", '{"enabled":false,"max_active_agents":1,"priority_order":[]}')
[IO.File]::WriteAllText("$checkHome/.gwt/session.json", (@{tabs=@(
  @{id='handoff';title='Update handoff';project_root=$repoRoot;kind='git'}
);active_tab_id='handoff';recent_projects=@()} | ConvertTo-Json -Depth 10))
$hookParameters = @{expected_hook_bin=$hookBin;runtime_state_path="$checkHome/.gwt/browser-check-missing-runtime-state.json"}
[void](Invoke-FixtureGwtd 'hook.doctor' ($hookParameters + @{repair=$true}))
[void](Invoke-FixtureGwtd 'hook.register_codex_managed_hook_trust' @{
  codex_config="$checkHome/.codex/config.toml";codex_hook_discovery='both';project_root=$repoRoot
})
Assert-HookHealth ((Invoke-FixtureGwtd 'hook.health' $hookParameters).output | ConvertFrom-Json)

$ownedProcess = $null
$residentExecutable = $checkoutGwt
$anchorSocket = $null
$savedRunnerEnvironment = @{}
$fixtureStarted = [DateTime]::UtcNow
$exitCode = 1
try {
  $info = New-FixtureStartInfo $checkoutGwt
  $info.ArgumentList.Add('--no-tray')
  $info.ArgumentList.Add('--no-open')
  $info.RedirectStandardOutput = $true
  $info.RedirectStandardError = $true
  $ownedProcess = [Diagnostics.Process]::Start($info)
  $stdoutTask = $ownedProcess.StandardOutput.ReadToEndAsync()
  $stderrTask = $ownedProcess.StandardError.ReadToEndAsync()
  $url = Wait-FixtureReady $ownedProcess
  Assert-HookHealth ((Invoke-FixtureGwtd 'hook.health' $hookParameters).output | ConvertFrom-Json)
  $smokePid = $ownedProcess.Id
  $ownedProcess.Kill($true)
  if (-not $ownedProcess.WaitForExit(10000)) { throw 'Owned no-tray smoke process remained.' }
  [IO.File]::WriteAllText("$evidence/no-tray-startup.log", $stdoutTask.GetAwaiter().GetResult())
  [IO.File]::WriteAllText("$evidence/no-tray-startup-error.log", $stderrTask.GetAwaiter().GetResult())
  $ownedProcess.Dispose()
  $ownedProcess = $null
  Remove-Item -LiteralPath $urlFile
  # browser-check smoke above follows --no-tray --no-open. AC-4 exercises the
  # resident GUI's update handoff, so launch that mode explicitly in the same
  # fresh HOME. Keep production restart arguments and helper files unmodified.
  # A canonical verify.run parent executes checkout gwtd.exe throughout both
  # themes. Windows permits renaming that live image to .old but denies its
  # deletion on the next update. Exercise a separate install of byte-identical
  # checkout binaries so the verifier itself is outside the update artifact.
  $installRoot = Join-Path $checkHome 'install'
  [void](New-Item -ItemType Directory -Path $installRoot)
  $installHashes = @{}
  foreach ($sourceBinary in @($checkoutGwt, $checkoutGwtd)) {
    $installedBinary = Join-Path $installRoot ([IO.Path]::GetFileName($sourceBinary))
    Copy-Item -LiteralPath $sourceBinary -Destination $installedBinary
    $sourceHash = (Get-FileHash -LiteralPath $sourceBinary -Algorithm SHA256).Hash.ToLowerInvariant()
    if ((Get-FileHash -LiteralPath $installedBinary -Algorithm SHA256).Hash.ToLowerInvariant() -ne $sourceHash) {
      throw 'Private install is not byte-identical to this checkout.'
    }
    $installHashes[[IO.Path]::GetFileName($sourceBinary)] = $sourceHash
  }
  $residentExecutable = Join-Path $installRoot 'gwt.exe'
  $info.FileName = $residentExecutable
  $info.ArgumentList.Clear()
  $info.ArgumentList.Add('--no-open')
  $ownedProcess = [Diagnostics.Process]::Start($info)
  $stdoutTask = $ownedProcess.StandardOutput.ReadToEndAsync()
  $stderrTask = $ownedProcess.StandardError.ReadToEndAsync()
  $url = Wait-FixtureReady $ownedProcess
  $anchorSocket = [Net.WebSockets.ClientWebSocket]::new()
  $anchorUrl = [UriBuilder]::new($url)
  $anchorUrl.Scheme = 'ws'
  $anchorUrl.Path = '/ws'
  $anchorSocket.ConnectAsync($anchorUrl.Uri, [Threading.CancellationToken]::None).GetAwaiter().GetResult() | Out-Null
  Assert-HookHealth ((Invoke-FixtureGwtd 'hook.health' $hookParameters).output | ConvertFrom-Json)
  [IO.File]::WriteAllText("$checkHome/fixture.json", (@{home=$checkHome;checkout=$repoRoot;pid=$ownedProcess.Id;smoke_pid=$smokePid;
    url=$url;preferences=$preferences;gwt=$residentExecutable;gwtd=(Join-Path $installRoot 'gwtd.exe');
    source_gwt=$checkoutGwt;source_gwtd=$checkoutGwtd;source_sha256=$installHashes;install_root=$installRoot;
    release_discovery='Disabled by existing CI mode; explicit HTTP apply and helper remain real.';
    artifact_boundary='Byte-identical private install; the live canonical verifier is outside the update target.'} | ConvertTo-Json -Depth 5))

  $version = [IO.File]::ReadAllText((Join-Path $repoRoot 'scripts/playwright-version.txt')).Trim()
  $deps = if ($env:GWT_PLAYWRIGHT_DEPS_DIR) { $env:GWT_PLAYWRIGHT_DEPS_DIR } else { Join-Path $env:LOCALAPPDATA "Temp/gwt-playwright-$version" }
  $cli = Join-Path $deps 'node_modules/playwright/cli.js'
  $resolver = Join-Path $deps 'resolve-playwright.cjs'
  if (-not (Test-Path -LiteralPath $cli) -or -not (Test-Path -LiteralPath $resolver)) { throw "Pinned Playwright $version is missing." }
  $runnerEnvironment = @{
    GWT_PLAYWRIGHT_BASE_URL=$url;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;GWT_PLAYWRIGHT_PROJECT_ROOT=$repoRoot;
    GWT_PLAYWRIGHT_UPDATE_HANDOFF='1';GWT_PLAYWRIGHT_NODE_MODULES=(Join-Path $deps 'node_modules');
    PLAYWRIGHT_JSON_OUTPUT_NAME=(Join-Path $evidence 'playwright-results.json');
    NODE_OPTIONS="--require `"$($resolver.Replace('\', '/'))`" $env:NODE_OPTIONS"
  }
  foreach ($key in $runnerEnvironment.Keys) {
    $savedRunnerEnvironment[$key] = [Environment]::GetEnvironmentVariable($key)
    [Environment]::SetEnvironmentVariable($key, $runnerEnvironment[$key])
  }
  Push-Location $repoRoot
  try {
    & node $cli test --config crates/gwt/playwright/playwright.config.ts --workers=1 --headed --reporter=list,json --output "$evidence/playwright" crates/gwt/playwright/tests/update-handoff-live.spec.ts @PlaywrightArgs
    $exitCode = $LASTEXITCODE
  } finally { Pop-Location }
} finally {
  foreach ($key in $savedRunnerEnvironment.Keys) { [Environment]::SetEnvironmentVariable($key, $savedRunnerEnvironment[$key]) }
  if ($anchorSocket) { $anchorSocket.Abort(); $anchorSocket.Dispose() }
  if ($ownedProcess) {
    if (-not $ownedProcess.HasExited) { $ownedProcess.Kill($true) }
    if (-not $ownedProcess.WaitForExit(10000)) { throw 'Owned initial GUI remained after cleanup.' }
  }
  # On a failed assertion an update helper can still be waiting to restart.
  # Stop only helpers executing from this fixture's private updates directory,
  # including their children, before discovering the final resident lock.
  $privateUpdates = [IO.Path]::GetFullPath("$checkHome/.gwt/updates") + [IO.Path]::DirectorySeparatorChar
  foreach ($candidate in @(Get-CimInstance Win32_Process -Filter "Name = 'gwt-restart-helper.exe'")) {
    if (-not $candidate.ExecutablePath -or -not $candidate.ExecutablePath.StartsWith($privateUpdates, [StringComparison]::OrdinalIgnoreCase)) { continue }
    $helper = Get-Process -Id $candidate.ProcessId -ErrorAction SilentlyContinue
    if ($helper -and $helper.Path -eq $candidate.ExecutablePath -and
        $helper.StartTime.ToUniversalTime() -ge $fixtureStarted -and
        [Math]::Abs(($helper.StartTime.ToUniversalTime() - $candidate.CreationDate.ToUniversalTime()).TotalSeconds) -lt 1) {
      $helper.Kill($true)
      if (-not $helper.WaitForExit(10000)) { throw "Owned update helper PID $($helper.Id) remained." }
    }
  }
  # The restarted resident is not a child of this script. Its isolated lock,
  # executable path and creation time jointly establish exact PID ownership.
  foreach ($lockFile in @(Get-ChildItem -LiteralPath "$checkHome/.gwt/run" -Filter 'tray-*.lock' -ErrorAction SilentlyContinue)) {
    $receipt = Read-FixtureLock $lockFile.FullName
    $resident = Get-Process -Id $receipt.pid -ErrorAction SilentlyContinue
    if ($resident) {
      $created = $resident.StartTime.ToUniversalTime()
      $lockTime = ([DateTime]$receipt.started_at).ToUniversalTime()
      if ($resident.Path -ne $residentExecutable -or $created -lt $fixtureStarted -or $created -gt $lockTime -or ($lockTime - $created).TotalSeconds -gt 120) {
        throw 'Resident ownership receipt did not match; refusing cleanup.'
      }
      $resident.Kill($true)
      if (-not $resident.WaitForExit(10000)) { throw "Owned resident PID $($receipt.pid) remained." }
    }
  }
  foreach ($line in [IO.File]::ReadAllLines($providerLog)) {
    $receipt = $line | ConvertFrom-Json
    $provider = Get-Process -Id $receipt.pid -ErrorAction SilentlyContinue
    if ($provider) {
      $created = ([DateTimeOffset]$provider.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
      if ([Math]::Abs($created - $receipt.started_at) -lt 1000) {
        $provider.Kill($true)
        if (-not $provider.WaitForExit(10000)) { throw "Owned PTY process PID $($receipt.pid) remained." }
      }
    }
  }
  if ($ownedProcess) {
    # A helper/restarted GUI may inherit the initial resident's output pipes.
    # Drain them only after every owned process has stopped.
    [IO.File]::WriteAllText("$evidence/startup.log", $stdoutTask.GetAwaiter().GetResult())
    [IO.File]::WriteAllText("$evidence/startup-error.log", $stderrTask.GetAwaiter().GetResult())
    $ownedProcess.Dispose()
  }
  Write-Output "Isolated evidence: $evidence; owned GUI and PTY cleanup finished."
}
if ($exitCode -ne 0) { throw "Issue #5510 headed verification failed ($exitCode)." }
