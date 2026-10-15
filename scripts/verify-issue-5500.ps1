[CmdletBinding(PositionalBinding = $false)]
param(
    [switch]$MeasureOnly,
    [string]$MeasurementPath,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArgs
)
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$checkoutGwt = Join-Path $repoRoot 'target/debug/gwt.exe'
$checkoutGwtd = Join-Path $repoRoot 'target/debug/gwtd.exe'
if (!(Test-Path -LiteralPath $checkoutGwt) -or !(Test-Path -LiteralPath $checkoutGwtd)) {
    throw 'Build this checkout with cargo build -p gwt --bin gwt --bin gwtd first.'
}
$checkHome = Join-Path ([IO.Path]::GetTempPath()) ('gwt-fresh-home.5500-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path (Join-Path $checkHome '.gwt') -Force | Out-Null
$sessionsFixture = $env:GWT_CAPACITY_SESSIONS_FIXTURE
if ($sessionsFixture) {
    $sessionsFixture = (Resolve-Path -LiteralPath $sessionsFixture).Path
    $fixtureItems = @((Get-Item -LiteralPath $sessionsFixture)) + @(Get-ChildItem -LiteralPath $sessionsFixture -Force -Recurse)
    if (!(Test-Path -LiteralPath $sessionsFixture -PathType Container) -or @($fixtureItems | Where-Object {
        $_.Attributes -band [IO.FileAttributes]::ReparsePoint
    }).Count) { throw 'The sessions fixture must be a directory containing only copied files, without links or junctions.' }
    Copy-Item -LiteralPath $sessionsFixture -Destination (Join-Path $checkHome '.gwt/sessions') -Recurse -Force
}
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
@{active_tab_id='issue-5500';tabs=@(@{id='issue-5500';title='Issue 5500';project_root=$repoRoot;kind='git'});recent_projects=@()} |
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
function Measure-CapacityThread {
    if (-not ('GwtCapacityThreadNames' -as [type])) {
        Add-Type @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
public static class GwtCapacityThreadNames {
    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern IntPtr OpenThread(uint access, bool inherit, uint threadId);
    [DllImport("kernel32.dll")]
    private static extern int GetThreadDescription(IntPtr thread, out IntPtr description);
    [DllImport("kernel32.dll")]
    private static extern IntPtr LocalFree(IntPtr memory);
    [DllImport("kernel32.dll")]
    private static extern bool CloseHandle(IntPtr handle);
    public static string Get(uint threadId) {
        IntPtr handle = OpenThread(0x0800, false, threadId);
        if (handle == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
        IntPtr description = IntPtr.Zero;
        try {
            Marshal.ThrowExceptionForHR(GetThreadDescription(handle, out description));
            return Marshal.PtrToStringUni(description);
        } finally {
            if (description != IntPtr.Zero) LocalFree(description);
            CloseHandle(handle);
        }
    }
}
'@
    }
    $ownedGui.Refresh()
    $threads = @($ownedGui.Threads | Where-Object { [GwtCapacityThreadNames]::Get($_.Id) -eq 'gwt-agent-capacity' })
    if ($threads.Count -ne 1) { throw "Expected one gwt-agent-capacity thread, found $($threads.Count)." }
    $threadId = $threads[0].Id
    $cpuBefore = $threads[0].TotalProcessorTime.TotalSeconds
    $processCpuBefore = $ownedGui.TotalProcessorTime.TotalSeconds
    $startedAt = [DateTime]::UtcNow
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while ($timer.Elapsed.TotalSeconds -lt 60) {
        if ($ownedGui.HasExited) { throw 'Owned GUI exited during capacity measurement.' }
        Start-Sleep -Milliseconds 250
    }
    $ownedGui.Refresh()
    $thread = @($ownedGui.Threads | Where-Object { $_.Id -eq $threadId })
    if ($thread.Count -ne 1 -or [GwtCapacityThreadNames]::Get($threadId) -ne 'gwt-agent-capacity') {
        throw 'Capacity sampler thread identity changed during measurement.'
    }
    $cpuAfter = $thread[0].TotalProcessorTime.TotalSeconds
    $elapsed = $timer.Elapsed.TotalSeconds
    $result = [ordered]@{
        project=$repoRoot; binary=$checkoutGwt; binary_sha256=(Get-FileHash -LiteralPath $checkoutGwt -Algorithm SHA256).Hash
        pid=$ownedGui.Id; tid=$threadId
        thread_name='gwt-agent-capacity'; started_at=$startedAt.ToString('o'); elapsed_seconds=$elapsed
        total_processor_time_before_seconds=$cpuBefore; total_processor_time_after_seconds=$cpuAfter
        cpu_seconds=($cpuAfter - $cpuBefore); average_cores=(($cpuAfter - $cpuBefore) / $elapsed)
        process_cpu_seconds=($ownedGui.TotalProcessorTime.TotalSeconds - $processCpuBefore)
        sessions_fixture=$sessionsFixture; isolated_home=$checkHome
    }
    $outputPath = if ($MeasurementPath) { [IO.Path]::GetFullPath($MeasurementPath) } else { Join-Path $checkHome 'issue-5500-capacity-cpu.json' }
    $result | ConvertTo-Json | Set-Content -LiteralPath $outputPath
    Write-Output "Capacity measurement: $outputPath"
    $result | ConvertTo-Json -Compress | Write-Output
}
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
        Set-Content -LiteralPath (Join-Path $checkHome 'issue-5500-isolated.json')
    if ($MeasureOnly) { Measure-CapacityThread; return }
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
    & node $playwright test --config (Join-Path $repoRoot 'crates/gwt/playwright/playwright.config.ts') --workers=1 `
        --output (Join-Path $checkHome 'playwright') --headed `
        'issue-monitor-capacity-live.spec.ts' @PlaywrightArgs
    if ($LASTEXITCODE) { throw 'Issue #5500 headed verification failed.' }
} finally {
    foreach ($name in $savedVars.Keys) { [Environment]::SetEnvironmentVariable($name, $savedVars[$name], 'Process') }
    if ($anchorSocket) { $anchorSocket.Abort(); $anchorSocket.Dispose() }
    if ($ownedGui -and !$ownedGui.HasExited) { Stop-Process -Id $ownedGui.Id -Force; $ownedGui.WaitForExit(10000) | Out-Null }
    if ($ownedGui -and !$ownedGui.HasExited) { throw "Owned GUI PID $($ownedGui.Id) remains alive." }
    if ($stdoutTask) { $stdoutTask.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $checkHome 'startup.log') }
    if ($stderrTask) { $stderrTask.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $checkHome 'stderr.log') }
}
