[CmdletBinding(PositionalBinding = $false)]
param(
    [ValidateSet('before', 'after')][string]$Label = 'after',
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArguments
)

$ErrorActionPreference = 'Stop'
$checkout = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$gwtBinary = Join-Path $checkout 'target/debug/gwt.exe'
$gwtdBinary = Join-Path $checkout 'target/debug/gwtd.exe'
$deps = Join-Path $env:LOCALAPPDATA 'Temp/gwt-playwright-win-1.49.1'
$realProfile = $env:USERPROFILE
$realGit = (Get-Command git.exe -ErrorAction Stop).Source
$runId = [Guid]::NewGuid().ToString('N')
$checkHome = Join-Path $checkout "target/issue-5537-home-$runId"
$artifacts = Join-Path $checkout "crates/gwt/playwright/test-results/issue-5537-$Label-$runId"
$gateRoot = Join-Path $checkHome 'git-gate'
$shimRoot = Join-Path $checkHome 'bin'
$process = $null
$keeper = $null
$savedEnvironment = @{}

function Invoke-CheckoutEnvelope($operation, $parameters) {
    $envelope = @{schema_version=1;operation=$operation;params=$parameters} | ConvertTo-Json -Depth 12 -Compress
    $result = $envelope | & $gwtdBinary | ConvertFrom-Json
    if (!$result.ok) { throw "$operation failed: $($result.output)" }
    return $result
}
function Assert-HookHealth($health) {
    $blocking = @($health.issues | Where-Object { $_ -notlike 'managed hook failure:*state=fail-open*' })
    if ($health.status -eq 'inactive' -or $blocking.Count) { throw "hook convergence failed: $($blocking -join ', ')" }
}

try {
    Set-Location $checkout
    if (!(Test-Path $gwtBinary) -or !(Test-Path $gwtdBinary)) { throw 'Build checkout gwt and gwtd before browser-check.' }
    if (!(Test-Path "$deps/node_modules/@playwright/test/cli.js") -or !(Test-Path "$deps/resolve-playwright.cjs")) {
        throw "Install pinned Playwright dependencies and resolver at $deps before browser-check."
    }
    New-Item -ItemType Directory -Path "$checkHome/.gwt", $artifacts, $gateRoot, $shimRoot -Force | Out-Null
    if (Test-Path "$realProfile/.gwt/runtime") {
        New-Item -ItemType Junction -Path "$checkHome/.gwt/runtime" -Target "$realProfile/.gwt/runtime" | Out-Null
    }
    $monitor = Invoke-CheckoutEnvelope 'issue.monitor.status' @{}
    $repoHash = $monitor.project_store.hash
    if (!$repoHash) { throw 'project_store.hash missing' }
    $projectState = "$checkHome/.gwt/projects/$repoHash/project-state"
    New-Item -ItemType Directory -Path $projectState -Force | Out-Null
    '{"settings":{"auto_start":false}}' | Set-Content "$projectState/pm.json" -Encoding utf8NoBOM
    '{"enabled":false,"max_active_agents":1,"priority_order":[]}' | Set-Content "$projectState/issue-monitor.json" -Encoding utf8NoBOM
    @{tabs=@(@{id='monitor-list-5537';title='Issue Monitor';project_root=$checkout;kind='git'});recent_projects=@()} |
        ConvertTo-Json -Depth 8 | Set-Content "$checkHome/.gwt/session.json" -Encoding utf8NoBOM

    # Only the local candidate reader's exact origin probe is delayed. Its
    # existing one-second operation deadline terminates this owned process
    # tree; the six-second watchdog contract is covered by the Rust test.
    @'
@echo off
if not "%~1"=="remote" goto run
if not "%~2"=="get-url" goto run
if not "%~3"=="origin" goto run
if not exist "%GWT_LIST_WORKER_GATE%\armed" goto run
if exist "%GWT_LIST_WORKER_GATE%\started" goto run
copy /y "%GWT_LIST_WORKER_GATE%\armed" "%GWT_LIST_WORKER_GATE%\started" >nul
"%SystemRoot%\System32\ping.exe" -n 7 127.0.0.1 >nul
:run
"%GWT_LIST_WORKER_REAL_GIT%" %*
exit /b %errorlevel%
'@ | Set-Content "$shimRoot/git.cmd" -Encoding ascii

    $hook = (Invoke-CheckoutEnvelope 'hook.doctor' @{}).output | ConvertFrom-Json
    $hookBinary = $hook.managed_hook_bin
    if (!$hookBinary -or $hookBinary -match '[/\\]target[/\\].*[/\\]gwtd(\.exe)?$') { throw 'Stable hook fallback missing' }
    $hookParams = @{repair=$true;expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}
    Assert-HookHealth ((Invoke-CheckoutEnvelope 'hook.doctor' $hookParams).output | ConvertFrom-Json).health
    $childVariables = @{HOME=$checkHome;USERPROFILE=$checkHome;GWT_BROWSER_URL_FILE="$checkHome/url.txt";
        GWT_HOOK_BIN=$hookBinary;GIT_TERMINAL_PROMPT='0';GH_PROMPT_DISABLED='1';GWT_BIN_PATH=$null;
        PATH="$shimRoot;$env:PATH";GWT_LIST_WORKER_GATE=$gateRoot;GWT_LIST_WORKER_REAL_GIT=$realGit}
    foreach ($key in $childVariables.Keys) {
        $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key)
        [Environment]::SetEnvironmentVariable($key, $childVariables[$key])
    }
    $process = Start-Process -FilePath $gwtBinary -ArgumentList '--no-tray','--no-open' -WorkingDirectory $checkout -WindowStyle Hidden -PassThru -RedirectStandardOutput "$checkHome/stdout.log" -RedirectStandardError "$checkHome/stderr.log"
    foreach ($key in $childVariables.Keys) { [Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key]) }
    Write-Output "Owned checkout gwt PID: $($process.Id); isolated HOME: $checkHome"
    $deadline = [DateTime]::UtcNow.AddSeconds(120)
    while (!(Test-Path "$checkHome/url.txt")) {
        if ($process.HasExited -or [DateTime]::UtcNow -ge $deadline) { throw "Checkout startup failed; inspect $checkHome" }
        Start-Sleep -Milliseconds 200
    }
    $baseUrl = (Get-Content "$checkHome/url.txt" -Raw).Trim()
    if ((Invoke-WebRequest -Method Head -Uri $baseUrl).StatusCode -ne 200) { throw 'Fresh checkout URL is not ready' }
    # Keep this exact no-tray instance alive between the two theme contexts.
    $keeper = [System.Net.WebSockets.ClientWebSocket]::new()
    $keeper.Options.SetRequestHeader('Origin', $baseUrl.TrimEnd('/'))
    $null = $keeper.ConnectAsync([Uri](($baseUrl -replace '^http', 'ws') + 'ws'), [Threading.CancellationToken]::None).GetAwaiter().GetResult()
    $healthParams = @{expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}
    Assert-HookHealth ((Invoke-CheckoutEnvelope 'hook.health' $healthParams).output | ConvertFrom-Json)
    @{project=$checkout;repo_hash=$repoHash;gate_root=$gateRoot;pid=$process.Id;label=$Label} |
        ConvertTo-Json | Set-Content "$checkHome/issue-5537-isolated.json" -Encoding utf8NoBOM
    $testVariables = @{GWT_PLAYWRIGHT_BASE_URL=$baseUrl;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;
        GWT_PLAYWRIGHT_PID=[string]$process.Id;GWT_PLAYWRIGHT_PROJECT_ROOT=$checkout;
        GWT_PLAYWRIGHT_NODE_MODULES="$deps/node_modules";
        NODE_OPTIONS="--require $($deps.Replace('\','/'))/resolve-playwright.cjs";NODE_PATH="$deps/node_modules"}
    foreach ($key in $testVariables.Keys) {
        if (!$savedEnvironment.ContainsKey($key)) { $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key) }
        [Environment]::SetEnvironmentVariable($key, $testVariables[$key])
    }
    & node "$deps/node_modules/@playwright/test/cli.js" test --config crates/gwt/playwright/playwright.config.ts --output $artifacts --workers=1 issue-monitor-list-worker-live.spec.ts @PlaywrightArguments
    if ($LASTEXITCODE -ne 0) { throw "Playwright failed with exit $LASTEXITCODE" }
    Write-Output "Browser evidence: $artifacts"
} finally {
    if ($keeper) { $keeper.Dispose() }
    if ($process -and !$process.HasExited) { $null = $process.WaitForExit(12000) }
    if ($process -and !$process.HasExited) {
        # Terminate only this owned GUI PID and its descendants.
        & taskkill.exe /PID $process.Id /T /F 2>$null | Out-Null
        $null = $process.WaitForExit(5000)
        if (!$process.HasExited) { throw "Owned checkout PID $($process.Id) survived cleanup" }
    }
    foreach ($key in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key]) }
}
