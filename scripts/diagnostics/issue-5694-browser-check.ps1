[CmdletBinding(PositionalBinding = $false)]
param(
    [ValidateSet('chromium-dark', 'chromium-light')][string]$Theme = 'chromium-dark',
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArguments
)

$ErrorActionPreference = 'Stop'
$checkout = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$gwtBinary = Join-Path $checkout 'target/debug/gwt.exe'
$gwtdBinary = Join-Path $checkout 'target/debug/gwtd.exe'
$version = (Get-Content (Join-Path $checkout 'scripts/playwright-version.txt') -Raw).Trim()
$deps = Join-Path $env:LOCALAPPDATA "Temp/gwt-playwright-win-$version"
$realProfile = $env:USERPROFILE
$checkHome = Join-Path $checkout ('target/issue-5694-home-' + [Guid]::NewGuid().ToString('N'))
$process = $null
$daemon = $null
$keeper = $null
$savedEnvironment = @{}
$fixtureProject = Join-Path $checkHome 'project'
$headedReport = $env:GWT_HEADED_E2E_REPORT

function Invoke-CheckoutEnvelope($operation, $parameters) {
    if ($operation.StartsWith('issue.monitor.')) { $parameters.project_root = $fixtureProject }
    $envelope = @{schema_version=1;operation=$operation;params=$parameters} | ConvertTo-Json -Depth 8 -Compress
    $start = [Diagnostics.ProcessStartInfo]::new($gwtdBinary)
    $start.WorkingDirectory = $fixtureProject
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardInput = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
    $start.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $start.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    foreach ($key in @($start.Environment.Keys)) {
        if ($key.StartsWith('GWT_') -or $key -in @('GH_TOKEN','GITHUB_TOKEN','OPENAI_API_KEY','ANTHROPIC_API_KEY')) { $null = $start.Environment.Remove($key) }
    }
    $start.Environment['HOME'] = $checkHome
    $start.Environment['USERPROFILE'] = $checkHome
    $start.Environment['CODEX_HOME'] = "$checkHome/.codex"
    $start.Environment['CLAUDE_CONFIG_DIR'] = "$checkHome/.claude"
    $start.Environment['PATH'] = "$checkHome/bin;$env:PATH"
    if (Test-Path "$checkHome/bin/gh.cmd") { $start.Environment['GWT_TEST_GH'] = "$checkHome/bin/gh.cmd" }
    if ($script:hookBinary) { $start.Environment['GWT_HOOK_BIN'] = $script:hookBinary }
    $command = [Diagnostics.Process]::Start($start)
    try {
        $stdout = $command.StandardOutput.ReadToEndAsync()
        $stderr = $command.StandardError.ReadToEndAsync()
        $command.StandardInput.WriteLine($envelope)
        $command.StandardInput.Close()
        if (!$command.WaitForExit(60000)) { $command.Kill($true); throw "$operation timed out in the private fixture." }
        $result = $stdout.GetAwaiter().GetResult() | ConvertFrom-Json
        if ($command.ExitCode -ne 0 -or !$result.ok) { throw "$operation failed: $($result.output) $($stderr.GetAwaiter().GetResult())" }
        return $result
    } finally { $command.Dispose() }
}
function Assert-HookHealth($health) {
    $blocking = @($health.issues | Where-Object { $_ -notmatch '^managed hook failure: .* state=fail-open( |$)' })
    if ($health.status -eq 'inactive' -or $blocking.Count) { throw "Hook convergence failed: $($blocking -join ', ')" }
}
function Invoke-FixtureGit([string[]]$arguments) {
    $result = & git @arguments 2>&1
    if ($LASTEXITCODE -ne 0) { throw "Private fixture Git failed: $result" }
    return ($result -join "`n").Trim()
}
function Start-OwnedDaemon([string]$label) {
    $owned = Start-Process -FilePath $gwtdBinary -WorkingDirectory $fixtureProject -WindowStyle Hidden -PassThru -RedirectStandardInput "$checkHome/daemon-start.json" -RedirectStandardOutput "$checkHome/daemon-$label-stdout.log" -RedirectStandardError "$checkHome/daemon-$label-stderr.log"
    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    try {
        do {
            try { $status = (Invoke-CheckoutEnvelope 'daemon.status' @{}).output }
            catch {
                # A replacement may not have replayed the dead owner's fence
                # on its first readiness probe. Preserve the startup deadline.
                if ($_.Exception.Message -notmatch 'authority fence retains pending crash recovery') { throw }
                $status = $_.Exception.Message
            }
            if ($status -match '^running pid=(\d+).*probe=ok' -and [int]$Matches[1] -eq $owned.Id) { return $owned }
            if ($owned.HasExited -or [DateTime]::UtcNow -ge $deadline) { throw "Owned isolated daemon startup failed: $status" }
            Start-Sleep -Milliseconds 200
        } while ($true)
    } catch {
        if (!$owned.HasExited) { Stop-Process -Id $owned.Id }
        throw
    }
}

try {
    Set-Location $checkout
    if (!(Test-Path $gwtBinary) -or !(Test-Path $gwtdBinary)) { throw 'Build this checkout gwt and gwtd first.' }
    if (!(Test-Path "$deps/node_modules/@playwright/test/cli.js") -or !(Test-Path "$deps/resolve-playwright.cjs")) {
        throw "Install pinned Playwright $version and its resolver at $deps first."
    }
    New-Item -ItemType Directory -Path "$checkHome/.gwt" -Force | Out-Null
    $fixtureOrigin = Join-Path $checkHome 'origin.git'
    # Exercise real Git materialization without checking out this source tree
    # twelve times. All fixture refs, pushes and worktrees stay in this HOME.
    $null = Invoke-FixtureGit @('init', '--bare', '--quiet', $fixtureOrigin)
    $null = Invoke-FixtureGit @('init', '--quiet', '--initial-branch=develop', $fixtureProject)
    $null = Invoke-FixtureGit @('-C', $fixtureProject, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '--quiet', '--allow-empty', '-m', 'test: monitor delivery fixture')
    $null = Invoke-FixtureGit @('-C', $fixtureProject, 'remote', 'add', 'origin', $fixtureOrigin)
    $null = Invoke-FixtureGit @('-C', $fixtureProject, 'push', '--quiet', '--set-upstream', 'origin', 'develop')
    $null = Invoke-FixtureGit @('-C', $fixtureOrigin, 'symbolic-ref', 'HEAD', 'refs/heads/develop')
    $null = Invoke-FixtureGit @('-C', $fixtureProject, 'symbolic-ref', 'refs/remotes/origin/HEAD', 'refs/remotes/origin/develop')
    New-Item -ItemType Directory -Path "$fixtureProject/.claude", "$fixtureProject/.codex" -Force | Out-Null
    $privatePushUrl = Invoke-FixtureGit @('-C', $fixtureProject, 'remote', 'get-url', '--push', 'origin')
    if ([IO.Path]::GetFullPath($privatePushUrl) -ne [IO.Path]::GetFullPath($fixtureOrigin)) { throw 'Fixture origin escaped the isolated HOME.' }
    if (Test-Path "$realProfile/.gwt/runtime") {
        New-Item -ItemType Junction -Path "$checkHome/.gwt/runtime" -Target "$realProfile/.gwt/runtime" | Out-Null
    }
    $monitor = Invoke-CheckoutEnvelope 'issue.monitor.status' @{project_root=$fixtureProject}
    $repoHash = $monitor.project_store.hash
    if (!$repoHash) { throw 'Canonical project_store.hash missing.' }
    $projectState = "$checkHome/.gwt/projects/$repoHash/project-state"
    New-Item -ItemType Directory -Path $projectState -Force | Out-Null
    '{"settings":{"auto_start":false}}' | Set-Content "$projectState/pm.json" -Encoding utf8NoBOM
    $allowedLabel = 'browser-check-5694-' + [Guid]::NewGuid().ToString('N')
    # A future-version manual drain survives applied-version reconciliation.
    @{enabled=$false;autonomous_mode=$false;max_active_agents=1;priority_order=@();allowed_labels=@($allowedLabel);
        update_drain=@{reason='manual';version='999.0.0';since=[DateTime]::UtcNow.ToString('o');automatic=$false}} |
        ConvertTo-Json -Depth 6 | Set-Content "$projectState/issue-monitor.json" -Encoding utf8NoBOM
    @{project_root=$fixtureProject;repo_hash=$repoHash;allowed_label=$allowedLabel} |
        ConvertTo-Json | Set-Content "$checkHome/issue-5694-isolated.json" -Encoding utf8NoBOM
    @{active_tab_id='monitor-authority-5694';tabs=@(@{id='monitor-authority-5694';title='Monitor authority';project_root=$fixtureProject;kind='git'});recent_projects=@()} |
        ConvertTo-Json -Depth 6 | Set-Content "$checkHome/.gwt/session.json" -Encoding utf8NoBOM
    $workload = Join-Path $checkHome 'workload'
    New-Item -ItemType Directory -Path $workload, "$checkHome/bin" -Force | Out-Null
    # No GitHub CLI request may leave this disposable test process tree.
    "@echo off`r`nexit /b 64`r`n" | Set-Content "$checkHome/bin/gh.cmd" -Encoding ascii
    $provider = @'
const fs = require('node:fs');
const path = require('node:path');
const cp = require('node:child_process');
const readline = require('node:readline');
const session = process.env.GWT_SESSION_ID;
const directory = process.argv[2];
if (!session || !directory || !process.env.GWT_BIN_PATH) throw new Error('Regular gwt launch required');
fs.writeFileSync(path.join(directory, session + '.json'), JSON.stringify({ session_id: session,
  pid: process.pid, cwd: process.cwd(), cli_binary: process.env.GWT_BIN_PATH }));
process.stdout.write('GWT_5694_PROVIDER_READY ' + session + '\r\n');
readline.createInterface({ input: process.stdin }).on('line', line => {
  let command;
  try { command = JSON.parse(line); } catch { return; }
  if (command.kind !== 'probe' || typeof command.id !== 'string') return;
  const result = cp.spawnSync(process.env.GWT_BIN_PATH, [], { env: process.env, windowsHide: true,
    encoding: 'utf8', timeout: 30000, input: JSON.stringify({ schema_version: 1,
      operation: 'pane.read', params: { id: command.id, lines: 1 } }) });
  let ok = false;
  try { ok = result.status === 0 && JSON.parse(result.stdout).ok === true; } catch {}
  fs.writeFileSync(path.join(directory, session + '.probe.json'), JSON.stringify({ ok }));
  process.stdout.write('GWT_5694_PTY_' + session + '\r\n');
});
'@
    [IO.File]::WriteAllText("$workload/provider.cjs", $provider, [Text.UTF8Encoding]::new($false))
    $nodePath = (Get-Command node -CommandType Application).Source.Replace('\', '/')
    [IO.File]::WriteAllText("$checkHome/.gwt/config.toml", @"
[tools.customCodingAgents.monitor-delivery-fixture]
displayName = "Monitor delivery fixture"
agentType = "command"
command = "$nodePath"
defaultArgs = ["$($workload.Replace('\', '/'))/provider.cjs", "$($workload.Replace('\', '/'))"]
skipPermissionsArgs = ["--fixture-unattended"]
"@, [Text.UTF8Encoding]::new($false))
    $hook = (Invoke-CheckoutEnvelope 'hook.doctor' @{}).output | ConvertFrom-Json
    $hookBinary = $hook.managed_hook_bin
    if (!$hookBinary -or $hookBinary -match '[/\\]target[/\\].*[/\\]gwtd(\.exe)?$') { throw 'Stable hook fallback missing.' }
    $hookParams = @{repair=$true;expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}
    # Repair creates fresh hook files before trust can register their identities.
    $null = Invoke-CheckoutEnvelope 'hook.doctor' $hookParams
    $null = Invoke-CheckoutEnvelope 'hook.register_codex_managed_hook_trust' @{
        project_root=$fixtureProject;codex_config="$checkHome/.codex/config.toml";codex_hook_discovery='both'
    }
    Assert-HookHealth ((Invoke-CheckoutEnvelope 'hook.health' @{
        expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"
    }).output | ConvertFrom-Json)
    $childVariables = @{}
    # A fresh GUI must not inherit this execution's pane/session authority.
    Get-ChildItem Env:GWT_* | ForEach-Object { $childVariables[$_.Name] = $null }
    foreach ($key in @('GH_TOKEN','GITHUB_TOKEN','OPENAI_API_KEY','ANTHROPIC_API_KEY')) { $childVariables[$key] = $null }
    $childVariables.HOME = $checkHome
    $childVariables.USERPROFILE = $checkHome
    $childVariables.CODEX_HOME = "$checkHome/.codex"
    $childVariables.CLAUDE_CONFIG_DIR = "$checkHome/.claude"
    $childVariables.PATH = "$checkHome/bin;$env:PATH"
    $childVariables.GWT_TEST_GH = "$checkHome/bin/gh.cmd"
    $childVariables.GWT_BROWSER_URL_FILE = "$checkHome/url.txt"
    $childVariables.GWT_HOOK_BIN = $hookBinary
    # Settings verification does not exercise automatic repository indexing.
    $childVariables.GWT_DISABLE_BACKGROUND_INDEX = '1'
    # Codex's inherited RUST_LOG filters out gwt timing warnings by default.
    $childVariables.RUST_LOG = 'warn'
    $childVariables.GIT_TERMINAL_PROMPT = '0'
    $childVariables.GH_PROMPT_DISABLED = '1'
    foreach ($key in $childVariables.Keys) {
        $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key)
        [Environment]::SetEnvironmentVariable($key, $childVariables[$key])
    }
    '{"schema_version":1,"operation":"daemon.start","params":{}}' | Set-Content "$checkHome/daemon-start.json" -Encoding utf8NoBOM
    $daemon = Start-OwnedDaemon 'initial'
    $oldDaemonId = $daemon.Id
    $oldEpoch = [long](Get-Content "$projectState/issue-monitor.json" -Raw | ConvertFrom-Json).effect_authority_epoch
    $beforeStop = (Invoke-CheckoutEnvelope 'daemon.status' @{}).output
    if ($beforeStop -notmatch "^running pid=$oldDaemonId\b.*probe=ok") { throw 'Refusing to stop an unverified daemon owner.' }
    $recoveryStarted = [DateTime]::UtcNow.ToString('o')
    # Deliberate crash recovery, not cooperative shutdown: terminate only the
    # daemon whose fresh HOME/project IPC identity and owned PID were verified.
    Stop-Process -Id $oldDaemonId
    if (!$daemon.WaitForExit(5000)) { throw 'Owned previous daemon survived termination.' }
    $daemon = Start-OwnedDaemon 'replacement'
    $newEpoch = [long](Get-Content "$projectState/issue-monitor.json" -Raw | ConvertFrom-Json).effect_authority_epoch
    if ($daemon.Id -eq $oldDaemonId -or $newEpoch -le $oldEpoch) { throw 'Replacement daemon did not advance its PID and authority epoch.' }
    @{project_root=$fixtureProject;repo_hash=$repoHash;allowed_label=$allowedLabel;
        recovery=@{route='forced-owned-pid';previous_pid=$oldDaemonId;pid=$daemon.Id;previous_epoch=$oldEpoch;epoch=$newEpoch;started_at=$recoveryStarted}} |
        ConvertTo-Json -Depth 5 | Set-Content "$checkHome/issue-5694-isolated.json" -Encoding utf8NoBOM
    $process = Start-Process -FilePath $gwtBinary -ArgumentList '--no-tray','--no-open' -WorkingDirectory $checkout -WindowStyle Hidden -PassThru -RedirectStandardOutput "$checkHome/stdout.log" -RedirectStandardError "$checkHome/stderr.log"
    foreach ($key in $childVariables.Keys) { [Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key]) }
    Write-Output "Owned checkout gwt PID: $($process.Id); isolated HOME: $checkHome"
    $deadline = [DateTime]::UtcNow.AddSeconds(120)
    while (!(Test-Path "$checkHome/url.txt")) {
        if ($process.HasExited -or [DateTime]::UtcNow -ge $deadline) { throw "Checkout startup failed; inspect $checkHome" }
        Start-Sleep -Milliseconds 200
    }
    $baseUrl = (Get-Content "$checkHome/url.txt" -Raw).Trim()
    if ((Invoke-WebRequest -Method Head -Uri $baseUrl).StatusCode -ne 200) { throw 'Fresh checkout URL is not ready.' }
    $keeper = [System.Net.WebSockets.ClientWebSocket]::new()
    $keeper.Options.SetRequestHeader('Origin', $baseUrl.TrimEnd('/'))
    $null = $keeper.ConnectAsync([Uri](($baseUrl -replace '^http', 'ws') + 'ws'), [Threading.CancellationToken]::None).GetAwaiter().GetResult()
    $healthParams = @{expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}
    Assert-HookHealth ((Invoke-CheckoutEnvelope 'hook.health' $healthParams).output | ConvertFrom-Json)
    $testVariables = @{GWT_PLAYWRIGHT_BASE_URL=$baseUrl;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;
        GWT_PLAYWRIGHT_ISSUE_5694='1';GWT_PLAYWRIGHT_PID=[string]$process.Id;GWT_PLAYWRIGHT_CHECKOUT_ROOT=$checkout;
        GWT_PLAYWRIGHT_DAEMON_PID=[string]$daemon.Id;GWT_HEADED_E2E_REPORT=$headedReport;
        GWT_PLAYWRIGHT_PROJECT_ROOT=$fixtureProject;GWT_PLAYWRIGHT_NODE_MODULES="$deps/node_modules";
        NODE_OPTIONS="--require $($deps.Replace('\','/'))/resolve-playwright.cjs";NODE_PATH="$deps/node_modules"}
    foreach ($key in $testVariables.Keys) {
        $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key)
        [Environment]::SetEnvironmentVariable($key, $testVariables[$key])
    }
    & node "$deps/node_modules/@playwright/test/cli.js" test --config crates/gwt/playwright/playwright.config.ts --workers=1 "--project=$Theme" issue-monitor-authority-live.spec.ts @PlaywrightArguments
    if ($LASTEXITCODE -ne 0) { throw "Playwright failed with exit $LASTEXITCODE" }
} finally {
    try {
        if ($keeper) { $keeper.Dispose() }
        if ($process -and !$process.HasExited) { $null = $process.WaitForExit(12000) }
        if ($process -and !$process.HasExited) {
            & taskkill.exe /PID $process.Id /T /F 2>$null | Out-Null
            $null = $process.WaitForExit(5000)
            if (!$process.HasExited) { throw "Owned checkout PID $($process.Id) survived cleanup." }
        }
    } finally {
        try {
            if ($daemon -and !$daemon.HasExited) {
                Stop-Process -Id $daemon.Id
                if (!$daemon.WaitForExit(5000)) { throw 'Owned isolated daemon survived cleanup.' }
            }
        } finally {
            foreach ($key in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key]) }
            if ($process -and $daemon) {
                @{gui_pid=$process.Id;gui_exited=$process.HasExited;daemon_pid=$daemon.Id;daemon_exited=$daemon.HasExited;
                    checked_at=[DateTime]::UtcNow.ToString('o')} | ConvertTo-Json |
                    Set-Content "$checkHome/owned-pid-cleanup.json" -Encoding utf8NoBOM
            }
        }
    }
}
