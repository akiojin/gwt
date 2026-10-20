# Issue #5351: Windows checkout/backend/PTY/Chromium verification. Build gwt/gwtd first.
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArguments)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This fixture requires Windows and PowerShell 7.' }
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$checkoutGwt = Join-Path $repoRoot 'target/debug/gwt.exe'
$checkoutGwtd = Join-Path $repoRoot 'target/debug/gwtd.exe'
foreach ($binary in @($checkoutGwt, $checkoutGwtd)) {
  if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
    throw 'Build the checkout first: cargo build -p gwt --bin gwt --bin gwtd'
  }
}
$checkHome = Join-Path ([IO.Path]::GetTempPath()) ('gwt-issue-5351-' + [guid]::NewGuid())
$project = Join-Path $checkHome 'project'
$fixtureBin = Join-Path $checkHome 'bin'
$providerLog = Join-Path $checkHome 'provider.jsonl'
$urlFile = Join-Path $checkHome 'url.txt'
$stdoutLog = Join-Path $checkHome 'startup.log'
$stderrLog = Join-Path $checkHome 'startup-error.log'
foreach ($directory in @($project, "$project/.codex", $fixtureBin, "$checkHome/.gwt", "$checkHome/.codex", "$checkHome/tmp")) {
  [void](New-Item -ItemType Directory -Path $directory)
}
[IO.File]::WriteAllText("$checkHome/.codex/config.toml", '')
[IO.File]::WriteAllText($providerLog, '')
Push-Location $project
try {
  & git init --quiet -b main
  if ($LASTEXITCODE -ne 0) { throw 'Fixture git init failed.' }
  & git -c user.name=gwt -c user.email=gwt@example.invalid commit --quiet --allow-empty -m 'drain fixture'
  if ($LASTEXITCODE -ne 0) { throw 'Fixture git commit failed.' }
} finally { Pop-Location }

# The provider records its own launch and stays alive in the PTY; it never calls a model.
$providerSource = @'
const fs = require('node:fs');
const args = process.argv.slice(2);
if (args.includes('--version') || args.includes('-V')) {
  console.log('codex-cli 0.116.0'); process.exit(0);
}
const identity = {pid:process.pid, started_at:Date.now()-process.uptime()*1000,
  cwd:process.cwd(), session:process.env.GWT_SESSION_ID || null};
const record = event => fs.appendFileSync(process.env.GWT_DRAIN_RECORDER,
  JSON.stringify({...identity,...event})+'\n');
record({kind:'launch',argv:args});
console.log('GWT_DRAIN_PROVIDER_READY');
if (process.stdin.isTTY) process.stdin.setRawMode(true);
process.stdin.on('data', () => {});
process.stdin.resume();
'@
[IO.File]::WriteAllText((Join-Path $fixtureBin 'provider.cjs'), $providerSource)
$node = (Get-Command node -CommandType Application).Source
[IO.File]::WriteAllText((Join-Path $fixtureBin 'codex.cmd'), "@echo off`r`n`"$node`" `"%~dp0provider.cjs`" %*`r`n")
foreach ($runner in @('bunx', 'npx', 'npm')) {
  [IO.File]::WriteAllText((Join-Path $fixtureBin "$runner.cmd"), "@echo off`r`necho Fixture requires installed Codex 1>&2`r`nexit /b 64`r`n")
}
$runtime = Join-Path $env:USERPROFILE '.gwt/runtime'
if (Test-Path -LiteralPath $runtime -PathType Container) {
  [void](New-Item -ItemType Junction -Path "$checkHome/.gwt/runtime" -Target $runtime)
}

# Keep a stable/public hook fallback separate from the checkout daemon.
$localHookPattern = '(?i)(^|[/\\])target[/\\](?:[^/\\]+[/\\])*(debug|release)[/\\]gwtd(\.exe)?$'
$hookBin = $env:GWT_HOOK_BIN
if ($hookBin -and ($hookBin -match $localHookPattern -or
    ($hookBin -ne 'gwtd' -and -not (Test-Path -LiteralPath $hookBin -PathType Leaf)))) { $hookBin = '' }
$publicGwtd = Get-Command gwtd -CommandType Application -ErrorAction SilentlyContinue
if (-not $hookBin -and $publicGwtd -and $publicGwtd.Source -notmatch $localHookPattern) { $hookBin = 'gwtd' }
if (-not $hookBin) { $hookBin = 'gwtd' }
$childEnvironment = @{}
foreach ($entry in [Environment]::GetEnvironmentVariables().GetEnumerator()) {
  if ($entry.Key -notmatch '^(GWT_|CODEX_|CLAUDE_|GH_TOKEN$|GITHUB_TOKEN$|OPENAI_API_KEY$|ANTHROPIC_API_KEY$)') {
    $childEnvironment[$entry.Key] = [string]$entry.Value
  }
}
$childEnvironment.HOME = $checkHome
$childEnvironment.USERPROFILE = $checkHome
$childEnvironment.CODEX_HOME = "$checkHome/.codex"
$childEnvironment.PATH = "$fixtureBin;$env:PATH"
$childEnvironment.TEMP = "$checkHome/tmp"
$childEnvironment.TMP = "$checkHome/tmp"
$childEnvironment.GIT_TERMINAL_PROMPT = '0'
$childEnvironment.GH_PROMPT_DISABLED = '1'
$childEnvironment.GWT_PROJECT_ROOT = $repoRoot
$childEnvironment.GWT_HOOK_BIN = $hookBin
$childEnvironment.GWT_DISABLE_BACKGROUND_INDEX = '1'
$childEnvironment.GWT_DRAIN_RECORDER = $providerLog

function New-FixtureStartInfo([string]$FileName) {
  $startInfo = [Diagnostics.ProcessStartInfo]::new($FileName)
  $startInfo.WorkingDirectory = $repoRoot
  $startInfo.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
  $startInfo.UseShellExecute = $false
  $startInfo.CreateNoWindow = $true
  $startInfo.Environment.Clear()
  foreach ($key in $childEnvironment.Keys) { $startInfo.Environment[$key] = $childEnvironment[$key] }
  return $startInfo
}
function Invoke-FixtureGwtd([string]$Operation, [hashtable]$Parameters, [string]$Root = $repoRoot) {
  $startInfo = New-FixtureStartInfo $checkoutGwtd
  $startInfo.WorkingDirectory = $Root
  $startInfo.Environment['GWT_PROJECT_ROOT'] = $Root
  $startInfo.RedirectStandardInput = $true
  $startInfo.RedirectStandardOutput = $true
  $startInfo.RedirectStandardError = $true
  $startInfo.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
  $startInfo.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
  $startInfo.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
  $process = [Diagnostics.Process]::Start($startInfo)
  try {
    $process.StandardInput.WriteLine((@{schema_version=1; operation=$Operation; params=$Parameters} | ConvertTo-Json -Depth 10 -Compress))
    $process.StandardInput.Close()
    $output = $process.StandardOutput.ReadToEndAsync()
    $errorOutput = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(60000)) { $process.Kill($true); throw "$Operation timed out." }
    $envelope = $output.GetAwaiter().GetResult() | ConvertFrom-Json
    if ($process.ExitCode -ne 0 -or -not $envelope.ok) { throw "$Operation failed: $($envelope | ConvertTo-Json -Depth 10 -Compress); stderr=$($errorOutput.GetAwaiter().GetResult())" }
    return $envelope
  } finally { $process.Dispose() }
}
function Assert-HookHealth($Health) {
  $blocking = @($Health.issues | Where-Object {
    -not ($_ -match '^managed hook failure: .* state=fail-open( |$)' -or
      ($hookBin -eq 'gwtd' -and -not $publicGwtd -and $_ -match '^managed hook binary missing: .* uses gwtd$'))
  })
  if ($Health.status -eq 'inactive' -or $blocking.Count) { throw "Hook convergence failed: $($blocking -join '; ')" }
}
$preferencePaths = @()
foreach ($root in @($repoRoot, $project)) {
  $status = Invoke-FixtureGwtd 'issue.monitor.status' @{} $root
  $repoHash = $status.project_store.hash
  if ($repoHash -notmatch '^[a-zA-Z0-9_-]+$') { throw 'Missing canonical project_store.hash.' }
  $preferences = "$checkHome/.gwt/projects/$repoHash/project-state"
  [void](New-Item -ItemType Directory -Path $preferences -Force)
  [IO.File]::WriteAllText("$preferences/pm.json", '{"settings":{"auto_start":false}}')
  [IO.File]::WriteAllText("$preferences/issue-monitor.json", '{"enabled":false,"max_active_agents":1,"priority_order":[]}')
  $preferencePaths += "$preferences/issue-monitor.json"
}
[IO.File]::WriteAllText("$checkHome/.gwt/session.json", (@{tabs=@(
  @{id='target';title='Target B';project_root=$repoRoot;kind='git'},
  @{id='drain';title='Drain A';project_root=$project;kind='git'}
);active_tab_id='target';recent_projects=@()} | ConvertTo-Json -Depth 10))
$hookParameters = @{expected_hook_bin=$hookBin;runtime_state_path="$checkHome/.gwt/browser-check-missing-runtime-state.json"}
$doctor = Invoke-FixtureGwtd 'hook.doctor' ($hookParameters + @{repair=$true})
[void](Invoke-FixtureGwtd 'hook.register_codex_managed_hook_trust' @{
  codex_config="$checkHome/.codex/config.toml";codex_hook_discovery='both';project_root=$repoRoot
})
$preflightHealth = Invoke-FixtureGwtd 'hook.health' $hookParameters
Assert-HookHealth ($preflightHealth.output | ConvertFrom-Json)

$anchorSocket = $null
$ownedProcess = $null
$savedRunnerEnvironment = @{}
$exitCode = 1
try {
  $startInfo = New-FixtureStartInfo $checkoutGwt
  $startInfo.ArgumentList.Add('--no-tray')
  $startInfo.ArgumentList.Add('--no-open')
  $startInfo.Environment['GWT_BROWSER_URL_FILE'] = $urlFile
  $startInfo.RedirectStandardOutput = $true
  $startInfo.RedirectStandardError = $true
  $startInfo.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
  $startInfo.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
  $ownedProcess = [Diagnostics.Process]::Start($startInfo)
  $ownedPid = $ownedProcess.Id
  Write-Output "Fresh gwt PID: $ownedPid; isolated HOME: $checkHome"
  $stdoutTask = $ownedProcess.StandardOutput.ReadToEndAsync()
  $stderrTask = $ownedProcess.StandardError.ReadToEndAsync()
  $url = ''
  $deadline = [DateTime]::UtcNow.AddSeconds(90)
  do {
    if ($ownedProcess.HasExited) { throw 'Fresh gwt exited during startup.' }
    if (Test-Path -LiteralPath $urlFile -PathType Leaf) {
      $candidate = [IO.File]::ReadAllText($urlFile).Trim()
      if ($candidate) {
        try {
          $response = Invoke-WebRequest -Uri $candidate -Method Head -TimeoutSec 2
          if ($response.StatusCode -eq 200) { $url = $candidate; break }
        } catch { }
      }
    }
    Start-Sleep -Milliseconds 200
  } while ([DateTime]::UtcNow -lt $deadline)
  if (-not $url) { throw 'Fresh gwt readiness timed out.' }
  $anchorSocket = [Net.WebSockets.ClientWebSocket]::new()
  $anchorUrl = [UriBuilder]::new($url)
  $anchorUrl.Scheme = 'ws'
  $anchorUrl.Path = '/ws'
  $anchorSocket.ConnectAsync($anchorUrl.Uri, [Threading.CancellationToken]::None).GetAwaiter().GetResult() | Out-Null
  $health = Invoke-FixtureGwtd 'hook.health' $hookParameters
  Assert-HookHealth ($health.output | ConvertFrom-Json)
  [IO.File]::WriteAllText("$checkHome/fixture.json", (@{home=$checkHome;project=$project;checkout=$repoRoot;pid=$ownedPid;url=$url;provider_log=$providerLog;hook_bin=$hookBin;preferences=$preferencePaths;version=((Get-Content (Join-Path $repoRoot "Cargo.toml") -Raw) -split 'version = "')[1].Split('"')[0]} | ConvertTo-Json))

  $version = [IO.File]::ReadAllText((Join-Path $repoRoot 'scripts/playwright-version.txt')).Trim()
  # Canonical verification gives each command a private TEMP. The pinned cache
  # remains in the user's stable Windows temp root, outside that child TEMP.
  $deps = if ($env:GWT_PLAYWRIGHT_DEPS_DIR) { $env:GWT_PLAYWRIGHT_DEPS_DIR } else { Join-Path $env:LOCALAPPDATA "Temp/gwt-playwright-$version" }
  $cli = Join-Path $deps 'node_modules/playwright/cli.js'
  $resolver = Join-Path $deps 'resolve-playwright.cjs'
  if (-not (Test-Path -LiteralPath $cli) -or -not (Test-Path -LiteralPath $resolver)) {
    throw "Pinned Playwright $version is missing; run scripts/run-visual-tests.sh --list once."
  }
  $runnerEnvironment = @{
    GWT_PLAYWRIGHT_BASE_URL=$url;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;GWT_PLAYWRIGHT_PROJECT_ROOT=$repoRoot;
    GWT_PLAYWRIGHT_CHECKOUT_ROOT=$repoRoot;GWT_PLAYWRIGHT_UPDATE_DRAIN='1';
    GWT_PLAYWRIGHT_NODE_MODULES=(Join-Path $deps 'node_modules');
    PLAYWRIGHT_JSON_OUTPUT_NAME=(Join-Path $checkHome 'playwright-results.json');
    NODE_OPTIONS="--require `"$($resolver.Replace('\', '/'))`" $env:NODE_OPTIONS"
  }
  foreach ($key in $runnerEnvironment.Keys) {
    $savedRunnerEnvironment[$key] = [Environment]::GetEnvironmentVariable($key)
    [Environment]::SetEnvironmentVariable($key, $runnerEnvironment[$key])
  }
  $artifacts = Join-Path $repoRoot ('crates/gwt/playwright/test-results/issue-5351-' + [guid]::NewGuid())
  Push-Location $repoRoot
  try {
    & $node $cli test --config crates/gwt/playwright/playwright.config.ts --workers=1 --headed --reporter=list,json --output $artifacts crates/gwt/playwright/tests/update-drain-live.spec.ts @PlaywrightArguments
    $exitCode = $LASTEXITCODE
  } finally { Pop-Location }
} finally {
  foreach ($key in $savedRunnerEnvironment.Keys) { [Environment]::SetEnvironmentVariable($key, $savedRunnerEnvironment[$key]) }
  if ($anchorSocket) { $anchorSocket.Abort(); $anchorSocket.Dispose() }
  if ($ownedProcess) {
    if (-not $ownedProcess.HasExited) { $ownedProcess.Kill($true) }
    if (-not $ownedProcess.WaitForExit(10000)) { throw "Owned gwt PID $ownedPid remained after cleanup." }
    [IO.File]::WriteAllText($stdoutLog, $stdoutTask.GetAwaiter().GetResult())
    [IO.File]::WriteAllText($stderrLog, $stderrTask.GetAwaiter().GetResult())
    $ownedProcess.Dispose()
  }
  # ConPTY providers can outlive their host. Only recorded fixture PIDs whose
  # creation time matches the receipt are eligible for cleanup.
  foreach ($line in [IO.File]::ReadAllLines($providerLog)) {
    $receipt = $line | ConvertFrom-Json
    if ($receipt.kind -ne 'launch') { continue }
    $providerProcess = Get-Process -Id $receipt.pid -ErrorAction SilentlyContinue
    if ($providerProcess) {
      $created = ([DateTimeOffset]$providerProcess.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
      if ([Math]::Abs($created - $receipt.started_at) -lt 1000) {
        $providerProcess.Kill($true)
        if (-not $providerProcess.WaitForExit(10000)) { throw "Owned provider PID $($receipt.pid) remained." }
      }
    }
  }
  Write-Output "Isolated evidence: $checkHome; owned process cleanup finished."
}
if ($exitCode -ne 0) { throw "Issue #5351 headed verification failed ($exitCode)." }
