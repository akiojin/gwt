[CmdletBinding(PositionalBinding = $false)]
param(
    [ValidateSet('before', 'after')][string]$Label = 'after',
    [ValidateRange(1000, 1800000)][int]$SampleMilliseconds = 30000,
    [switch]$AssertCpuBudget,
    [switch]$FunctionalOnly,
    [ValidateSet('debug', 'release')][string]$BuildProfile = 'debug',
    [ValidateRange(0.001, 1000)][double]$MaxCpuCores = 0.2,
    [string]$SessionsFixture,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArguments
)

$ErrorActionPreference = 'Stop'
if ($FunctionalOnly -and $AssertCpuBudget) { throw '-FunctionalOnly cannot assert the idle CPU budget.' }
if ($FunctionalOnly -and $BuildProfile -ne 'debug') { throw 'Canonical GUI verification requires the checkout debug binaries.' }
$measurementScope = if ($FunctionalOnly) { 'functional (includes startup)' } else { 'steady idle' }
$checkout = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$gwtRelative = "target/$BuildProfile/gwt.exe"
$gwtdRelative = "target/$BuildProfile/gwtd.exe"
$gwtBinary = Join-Path $checkout $gwtRelative
$gwtdBinary = Join-Path $checkout $gwtdRelative
$deps = Join-Path $env:LOCALAPPDATA 'Temp/gwt-playwright-win-1.49.1'
$realProfile = $env:USERPROFILE
$runId = [Guid]::NewGuid().ToString('N')
$checkHome = Join-Path $checkout "target/issue-5658-home-$runId"
$artifacts = Join-Path $checkout "crates/gwt/playwright/test-results/issue-5658-$Label-$runId"
$process = $null
$daemon = $null
$keeper = $null
$launchEvidence = $null
$savedEnvironment = @{}
function Invoke-FixtureGit([string[]]$Arguments) {
    $start = [Diagnostics.ProcessStartInfo]::new('git')
    $start.WorkingDirectory = $checkout
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($variable in @($start.Environment.Keys)) {
        if ($variable.StartsWith('GWT_', [StringComparison]::OrdinalIgnoreCase)) { $null = $start.Environment.Remove($variable) }
    }
    foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
    $gitProcess = [Diagnostics.Process]::Start($start)
    try {
        $stdout = $gitProcess.StandardOutput.ReadToEndAsync()
        $stderr = $gitProcess.StandardError.ReadToEndAsync()
        if (!$gitProcess.WaitForExit(120000)) { $gitProcess.Kill($true); throw 'Disposable fixture Git setup timed out.' }
        if ($gitProcess.ExitCode -ne 0) { throw "Disposable fixture Git setup failed: $($stderr.GetAwaiter().GetResult())" }
        return $stdout.GetAwaiter().GetResult().Trim()
    } finally { $gitProcess.Dispose() }
}
function Set-CheckEnvironment($variables) {
    foreach ($key in $variables.Keys) {
        if (!$savedEnvironment.ContainsKey($key)) { $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key) }
        [Environment]::SetEnvironmentVariable($key, $variables[$key])
    }
}
function Invoke-CheckoutEnvelope($operation, $parameters) {
    $attempts = if ($operation -eq 'issue.monitor.status') { 6 } else { 1 }
    for ($attempt = 1; $attempt -le $attempts; $attempt++) {
        $startInfo = [Diagnostics.ProcessStartInfo]::new($gwtdBinary)
        $startInfo.WorkingDirectory = $checkout
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.RedirectStandardInput = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        $startInfo.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
        $startInfo.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
        $startInfo.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
        $command = [Diagnostics.Process]::Start($startInfo)
        try {
            # Drain both pipes concurrently, including failures with no JSON stdout.
            $stdoutRead = $command.StandardOutput.ReadToEndAsync()
            $stderrRead = $command.StandardError.ReadToEndAsync()
            $command.StandardInput.WriteLine((@{schema_version=1;operation=$operation;params=$parameters} | ConvertTo-Json -Depth 12 -Compress))
            $command.StandardInput.Close()
            $command.WaitForExit()
            $stdout = $stdoutRead.GetAwaiter().GetResult()
            $stderr = $stderrRead.GetAwaiter().GetResult()
            $nativeExitCode = $command.ExitCode
        } finally { $command.Dispose() }
        try { $result = $stdout | ConvertFrom-Json -ErrorAction Stop } catch { $result = $null }
        if ($result -and $result.ok -eq $true -and $result.operation -eq $operation -and $nativeExitCode -eq 0) { return $result }
        $diagnostic = (@($stdout, $stderr) | Where-Object { ![string]::IsNullOrWhiteSpace($_) }) -join "`n"
        if ($attempt -lt $attempts -and $diagnostic -like '*network error: status connect timeout after*') {
            Write-Host "Retrying read-only issue.monitor.status connection ($attempt/$attempts)."
            Start-Sleep -Seconds 1
            continue
        }
        throw "$operation failed (exit $nativeExitCode): $diagnostic"
    }
}
function Assert-HookHealth($health) {
    $blocking = @($health.issues | Where-Object { $_ -notlike 'managed hook failure:*state=fail-open*' })
    if ($health.status -eq 'inactive' -or $blocking.Count) { throw "hook convergence failed: $($blocking -join ', ')" }
}
try {
    Set-Location $checkout
    if (!(Test-Path $gwtBinary) -or !(Test-Path $gwtdBinary)) { throw 'Build checkout gwt and gwtd before browser-check.' }
    if (!(Test-Path "$deps/node_modules/@playwright/test/cli.js") -or !(Test-Path "$deps/resolve-playwright.cjs")) { throw "Pinned Playwright dependencies missing at $deps" }
    New-Item -ItemType Directory -Path "$checkHome/.gwt", "$checkHome/workload", $artifacts -Force | Out-Null
    $sourceHead = Invoke-FixtureGit @('-C', $checkout, 'rev-parse', 'HEAD')
    $sourceBranch = Invoke-FixtureGit @('-C', $checkout, 'branch', '--show-current')
    if (!$sourceBranch) { throw 'The checkout must have a named source branch for the local shared clone.' }
    $fixtureOrigin = Join-Path $checkHome 'origin.git'
    $fixtureProject = Join-Path $checkHome 'project'
    # Launch pushes/fetches branches normally, entirely inside this private
    # origin. Its wildcard mapping cannot import the source checkout's fleet.
    # Launch Wizard owns all twelve fixture branches/worktrees.
    Write-Host 'Preparing private single-branch origin and shared checkout fixture.'
    $null = Invoke-FixtureGit @('clone', '--bare', '--shared', '--single-branch', '--no-tags', '--branch', $sourceBranch, '--', $checkout, $fixtureOrigin)
    $null = Invoke-FixtureGit @('clone', '--shared', '--single-branch', '--no-tags', '--branch', $sourceBranch, '--', $fixtureOrigin, $fixtureProject)
    $null = Invoke-FixtureGit @('-C', $fixtureProject, 'config', '--local', '--replace-all', 'remote.origin.fetch', '+refs/heads/*:refs/remotes/origin/*')
    $fixtureHead = Invoke-FixtureGit @('-C', $fixtureProject, 'rev-parse', 'HEAD')
    $originHead = Invoke-FixtureGit @('-C', $fixtureOrigin, 'rev-parse', 'HEAD')
    if ($fixtureHead -ne $sourceHead -or $originHead -ne $sourceHead) { throw 'Private fixture HEAD differs from the recorded checkout HEAD.' }
    $originRefs = @((Invoke-FixtureGit @('-C', $fixtureOrigin, 'for-each-ref', '--format=%(refname)')).Split("`n", [StringSplitOptions]::RemoveEmptyEntries))
    $trackingRefs = @((Invoke-FixtureGit @('-C', $fixtureProject, 'for-each-ref', '--format=%(refname)', 'refs/remotes/origin')).Split("`n", [StringSplitOptions]::RemoveEmptyEntries) | Where-Object { $_ -ne 'refs/remotes/origin/HEAD' })
    if ($originRefs.Count -ne 1 -or $originRefs[0] -ne "refs/heads/$sourceBranch" -or $trackingRefs.Count -ne 1 -or $trackingRefs[0] -ne "refs/remotes/origin/$sourceBranch") { throw 'Private fixture must start with exactly one source branch and one origin tracking branch.' }
    $fixturePushUrl = Invoke-FixtureGit @('-C', $fixtureProject, 'remote', 'get-url', '--push', 'origin')
    if (![IO.Path]::GetFullPath($fixturePushUrl).Equals([IO.Path]::GetFullPath($fixtureOrigin), [StringComparison]::OrdinalIgnoreCase)) { throw 'Fixture push URL must be its private origin inside the fresh HOME.' }
    # Resolve the canonical fixture identity before copying/hashing Sessions.
    $repoHash = (Invoke-CheckoutEnvelope 'issue.monitor.status' @{project_root=$fixtureProject}).project_store.hash
    if (!$repoHash) { throw 'project_store.hash missing' }
    if (Test-Path "$realProfile/.gwt/runtime") { New-Item -ItemType Junction -Path "$checkHome/.gwt/runtime" -Target "$realProfile/.gwt/runtime" | Out-Null }
    $fixture = @{copied=$false;files=0;bytes=0;sha256=$null}
    if ($SessionsFixture) {
        $fixturePath = (Resolve-Path -LiteralPath $SessionsFixture).Path.TrimEnd('/','\')
        $realSessions = [IO.Path]::GetFullPath((Join-Path $realProfile '.gwt/sessions')).TrimEnd('/','\')
        if ($fixturePath.Equals($realSessions, [StringComparison]::OrdinalIgnoreCase) -or $fixturePath.StartsWith("$realSessions\", [StringComparison]::OrdinalIgnoreCase)) { throw 'Pass a private copied sessions fixture, never the real HOME sessions directory.' }
        $fixtureRoot = Get-Item -LiteralPath $fixturePath -Force
        $fixtureEntries = @($fixtureRoot) + @(Get-ChildItem -LiteralPath $fixturePath -Recurse -Force)
        if (!$fixtureRoot.PSIsContainer -or @($fixtureEntries | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }).Count) { throw 'Sessions fixture must be a directory containing no links or junctions.' }
        Write-Host "Preparing sessions fixture: $(@($fixtureEntries | Where-Object { !$_.PSIsContainer }).Count) files; copying and hashing."
        $targetSessions = "$checkHome/.gwt/sessions"
        foreach ($directory in $fixtureEntries | Where-Object PSIsContainer) {
            $null = [IO.Directory]::CreateDirectory((Join-Path $targetSessions ([IO.Path]::GetRelativePath($fixturePath, $directory.FullName))))
        }
        $fixtureFiles = @($fixtureEntries | Where-Object { !$_.PSIsContainer } | Sort-Object FullName)
        $fixtureBytes = 0L
        $manifest = @(foreach ($file in $fixtureFiles) {
            $relativePath = [IO.Path]::GetRelativePath($fixturePath, $file.FullName)
            $destination = Join-Path $targetSessions $relativePath
            [IO.File]::Copy($file.FullName, $destination, $false)
            $stream = [IO.File]::OpenRead($destination)
            try {
                $fixtureBytes += $stream.Length
                $relativePath + ':' + [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($stream))
            } finally { $stream.Dispose() }
        })
        $fixture = @{copied=$true;files=$fixtureFiles.Count;bytes=$fixtureBytes;
            sha256=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes(($manifest -join "`n")))).ToLowerInvariant()}
        Write-Host "Sessions fixture ready: $($fixture.files) files, $($fixture.bytes) bytes."
    }
    $projectState = "$checkHome/.gwt/projects/$repoHash/project-state"
    New-Item -ItemType Directory -Path $projectState -Force | Out-Null
    '{"settings":{"auto_start":false}}' | Set-Content "$projectState/pm.json" -Encoding utf8NoBOM
    '{"enabled":false,"max_active_agents":1,"priority_order":[]}' | Set-Content "$projectState/issue-monitor.json" -Encoding utf8NoBOM
    # Copied Sessions are census data only. The new fixture is the sole open
    # Project and every measured Session must come from a new regular launch.
    @{tabs=@(@{id='idle-cpu-5658';title='Idle CPU';project_root=$fixtureProject;kind='git'});recent_projects=@()} | ConvertTo-Json -Depth 8 | Set-Content "$checkHome/.gwt/session.json" -Encoding utf8NoBOM
    $providerPath = "$checkHome/workload/provider.cjs"
    $provider = @'
const fs = require('node:fs');
const path = require('node:path');
const cp = require('node:child_process');
const readline = require('node:readline');
const session = process.env.GWT_SESSION_ID;
const directory = process.argv[2];
if (!session || !directory || !process.env.GWT_BIN_PATH || !process.env.GWT_PANE_WS_URL || !process.env.GWT_HOOK_FORWARD_TOKEN) {
  throw new Error('A regular isolated Agent launch is required');
}
const identity = { session_id: session, pid: process.pid, started_at: new Date(Date.now() - process.uptime() * 1000).toISOString(),
  cwd: process.cwd(), history_lines: 600, padding_bytes: 80, cli_binary: process.env.GWT_BIN_PATH };
const receipt = path.join(directory, session + '.json');
const events = receipt + 'l';
fs.writeFileSync(receipt, JSON.stringify(identity));
fs.writeFileSync(events, '');
let ownWindow;
let busy = false;
let started = false;
const period = 120000 / 11;
const record = value => fs.appendFileSync(events, JSON.stringify(value) + '\n');
async function readPane() {
  const at = new Date().toISOString();
  record({ kind: 'attempt', at });
  if (busy) { record({ kind: 'read', at, ended_at: at, ok: false, reason: 'overlapping_period' }); return; }
  busy = true;
  const clock = performance.now();
  try {
    // All launch-issued credentials stay in this process and its own CLI child.
    // stdout is inspected in memory, never forwarded to the PTY or receipt.
    const result = await new Promise(resolve => {
      const child = cp.spawn(process.env.GWT_BIN_PATH, [], { env: process.env, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
      let output = '';
      child.stdout.setEncoding('utf8').on('data', data => { output += data; });
      child.stderr.resume();
      const timeout = setTimeout(() => child.kill(), 30000);
      child.on('error', () => { clearTimeout(timeout); resolve({ ok: false, reason: 'spawn_error' }); });
      child.on('close', code => {
        clearTimeout(timeout);
        let envelope;
        try { envelope = JSON.parse(output); } catch {}
        resolve({ ok: code === 0 && envelope?.ok === true && envelope?.operation === 'pane.read', reason: code === 0 ? 'envelope' : 'exit' });
      });
      child.stdin.on('error', () => {});
      child.stdin.end(JSON.stringify({ schema_version: 1, operation: 'pane.read', params: { id: ownWindow, lines: 1 } }));
    });
    record({ kind: 'read', at, ended_at: new Date().toISOString(), latency_ms: performance.now() - clock, ...result });
  } finally { busy = false; }
}
readline.createInterface({ input: process.stdin, crlfDelay: Infinity }).on('line', line => {
  let command;
  try { command = JSON.parse(line); } catch { return; }
  if (command.kind === 'bind' && !ownWindow && typeof command.id === 'string') {
    ownWindow = command.id;
    record({ kind: 'bound', window_id: ownWindow, index: command.index, at: new Date().toISOString() });
    process.stdout.write(Array.from({ length: 600 }, (_, index) => '\x1b[36mGWT_IDLE_HISTORY_' + String(index).padStart(4, '0')
      + ' ' + '.'.repeat(80) + '\x1b[0m\r\n').join('') + 'GWT_IDLE_5658_SEEDED_' + command.index + '\r\n');
    void readPane();
  } else if (command.kind === 'start' && ownWindow && !started && Number.isFinite(command.at) && Number.isInteger(command.index)) {
    started = true;
    const first = command.at + command.index * period / 12;
    setTimeout(() => {
      record({ kind: 'started', at: new Date().toISOString(), first_at: new Date(first).toISOString(), period_ms: period });
      void readPane();
      setInterval(() => { void readPane(); }, period);
    }, Math.max(0, first - Date.now()));
  } else if (command.kind === 'probe' && ownWindow && ['BEFORE', 'AFTER'].includes(command.phase)
      && ['dark', 'light'].includes(command.theme) && Number.isInteger(command.index)) {
    process.stdout.write('GWT_IDLE_5658_' + command.theme + '_' + command.phase + '_' + command.index + '\r\n');
  }
});
'@
    [IO.File]::WriteAllText($providerPath, $provider, [Text.UTF8Encoding]::new($false))
    $providerHash = (Get-FileHash -LiteralPath $providerPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $nodeToml = (Get-Command node -CommandType Application).Source.Replace('\', '/')
    $providerToml = $providerPath.Replace('\', '/')
    $receiptsToml = "$checkHome/workload".Replace('\', '/')
    [IO.File]::WriteAllText("$checkHome/.gwt/config.toml", @"
[tools.customCodingAgents.idle-cpu-workload]
displayName = "Idle CPU workload"
agentType = "command"
command = "$nodeToml"
defaultArgs = ["$providerToml", "$receiptsToml"]
"@, [Text.UTF8Encoding]::new($false))
    $hookBinary = ((Invoke-CheckoutEnvelope 'hook.doctor' @{}).output | ConvertFrom-Json).managed_hook_bin
    if (!$hookBinary -or $hookBinary -match '[/\\]target[/\\].*[/\\]gwtd(\.exe)?$') { throw 'Stable hook fallback missing' }
    $hookParams = @{repair=$true;expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}
    Assert-HookHealth ((Invoke-CheckoutEnvelope 'hook.doctor' $hookParams).output | ConvertFrom-Json).health
    # Launch children without the calling agent's capability, Session, execution,
    # hook-forwarding URLs, or launch-scoped binary. Never print their values.
    $clearLaunch = @{}
    foreach ($variable in Get-ChildItem Env:GWT_*) { $clearLaunch[$variable.Name] = $null }
    Set-CheckEnvironment $clearLaunch
    Set-CheckEnvironment @{HOME=$checkHome;USERPROFILE=$checkHome;CODEX_HOME="$checkHome/.codex";
        CLAUDE_CONFIG_DIR="$checkHome/.claude";GWT_BROWSER_URL_FILE="$checkHome/url.txt";
        GWT_HOOK_BIN=$hookBinary;GIT_TERMINAL_PROMPT='0';GH_PROMPT_DISABLED='1'}
    '{"schema_version":1,"operation":"daemon.start","params":{}}' | Set-Content "$checkHome/daemon-start.json" -Encoding utf8NoBOM
    $daemon = Start-Process -FilePath $gwtdBinary -WorkingDirectory $checkout -WindowStyle Hidden -PassThru -RedirectStandardInput "$checkHome/daemon-start.json" -RedirectStandardOutput "$checkHome/daemon-stdout.log" -RedirectStandardError "$checkHome/daemon-stderr.log"
    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    do {
        $daemonStatus = (Invoke-CheckoutEnvelope 'daemon.status' @{}).output
        if ($daemonStatus -match '^running pid=(\d+).*probe=ok') { break }
        if ($daemon.HasExited -or [DateTime]::UtcNow -ge $deadline) { throw 'Isolated daemon did not become ready' }
        Start-Sleep -Milliseconds 200
    } while ($true)
    if ([int]$Matches[1] -ne $daemon.Id) { throw 'Isolated daemon PID does not match its owner' }
    $process = Start-Process -FilePath $gwtBinary -ArgumentList '--no-tray','--no-open' -WorkingDirectory $checkout -WindowStyle Hidden -PassThru -RedirectStandardOutput "$checkHome/stdout.log" -RedirectStandardError "$checkHome/stderr.log"
    Write-Host "Owned checkout GUI PID: $($process.Id); started_at: $($process.StartTime.ToUniversalTime().ToString('o')); HOME: $checkHome"
    $deadline = [DateTime]::UtcNow.AddSeconds(120)
    while (!(Test-Path "$checkHome/url.txt")) {
        if ($process.HasExited -or [DateTime]::UtcNow -ge $deadline) { throw "Checkout startup failed; inspect $checkHome" }
        Start-Sleep -Milliseconds 200
    }
    $baseUrl = (Get-Content "$checkHome/url.txt" -Raw).Trim()
    if ((Invoke-WebRequest -Method Head -Uri $baseUrl).StatusCode -ne 200) { throw 'Fresh checkout URL is not ready' }
    $keeper = [System.Net.WebSockets.ClientWebSocket]::new()
    $keeper.Options.SetRequestHeader('Origin', $baseUrl.TrimEnd('/'))
    $null = $keeper.ConnectAsync([Uri](($baseUrl -replace '^http', 'ws') + 'ws'), [Threading.CancellationToken]::None).GetAwaiter().GetResult()
    # A URL can be available before a large Session fixture finishes bootstrap.
    # Wait for the seeded checkout's catalog entry, then route directly to it.
    $readyDeadline = [Threading.CancellationTokenSource]::new([TimeSpan]::FromSeconds(180))
    $frame = [IO.MemoryStream]::new()
    try {
        $readyBytes = [Text.Encoding]::UTF8.GetBytes('{"kind":"frontend_ready"}')
        $null = $keeper.SendAsync([ArraySegment[byte]]::new($readyBytes), [Net.WebSockets.WebSocketMessageType]::Text, $true, $readyDeadline.Token).GetAwaiter().GetResult()
        $buffer = [byte[]]::new(65536)
        $projectKey = $null
        while (!$projectKey) {
            $received = $keeper.ReceiveAsync([ArraySegment[byte]]::new($buffer), $readyDeadline.Token).GetAwaiter().GetResult()
            if ($received.MessageType -eq [Net.WebSockets.WebSocketMessageType]::Close) { throw 'Checkout Hub closed before project readiness.' }
            $frame.Write($buffer, 0, $received.Count)
            if (!$received.EndOfMessage) { continue }
            $message = [Text.Encoding]::UTF8.GetString($frame.ToArray()) | ConvertFrom-Json
            $frame.SetLength(0)
            if ($message.kind -eq 'hub_state') {
                $projectKey = @($message.hub.projects | Where-Object { $_.id -eq 'idle-cpu-5658' -and $_.project_key } | Select-Object -First 1).project_key
            }
        }
    } catch {
        if ($readyDeadline.IsCancellationRequested) { throw 'Checkout Hub did not announce the seeded checkout project within 180 seconds.' }
        throw
    } finally { $frame.Dispose(); $readyDeadline.Dispose() }
    $projectUrl = [Uri]::new([Uri]$baseUrl, "/p/$projectKey").AbsoluteUri
    Write-Host 'Checkout Hub ready: seeded project catalog entry is present.'
    $healthParams = @{expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}
    # Audit the same existing provider hook surfaces repaired before launch.
    Set-CheckEnvironment @{HOME=$savedEnvironment.HOME;USERPROFILE=$realProfile;
        CODEX_HOME=$savedEnvironment.CODEX_HOME;CLAUDE_CONFIG_DIR=$savedEnvironment.CLAUDE_CONFIG_DIR}
    Assert-HookHealth ((Invoke-CheckoutEnvelope 'hook.health' $healthParams).output | ConvertFrom-Json)
    Set-CheckEnvironment @{HOME=$checkHome;USERPROFILE=$checkHome;CODEX_HOME="$checkHome/.codex";CLAUDE_CONFIG_DIR="$checkHome/.claude"}
    $hashes = @{}
    foreach ($path in @($gwtRelative, $gwtdRelative, 'crates/gwt/src/main.rs',
        'crates/gwt/src/session_inventory.rs', 'crates/gwt/src/index_worker.rs', 'crates/gwt/src/runtime_health_poller.rs', 'crates/gwt/src/agent_capacity.rs',
        'crates/gwt-terminal/src/pane.rs', 'crates/gwt-terminal/src/test_util.rs',
        'scripts/diagnostics/issue-5658-browser-check.ps1', 'scripts/diagnostics/issue-5658-cpu.ps1',
        'crates/gwt/playwright/tests/idle-cpu-live.spec.ts')) { $hashes[$path] = (Get-FileHash -LiteralPath (Join-Path $checkout $path) -Algorithm SHA256).Hash.ToLowerInvariant() }
    $launchEvidence = @{git_head=$sourceHead;git_status=@(& git -C $checkout status --short);project=$fixtureProject;repo_hash=$repoHash;
        measurement_scope=$measurementScope;build_profile=$BuildProfile;fixture_clone_sha=$fixtureHead;fixture_origin=$fixtureOrigin;fixture_push_url=$fixturePushUrl;
        fixture_origin_branch_refs=$originRefs.Count;fixture_project_origin_branch_refs=$trackingRefs.Count;
        provider_sha256=$providerHash;ready_requests_per_minute=66;agent_count=12;
        gui_pid=$process.Id;gui_started_at=$process.StartTime.ToUniversalTime().ToString('o');daemon_pid=$daemon.Id;project_key=$projectKey;
        sha256=$hashes;sessions_fixture=$fixture;pm_auto_start=$false;issue_monitor_enabled=$false;background_index='normal';
        user_verification_result='n/a (autonomous)'}
    Set-CheckEnvironment @{GWT_PLAYWRIGHT_BASE_URL=$projectUrl;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;
        GWT_PLAYWRIGHT_PID=[string]$process.Id;GWT_PLAYWRIGHT_PROJECT_ROOT=$checkout;
        GWT_PLAYWRIGHT_GUI_STARTED_AT=$process.StartTime.ToUniversalTime().ToString('o');
        GWT_IDLE_FIXTURE_SHA=$fixtureHead;GWT_IDLE_FIXTURE_BRANCH=$sourceBranch;GWT_IDLE_PROVIDER_SHA=$providerHash;
        GWT_IDLE_PROJECT_STATE=$projectState;
        GWT_IDLE_CPU_LABEL=$Label;GWT_IDLE_SAMPLE_MS=[string]$SampleMilliseconds;
        GWT_IDLE_BUILD_PROFILE=$BuildProfile;
        GWT_IDLE_FUNCTIONAL_ONLY=[string][int]$FunctionalOnly.IsPresent;
        GWT_IDLE_ASSERT_BUDGET=[string][int]$AssertCpuBudget.IsPresent;GWT_IDLE_MAX_CORES=$MaxCpuCores.ToString([Globalization.CultureInfo]::InvariantCulture);
        GWT_HEADED_E2E_REPORT=$savedEnvironment['GWT_HEADED_E2E_REPORT'];
        GWT_PLAYWRIGHT_NODE_MODULES="$deps/node_modules";NODE_PATH="$deps/node_modules";
        NODE_OPTIONS="--require $($deps.Replace('\','/'))/resolve-playwright.cjs"}
    # verify.run appends --headed and its evidence reporter through this array.
    & node "$deps/node_modules/@playwright/test/cli.js" test --config crates/gwt/playwright/playwright.config.ts --output $artifacts --workers=1 idle-cpu-live.spec.ts @PlaywrightArguments
    if ($LASTEXITCODE -ne 0) { throw "Playwright failed with exit $LASTEXITCODE" }
    Write-Output "Idle CPU evidence: $artifacts"
} finally {
    try {
        if ($keeper) { $keeper.Dispose() }
        if ($process -and !$process.HasExited) { $null = $process.WaitForExit(12000) }
        if ($process -and !$process.HasExited) {
            & taskkill.exe /PID $process.Id /T /F 2>$null | Out-Null
            $null = $process.WaitForExit(5000)
            if (!$process.HasExited) { throw "Owned GUI PID $($process.Id) survived cleanup" }
        }
    } finally {
        try {
            if ($daemon -and !$daemon.HasExited) { Stop-Process -Id $daemon.Id; if (!$daemon.WaitForExit(5000)) { throw 'Owned daemon survived cleanup' } }
        } finally {
            foreach ($key in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key]) }
            # Playwright clears its output directory on entry; retain launch
            # measurements in memory until the runner has finished.
            if ($launchEvidence) { $launchEvidence | ConvertTo-Json -Depth 4 | Set-Content "$artifacts/launch.json" -Encoding utf8NoBOM }
            if ($process) { @{gui_pid=$process.Id;gui_exited=$process.HasExited;daemon_pid=$daemon.Id;daemon_exited=$daemon.HasExited;
                cleaned_at=[DateTime]::UtcNow.ToString('o')} | ConvertTo-Json | Set-Content "$artifacts/owned-pid-cleanup.json" -Encoding utf8NoBOM }
        }
    }
}
