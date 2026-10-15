# Build this checkout's gwt and gwtd first. Additional Playwright arguments are forwarded.
[CmdletBinding(PositionalBinding = $false)]
param(
    [string]$Spec = 'knowledge-load-recovery.spec.ts',
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArguments
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This fixture requires Windows and PowerShell 7.' }
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$checkoutGwt = Join-Path $repoRoot 'target/debug/gwt.exe'
$checkoutGwtd = Join-Path $repoRoot 'target/debug/gwtd.exe'
foreach ($binary in @($checkoutGwt, $checkoutGwtd)) {
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw 'Build this checkout with cargo build -p gwt --bin gwt --bin gwtd first.' }
}
$checkHome = Join-Path ([IO.Path]::GetTempPath()) ('gwt-issue-5413-' + [guid]::NewGuid())
foreach ($relative in @('.gwt', '.codex', 'tmp')) { [void](New-Item -ItemType Directory -Path (Join-Path $checkHome $relative) -Force) }
[IO.File]::WriteAllText((Join-Path $checkHome '.codex/config.toml'), '')
$runtime = Join-Path ([Environment]::GetFolderPath('UserProfile')) '.gwt/runtime'
if (Test-Path -LiteralPath $runtime -PathType Container) { [void](New-Item -ItemType Junction -Path (Join-Path $checkHome '.gwt/runtime') -Target $runtime) }
$childEnvironment = @{}
foreach ($entry in [Environment]::GetEnvironmentVariables().GetEnumerator()) {
    if ($entry.Key -notmatch '^(GWT_|CODEX_|CLAUDE_|GH_TOKEN$|GITHUB_TOKEN$|OPENAI_API_KEY$|ANTHROPIC_API_KEY$)') { $childEnvironment[$entry.Key] = [string]$entry.Value }
}
$childEnvironment.HOME = $checkHome
$childEnvironment.USERPROFILE = $checkHome
$childEnvironment.CODEX_HOME = Join-Path $checkHome '.codex'
$childEnvironment.TEMP = Join-Path $checkHome 'tmp'
$childEnvironment.TMP = $childEnvironment.TEMP
$childEnvironment.GIT_TERMINAL_PROMPT = '0'
$childEnvironment.GH_PROMPT_DISABLED = '1'
$childEnvironment.GWT_PROJECT_ROOT = $repoRoot
$childEnvironment.GWT_DISABLE_BACKGROUND_INDEX = '1'

function New-CheckoutStartInfo([string]$Binary) {
    $startInfo = [Diagnostics.ProcessStartInfo]::new($Binary)
    $startInfo.WorkingDirectory = $repoRoot
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $startInfo.Environment.Clear()
    foreach ($key in $childEnvironment.Keys) { $startInfo.Environment[$key] = $childEnvironment[$key] }
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $startInfo.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    return $startInfo
}
function Invoke-CheckoutGwtd([string]$Operation, [hashtable]$Parameters) {
    $startInfo = New-CheckoutStartInfo $checkoutGwtd
    $startInfo.RedirectStandardInput = $true
    $startInfo.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
    $process = [Diagnostics.Process]::Start($startInfo)
    try {
        $process.StandardInput.WriteLine((@{schema_version=1;operation=$Operation;params=$Parameters} | ConvertTo-Json -Depth 8 -Compress))
        $process.StandardInput.Close()
        $output = $process.StandardOutput.ReadToEndAsync()
        $errorOutput = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(60000)) { $process.Kill($true); throw "$Operation timed out." }
        $envelope = $output.GetAwaiter().GetResult() | ConvertFrom-Json
        if ($process.ExitCode -ne 0 -or -not $envelope.ok) { throw "$Operation failed: $($errorOutput.GetAwaiter().GetResult()); $($envelope | ConvertTo-Json -Depth 8 -Compress)" }
        return $envelope
    } finally { $process.Dispose() }
}
function Assert-HookHealth($Health) {
    $blocking = @($Health.issues | Where-Object {
        $_ -notmatch '^managed hook failure: .* state=fail-open( |$)' -and
        -not ($hookBin -eq 'gwtd' -and -not $publicGwtd -and $_ -match '^managed hook binary missing: .* uses gwtd$')
    })
    if ($Health.status -eq 'inactive' -or $blocking.Count) { throw "Hook convergence failed: $($blocking -join '; ')" }
}

$status = Invoke-CheckoutGwtd 'issue.monitor.status' @{}
$repoHash = $status.project_store.hash
if ($repoHash -notmatch '^[a-zA-Z0-9_-]+$') { throw 'Missing canonical project_store.hash.' }
$statePath = Join-Path $checkHome ".gwt/projects/$repoHash/project-state"
[void](New-Item -ItemType Directory -Path $statePath -Force)
[IO.File]::WriteAllText((Join-Path $statePath 'pm.json'), '{"settings":{"auto_start":false}}')
[IO.File]::WriteAllText((Join-Path $statePath 'issue-monitor.json'), '{"enabled":false,"max_active_agents":1,"priority_order":[]}')
@{active_tab_id='issue-5413';tabs=@(@{id='issue-5413';title='Load recovery';project_root=$repoRoot;kind='git'});recent_projects=@()} |
    ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $checkHome '.gwt/session.json')
$localHookPattern = '(?i)(^|[/\\])target[/\\](?:[^/\\]+[/\\])*(debug|release)[/\\]gwtd(\.exe)?$'
$hookBin = $env:GWT_HOOK_BIN
if ($hookBin -and ($hookBin -match $localHookPattern -or ($hookBin -ne 'gwtd' -and -not (Test-Path -LiteralPath $hookBin -PathType Leaf)))) { $hookBin = '' }
$publicGwtd = Get-Command gwtd -CommandType Application -ErrorAction SilentlyContinue
if (-not $hookBin) { $hookBin = ((Invoke-CheckoutGwtd 'hook.doctor' @{}).output | ConvertFrom-Json).managed_hook_bin }
if (-not $hookBin -or $hookBin -match $localHookPattern) { throw 'Managed hook resolver did not provide a stable fallback.' }
$childEnvironment.GWT_HOOK_BIN = $hookBin
$hookParameters = @{expected_hook_bin=$hookBin;runtime_state_path=(Join-Path $checkHome '.gwt/missing-runtime-state.json')}
$doctor = Invoke-CheckoutGwtd 'hook.doctor' ($hookParameters + @{repair=$true})
[void](Invoke-CheckoutGwtd 'hook.register_codex_managed_hook_trust' @{
    codex_config=(Join-Path $checkHome '.codex/config.toml');codex_hook_discovery='both';project_root=$repoRoot
})
Assert-HookHealth (((Invoke-CheckoutGwtd 'hook.health' $hookParameters).output | ConvertFrom-Json))

$ownedGui = $null
$anchorSocket = $null
$savedRunnerEnvironment = @{}
$exitCode = 1
try {
    $urlFile = Join-Path $checkHome 'url.txt'
    $startInfo = New-CheckoutStartInfo $checkoutGwt
    $startInfo.ArgumentList.Add('--no-tray')
    $startInfo.ArgumentList.Add('--no-open')
    $startInfo.Environment['GWT_BROWSER_URL_FILE'] = $urlFile
    $ownedGui = [Diagnostics.Process]::Start($startInfo)
    $ownedPid = $ownedGui.Id
    Write-Output "Fresh checkout PID: $ownedPid; isolated HOME: $checkHome"
    $stdoutTask = $ownedGui.StandardOutput.ReadToEndAsync()
    $stderrTask = $ownedGui.StandardError.ReadToEndAsync()
    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    $checkUrl = ''
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($ownedGui.HasExited) { throw 'Fresh gwt exited during startup.' }
        if (Test-Path -LiteralPath $urlFile -PathType Leaf) {
            $candidate = [IO.File]::ReadAllText($urlFile).Trim()
            try { if ($candidate -and (Invoke-WebRequest -Uri $candidate -Method Head -TimeoutSec 2).StatusCode -eq 200) { $checkUrl = $candidate; break } } catch { }
        }
        Start-Sleep -Milliseconds 100
    }
    if (-not $checkUrl) { throw 'Fresh checkout URL never became ready.' }
    # Hold one owned connection across Playwright worker/browser restarts.
    $anchorSocket = [Net.WebSockets.ClientWebSocket]::new()
    $anchorUrl = [UriBuilder]::new($checkUrl)
    $anchorUrl.Scheme = 'ws'
    $anchorUrl.Path = '/ws'
    $anchorSocket.ConnectAsync($anchorUrl.Uri, [Threading.CancellationToken]::None).GetAwaiter().GetResult() | Out-Null
    Assert-HookHealth (((Invoke-CheckoutGwtd 'hook.health' $hookParameters).output | ConvertFrom-Json))

    $version = [IO.File]::ReadAllText((Join-Path $repoRoot 'scripts/playwright-version.txt')).Trim()
    $deps = if ($env:GWT_PLAYWRIGHT_DEPS_DIR) { $env:GWT_PLAYWRIGHT_DEPS_DIR } else { Join-Path $env:LOCALAPPDATA "Temp/gwt-playwright-$version" }
    $cli = Join-Path $deps 'node_modules/@playwright/test/cli.js'
    if (-not (Test-Path -LiteralPath $cli)) { throw "Pinned Playwright $version is missing; run scripts/run-visual-tests.sh --list once." }
    $resolver = Join-Path $checkHome 'resolve-playwright.cjs'
    @'
const Module = require('module');
const path = require('path');
const original = Module._resolveFilename;
Module._resolveFilename = function(request, parent, ...rest) {
  if (request === '@playwright/test' || request.startsWith('@playwright/test/')) {
    return original.call(this, path.join(process.env.GWT_PLAYWRIGHT_NODE_MODULES, request), parent, ...rest);
  }
  return original.call(this, request, parent, ...rest);
};
'@ | Set-Content -LiteralPath $resolver
    $runnerEnvironment = @{
        GWT_PLAYWRIGHT_BASE_URL=$checkUrl;GWT_PLAYWRIGHT_PROJECT_KEY=$repoHash;
        GWT_PLAYWRIGHT_PROJECT_ROOT=$repoRoot;GWT_PLAYWRIGHT_CHECKOUT_ROOT=$repoRoot;
        GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;
        GWT_PLAYWRIGHT_NODE_MODULES=(Join-Path $deps 'node_modules');
        NODE_OPTIONS="--require `"$($resolver.Replace('\', '/'))`" $env:NODE_OPTIONS"
    }
    foreach ($key in $runnerEnvironment.Keys) {
        $savedRunnerEnvironment[$key] = [Environment]::GetEnvironmentVariable($key)
        [Environment]::SetEnvironmentVariable($key, $runnerEnvironment[$key])
    }
    Push-Location $repoRoot
    try {
        & node $cli test --config crates/gwt/playwright/playwright.config.ts --workers=1 --headed `
            --output (Join-Path $checkHome 'playwright') $Spec @PlaywrightArguments
        $exitCode = $LASTEXITCODE
    } finally { Pop-Location }
} finally {
    foreach ($key in $savedRunnerEnvironment.Keys) { [Environment]::SetEnvironmentVariable($key, $savedRunnerEnvironment[$key]) }
    if ($anchorSocket) { $anchorSocket.Abort(); $anchorSocket.Dispose() }
    if ($ownedGui) {
        if (-not $ownedGui.HasExited) { $ownedGui.Kill($true) }
        if (-not $ownedGui.WaitForExit(10000)) { throw "Owned GUI PID $ownedPid remained after cleanup." }
        [IO.File]::WriteAllText((Join-Path $checkHome 'startup.log'), $stdoutTask.GetAwaiter().GetResult())
        [IO.File]::WriteAllText((Join-Path $checkHome 'stderr.log'), $stderrTask.GetAwaiter().GetResult())
        $ownedGui.Dispose()
    }
    Write-Output "Isolated evidence: $checkHome; owned process cleanup finished."
}
if ($exitCode -ne 0) { throw "Issue #5413 headed verification failed ($exitCode)." }
