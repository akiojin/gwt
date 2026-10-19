# #5379: deterministic Windows PTY output and native GUI dispatch observation.
# Full comparison: dark 1800 seconds; light 10 seconds is functional evidence.
[CmdletBinding(PositionalBinding = $false)]
param(
    [ValidateSet('before','after')][string]$Arm = 'after',
    [ValidateRange(1,1800)][int]$DurationSeconds = $(if ($env:GWT5379_SECONDS) { [int]$env:GWT5379_SECONDS } else { 1800 }),
    [string]$BinaryPath,
    [string]$BaselineEvidence,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArguments
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This fixture requires Windows and PowerShell 7.' }
$checkout = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$target = (Resolve-Path (Join-Path $checkout 'target')).Path
$normalBinary = Join-Path $target 'debug/gwt.exe'
$binary = if ($BinaryPath) { (Resolve-Path -LiteralPath $BinaryPath).Path } else { $normalBinary }
if (-not $binary.StartsWith($target + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or
    [IO.Path]::GetFileName($binary) -ne 'gwt.exe') {
    throw 'BinaryPath must be this checkout target/debug/gwt.exe or a preserved gwt.exe under this checkout target directory.'
}
$gwtd = Join-Path $target 'debug/gwtd.exe'
foreach ($file in @($binary, $gwtd)) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw 'Build this checkout gwt/gwtd before browser-check.' }
}
if ($BaselineEvidence) { $BaselineEvidence = (Resolve-Path -LiteralPath $BaselineEvidence).Path }
$node = (Get-Command node -CommandType Application).Source
$artifactRoot = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Temp'
$checkHome = Join-Path $artifactRoot ("gwt-5379-$Arm-" + [guid]::NewGuid().ToString('N'))
foreach ($relative in @('.gwt', '.codex', '.claude', 'workload', 'tmp')) {
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
const { performance } = require('node:perf_hooks');
const directory = process.env.GWT5379_WORKLOAD;
const index = Number(process.argv[2]);
if (!directory || !process.env.GWT_SESSION_ID || !Number.isInteger(index) || index < 0 || index > 4) {
  throw new Error('isolated custom Agent launch required');
}
const identity = { index, pid: process.pid, started_at: Date.now() - process.uptime() * 1000,
  session: process.env.GWT_SESSION_ID };
const receipt = path.join(directory, String(process.pid) + '.json');
fs.writeFileSync(receipt, JSON.stringify(identity));
process.stdout.write('GWT5379_READY ' + index + '\r\n');
const delay = ms => new Promise(resolve => setTimeout(resolve, Math.max(0, ms)));
async function run() {
  const barrierPath = path.join(directory, 'barrier.json');
  while (!fs.existsSync(barrierPath)) await delay(100);
  const barrier = JSON.parse(fs.readFileSync(barrierPath, 'utf8'));
  if (!Number.isInteger(barrier.seconds) || barrier.seconds < 1 || barrier.seconds > 1800) throw new Error('bad barrier');
  const clockStart = performance.now() + barrier.start_at - Date.now();
  for (let sequence = 1; sequence <= barrier.seconds; sequence++) {
    await delay(clockStart + sequence * 1000 - performance.now());
    const marker = 'GWT5379_SEQ_' + index + '_' + String(sequence).padStart(4, '0');
    const chunk = ('x'.repeat(126) + '\r\n').repeat(63) + marker + 'x'.repeat(126 - marker.length) + '\r\n';
    if (Buffer.byteLength(chunk) !== 8192) throw new Error('incorrect workload size');
    await new Promise((resolve, reject) => process.stdout.write(chunk, error => error ? reject(error) : resolve()));
    fs.appendFileSync(receipt + '.progress', JSON.stringify({ run_id: barrier.run_id, sequence,
      bytes: sequence * 8192, observed_at: new Date().toISOString(), complete: sequence === barrier.seconds }) + '\n');
  }
}
run().catch(error => { console.error(error.message); process.exitCode = 1; process.stdin.destroy(); });
process.stdin.resume();
'@
[IO.File]::WriteAllText("$checkHome/workload/producer.cjs", $producer)
$nodeToml = $node.Replace('\', '/')
$producerToml = "$checkHome/workload/producer.cjs".Replace('\', '/')
[IO.File]::WriteAllText("$checkHome/.gwt/config.toml", ((0..4 | ForEach-Object { @"
[tools.customCodingAgents.runtime-output-5379-$_]
displayName = "Runtime output workload"
agentType = "command"
command = "$nodeToml"
defaultArgs = ["$producerToml", "$_"]
"@ }) -join [Environment]::NewLine))
$childEnvironment = @{}
foreach ($entry in [Environment]::GetEnvironmentVariables().GetEnumerator()) {
    if ($entry.Key -notmatch '^(GWT_|CODEX_|CLAUDE_|GH_TOKEN$|GITHUB_TOKEN$|OPENAI_API_KEY$|ANTHROPIC_API_KEY$)') {
        $childEnvironment[$entry.Key] = [string]$entry.Value
    }
}
$childEnvironment.HOME = $checkHome
$childEnvironment.USERPROFILE = $checkHome
$childEnvironment.CODEX_HOME = "$checkHome/.codex"
$childEnvironment.CLAUDE_CONFIG_DIR = "$checkHome/.claude"
$childEnvironment.TEMP = "$checkHome/tmp"
$childEnvironment.TMP = "$checkHome/tmp"
$childEnvironment.GWT_PROJECT_ROOT = $checkout
$childEnvironment.GWT5379_WORKLOAD = "$checkHome/workload"
$childEnvironment.GWT_DISABLE_BACKGROUND_INDEX = '1'
$childEnvironment.GIT_TERMINAL_PROMPT = '0'
$childEnvironment.GH_PROMPT_DISABLED = '1'
$childEnvironment.GIT_CONFIG_GLOBAL = "$checkHome/.gitconfig"
$childEnvironment.GIT_CONFIG_NOSYSTEM = '1'
$childEnvironment.RUST_LOG = 'gwt.frontend.timing=debug'
[IO.File]::WriteAllText("$checkHome/.gitconfig", "[user]" + [Environment]::NewLine +
    "name = GwtFixture" + [Environment]::NewLine + "email = fixture@example.invalid" + [Environment]::NewLine)
function New-IsolatedProcess([string]$FileName, [string]$WorkingDirectory = $checkout) {
    $start = [Diagnostics.ProcessStartInfo]::new($FileName)
    $start.WorkingDirectory = $WorkingDirectory
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $start.Environment.Clear()
    foreach ($key in $childEnvironment.Keys) { $start.Environment[$key] = $childEnvironment[$key] }
    return $start
}
function Invoke-IsolatedGwtd([string]$Operation, [hashtable]$Parameters, [string]$Project = $checkout) {
    $start = New-IsolatedProcess $gwtd $Project
    $start.Environment['GWT_PROJECT_ROOT'] = $Project
    $start.RedirectStandardInput = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
    $process = [Diagnostics.Process]::Start($start)
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
function Assert-HookHealth($Health) {
    $blocking = @($Health.issues | Where-Object { $_ -notmatch '^managed hook failure: .* state=fail-open( |$)' })
    if ($Health.status -eq 'inactive' -or $blocking.Count) { throw "Hook convergence failed: $($blocking -join '; ')" }
}
function Invoke-FixtureGit([string]$Project, [string[]]$Arguments) {
    $start = New-IsolatedProcess (Get-Command git -CommandType Application).Source $Project
    foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $process = [Diagnostics.Process]::Start($start)
    try {
        $output = $process.StandardOutput.ReadToEndAsync()
        $errors = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(60000)) { $process.Kill($true); throw 'Fixture Git setup timed out.' }
        if ($process.ExitCode -ne 0) { throw ('Fixture Git setup failed: ' + $errors.GetAwaiter().GetResult()) }
        return $output.GetAwaiter().GetResult().Trim()
    } finally { $process.Dispose() }
}
$resolver = Invoke-IsolatedGwtd 'hook.doctor' @{}
$hookBin = ($resolver.output | ConvertFrom-Json).managed_hook_bin
if (-not $hookBin -or $hookBin -match '(?i)[/\\]target[/\\].*[/\\]gwtd(\.exe)?$') { throw 'Stable hook fallback unavailable.' }
$childEnvironment.GWT_HOOK_BIN = $hookBin
$hookParams = @{expected_hook_bin=$hookBin;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}
[void](Invoke-IsolatedGwtd 'hook.doctor' ($hookParams + @{repair=$true}))
[void](Invoke-IsolatedGwtd 'hook.register_codex_managed_hook_trust' @{
    codex_config="$checkHome/.codex/config.toml";codex_hook_discovery='both';project_root=$checkout
})
Assert-HookHealth ((Invoke-IsolatedGwtd 'hook.health' $hookParams).output | ConvertFrom-Json)
$tabs = @()
$projects = @()
for ($index = 0; $index -lt 5; $index++) {
    $project = "$checkHome/workload/project-$index"
    [void](New-Item -ItemType Directory -Path $project -Force)
    # Disposable input only; gwt owns any subsequent Launch materialization.
    [void](Invoke-FixtureGit $project @('init','--quiet','--initial-branch=main'))
    [void](Invoke-FixtureGit $project @('commit','--allow-empty','--quiet','--no-gpg-sign','-m','chore: initialize output fixture'))
    # Local-only bare origin keeps any launch/materialization Git calls offline.
    $fixtureOrigin = "$checkHome/workload/origin-$index.git"
    [void](Invoke-FixtureGit $project @('clone','--bare','--quiet','.',$fixtureOrigin))
    [void](Invoke-FixtureGit $project @('remote','add','origin',$fixtureOrigin))
    $projectKey = (Invoke-IsolatedGwtd 'issue.monitor.status' @{} $project).project_store.hash
    if ($projectKey -notmatch '^[a-zA-Z0-9_-]+$') { throw 'Canonical ProjectKey unavailable.' }
    $state = "$checkHome/.gwt/projects/$projectKey/project-state"
    [void](New-Item -ItemType Directory -Path $state -Force)
    [IO.File]::WriteAllText("$state/pm.json", '{"settings":{"auto_start":false}}')
    [IO.File]::WriteAllText("$state/issue-monitor.json", '{"enabled":false,"max_active_agents":1,"priority_order":[]}')
    $tabs += @{id="output-$index";title="Output $index";project_root=$project;kind='git'}
    $projects += @{path=$project;project_key=$projectKey;branch_name='main'}
}
[IO.File]::WriteAllText("$checkHome/.gwt/session.json", (@{active_tab_id='output-0';tabs=$tabs;recent_projects=@()} | ConvertTo-Json -Depth 8))
$gui = $null
$anchor = $null
$savedRunner = @{}
$exitCode = 1
try {
    $start = New-IsolatedProcess $binary
    $start.ArgumentList.Add('--no-tray')
    $start.ArgumentList.Add('--no-open')
    $start.Environment['GWT_BROWSER_URL_FILE'] = "$checkHome/url"
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $gui = [Diagnostics.Process]::Start($start)
    $stdout = $gui.StandardOutput.ReadToEndAsync()
    $stderr = $gui.StandardError.ReadToEndAsync()
    $fixture = @{issue=5379;arm=$Arm;duration_seconds=$DurationSeconds;projects=$projects;gui_pid=$gui.Id;
        gui_started_at=$gui.StartTime.ToUniversalTime().ToString('o');isolated_home=$checkHome;checkout=$checkout;
        checkout_head=(& git -C $checkout rev-parse HEAD).Trim();binary=$binary;
        binary_sha256=(Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash;
        binary_origin=$(if ($binary -eq $normalBinary) { 'current checkout target/debug' } else { 'preserved checkout target binary; source provenance supplied by owner' })}
    [IO.File]::WriteAllText("$checkHome/fixture.json", ($fixture | ConvertTo-Json -Depth 8))
    Write-Output "Owned checkout PID: $($gui.Id); isolated HOME: $checkHome"
    $deadline = [DateTime]::UtcNow.AddSeconds(120)
    $baseUrl = ''
    do {
        if ($gui.HasExited) { throw 'Owned GUI exited during startup.' }
        if (Test-Path -LiteralPath "$checkHome/url" -PathType Leaf) {
            $candidate = [IO.File]::ReadAllText("$checkHome/url").Trim()
            try { if ((Invoke-WebRequest -Uri $candidate -Method Head -TimeoutSec 2).StatusCode -eq 200) { $baseUrl = $candidate; break } } catch {}
        }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    if (-not $baseUrl) { throw 'Fresh checkout readiness timed out.' }
    $anchor = [Net.WebSockets.ClientWebSocket]::new()
    $anchor.Options.SetRequestHeader('Origin', $baseUrl.TrimEnd('/'))
    $anchor.ConnectAsync([Uri](($baseUrl -replace '^http','ws') + 'ws'), [Threading.CancellationToken]::None).GetAwaiter().GetResult() | Out-Null
    Assert-HookHealth ((Invoke-IsolatedGwtd 'hook.health' $hookParams).output | ConvertFrom-Json)
    $version = [IO.File]::ReadAllText((Join-Path $checkout 'scripts/playwright-version.txt')).Trim()
    $deps = if ($env:GWT_PLAYWRIGHT_DEPS_DIR) { $env:GWT_PLAYWRIGHT_DEPS_DIR } else { Join-Path $artifactRoot "gwt-playwright-$version" }
    $cli = Join-Path $deps 'node_modules/playwright/cli.js'
    $moduleResolver = Join-Path $deps 'resolve-playwright.cjs'
    if (-not (Test-Path -LiteralPath $cli) -or -not (Test-Path -LiteralPath $moduleResolver)) { throw "Pinned Playwright $version missing at $deps." }
    $runner = @{GWT_PLAYWRIGHT_BASE_URL=$baseUrl;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;GWT_PLAYWRIGHT_PROJECT_ROOT=$checkout;
        GWT5379_BASELINE=$BaselineEvidence;GWT_PLAYWRIGHT_NODE_MODULES=(Join-Path $deps 'node_modules');
        NODE_OPTIONS=('--require "' + $moduleResolver.Replace('\','/') + '" ' + $env:NODE_OPTIONS)}
    foreach ($key in $runner.Keys) {
        $savedRunner[$key] = [Environment]::GetEnvironmentVariable($key)
        [Environment]::SetEnvironmentVariable($key, $runner[$key])
    }
    Push-Location $checkout
    try {
        & $node $cli test --config crates/gwt/playwright/playwright.config.ts --workers=1 --timeout (($DurationSeconds + 900) * 1000) --output "$checkHome/playwright" runtime-output-batch-live.spec.ts @PlaywrightArguments
        $exitCode = $LASTEXITCODE
    } finally { Pop-Location }
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
                [IO.File]::WriteAllText("$checkHome/startup.log", $stdout.GetAwaiter().GetResult())
                [IO.File]::WriteAllText("$checkHome/stderr.log", $stderr.GetAwaiter().GetResult())
            }
        } catch { $cleanupErrors.Add("Owned GUI cleanup failed: $($_.Exception.Message)") }
        finally { $gui.Dispose() }
    }
    foreach ($file in Get-ChildItem -LiteralPath "$checkHome/workload" -File -Filter '*.json') {
        if ($file.BaseName -notmatch '^\d+$') { continue }
        try {
            $receipt = [IO.File]::ReadAllText($file.FullName) | ConvertFrom-Json
            $producerProcess = Get-Process -Id $receipt.pid -ErrorAction SilentlyContinue
            if ($producerProcess) {
                $created = ([DateTimeOffset]$producerProcess.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
                if ([Math]::Abs($created - $receipt.started_at) -lt 1000) {
                    $producerProcess.Kill($true)
                    if (-not $producerProcess.WaitForExit(10000)) { $cleanupErrors.Add("Owned producer PID $($receipt.pid) remains.") }
                }
            }
        } catch { $cleanupErrors.Add("Owned producer cleanup failed: $($_.Exception.Message)") }
    }
    [IO.File]::WriteAllText("$checkHome/shutdown.json", (@{owned_gui_stopped=$stopped;test_exit_code=$exitCode;
        stopped_at=[DateTime]::UtcNow.ToString('o');cleanup_errors=@($cleanupErrors)} | ConvertTo-Json -Depth 4))
    Write-Output "Evidence: $checkHome"
    if ($cleanupErrors.Count) { throw ($cleanupErrors -join '; ') }
}
exit $exitCode
