param(
    [switch]$CandidateSettings,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArgs
)
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$checkoutGwt = Join-Path $repoRoot 'target/debug/gwt.exe'
$checkoutGwtd = Join-Path $repoRoot 'target/debug/gwtd.exe'
if (!(Test-Path -LiteralPath $checkoutGwt) -or !(Test-Path -LiteralPath $checkoutGwtd)) {
    throw 'Build this checkout with cargo build -p gwt --bin gwt --bin gwtd first.'
}
$checkHome = Join-Path ([IO.Path]::GetTempPath()) ('gwt-fresh-home.5503-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path (Join-Path $checkHome '.gwt') -Force | Out-Null
$realProfile = [Environment]::GetFolderPath('UserProfile')
foreach ($relative in @('.gwt/runtime', '.config', '.docker')) {
    $source = Join-Path $realProfile $relative
    if (Test-Path -LiteralPath $source -PathType Container) {
        New-Item -ItemType Junction -Path (Join-Path $checkHome $relative) -Target $source | Out-Null
    }
}
# Resolve canonical project identity locally before starting the isolated runtime.
$identityJson = '{"schema_version":1,"operation":"issue.monitor.profiles","params":{}}' | & $checkoutGwtd
if ($LASTEXITCODE -ne 0) { throw "gwtd issue.monitor.profiles failed with exit code $LASTEXITCODE." }
$identity = $identityJson | ConvertFrom-Json
if (!$identity.ok -or !$identity.project_store.hash) { throw 'Cannot resolve the canonical project hash.' }
$repoHash = $identity.project_store.hash
$statePath = Join-Path $checkHome ".gwt/projects/$repoHash/project-state"
New-Item -ItemType Directory -Path $statePath -Force | Out-Null
'{"settings":{"auto_start":false}}' | Set-Content -LiteralPath (Join-Path $statePath 'pm.json')
'{"enabled":false,"max_active_agents":1,"priority_order":[]}' | Set-Content -LiteralPath (Join-Path $statePath 'issue-monitor.json')
@{active_tab_id='issue-5503';tabs=@(@{id='issue-5503';title='Issue 5503';project_root=$repoRoot;kind='git'});recent_projects=@()} |
    ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $checkHome '.gwt/session.json')
$hookBin = $env:GWT_HOOK_BIN
if ($hookBin -match '[/\\]target[/\\]' -or ($hookBin -and $hookBin -ne 'gwtd' -and !(Test-Path -LiteralPath $hookBin -PathType Leaf))) {
    $hookBin = $null
}
function Invoke-HookOperation($operation, $parameters) {
    $previousBin = $env:GWT_BIN_PATH
    $previousHook = $env:GWT_HOOK_BIN
    try {
        $env:GWT_BIN_PATH = $null
        $env:GWT_HOOK_BIN = $hookBin
        $result = @{schema_version=1;operation=$operation;params=$parameters} |
            ConvertTo-Json -Depth 5 -Compress | & $checkoutGwtd
        if ($LASTEXITCODE -ne 0) { throw "gwtd $operation failed with exit code $LASTEXITCODE." }
        $envelope = $result | ConvertFrom-Json
        if (!$envelope.ok) { throw $envelope.error }
        return ($envelope.output | ConvertFrom-Json)
    } finally {
        $env:GWT_BIN_PATH = $previousBin
        $env:GWT_HOOK_BIN = $previousHook
    }
}
if (!$hookBin) { $hookBin = (Invoke-HookOperation 'hook.doctor' @{}).managed_hook_bin }
if (!$hookBin -or $hookBin -match '[/\\]target[/\\]') { throw 'Managed hook resolver did not return a stable fallback.' }
function Assert-HookHealth($health) {
    $blockingIssues = @($health.issues | Where-Object {
        $_ -notmatch '^managed hook failure: .* state=fail-open( |$)' -and
        !($hookBin -eq 'gwtd' -and !(Get-Command gwtd -ErrorAction SilentlyContinue) -and
            $_ -match '^managed hook binary missing: .* uses gwtd$')
    })
    if ($health.status -eq 'inactive' -or $blockingIssues.Count) {
        throw ('Hook convergence failed: ' + ($health | ConvertTo-Json -Depth 5 -Compress))
    }
}
$hookParams = @{expected_hook_bin=$hookBin;runtime_state_path=(Join-Path $checkHome '.gwt/missing-runtime-state.json')}
Assert-HookHealth (Invoke-HookOperation 'hook.doctor' ($hookParams + @{repair=$true})).health
$isolatedEnv = @{}
Get-ChildItem Env: | Where-Object { $_.Name -notmatch '^(GWT_|CODEX_|CLAUDE_)' } | ForEach-Object { $isolatedEnv[$_.Name] = $_.Value }
$isolatedEnv['HOME'] = $checkHome
$isolatedEnv['USERPROFILE'] = $checkHome
$isolatedEnv['GWT_BROWSER_URL_FILE'] = Join-Path $checkHome 'url'
$isolatedEnv['GWT_HOOK_BIN'] = $hookBin
$isolatedEnv['GIT_TERMINAL_PROMPT'] = '0'
if ($CandidateSettings) {
    $isolatedEnv['GWT_DISABLE_BACKGROUND_INDEX'] = '1'
    $isolatedEnv['RUST_LOG'] = 'warn,gwt.process.summary=info'
    # Load custom-agent configuration before either runtime caches its catalog.
    @'
[tools.customCodingAgents.fixture-candidate]
id = "fixture-candidate"
displayName = "Fixture Candidate"
agentType = "command"
command = "fixture-candidate-cli"
'@ | Set-Content -LiteralPath (Join-Path $checkHome '.gwt/config.toml')
    $isolatedEnv['HERMES_HOME'] = Join-Path $checkHome 'hermes-fixture'
    New-Item -ItemType Directory -Path $isolatedEnv['HERMES_HOME'] -Force | Out-Null
    "model:`n  provider: fixture`n  default: fixture-model`nproviders:`n  fixture:`n    models: [fixture-model, fixture-alternative]" |
        Set-Content -LiteralPath (Join-Path $isolatedEnv['HERMES_HOME'] 'config.yaml')
}
# Provider UI fixtures need no live credentials or transcript data.
$isolatedEnv['CODEX_HOME'] = Join-Path $checkHome 'codex-fixture'
$isolatedEnv['CLAUDE_CONFIG_DIR'] = Join-Path $checkHome 'claude-fixture'
New-Item -ItemType Directory -Path $isolatedEnv['CODEX_HOME'], $isolatedEnv['CLAUDE_CONFIG_DIR'] -Force | Out-Null
$ownedGui = $null
$ownedDaemon = $null
$daemonStdoutTask = $null
$daemonStderrTask = $null
$stdoutTask = $null
$stderrTask = $null
$anchorSocket = $null
$savedVars = @{}
# Preserve diagnostics even when verify.run removes its disposable TEMP.
$artifacts = Join-Path $repoRoot ('crates/gwt/playwright/test-results/issue-5503-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $artifacts -Force | Out-Null
try {
    if ($CandidateSettings) {
        # Profile writes use the production daemon transport. The 250ms GUI
        # fallback is a separate bounded recovery path, not this fixture's subject.
        $daemonInfo = [Diagnostics.ProcessStartInfo]::new($checkoutGwtd)
        $daemonInfo.WorkingDirectory = $repoRoot
        $daemonInfo.UseShellExecute = $false
        $daemonInfo.CreateNoWindow = $true
        $daemonInfo.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
        $daemonInfo.RedirectStandardInput = $true
        $daemonInfo.RedirectStandardOutput = $true
        $daemonInfo.RedirectStandardError = $true
        $daemonInfo.Environment.Clear()
        foreach ($name in $isolatedEnv.Keys) { $daemonInfo.Environment[$name] = $isolatedEnv[$name] }
        $ownedDaemon = [Diagnostics.Process]::Start($daemonInfo)
        $daemonStderrTask = $ownedDaemon.StandardError.ReadToEndAsync()
        $ownedDaemon.StandardInput.WriteLine('{"schema_version":1,"operation":"daemon.start","params":{}}')
        $ownedDaemon.StandardInput.Close()
        foreach ($expected in @('gwtd daemon start: bind=', "gwtd daemon start: pid=$($ownedDaemon.Id) ")) {
            $lineTask = $ownedDaemon.StandardOutput.ReadLineAsync()
            if (!$lineTask.Wait(30000) -or !$lineTask.Result -or !$lineTask.Result.StartsWith($expected)) { throw 'Isolated daemon did not become ready.' }
        }
        $daemonStdoutTask = $ownedDaemon.StandardOutput.ReadToEndAsync()
        $readyDeadline = [DateTime]::UtcNow.AddSeconds(30)
        do {
            $probeInfo = [Diagnostics.ProcessStartInfo]::new($checkoutGwtd)
            $probeInfo.WorkingDirectory = $repoRoot
            $probeInfo.UseShellExecute = $false
            $probeInfo.CreateNoWindow = $true
            $probeInfo.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
            $probeInfo.RedirectStandardInput = $true
            $probeInfo.RedirectStandardOutput = $true
            $probeInfo.RedirectStandardError = $true
            $probeInfo.Environment.Clear()
            foreach ($name in $isolatedEnv.Keys) { $probeInfo.Environment[$name] = $isolatedEnv[$name] }
            $probe = [Diagnostics.Process]::Start($probeInfo)
            try {
                $probeOutput = $probe.StandardOutput.ReadToEndAsync()
                $probeError = $probe.StandardError.ReadToEndAsync()
                $probe.StandardInput.WriteLine('{"schema_version":1,"operation":"daemon.status","params":{}}')
                $probe.StandardInput.Close()
                if (!$probe.WaitForExit(10000)) { throw 'Isolated daemon status probe timed out.' }
                if ($probe.ExitCode) { throw $probeError.GetAwaiter().GetResult() }
                $daemonStatus = ($probeOutput.GetAwaiter().GetResult() | ConvertFrom-Json).output
            } finally {
                if (!$probe.HasExited) { $probe.Kill(); $probe.WaitForExit(10000) | Out-Null }
                $probe.Dispose()
            }
            if ($daemonStatus -match '^running pid=(\d+).*probe=ok') {
                if ([int]$Matches[1] -ne $ownedDaemon.Id) { throw 'Isolated daemon PID does not match its owner.' }
                break
            }
            if ($ownedDaemon.HasExited -or [DateTime]::UtcNow -ge $readyDeadline) { throw 'Isolated daemon did not pass its authenticated status probe.' }
            Start-Sleep -Milliseconds 200
        } while ($true)
        Write-Output "Fresh checkout daemon PID: $($ownedDaemon.Id); isolated HOME: $checkHome"
    }
    $startInfo = [Diagnostics.ProcessStartInfo]::new($checkoutGwt)
    $startInfo.WorkingDirectory = $repoRoot
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.ArgumentList.Add('--no-tray')
    $startInfo.ArgumentList.Add('--no-open')
    $startInfo.Environment.Clear()
    foreach ($name in $isolatedEnv.Keys) { $startInfo.Environment[$name] = $isolatedEnv[$name] }
    $ownedGui = [Diagnostics.Process]::Start($startInfo)
    $stdoutTask = $ownedGui.StandardOutput.ReadToEndAsync()
    $stderrTask = $ownedGui.StandardError.ReadToEndAsync()
    Write-Output "Fresh checkout PID: $($ownedGui.Id); isolated HOME: $checkHome"
    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    $checkUrl = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($ownedGui.HasExited) { throw ('Fresh gwt exited: ' + $stderrTask.GetAwaiter().GetResult()) }
        if (Test-Path -LiteralPath (Join-Path $checkHome 'url')) {
            $candidate = (Get-Content -LiteralPath (Join-Path $checkHome 'url') -Raw).Trim()
            try { if ((Invoke-WebRequest -Uri $candidate -Method Head -TimeoutSec 2).StatusCode -eq 200) { $checkUrl = $candidate; break } } catch {}
        }
        Start-Sleep -Milliseconds 100
    }
    if (!$checkUrl) { throw 'Fresh checkout URL never became ready.' }
    # The transient no-tray server follows browser lifetime. Retain one owned
    # connection across Playwright worker/browser restarts, then release it.
    $anchorSocket = [Net.WebSockets.ClientWebSocket]::new()
    $anchorUrl = [UriBuilder]::new($checkUrl)
    $anchorUrl.Scheme = 'ws'
    $anchorUrl.Path = '/ws'
    $anchorSocket.ConnectAsync($anchorUrl.Uri, [Threading.CancellationToken]::None).GetAwaiter().GetResult() | Out-Null
    Assert-HookHealth (Invoke-HookOperation 'hook.health' $hookParams)
    @{project=$repoRoot;repo_hash=$repoHash;pid=$ownedGui.Id;url=$checkUrl} | ConvertTo-Json |
        Set-Content -LiteralPath (Join-Path $checkHome 'issue-5503-isolated.json')
    $version = (Get-Content -LiteralPath (Join-Path $repoRoot 'scripts/playwright-version.txt') -Raw).Trim()
    $deps = if ($env:GWT_PLAYWRIGHT_DEPS_DIR) { $env:GWT_PLAYWRIGHT_DEPS_DIR } else {
        Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) "Temp/gwt-playwright-$version"
    }
    $playwright = Join-Path $deps 'node_modules/@playwright/test/cli.js'
    if (!(Test-Path -LiteralPath $playwright)) {
        New-Item -ItemType Directory -Path $deps -Force | Out-Null
        @{private=$true;dependencies=@{'@playwright/test'=$version}} | ConvertTo-Json | Set-Content (Join-Path $deps 'package.json')
        $npmCli = Join-Path (Split-Path (Get-Command node).Source) 'node_modules/npm/bin/npm-cli.js'
        & node $npmCli install --prefix $deps --silent --no-audit --no-fund --package-lock=false
        if ($LASTEXITCODE) { throw 'Pinned Playwright installation failed.' }
    }
    $resolver = Join-Path $checkHome 'playwright-resolver.cjs'
    @'
const Module = require('module');
const path = require('path');
const original = Module._resolveFilename;
Module._resolveFilename = function(request, parent, ...rest) {
  if (request === '@playwright/test' || request.startsWith('@playwright/test/')) {
    return original.call(this, path.join(process.env.GWT_PLAYWRIGHT_DEPS_DIR, 'node_modules', request), parent, ...rest);
  }
  return original.call(this, request, parent, ...rest);
};
'@ | Set-Content -LiteralPath $resolver
    # The existing provider fixture mounts a workspace, so enter its Project route.
    $projectUrl = [Uri]::new([Uri]$checkUrl, "p/$repoHash").AbsoluteUri
    if ((Invoke-WebRequest -Uri $projectUrl -Method Head -TimeoutSec 5).StatusCode -ne 200) { throw 'Fresh Project route is unavailable.' }
    $testVars = @{GWT_PLAYWRIGHT_BASE_URL=$projectUrl;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;
        GWT_PLAYWRIGHT_PROJECT_ROOT=$repoRoot;GWT_PLAYWRIGHT_DEPS_DIR=$deps;
        NODE_OPTIONS=($env:NODE_OPTIONS + " --require=`"$($resolver.Replace('\', '/'))`"")}
    foreach ($name in $testVars.Keys) {
        $savedVars[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
        [Environment]::SetEnvironmentVariable($name, $testVars[$name], 'Process')
    }
    & node $playwright install chromium
    if ($LASTEXITCODE) { throw 'Chromium installation failed.' }
    $specs = if ($CandidateSettings) { @('settings-issues-live.spec.ts', 'issue-monitor-candidate-pool-live.spec.ts', '--grep', 'Issue #5787|Issue Monitor candidate pool') } else { @('provider-usage.spec.ts') }
    & node $playwright test --config (Join-Path $repoRoot 'crates/gwt/playwright/playwright.config.ts') --workers=1 `
        --output $artifacts `
        @specs @PlaywrightArgs
    if ($LASTEXITCODE) { throw 'Issue #5503 headed verification failed.' }
} finally {
    foreach ($name in $savedVars.Keys) { [Environment]::SetEnvironmentVariable($name, $savedVars[$name], 'Process') }
    foreach ($ownedProcess in @($ownedGui, $ownedDaemon)) {
        if (!$ownedProcess) { continue }
        try {
            if (!$ownedProcess.HasExited) { $ownedProcess.Kill(); $ownedProcess.WaitForExit(10000) | Out-Null }
        } catch {
            # A child can exit between HasExited and Kill. Check both children
            # after attempting every owned handle, even if one stop failed.
        }
    }
    if ($anchorSocket) { $anchorSocket.Abort(); $anchorSocket.Dispose() }
    $remaining = @(@($ownedGui, $ownedDaemon) | Where-Object { $_ -and !$_.HasExited })
    if ($remaining.Count) { throw "Owned PID(s) remain alive: $(($remaining | ForEach-Object { $_.Id }) -join ', ')." }
    if ($stdoutTask) { $stdoutTask.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $artifacts 'startup.log') }
    if ($stderrTask) { $stderrTask.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $artifacts 'stderr.log') }
    if ($daemonStdoutTask) { $daemonStdoutTask.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $artifacts 'daemon-startup.log') }
    if ($daemonStderrTask) { $daemonStderrTask.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $artifacts 'daemon-stderr.log') }
    if ($daemonStatus) { $daemonStatus | Set-Content -LiteralPath (Join-Path $artifacts 'daemon-status.txt') }
    $fixtureLogs = Join-Path $checkHome '.gwt/logs'
    if (Test-Path -LiteralPath $fixtureLogs -PathType Container) {
        Copy-Item -LiteralPath $fixtureLogs -Destination (Join-Path $artifacts 'fixture-logs') -Recurse
    }
    Write-Output "Browser evidence: $artifacts"
}
