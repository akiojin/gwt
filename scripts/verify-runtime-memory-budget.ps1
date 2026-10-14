# #5381: owned Windows checkout, fresh HOME, eight deterministic real Agent PTYs.
# Build first: cargo build -p gwt --bin gwt --bin gwtd
# Set GWT_PLAYWRIGHT_MEMORY_SECONDS=600 for the GUI regression budget observation.
# The actual agent fleet one-hour observation belongs to #5385.
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArgs)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This fixture requires Windows and PowerShell 7.' }
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$checkoutGwt = Join-Path $repoRoot 'target/debug/gwt.exe'
$checkoutGwtd = Join-Path $repoRoot 'target/debug/gwtd.exe'
foreach ($binary in @($checkoutGwt, $checkoutGwtd)) {
  if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw 'Build this checkout gwt/gwtd first.' }
}
$node = (Get-Command node -CommandType Application).Source
# verify.run gives each command a disposable TEMP. Keep owned evidence and
# the pinned shared dependency cache outside that disposable command folder.
$artifactRoot = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Temp'
$checkHome = Join-Path $artifactRoot ('gwt-runtime-memory-' + [guid]::NewGuid())
foreach ($relative in @('.gwt', '.codex', 'workload', 'tmp')) {
  [void](New-Item -ItemType Directory -Path (Join-Path $checkHome $relative) -Force)
}
[IO.File]::WriteAllText("$checkHome/.codex/config.toml", '')
$realRuntime = Join-Path ([Environment]::GetFolderPath('UserProfile')) '.gwt/runtime'
if (Test-Path -LiteralPath $realRuntime -PathType Container) {
  [void](New-Item -ItemType Junction -Path "$checkHome/.gwt/runtime" -Target $realRuntime)
}
$producer = @'
const fs = require('node:fs');
const path = require('node:path');
const directory = process.env.GWT_MEMORY_WORKLOAD_DIR;
if (!directory || !process.env.GWT_SESSION_ID) throw new Error('isolated Agent launch required');
const identity = {pid:process.pid, started_at:Date.now()-process.uptime()*1000, session:process.env.GWT_SESSION_ID};
const file = path.join(directory, `${process.pid}.json`);
fs.writeFileSync(file, JSON.stringify({...identity,tick:0,observed_at:new Date().toISOString()}));
let tick = 0;
setInterval(() => {
  tick += 1;
  // Windows readers may hold the receipt open: keep identity immutable and
  // append heartbeats instead of replacing a file being sampled.
  fs.appendFileSync(file+'.heartbeat', JSON.stringify({tick,observed_at:new Date().toISOString()})+'\n');
  process.stdout.write(`GWT_MEMORY_TICK ${process.pid} ${tick} ${'x'.repeat(512)}\n`);
}, 1000);
process.stdin.resume();
'@
[IO.File]::WriteAllText("$checkHome/workload/producer.cjs", $producer)
$nodeToml = $node.Replace('\', '/')
$producerToml = "$checkHome/workload/producer.cjs".Replace('\', '/')
[IO.File]::WriteAllText("$checkHome/.gwt/config.toml", @"
[tools.customCodingAgents.runtime-memory-workload]
displayName = "Runtime memory workload"
agentType = "command"
command = "$nodeToml"
defaultArgs = ["$producerToml"]
"@)
$environment = @{}
foreach ($entry in [Environment]::GetEnvironmentVariables().GetEnumerator()) {
  if ($entry.Key -notmatch '^(GWT_|CODEX_|CLAUDE_|GH_TOKEN$|GITHUB_TOKEN$|OPENAI_API_KEY$|ANTHROPIC_API_KEY$)') {
    $environment[$entry.Key] = [string]$entry.Value
  }
}
$environment.HOME = $checkHome
$environment.USERPROFILE = $checkHome
$environment.CODEX_HOME = "$checkHome/.codex"
$environment.TEMP = "$checkHome/tmp"
$environment.TMP = "$checkHome/tmp"
$environment.GWT_PROJECT_ROOT = $repoRoot
$environment.GWT_MEMORY_WORKLOAD_DIR = "$checkHome/workload"
$environment.GIT_TERMINAL_PROMPT = '0'
$environment.GH_PROMPT_DISABLED = '1'
function New-IsolatedProcess([string]$FileName) {
  $start = [Diagnostics.ProcessStartInfo]::new($FileName)
  $start.WorkingDirectory = $repoRoot
  $start.UseShellExecute = $false
  $start.CreateNoWindow = $true
  $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
  $start.Environment.Clear()
  foreach ($key in $environment.Keys) { $start.Environment[$key] = $environment[$key] }
  return $start
}
function Invoke-IsolatedGwtd([string]$Operation, [hashtable]$Parameters, [string]$WorkingDirectory = $repoRoot) {
  $start = New-IsolatedProcess $checkoutGwtd
  $start.WorkingDirectory = $WorkingDirectory
  $start.Environment['GWT_PROJECT_ROOT'] = $WorkingDirectory
  $start.RedirectStandardInput = $true
  $start.RedirectStandardOutput = $true
  $start.RedirectStandardError = $true
  $start.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
  $start.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
  $process = [Diagnostics.Process]::Start($start)
  try {
    $process.StandardInput.WriteLine((@{schema_version=1;operation=$Operation;params=$Parameters} | ConvertTo-Json -Depth 10 -Compress))
    $process.StandardInput.Close()
    $output = $process.StandardOutput.ReadToEndAsync()
    $errorOutput = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(60000)) { $process.Kill($true); throw "$Operation timed out." }
    $envelope = $output.GetAwaiter().GetResult() | ConvertFrom-Json
    if ($process.ExitCode -ne 0 -or -not $envelope.ok) { throw "$Operation failed: $($errorOutput.GetAwaiter().GetResult())" }
    return $envelope
  } finally { $process.Dispose() }
}
function Assert-HookHealth($Health) {
  $blocking = @($Health.issues | Where-Object { $_ -notmatch '^managed hook failure: .* state=fail-open( |$)' })
  if ($Health.status -eq 'inactive' -or $blocking.Count) { throw "Hook convergence failed: $($blocking -join '; ')" }
}
$resolver = Invoke-IsolatedGwtd 'hook.doctor' @{}
$hookBin = ($resolver.output | ConvertFrom-Json).managed_hook_bin
if (-not $hookBin -or $hookBin -match '(?i)[/\\]target[/\\].*[/\\]gwtd(\.exe)?$') { throw 'Stable/public hook fallback unavailable.' }
$environment.GWT_HOOK_BIN = $hookBin
$hookParams = @{expected_hook_bin=$hookBin;runtime_state_path="$checkHome/.gwt/browser-check-missing-runtime-state.json"}
$doctor = Invoke-IsolatedGwtd 'hook.doctor' ($hookParams + @{repair=$true})
[void](Invoke-IsolatedGwtd 'hook.register_codex_managed_hook_trust' @{
  codex_config="$checkHome/.codex/config.toml";codex_hook_discovery='both';project_root=$repoRoot
})
$preflightHealth = Invoke-IsolatedGwtd 'hook.health' $hookParams
Assert-HookHealth ($preflightHealth.output | ConvertFrom-Json)
$status = Invoke-IsolatedGwtd 'issue.monitor.status' @{}
$repoHash = $status.project_store.hash
if ($repoHash -notmatch '^[a-zA-Z0-9_-]+$') { throw 'Canonical project_store.hash unavailable.' }
$state = "$checkHome/.gwt/projects/$repoHash/project-state"
[void](New-Item -ItemType Directory -Path $state -Force)
[IO.File]::WriteAllText("$state/pm.json", '{"settings":{"auto_start":false}}')
[IO.File]::WriteAllText("$state/issue-monitor.json", '{"enabled":false,"max_active_agents":1,"priority_order":[]}')
$tabs = @(@{id='memory-source';title='Checkout source history';project_root=$repoRoot;kind='git'})
$workloadProjects = @()
for ($index = 0; $index -lt 8; $index++) {
  $folder = Join-Path $checkHome "workload/project-$index"
  [void](New-Item -ItemType Directory -Path $folder -Force)
  # Managed Agent assets require Git's local exclude file even for folder
  # Projects. Initialize metadata only: no commit, ref or worktree is created.
  & git -C $folder init --quiet
  if ($LASTEXITCODE -ne 0) { throw 'Isolated workload Git metadata initialization failed.' }
  $folderStatus = Invoke-IsolatedGwtd 'issue.monitor.status' @{} $folder
  $folderKey = $folderStatus.project_store.hash
  if ($folderKey -notmatch '^[a-zA-Z0-9_-]+$') { throw 'Workload folder ProjectKey unavailable.' }
  $folderState = "$checkHome/.gwt/projects/$folderKey/project-state"
  [void](New-Item -ItemType Directory -Path $folderState -Force)
  [IO.File]::WriteAllText("$folderState/pm.json", '{"settings":{"auto_start":false}}')
  [IO.File]::WriteAllText("$folderState/issue-monitor.json", '{"enabled":false,"max_active_agents":1,"priority_order":[]}')
  $tabs += @{id="memory-workload-$index";title="Memory workload $index";project_root=$folder;kind='non_repo'}
  $workloadProjects += @{path=$folder;project_key=$folderKey}
}
[IO.File]::WriteAllText("$checkHome/workload-projects.json", ($workloadProjects | ConvertTo-Json -Depth 5))
[IO.File]::WriteAllText("$checkHome/.gwt/session.json", (@{active_tab_id='memory-source';tabs=$tabs;recent_projects=@()} | ConvertTo-Json -Depth 8))
$gui = $null
$anchor = $null
$savedRunner = @{}
$exitCode = 1
try {
  $start = New-IsolatedProcess $checkoutGwt
  $start.ArgumentList.Add('--no-tray')
  $start.ArgumentList.Add('--no-open')
  $start.Environment['GWT_BROWSER_URL_FILE'] = "$checkHome/url"
  $start.RedirectStandardOutput = $true
  $start.RedirectStandardError = $true
  $gui = [Diagnostics.Process]::Start($start)
  [IO.File]::WriteAllText("$checkHome/fixture.json", (@{
    gui_pid=$gui.Id;checkout_binary=$checkoutGwt;checkout=$repoRoot;isolated_home=$checkHome;
    started_at=[DateTime]::UtcNow.ToString('o');workload_projects=$workloadProjects
  } | ConvertTo-Json -Depth 6))
  $stdout = $gui.StandardOutput.ReadToEndAsync()
  $stderr = $gui.StandardError.ReadToEndAsync()
  Write-Output "Owned checkout PID: $($gui.Id); isolated HOME: $checkHome"
  $url = ''
  $deadline = [DateTime]::UtcNow.AddSeconds(120)
  do {
    if ($gui.HasExited) { throw 'Owned checkout exited during startup.' }
    if (Test-Path -LiteralPath "$checkHome/url" -PathType Leaf) {
      $candidate = [IO.File]::ReadAllText("$checkHome/url").Trim()
      try { if ((Invoke-WebRequest -Uri $candidate -Method Head -TimeoutSec 2).StatusCode -eq 200) { $url = $candidate; break } } catch {}
    }
    Start-Sleep -Milliseconds 200
  } while ([DateTime]::UtcNow -lt $deadline)
  if (-not $url) { throw 'Fresh checkout URL readiness timed out.' }
  $anchor = [Net.WebSockets.ClientWebSocket]::new()
  $anchorUrl = [UriBuilder]::new($url)
  $anchorUrl.Scheme = 'ws'
  $anchorUrl.Path = '/ws'
  $anchor.ConnectAsync($anchorUrl.Uri, [Threading.CancellationToken]::None).GetAwaiter().GetResult() | Out-Null
  $health = Invoke-IsolatedGwtd 'hook.health' $hookParams
  Assert-HookHealth ($health.output | ConvertFrom-Json)
  [IO.File]::WriteAllText("$checkHome/hook-health.json", $health.output)
  $version = [IO.File]::ReadAllText((Join-Path $repoRoot 'scripts/playwright-version.txt')).Trim()
  $deps = if ($env:GWT_PLAYWRIGHT_DEPS_DIR) { $env:GWT_PLAYWRIGHT_DEPS_DIR } else { Join-Path $artifactRoot "gwt-playwright-$version" }
  $cli = Join-Path $deps 'node_modules/playwright/cli.js'
  $resolvePlaywright = Join-Path $deps 'resolve-playwright.cjs'
  if (-not (Test-Path -LiteralPath $cli) -or -not (Test-Path -LiteralPath $resolvePlaywright)) { throw "Pinned Playwright $version missing; run scripts/run-visual-tests.sh --list first." }
  $duration = if ($env:GWT_PLAYWRIGHT_MEMORY_SECONDS) { $env:GWT_PLAYWRIGHT_MEMORY_SECONDS } else { '60' }
  $runner = @{
    GWT_PLAYWRIGHT_BASE_URL=$url;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;GWT_PLAYWRIGHT_PROJECT_ROOT=$repoRoot;
    GWT_PLAYWRIGHT_MEMORY_PID=[string]$gui.Id;GWT_PLAYWRIGHT_MEMORY_SHA=(& git -C $repoRoot rev-parse HEAD).Trim();
    GWT_PLAYWRIGHT_MEMORY_BRANCH=(& git -C $repoRoot branch --show-current).Trim();GWT_PLAYWRIGHT_MEMORY_SECONDS=$duration;
    GWT_PLAYWRIGHT_MEMORY_WORKTREES=[string]@(& git -C $repoRoot worktree list --porcelain | Where-Object { $_.StartsWith('worktree ') }).Count;
    GWT_PLAYWRIGHT_NODE_MODULES=(Join-Path $deps 'node_modules');
    NODE_OPTIONS="--require `"$($resolvePlaywright.Replace('\', '/'))`" $env:NODE_OPTIONS"
  }
  foreach ($key in $runner.Keys) {
    $savedRunner[$key] = [Environment]::GetEnvironmentVariable($key)
    [Environment]::SetEnvironmentVariable($key, $runner[$key])
  }
  Push-Location $repoRoot
  try {
    # Playwright uses the project timeout, not test.setTimeout(), when saving
    # traces. The long eight-page observation needs the same budget there.
    $projectTimeout = [int](([double]$duration + 900) * 1000)
    & $node $cli test --config crates/gwt/playwright/playwright.config.ts --timeout $projectTimeout --workers=1 --output "$checkHome/playwright" crates/gwt/playwright/tests/runtime-memory-budget-live.spec.ts @PlaywrightArgs
    $exitCode = $LASTEXITCODE
  } finally { Pop-Location }
} finally {
  foreach ($key in $savedRunner.Keys) { [Environment]::SetEnvironmentVariable($key, $savedRunner[$key]) }
  if ($anchor) { $anchor.Abort(); $anchor.Dispose() }
  if ($gui) {
    if (-not $gui.HasExited) { $gui.Kill($true) }
    if (-not $gui.WaitForExit(10000)) { throw "Owned GUI PID $($gui.Id) remains." }
    [IO.File]::WriteAllText("$checkHome/startup.log", $stdout.GetAwaiter().GetResult())
    [IO.File]::WriteAllText("$checkHome/stderr.log", $stderr.GetAwaiter().GetResult())
    $gui.Dispose()
  }
  foreach ($file in Get-ChildItem -LiteralPath "$checkHome/workload" -File -Filter '*.json') {
    $receipt = [IO.File]::ReadAllText($file.FullName) | ConvertFrom-Json
    $producerProcess = Get-Process -Id $receipt.pid -ErrorAction SilentlyContinue
    if ($producerProcess) {
      $created = ([DateTimeOffset]$producerProcess.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
      if ([Math]::Abs($created - $receipt.started_at) -lt 1000) {
        $producerProcess.Kill($true)
        if (-not $producerProcess.WaitForExit(10000)) { throw "Owned producer PID $($receipt.pid) remains." }
      }
    }
  }
  [IO.File]::WriteAllText("$checkHome/shutdown.json", (@{
    owned_gui_stopped=($null -ne $gui);stopped_at=[DateTime]::UtcNow.ToString('o');test_exit_code=$exitCode
  } | ConvertTo-Json -Depth 4))
  Write-Output "Evidence: $checkHome; owned process cleanup finished."
}
exit $exitCode
