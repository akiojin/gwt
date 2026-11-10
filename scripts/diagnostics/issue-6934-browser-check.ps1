#Requires -Version 7.0
# Issue #6934 AC-5. Build the checkout first; verify.run can forward --headed/reporter arguments.
[CmdletBinding(PositionalBinding = $false)]
param(
    [ValidateRange(1,3600)][int]$DurationSeconds = 1800,
    [string]$SourceAuditDirectory,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArguments
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This fixture requires Windows and PowerShell 7.' }
$checkout = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$gwt = Join-Path $checkout 'target/debug/gwt.exe'
$gwtd = Join-Path $checkout 'target/debug/gwtd.exe'
$node = (Get-Command node -CommandType Application).Source
$checkHome = Join-Path ([IO.Path]::GetTempPath()) ('gwt-owner-6934-' + [guid]::NewGuid().ToString('N'))
$gui = $null
$anchor = $null
$savedRunner = @{}
$exitCode = 1
foreach ($file in @($gwt, $gwtd)) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw 'Build this checkout with cargo build -p gwt --bin gwt --bin gwtd first.' }
}
function Invoke-SourceEnvelope([string]$Operation, [hashtable]$Parameters) {
    $request = @{schema_version=1;operation=$Operation;params=$Parameters} | ConvertTo-Json -Depth 8 -Compress
    if ($SourceAuditDirectory -and $Operation -ceq 'hook.doctor') {
        # A verification worker has no managed native ancestry. Ask the actual
        # caller for typed evidence without copying its authority to the target.
        $requestId = [guid]::NewGuid().ToString('N')
        $requestPath = Join-Path $SourceAuditDirectory "$requestId.request.json"
        $responsePath = Join-Path $SourceAuditDirectory "$requestId.response.json"
        [IO.File]::WriteAllText("$requestPath.tmp", $request, [Text.UTF8Encoding]::new($false))
        [IO.File]::Move("$requestPath.tmp", $requestPath)
        $deadline = [DateTime]::UtcNow.AddSeconds(180)
        while (-not (Test-Path -LiteralPath $responsePath)) {
            if ([DateTime]::UtcNow -ge $deadline) { throw 'Managed source hook audit timed out.' }
            Start-Sleep -Milliseconds 100
        }
        $response = Get-Content -LiteralPath $responsePath -Raw | ConvertFrom-Json
    } else { $response = $request | & $gwtd | ConvertFrom-Json }
    if (-not $response.ok) { throw "$Operation failed: $($response.output)" }
    return $response
}
function Assert-Preflight($Doctor) {
    $preflight = $Doctor.preflight
    $invalidDiagnostics = @($preflight.diagnostics | Where-Object {
        -not $_.id -or $_.classification -notin @('recovered', 'fail_open', 'foreign')
    })
    if ($preflight.schema_version -ne 1 -or $preflight.result -ne 'proceed' -or
        $preflight.context -ne 'current' -or $preflight.normal_completion -ne 'normal' -or
        $preflight.convergence -ne 'converged' -or @($preflight.reasons).Count -or $invalidDiagnostics.Count) {
        throw ('Managed-browser preflight failed: ' + ($preflight | ConvertTo-Json -Depth 8 -Compress))
    }
}
function New-IsolatedStart([string]$File, [string]$WorkingDirectory = $checkout) {
    $start = [Diagnostics.ProcessStartInfo]::new($File)
    $start.WorkingDirectory = $WorkingDirectory
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($key in @($start.Environment.Keys)) {
        if ($key -match '^(GWT_|CODEX_|CLAUDE_|GH_TOKEN$|GITHUB_TOKEN$|OPENAI_API_KEY$|ANTHROPIC_API_KEY$)') { [void]$start.Environment.Remove($key) }
    }
    $start.Environment['HOME'] = $checkHome
    $start.Environment['USERPROFILE'] = $checkHome
    $start.Environment['CODEX_HOME'] = "$checkHome/.codex"
    $start.Environment['CLAUDE_CONFIG_DIR'] = "$checkHome/.claude"
    $start.Environment['TEMP'] = "$checkHome/tmp"
    $start.Environment['TMP'] = "$checkHome/tmp"
    $start.Environment['RUST_LOG'] = 'info'
    $start.Environment['GIT_TERMINAL_PROMPT'] = '0'
    $start.Environment['GH_PROMPT_DISABLED'] = '1'
    $start.Environment['GIT_CONFIG_GLOBAL'] = "$checkHome/.gitconfig"
    $start.Environment['GIT_CONFIG_NOSYSTEM'] = '1'
    return $start
}
function Invoke-FixtureGit([string]$Project, [string[]]$Arguments) {
    $start = New-IsolatedStart (Get-Command git -CommandType Application).Source $Project
    foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::Start($start)
    try {
        $output = $process.StandardOutput.ReadToEndAsync()
        $errors = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(60000)) { $process.Kill($true); throw 'Disposable Git setup timed out.' }
        if ($process.ExitCode) { throw ('Disposable Git setup failed: ' + $errors.GetAwaiter().GetResult()) }
        return $output.GetAwaiter().GetResult().Trim()
    } finally { $process.Dispose() }
}
function Read-FixtureProjectHash([string]$Project) {
    $start = New-IsolatedStart $gwtd $Project
    $start.RedirectStandardInput = $true
    $start.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
    $process = [Diagnostics.Process]::Start($start)
    try {
        $output = $process.StandardOutput.ReadToEndAsync()
        $errors = $process.StandardError.ReadToEndAsync()
        $process.StandardInput.WriteLine('{"schema_version":1,"operation":"issue.monitor.status","params":{}}')
        $process.StandardInput.Close()
        if (-not $process.WaitForExit(60000)) { $process.Kill($true); throw 'Disposable Project identity timed out.' }
        $response = $output.GetAwaiter().GetResult() | ConvertFrom-Json
        if ($process.ExitCode -or -not $response.ok) { throw ('Disposable Project identity failed: ' + $errors.GetAwaiter().GetResult()) }
        return $response.project_store.hash
    } finally { $process.Dispose() }
}
Push-Location -LiteralPath $checkout
try {
    $doctor = (Invoke-SourceEnvelope 'hook.doctor' @{purpose='managed_browser'}).output | ConvertFrom-Json
    Assert-Preflight $doctor
    $hookBin = $doctor.managed_hook_bin
    if (-not $hookBin -or $hookBin -match '(?i)[/\\]target[/\\].*[/\\]gwtd(\.exe)?$') { throw 'Stable managed-hook fallback missing.' }
    Assert-Preflight ((Invoke-SourceEnvelope 'hook.doctor' @{purpose='managed_browser';repair=$true;expected_hook_bin=$hookBin}).output | ConvertFrom-Json)
    $project = Join-Path $checkHome 'project'
    [void](New-Item -ItemType Directory -Path $project -Force)
    [IO.File]::WriteAllText("$checkHome/.gitconfig", "[user]`nname = GwtFixture`nemail = fixture@example.invalid`n")
    # Only input repository setup is manual. All four work branches/worktrees
    # are subsequently created by the normal gwt Launch materialization.
    [void](Invoke-FixtureGit $project @('init','--quiet','--initial-branch=main'))
    [void](Invoke-FixtureGit $project @('commit','--allow-empty','--quiet','--no-gpg-sign','-m','chore: initialize owner fixture'))
    $fixtureOrigin = Join-Path $checkHome 'origin.git'
    [void](Invoke-FixtureGit $project @('clone','--bare','--quiet','.',$fixtureOrigin))
    [void](Invoke-FixtureGit $project @('remote','add','origin',$fixtureOrigin))
    $repoHash = Read-FixtureProjectHash $project
    if ($repoHash -notmatch '^[0-9a-f]{16}$') { throw 'Canonical project_store.hash unavailable.' }
    $branch = 'main'
    $state = Join-Path $checkHome ".gwt/projects/$repoHash/project-state"
    [void](New-Item -ItemType Directory -Path $state, "$checkHome/.codex", "$checkHome/.claude", "$checkHome/workload", "$checkHome/tmp" -Force)
    [IO.File]::WriteAllText("$checkHome/.codex/config.toml", '')
    [IO.File]::WriteAllText("$state/pm.json", '{"settings":{"auto_start":false}}')
    [IO.File]::WriteAllText("$state/issue-monitor.json", '{"enabled":false,"max_active_agents":1,"priority_order":[]}')
    [IO.File]::WriteAllText("$checkHome/.gwt/session.json", (@{active_tab_id='owner-6934';
        tabs=@(@{id='owner-6934';title='Owner recovery';project_root=$project;kind='git'});recent_projects=@()} | ConvertTo-Json -Depth 8))
    $runtime = Join-Path $env:USERPROFILE '.gwt/runtime'
    if (Test-Path -LiteralPath $runtime -PathType Container) {
        [void](New-Item -ItemType Junction -Path "$checkHome/.gwt/runtime" -Target $runtime)
    }
    # Four distinct custom Agents are real Session/owner/PTY launches on this
    # one disposable Project. No provider account or model is needed.
    $producer = @'
const fs = require('node:fs');
const path = require('node:path');
const readline = require('node:readline');
const index = Number(process.argv[2]);
const directory = path.dirname(__filename);
if (!process.env.GWT_SESSION_ID || !Number.isInteger(index) || index < 0 || index > 3)
  throw new Error('isolated custom Agent launch required');
fs.writeFileSync(path.join(directory, `${process.pid}.json`), JSON.stringify({ index, pid: process.pid,
  started_at: Date.now() - process.uptime() * 1000, session: process.env.GWT_SESSION_ID }));
process.stdout.write(`GWT6934_READY_${index}\r\n`);
let sequence = 0;
setInterval(() => process.stdout.write(`GWT6934_HEARTBEAT_${index}_${++sequence}\r\n`), 1000);
readline.createInterface({ input: process.stdin }).on('line', line => {
  const match = line.match(/^GWT6934_PING_([a-zA-Z0-9_]+)$/);
  if (match) {
    const [columns, rows] = process.stdout.getWindowSize();
    process.stdout.write(`GWT6934_ACK_${match[1]}:${columns}x${rows}\r\n`);
  }
});
'@
    [IO.File]::WriteAllText("$checkHome/workload/producer.cjs", $producer)
    $nodeToml = $node.Replace('\', '/')
    $producerToml = "$checkHome/workload/producer.cjs".Replace('\', '/')
    $agentsConfig = (0..3 | ForEach-Object { @"
[tools.customCodingAgents.owner-workload-6934-$_]
displayName = "Owner recovery workload"
agentType = "command"
command = "$nodeToml"
defaultArgs = ["$producerToml", "$_"]
"@ }) -join [Environment]::NewLine
    [IO.File]::WriteAllText("$checkHome/.gwt/config.toml", "auto_apply_updates = false`n" + $agentsConfig)
    $version = [IO.File]::ReadAllText((Join-Path $checkout 'scripts/playwright-version.txt')).Trim()
    $deps = if ($env:GWT_PLAYWRIGHT_DEPS_DIR) { $env:GWT_PLAYWRIGHT_DEPS_DIR } else { Join-Path $env:LOCALAPPDATA "Temp/gwt-playwright-win-$version" }
    $cli = Join-Path $deps 'node_modules/@playwright/test/cli.js'
    $resolver = Join-Path $deps 'resolve-playwright.cjs'
    foreach ($file in @($cli, $resolver)) { if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Pinned Playwright $version missing: $file" } }
    $start = New-IsolatedStart $gwt
    $start.ArgumentList.Add('--no-tray')
    $start.ArgumentList.Add('--no-open')
    $start.Environment['GWT_HOOK_BIN'] = $hookBin
    $start.Environment['GWT_BROWSER_URL_FILE'] = "$checkHome/url.txt"
    $start.Environment['GWT_DISABLE_BACKGROUND_INDEX'] = '1'
    $gui = [Diagnostics.Process]::Start($start)
    $stdout = $gui.StandardOutput.ReadToEndAsync()
    $stderr = $gui.StandardError.ReadToEndAsync()
    Write-Output "Owned checkout GUI PID: $($gui.Id); isolated HOME: $checkHome"
    $fixture = @{issue=6934;duration_seconds=$DurationSeconds;repo_hash=$repoHash;branch=$branch;project=$project;gui_pid=$gui.Id;
        gui_started_at=$gui.StartTime.ToUniversalTime().ToString('o');isolated_home=$checkHome;checkout=$checkout;
        checkout_head=(& git -C $checkout rev-parse HEAD).Trim();binary=$gwt;binary_sha256=(Get-FileHash -LiteralPath $gwt -Algorithm SHA256).Hash}
    [IO.File]::WriteAllText("$checkHome/fixture.json", ($fixture | ConvertTo-Json -Depth 8))
    $deadline = [DateTime]::UtcNow.AddSeconds(120)
    $baseUrl = ''
    do {
        if ($gui.HasExited) { throw 'Owned GUI exited during startup.' }
        if (Test-Path -LiteralPath "$checkHome/url.txt" -PathType Leaf) {
            $candidate = [IO.File]::ReadAllText("$checkHome/url.txt").Trim()
            try { if ((Invoke-WebRequest -Uri $candidate -Method Head -TimeoutSec 2).StatusCode -eq 200) { $baseUrl=$candidate; break } } catch {}
        }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    if (-not $baseUrl) { throw 'Fresh checkout readiness timed out.' }
    $anchor = [Net.WebSockets.ClientWebSocket]::new()
    $anchor.Options.SetRequestHeader('Origin', $baseUrl.TrimEnd('/'))
    $anchor.ConnectAsync([Uri](($baseUrl -replace '^http','ws') + 'ws'), [Threading.CancellationToken]::None).GetAwaiter().GetResult() | Out-Null
    Assert-Preflight ((Invoke-SourceEnvelope 'hook.doctor' @{purpose='managed_browser';expected_hook_bin=$hookBin}).output | ConvertFrom-Json)
    $runner = @{GWT_PLAYWRIGHT_BASE_URL=$baseUrl;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;GWT_PLAYWRIGHT_PROJECT_ROOT=$checkout;
        GWT_PLAYWRIGHT_NODE_MODULES=(Join-Path $deps 'node_modules');
        NODE_OPTIONS=('--require "' + $resolver.Replace('\','/') + '" ' + $env:NODE_OPTIONS)}
    foreach ($key in $runner.Keys) {
        $savedRunner[$key] = [Environment]::GetEnvironmentVariable($key)
        [Environment]::SetEnvironmentVariable($key, $runner[$key])
    }
    & $node $cli test --config scripts/diagnostics/issue-6934-owner-recovery.config.ts --workers=1 --retries=0 --headed --output "$checkHome/playwright" @PlaywrightArguments
    $exitCode = $LASTEXITCODE
} finally {
    foreach ($key in $savedRunner.Keys) { [Environment]::SetEnvironmentVariable($key, $savedRunner[$key]) }
    if ($anchor) { $anchor.Abort(); $anchor.Dispose() }
    $cleanupErrors = [Collections.Generic.List[string]]::new()
    $stopped = $false
    if ($gui) {
        try {
            if (-not $gui.HasExited) { $gui.Kill($true) }
            $stopped = $gui.WaitForExit(10000)
            if (-not $stopped) { $cleanupErrors.Add("Owned GUI PID $($gui.Id) remains.") }
            if ($stopped) {
                [IO.File]::WriteAllText("$checkHome/stdout.log", $stdout.GetAwaiter().GetResult())
                [IO.File]::WriteAllText("$checkHome/stderr.log", $stderr.GetAwaiter().GetResult())
            }
        } catch { $cleanupErrors.Add("Owned GUI cleanup failed: $($_.Exception.Message)") }
        finally { $gui.Dispose() }
    }
    # Owner daemons may outlive a GUI. Receipts bind only the exact workload
    # PIDs we created; never stop a process by executable name or pattern.
    if (Test-Path -LiteralPath "$checkHome/workload") {
        foreach ($file in Get-ChildItem -LiteralPath "$checkHome/workload" -File -Filter '*.json') {
            if ($file.BaseName -notmatch '^\d+$') { continue }
            try {
                $receipt = [IO.File]::ReadAllText($file.FullName) | ConvertFrom-Json
                $child = Get-Process -Id $receipt.pid -ErrorAction SilentlyContinue
                if ($child) {
                    $created = ([DateTimeOffset]$child.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
                    if ([Math]::Abs($created - $receipt.started_at) -lt 1000) {
                        $child.Kill($true)
                        if (-not $child.WaitForExit(10000)) { $cleanupErrors.Add("Owned workload PID $($receipt.pid) remains.") }
                    }
                }
            } catch { $cleanupErrors.Add("Owned workload cleanup failed: $($_.Exception.Message)") }
        }
    }
    if (Test-Path -LiteralPath $checkHome) {
        [IO.File]::WriteAllText("$checkHome/shutdown.json", (@{owned_gui_stopped=$stopped;test_exit_code=$exitCode;
            stopped_at=[DateTime]::UtcNow.ToString('o');cleanup_errors=@($cleanupErrors)} | ConvertTo-Json -Depth 4))
        Write-Output "Evidence: $checkHome"
    }
    Pop-Location
    if ($cleanupErrors.Count) { throw ($cleanupErrors -join '; ') }
}
exit $exitCode
