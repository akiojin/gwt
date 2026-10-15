param([Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArgs)
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$checkoutGwt = Join-Path $repoRoot 'target/debug/gwt.exe'
$checkoutGwtd = Join-Path $repoRoot 'target/debug/gwtd.exe'
if (!(Test-Path -LiteralPath $checkoutGwt) -or !(Test-Path -LiteralPath $checkoutGwtd)) {
    throw 'Build this checkout with cargo build -p gwt --bin gwt --bin gwtd first.'
}
$checkHome = Join-Path ([IO.Path]::GetTempPath()) ('gwt-fresh-home.5404-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path (Join-Path $checkHome '.gwt') -Force | Out-Null
$realProfile = [Environment]::GetFolderPath('UserProfile')
foreach ($relative in @('.gwt/runtime', '.config', '.docker', '.codex', '.claude')) {
    $source = Join-Path $realProfile $relative
    if (Test-Path -LiteralPath $source -PathType Container) {
        New-Item -ItemType Junction -Path (Join-Path $checkHome $relative) -Target $source | Out-Null
    }
}
$statusJson = '{"schema_version":1,"operation":"issue.monitor.status","params":{}}' | & $checkoutGwtd
if ($LASTEXITCODE -ne 0) { throw "gwtd issue.monitor.status failed with exit code $LASTEXITCODE." }
$status = $statusJson | ConvertFrom-Json
if (!$status.ok -or !$status.project_store.hash) { throw 'Cannot resolve the canonical project hash.' }
$repoHash = $status.project_store.hash
$statePath = Join-Path $checkHome ".gwt/projects/$repoHash/project-state"
New-Item -ItemType Directory -Path $statePath -Force | Out-Null
'{"settings":{"auto_start":false}}' | Set-Content -LiteralPath (Join-Path $statePath 'pm.json')
'{"enabled":false,"max_active_agents":1,"priority_order":[]}' | Set-Content -LiteralPath (Join-Path $statePath 'issue-monitor.json')
# Seed only the documented Current projection. This shell summary has no
# provider Session or ledger record, and cannot launch an agent or process.
$fixtureTime = [DateTime]::UtcNow.ToString('o')
@{
    id='issue-5404-current';project_root=$repoRoot;title='Work projection fixture';
    status_category='idle';status_text='Ready';summary='Current projection fixture';
    owner=$null;next_action=$null;git_details=$null;board_refs=@();updated_at=$fixtureTime;
    agents=@(@{
        session_id='issue-5404-shell';agent_id='shell';display_name='Shell';
        status_category='idle';current_focus='Initial Current projection';
        title_summary='Work projection fixture';worktree_path=$null;
        branch='fixture/issue-5404';last_board_entry_id=$null;updated_at=$fixtureTime
    })
} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $statePath 'current.json')
@{active_tab_id='issue-5404';tabs=@(@{id='issue-5404';title='Issue 5404';project_root=$repoRoot;kind='git'});recent_projects=@()} |
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
$ownedGui = $null
$stdoutTask = $null
$stderrTask = $null
$anchorSocket = $null
$savedVars = @{}
try {
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
        Set-Content -LiteralPath (Join-Path $checkHome 'issue-5404-isolated.json')
    $version = (Get-Content -LiteralPath (Join-Path $repoRoot 'scripts/playwright-version.txt') -Raw).Trim()
    $deps = Join-Path ([IO.Path]::GetTempPath()) "gwt-playwright-$version"
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
    $testVars = @{GWT_PLAYWRIGHT_BASE_URL=$checkUrl;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;
        GWT_PLAYWRIGHT_PROJECT_ROOT=$repoRoot;GWT_PLAYWRIGHT_DEPS_DIR=$deps;
        NODE_OPTIONS=($env:NODE_OPTIONS + " --require=`"$($resolver.Replace('\', '/'))`"")}
    foreach ($name in $testVars.Keys) {
        $savedVars[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
        [Environment]::SetEnvironmentVariable($name, $testVars[$name], 'Process')
    }
    & node $playwright install chromium
    if ($LASTEXITCODE) { throw 'Chromium installation failed.' }
    # Keep evidence outside verify.run's disposable TEMP, as run-visual-tests.sh does.
    $artifacts = Join-Path $repoRoot ('crates/gwt/playwright/test-results/issue-5404-' + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $artifacts -Force | Out-Null
    & node $playwright test --config (Join-Path $repoRoot 'crates/gwt/playwright/playwright.config.ts') --workers=1 `
        --output $artifacts --grep 'Issue #5404' `
        'work-projection-memo-live.spec.ts' @PlaywrightArgs
    if ($LASTEXITCODE) { throw 'Issue #5404 headed verification failed.' }
} finally {
    foreach ($name in $savedVars.Keys) { [Environment]::SetEnvironmentVariable($name, $savedVars[$name], 'Process') }
    if ($anchorSocket) { $anchorSocket.Abort(); $anchorSocket.Dispose() }
    if ($ownedGui -and !$ownedGui.HasExited) { Stop-Process -Id $ownedGui.Id -Force; $ownedGui.WaitForExit(10000) | Out-Null }
    if ($ownedGui -and !$ownedGui.HasExited) { throw "Owned GUI PID $($ownedGui.Id) remains alive." }
    if ($stdoutTask) { $stdoutTask.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $checkHome 'startup.log') }
    if ($stderrTask) { $stderrTask.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $checkHome 'stderr.log') }
}
