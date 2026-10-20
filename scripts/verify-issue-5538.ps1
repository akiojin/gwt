# Owned checkout verification. -Fleet is deliberately opt-in and contacts Codex.
[CmdletBinding()]
param(
  [switch]$Fleet,
  [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArgs
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This owned-process harness requires Windows and PowerShell 7.' }
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$checkoutGwt = Join-Path $repoRoot 'target/debug/gwt.exe'
$checkoutGwtd = Join-Path $repoRoot 'target/debug/gwtd.exe'
foreach ($binary in @($checkoutGwt, $checkoutGwtd)) {
  if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw 'Build this checkout: cargo build -p gwt --bin gwt --bin gwtd' }
}
$node = (Get-Command node -CommandType Application).Source
$artifactRoot = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Temp'
$checkHome = Join-Path $artifactRoot ('gwt-dispatch-5538-' + [guid]::NewGuid())
foreach ($directory in @('.gwt', '.codex', 'tmp', 'scratch')) {
  [void](New-Item -ItemType Directory -Path (Join-Path $checkHome $directory) -Force)
}
# Never junction the user's Codex state/config: Windows Codex requires explicit CODEX_HOME.
$codexConfig = "$checkHome/.codex/config.toml"
[IO.File]::WriteAllText($codexConfig, "approval_policy = 'never'`nsuppress_unstable_features_warning = true`n")
if ($Fleet) {
  [void](Get-Command codex -ErrorAction Stop)
  $credentialRoot = if ($env:CODEX_HOME) { $env:CODEX_HOME } else { Join-Path ([Environment]::GetFolderPath('UserProfile')) '.codex' }
  $authSource = Join-Path $credentialRoot 'auth.json'
  if (-not (Test-Path -LiteralPath $authSource -PathType Leaf)) { throw 'Fleet requires an existing Codex auth.json; credentials are never printed.' }
}
$realRuntime = Join-Path ([Environment]::GetFolderPath('UserProfile')) '.gwt/runtime'
if (Test-Path -LiteralPath $realRuntime -PathType Container) {
  [void](New-Item -ItemType Junction -Path "$checkHome/.gwt/runtime" -Target $realRuntime)
}
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
$environment.GIT_TERMINAL_PROMPT = '0'
$environment.GCM_INTERACTIVE = 'never'
$environment.GIT_CONFIG_PARAMETERS = (([string]$environment['GIT_CONFIG_PARAMETERS']) + " 'credential.helper='").Trim()
$environment.GH_PROMPT_DISABLED = '1'
$environment.GIT_CONFIG_GLOBAL = "$checkHome/.gitconfig"
$environment.GIT_CONFIG_NOSYSTEM = '1'
[IO.File]::WriteAllText("$checkHome/.gitconfig", "[user]`nname = GwtFixture`nemail = fixture@example.invalid`n")
function New-IsolatedProcess([string]$FileName, [string]$WorkingDirectory = $repoRoot) {
  $start = [Diagnostics.ProcessStartInfo]::new($FileName)
  $start.WorkingDirectory = $WorkingDirectory
  $start.UseShellExecute = $false
  $start.CreateNoWindow = $true
  $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
  $start.Environment.Clear()
  foreach ($key in $environment.Keys) { $start.Environment[$key] = $environment[$key] }
  return $start
}
function Invoke-IsolatedGwtd([string]$Operation, [hashtable]$Parameters, [string]$WorkingDirectory) {
  $start = New-IsolatedProcess $checkoutGwtd $WorkingDirectory
  $start.Environment['GWT_PROJECT_ROOT'] = $WorkingDirectory
  $start.RedirectStandardInput = $true
  $start.RedirectStandardOutput = $true
  $start.RedirectStandardError = $true
  $start.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
  $start.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
  $process = [Diagnostics.Process]::Start($start)
  try {
    $process.StandardInput.WriteLine((@{schema_version=1;operation=$Operation;params=$Parameters} | ConvertTo-Json -Depth 8 -Compress))
    $process.StandardInput.Close()
    $output = $process.StandardOutput.ReadToEndAsync()
    $errors = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(60000)) { $process.Kill($true); throw "$Operation timed out." }
    $envelope = $output.GetAwaiter().GetResult() | ConvertFrom-Json
    if ($process.ExitCode -ne 0 -or -not $envelope.ok) { throw "$Operation failed (diagnostic output withheld)." }
    [void]$errors.GetAwaiter().GetResult()
    return $envelope
  } finally { $process.Dispose() }
}
function Invoke-FixtureGit([string]$WorkingDirectory, [string[]]$Arguments) {
  $start = New-IsolatedProcess (Get-Command git -CommandType Application).Source $WorkingDirectory
  foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
  $start.RedirectStandardOutput = $true
  $start.RedirectStandardError = $true
  $process = [Diagnostics.Process]::Start($start)
  try {
    $output = $process.StandardOutput.ReadToEndAsync()
    $errors = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(60000)) { $process.Kill($true); throw 'Scratch Git setup timed out.' }
    if ($process.ExitCode -ne 0) { throw ('Scratch Git setup failed: ' + $errors.GetAwaiter().GetResult()) }
    return $output.GetAwaiter().GetResult().Trim()
  } finally { $process.Dispose() }
}
function Assert-HookHealth($Envelope) {
  $health = $Envelope.output | ConvertFrom-Json
  $blocking = @($health.issues | Where-Object { $_ -notmatch '^managed hook failure: .* state=fail-open( |$)' })
  if ($health.status -eq 'inactive' -or $blocking.Count) { throw 'Isolated managed-hook convergence failed.' }
}
$projects = @()
$tabs = @()
$gui = $null
$runner = $null
$anchor = $null
$exitCode = 1
$stdout = $null
$stderr = $null
$runnerStdout = $null
$runnerStderr = $null
try {
  if ($Fleet) { Copy-Item -LiteralPath $authSource -Destination "$checkHome/.codex/auth.json" }
  $count = if ($Fleet) { 8 } else { 1 }
  for ($index = 0; $index -lt $count; $index++) {
    $folder = "$checkHome/scratch/project-$index"
    [void](New-Item -ItemType Directory -Path $folder -Force)
    [void](New-Item -ItemType Directory -Path "$folder/.codex" -Force)
    # Disposable Git fixture only; never alter the checkout's branches/worktrees.
    [void](Invoke-FixtureGit $folder @('init','--quiet'))
    [void](Invoke-FixtureGit $folder @('commit','--allow-empty','--quiet','-m','chore: initialize measurement fixture'))
    # Work mutations require an origin-based repository identity. Settings can
    # request remote branches, so refuse transport while preserving that identity.
    [void](Invoke-FixtureGit $folder @('remote','add','origin',"https://fixture.invalid/gwt5538/project-$index.git"))
    [void](Invoke-FixtureGit $folder @('config','protocol.allow','never'))
    [void](Invoke-FixtureGit $folder @('config','protocol.https.allow','never'))
    $branchName = Invoke-FixtureGit $folder @('symbolic-ref','--short','HEAD')
    [IO.File]::WriteAllText("$folder/README.txt", 'Issue #5538 measurement fixture. Inspect this file, then execute heartbeat.ps1. Its private heartbeat-progress receipt is authorized. Do not modify source, Git state, or GitHub.')
    $heartbeat = @'
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Write-Output ("GWT5538_READY session={0} shell={1}" -f $env:GWT_SESSION_ID, $PID)
while (-not (Test-Path -LiteralPath $env:GWT5538_BARRIER)) { Start-Sleep -Seconds 1 }
$observationClock = [Diagnostics.Stopwatch]::StartNew()
$tick = 0
function Publish-Heartbeat([bool]$Completed) {
  $receipt = @{session_id=$env:GWT_SESSION_ID;shell_pid=$PID;tick=$tick;
    elapsed_ms=$observationClock.Elapsed.TotalMilliseconds;observed_at=[DateTime]::UtcNow.ToString('o');completed=$Completed}
  $temporary = Join-Path $PSScriptRoot 'heartbeat-progress.tmp'
  [IO.File]::WriteAllText($temporary, ($receipt | ConvertTo-Json -Compress))
  [IO.File]::Move($temporary, (Join-Path $PSScriptRoot 'heartbeat-progress.json'), $true)
}
while ($observationClock.Elapsed.TotalSeconds -lt 2400) {
  $tick += 1
  Publish-Heartbeat $false
  Write-Output ("GWT5538_TICK session={0} shell={1} tick={2}" -f $env:GWT_SESSION_ID, $PID, $tick)
  Start-Sleep -Seconds 5
}
Publish-Heartbeat $true
Write-Output 'GWT5538_DONE'
'@
    [IO.File]::WriteAllText("$folder/heartbeat.ps1", $heartbeat)
    $status = Invoke-IsolatedGwtd 'issue.monitor.status' @{} $folder
    $key = $status.project_store.hash
    if ($key -notmatch '^[a-zA-Z0-9_-]+$') { throw 'Canonical scratch ProjectKey unavailable.' }
    $state = "$checkHome/.gwt/projects/$key/project-state"
    [void](New-Item -ItemType Directory -Path $state -Force)
    [IO.File]::WriteAllText("$state/pm.json", '{"settings":{"auto_start":false}}')
    [IO.File]::WriteAllText("$state/issue-monitor.json", '{"enabled":false,"max_active_agents":1,"priority_order":[]}')
    # Only newly created disposable Projects are trusted, using canonical path spelling.
    [void](Invoke-IsolatedGwtd 'hook.register_codex_managed_project_trust' @{
      codex_config=$codexConfig;project_root=$folder
    } $folder)
    $projects += @{path=$folder;project_key=$key;branch_name=$branchName}
    $tabs += @{id="dispatch-$index";title="Dispatch measurement $index";project_root=$folder;kind='git'}
  }
  [IO.File]::WriteAllText("$checkHome/projects.json", ($projects | ConvertTo-Json -Depth 4 -AsArray))
  [IO.File]::WriteAllText("$checkHome/.gwt/session.json", (@{active_tab_id='dispatch-0';tabs=$tabs;recent_projects=@()} | ConvertTo-Json -Depth 7))
  $environment.GWT5538_BARRIER = "$checkHome/fleet-barrier"
  $resolver = Invoke-IsolatedGwtd 'hook.doctor' @{} $projects[0].path
  $hookBin = ($resolver.output | ConvertFrom-Json).managed_hook_bin
  if (-not $hookBin -or $hookBin -match '(?i)[/\\]target[/\\].*[/\\]gwtd(\.exe)?$') { throw 'Stable/public hook fallback unavailable.' }
  $environment.GWT_HOOK_BIN = $hookBin
  $hookParams = @{expected_hook_bin=$hookBin;runtime_state_path="$checkHome/.gwt/browser-check-missing-runtime-state.json"}
  foreach ($project in $projects) {
    [void](Invoke-IsolatedGwtd 'hook.doctor' ($hookParams + @{repair=$true}) $project.path)
    [void](Invoke-IsolatedGwtd 'hook.register_codex_managed_hook_trust' @{
      codex_config=$codexConfig;codex_hook_discovery='both';project_root=$project.path
    } $project.path)
    Assert-HookHealth (Invoke-IsolatedGwtd 'hook.health' $hookParams $project.path)
  }
  $start = New-IsolatedProcess $checkoutGwt $projects[0].path
  $start.ArgumentList.Add('--no-tray')
  $start.ArgumentList.Add('--no-open')
  $start.Environment['GWT_BROWSER_URL_FILE'] = "$checkHome/url"
  $start.RedirectStandardOutput = $true
  $start.RedirectStandardError = $true
  $gui = [Diagnostics.Process]::Start($start)
  $stdout = $gui.StandardOutput.ReadToEndAsync()
  $stderr = $gui.StandardError.ReadToEndAsync()
  $sourceSnapshot = @('crates/gwt/src/main.rs','crates/gwt/src/app_runtime/mod.rs','crates/gwt/src/perf/dispatch.rs',
    'crates/gwt/src/perf/mod.rs','crates/gwt/src/perf/record.rs','crates/gwt/src/perf/global.rs') | ForEach-Object {
      @{path=$_;blob_sha=(& git -C $repoRoot hash-object -- $_).Trim()}
    }
  $fixture = @{gui_pid=$gui.Id;gui_started_at=$gui.StartTime.ToUniversalTime().ToString('o');checkout_binary=$checkoutGwt;
    binary_sha256=(Get-FileHash -LiteralPath $checkoutGwt -Algorithm SHA256).Hash;source_snapshot=@($sourceSnapshot);
    checkout=$repoRoot;checkout_sha=(& git -C $repoRoot rev-parse HEAD).Trim();isolated_home=$checkHome;fleet=[bool]$Fleet;projects=$projects}
  [IO.File]::WriteAllText("$checkHome/fixture.json", ($fixture | ConvertTo-Json -Depth 6))
  Write-Output "Owned checkout PID: $($gui.Id); retained artifacts: $checkHome"
  $url = ''
  $deadline = [DateTime]::UtcNow.AddSeconds(120)
  do {
    if ($gui.HasExited) { throw 'Owned checkout exited during startup.' }
    if (Test-Path -LiteralPath "$checkHome/url" -PathType Leaf) {
      $candidate = [IO.File]::ReadAllText("$checkHome/url").Trim()
      try { if ((Invoke-WebRequest -Uri $candidate -Method Head -TimeoutSec 2).StatusCode -eq 200) { $url=$candidate; break } } catch {}
    }
    Start-Sleep -Milliseconds 200
  } while ([DateTime]::UtcNow -lt $deadline)
  if (-not $url) { throw 'Fresh checkout readiness timed out.' }
  $anchor = [Net.WebSockets.ClientWebSocket]::new()
  $anchorUrl = [UriBuilder]::new($url)
  $anchorUrl.Scheme = 'ws'
  $anchorUrl.Path = '/ws'
  $anchor.ConnectAsync($anchorUrl.Uri, [Threading.CancellationToken]::None).GetAwaiter().GetResult() | Out-Null
  foreach ($project in $projects) { Assert-HookHealth (Invoke-IsolatedGwtd 'hook.health' $hookParams $project.path) }
  $version = [IO.File]::ReadAllText((Join-Path $repoRoot 'scripts/playwright-version.txt')).Trim()
  $deps = if ($env:GWT_PLAYWRIGHT_DEPS_DIR) { $env:GWT_PLAYWRIGHT_DEPS_DIR } else { Join-Path $artifactRoot "gwt-playwright-$version" }
  $cli = Join-Path $deps 'node_modules/playwright/cli.js'
  $resolvePlaywright = Join-Path $deps 'resolve-playwright.cjs'
  if (-not (Test-Path -LiteralPath $cli) -or -not (Test-Path -LiteralPath $resolvePlaywright)) { throw "Pinned Playwright $version missing; run scripts/run-visual-tests.sh --list first." }
  $run = New-IsolatedProcess $node
  foreach ($argument in @($cli,'test','--config','crates/gwt/playwright/playwright.config.ts','--workers=1','--retries=0',
      '--timeout','2700000','--output',"$checkHome/playwright",'crates/gwt/playwright/tests/dispatch-perf-live.spec.ts') + $PlaywrightArgs) {
    $run.ArgumentList.Add($argument)
  }
  $run.Environment['GWT_PLAYWRIGHT_BASE_URL'] = $url
  $run.Environment['GWT5538_HOME'] = $checkHome
  $run.Environment['GWT5538_FLEET'] = [string][int][bool]$Fleet
  $run.Environment['GWT_PLAYWRIGHT_NODE_MODULES'] = Join-Path $deps 'node_modules'
  $run.Environment['NODE_OPTIONS'] = "--require `"$($resolvePlaywright.Replace('\', '/'))`""
  # Preserve canonical evidence capture only in the test runner, not the GUI or agents.
  if ($env:GWT_HEADED_E2E_REPORT) { $run.Environment['GWT_HEADED_E2E_REPORT'] = $env:GWT_HEADED_E2E_REPORT }
  $run.RedirectStandardOutput = $true
  $run.RedirectStandardError = $true
  $runner = [Diagnostics.Process]::Start($run)
  $runnerStdout = $runner.StandardOutput.ReadToEndAsync()
  $runnerStderr = $runner.StandardError.ReadToEndAsync()
  $runner.WaitForExit()
  $exitCode = $runner.ExitCode
  $testOutput = $runnerStdout.GetAwaiter().GetResult()
  $testErrors = $runnerStderr.GetAwaiter().GetResult()
  [IO.File]::WriteAllText("$checkHome/playwright-stdout.log", $testOutput)
  [IO.File]::WriteAllText("$checkHome/playwright-stderr.log", $testErrors)
  Write-Output $testOutput
  Write-Output $testErrors
} finally {
  $cleanupErrors = [Collections.Generic.List[string]]::new()
  if ($runner) {
    try { if (-not $runner.HasExited) { $runner.Kill($true); [void]$runner.WaitForExit(10000) } } catch { $cleanupErrors.Add('Owned Playwright cleanup failed.') }
    $runner.Dispose()
  }
  if ($anchor) { $anchor.Abort(); $anchor.Dispose() }
  $stopped = $false
  if ($gui) {
    try {
      if (-not $gui.HasExited) { $gui.Kill($true) }
      $stopped = $gui.WaitForExit(10000)
      if (-not $stopped) { $cleanupErrors.Add("Owned GUI PID $($gui.Id) remains.") }
      if ($stopped) {
        [IO.File]::WriteAllText("$checkHome/startup.log", $stdout.GetAwaiter().GetResult())
        [IO.File]::WriteAllText("$checkHome/stderr.log", $stderr.GetAwaiter().GetResult())
      }
    } catch { $cleanupErrors.Add('Owned GUI cleanup failed.') }
    $gui.Dispose()
  }
  # A crash may orphan a child before tree termination. Only exact recorded
  # identities from this owned GUI can be reaped; names/patterns are not authority.
  $receiptPath = "$checkHome/owned-processes.json"
  if (Test-Path -LiteralPath $receiptPath) {
    foreach ($receipt in ([IO.File]::ReadAllText($receiptPath) | ConvertFrom-Json)) {
      try {
        $liveIdentity = Get-CimInstance Win32_Process -Filter "ProcessId = $([int]$receipt.pid)" -ErrorAction SilentlyContinue
        $ownedChild = Get-Process -Id $receipt.pid -ErrorAction SilentlyContinue
        if ($ownedChild -and $liveIdentity -and $liveIdentity.CreationDate.ToUniversalTime().ToString('o') -eq $receipt.started_at) {
          $ownedChild.Kill($true)
          if (-not $ownedChild.WaitForExit(10000)) { $cleanupErrors.Add("Owned child PID $($receipt.pid) remains.") }
        }
      } catch { $cleanupErrors.Add("Exact owned child cleanup failed for PID $($receipt.pid).") }
    }
  }
  # Do not retain authentication material with the diagnostic artifacts.
  $privateAuth = "$checkHome/.codex/auth.json"
  if (Test-Path -LiteralPath $privateAuth) { Remove-Item -LiteralPath $privateAuth -Force }
  [IO.File]::WriteAllText("$checkHome/shutdown.json", (@{owned_gui_stopped=$stopped;test_exit_code=$exitCode;
    stopped_at=[DateTime]::UtcNow.ToString('o');cleanup_errors=@($cleanupErrors)} | ConvertTo-Json -Depth 4))
  Write-Output "Retained artifacts: $checkHome"
  if ($cleanupErrors.Count) { throw ($cleanupErrors -join '; ') }
}
exit $exitCode
