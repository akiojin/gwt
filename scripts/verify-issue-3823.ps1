[CmdletBinding(PositionalBinding = $false)]
param(
    [string]$EvidenceDirectory,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArgs
)
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (!$EvidenceDirectory) { $EvidenceDirectory = Join-Path $repoRoot 'target/verification/issue-3823/headed' }
$EvidenceDirectory = [IO.Path]::GetFullPath($EvidenceDirectory)
New-Item -ItemType Directory -Path $EvidenceDirectory -Force | Out-Null
$checkoutGwt = Join-Path $repoRoot 'target/debug/gwt.exe'
$checkoutGwtd = Join-Path $repoRoot 'target/debug/gwtd.exe'
if (!(Test-Path $checkoutGwt) -or !(Test-Path $checkoutGwtd)) {
    throw 'Build this checkout with cargo build -p gwt --bin gwt --bin gwtd first.'
}
$checkHome = Join-Path ([IO.Path]::GetTempPath()) ('gwt-fresh-home.3823-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path (Join-Path $checkHome '.gwt') -Force | Out-Null
$realProfile = [Environment]::GetFolderPath('UserProfile')
foreach ($relative in @('.gwt/runtime', '.config', '.docker', '.codex', '.claude')) {
    $source = Join-Path $realProfile $relative
    if (Test-Path -LiteralPath $source -PathType Container) {
        New-Item -ItemType Junction -Path (Join-Path $checkHome $relative) -Target $source | Out-Null
    }
}
$status = '{"schema_version":1,"operation":"issue.monitor.status","params":{}}' | & $checkoutGwtd | ConvertFrom-Json
if (!$status.ok -or !$status.project_store.hash) { throw 'Cannot resolve canonical project hash.' }
$projectState = Join-Path $checkHome ".gwt/projects/$($status.project_store.hash)/project-state"
New-Item -ItemType Directory -Path $projectState -Force | Out-Null
'{"settings":{"auto_start":false}}' | Set-Content (Join-Path $projectState 'pm.json')
'{"enabled":false,"max_active_agents":1,"priority_order":[]}' | Set-Content (Join-Path $projectState 'issue-monitor.json')
@{active_tab_id='issue-3823';tabs=@(@{id='issue-3823';title='Issue 3823';project_root=$repoRoot;kind='git'});recent_projects=@()} |
    ConvertTo-Json -Depth 5 | Set-Content (Join-Path $checkHome '.gwt/session.json')

function Invoke-HookDoctor($parameters) {
    $previousBin = $env:GWT_BIN_PATH
    try {
        $env:GWT_BIN_PATH = $null
        $response = @{schema_version=1;operation='hook.doctor';params=$parameters} |
            ConvertTo-Json -Compress | & $checkoutGwtd | ConvertFrom-Json
        if (!$response.ok) { throw 'hook.doctor failed.' }
        return ($response.output | ConvertFrom-Json)
    } finally { $env:GWT_BIN_PATH = $previousBin }
}
$hookBin = (Invoke-HookDoctor @{}).managed_hook_bin
if (!$hookBin -or $hookBin -match '[/\\]target[/\\]') { throw 'Stable managed hook fallback required.' }
function Assert-HookConvergence($doctor) {
    $preflight = $doctor.preflight
    if ($preflight.schema_version -ne 1 -or $preflight.result -ne 'proceed' -or
        $preflight.context -ne 'current' -or $preflight.normal_completion -ne 'normal' -or
        $preflight.convergence -ne 'converged' -or @($preflight.reasons).Count -ne 0 -or
        @($preflight.diagnostics | Where-Object { $_.classification -notin @('recovered','fail_open','foreign') }).Count) {
        throw ('Hook convergence failed: ' + ($preflight | ConvertTo-Json -Depth 6 -Compress))
    }
}
Assert-HookConvergence (Invoke-HookDoctor @{purpose='managed_browser';repair=$true;expected_hook_bin=$hookBin})
$ownedGui = $null
$anchor = $null
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
    foreach ($name in @($startInfo.Environment.Keys)) {
        if ($name -match '^(GWT_|CODEX_|CLAUDE_)') { $startInfo.Environment.Remove($name) | Out-Null }
    }
    $startInfo.Environment['HOME'] = $checkHome
    $startInfo.Environment['USERPROFILE'] = $checkHome
    $startInfo.Environment['GWT_BROWSER_URL_FILE'] = Join-Path $checkHome 'url'
    $startInfo.Environment['GWT_HOOK_BIN'] = $hookBin
    $startInfo.Environment['GWT_DISABLE_BACKGROUND_INDEX'] = '1'
    $startInfo.Environment['GIT_TERMINAL_PROMPT'] = '0'
    $ownedGui = [Diagnostics.Process]::Start($startInfo)
    $stdout = $ownedGui.StandardOutput.ReadToEndAsync()
    $stderr = $ownedGui.StandardError.ReadToEndAsync()
    Write-Output "Fresh checkout PID: $($ownedGui.Id); isolated HOME: $checkHome"
    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    $checkUrl = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($ownedGui.HasExited) { throw ('Fresh gwt exited: ' + $stderr.GetAwaiter().GetResult()) }
        if (Test-Path (Join-Path $checkHome 'url')) {
            $candidate = (Get-Content (Join-Path $checkHome 'url') -Raw).Trim()
            try {
                if ((Invoke-WebRequest $candidate -Method Head -TimeoutSec 2).StatusCode -eq 200) { $checkUrl=$candidate; break }
            } catch { }
        }
        Start-Sleep -Milliseconds 100
    }
    if (!$checkUrl) { throw 'Fresh checkout URL never became ready.' }
    $anchor = [Net.WebSockets.ClientWebSocket]::new()
    $anchorUrl = [UriBuilder]::new($checkUrl)
    $anchorUrl.Scheme = 'ws'
    $anchorUrl.Path = '/ws'
    $anchor.ConnectAsync($anchorUrl.Uri, [Threading.CancellationToken]::None).GetAwaiter().GetResult() | Out-Null
    Assert-HookConvergence (Invoke-HookDoctor @{purpose='managed_browser';expected_hook_bin=$hookBin})
    @{source_sha=(& git rev-parse HEAD).Trim();binary_sha256=(Get-FileHash $checkoutGwt).Hash;
        pid=$ownedGui.Id;isolated_home=$checkHome;project_root=$repoRoot} |
        ConvertTo-Json | Set-Content (Join-Path $EvidenceDirectory 'isolation.json')
    $version = (Get-Content (Join-Path $repoRoot 'scripts/playwright-version.txt') -Raw).Trim()
    # verify.run isolates TEMP for each command; use the persistent host cache.
    $deps = Join-Path (Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Temp') "gwt-playwright-$version"
    $playwright = Join-Path $deps 'node_modules/@playwright/test/cli.js'
    if (!(Test-Path $playwright)) {
        throw "Install @playwright/test@$version into $deps before running this check."
    }
    $testVars = @{GWT_PLAYWRIGHT_BASE_URL=$checkUrl;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;
        GWT_PLAYWRIGHT_PROJECT_ROOT=$repoRoot;NODE_PATH=(Join-Path $deps 'node_modules')}
    foreach ($name in $testVars.Keys) {
        $savedVars[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
        [Environment]::SetEnvironmentVariable($name, $testVars[$name], 'Process')
    }
    & node $playwright test --config (Join-Path $repoRoot 'crates/gwt/playwright/playwright.config.ts') --workers=1 `
        --output (Join-Path $EvidenceDirectory 'playwright') --grep 'Issue #3823' 'agent-title-badge.spec.ts' @PlaywrightArgs
    if ($LASTEXITCODE) { throw 'Issue #3823 headed verification failed.' }
} finally {
    foreach ($name in $savedVars.Keys) { [Environment]::SetEnvironmentVariable($name, $savedVars[$name], 'Process') }
    if ($anchor) { $anchor.Abort(); $anchor.Dispose() }
    if ($ownedGui -and !$ownedGui.HasExited) { Stop-Process -Id $ownedGui.Id -Force; $ownedGui.WaitForExit(10000) | Out-Null }
    if ($ownedGui -and !$ownedGui.HasExited) { throw "Owned GUI PID $($ownedGui.Id) remains alive." }
    if ($stdout) { $stdout.GetAwaiter().GetResult() | Set-Content (Join-Path $EvidenceDirectory 'startup.log') }
    if ($stderr) { $stderr.GetAwaiter().GetResult() | Set-Content (Join-Path $EvidenceDirectory 'stderr.log') }
}
