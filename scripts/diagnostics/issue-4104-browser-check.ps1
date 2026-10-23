#Requires -Version 7.0
[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArguments
)

$ErrorActionPreference = 'Stop'
$checkout = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$gwtBinary = Join-Path $checkout 'target/debug/gwt.exe'
$gwtdBinary = Join-Path $checkout 'target/debug/gwtd.exe'
$deps = Join-Path $env:LOCALAPPDATA 'Temp/gwt-playwright-win-1.49.1'
$checkHome = Join-Path $checkout ('target/issue-4104-home-' + [Guid]::NewGuid().ToString('N'))
$artifacts = Join-Path $checkHome 'headed'
$ownedProcess = $null
$keeper = $null
$savedEnvironment = @{}

function Invoke-CheckoutEnvelope($operation, $parameters) {
    $request = @{schema_version=1;operation=$operation;params=$parameters} | ConvertTo-Json -Depth 12 -Compress
    $response = $request | & $gwtdBinary | ConvertFrom-Json
    if (!$response.ok) { throw "$operation failed: $($response.output)" }
    return $response
}
function Assert-HookHealth($health) {
    $blocking = @($health.issues | Where-Object { $_ -notlike 'managed hook failure:*state=fail-open*' })
    if ($health.status -eq 'inactive' -or $blocking.Count) { throw "hook convergence failed: $($blocking -join ', ')" }
}
function Set-CheckEnvironment($variables) {
    foreach ($key in $variables.Keys) {
        if (!$savedEnvironment.ContainsKey($key)) { $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key) }
        [Environment]::SetEnvironmentVariable($key, $variables[$key])
    }
}

Push-Location -LiteralPath $checkout
try {
    if (!$SkipBuild) {
        & cargo build -p gwt --bin gwt --bin gwtd
        if ($LASTEXITCODE -ne 0) { throw 'Checkout build failed' }
    }
    if (!(Test-Path $gwtBinary) -or !(Test-Path $gwtdBinary)) { throw 'Build checkout binaries first' }
    if (!(Test-Path "$deps/node_modules/@playwright/test/cli.js") -or !(Test-Path "$deps/resolve-playwright.cjs")) {
        throw "Install the pinned Playwright runner and resolver at $deps"
    }
    New-Item -ItemType Directory -Path "$checkHome/.gwt" -Force | Out-Null
    if (Test-Path "$env:USERPROFILE/.gwt/runtime") {
        New-Item -ItemType Junction -Path "$checkHome/.gwt/runtime" -Target "$env:USERPROFILE/.gwt/runtime" | Out-Null
    }
    $repoHash = (Invoke-CheckoutEnvelope 'issue.monitor.status' @{}).project_store.hash
    if (!$repoHash) { throw 'project_store.hash missing' }
    $projectState = "$checkHome/.gwt/projects/$repoHash/project-state"
    New-Item -ItemType Directory -Path $projectState -Force | Out-Null
    '{"settings":{"auto_start":false}}' | Set-Content "$projectState/pm.json" -Encoding utf8NoBOM
    '{"enabled":false,"max_active_agents":1,"priority_order":[]}' | Set-Content "$projectState/issue-monitor.json" -Encoding utf8NoBOM
    @{tabs=@(@{id='selection-4104';title='Selection check';project_root=$checkout;kind='git'});recent_projects=@()} |
        ConvertTo-Json -Depth 8 | Set-Content "$checkHome/.gwt/session.json" -Encoding utf8NoBOM
    $hookBinary = ((Invoke-CheckoutEnvelope 'hook.doctor' @{}).output | ConvertFrom-Json).managed_hook_bin
    if (!$hookBinary -or $hookBinary -match '[/\\]target[/\\].*[/\\]gwtd(\.exe)?$') { throw 'Stable hook fallback missing' }
    $hookParams = @{repair=$true;expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}
    $null = Invoke-CheckoutEnvelope 'hook.doctor' $hookParams
    $null = Invoke-CheckoutEnvelope 'hook.register_codex_managed_hook_trust' @{
        project_root=$checkout;codex_config="$env:USERPROFILE/.codex/config.toml";codex_hook_discovery='both'
    }
    Assert-HookHealth ((Invoke-CheckoutEnvelope 'hook.health' @{expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}).output | ConvertFrom-Json)

    $childVariables = @{}
    Get-ChildItem Env:GWT_* | ForEach-Object { $childVariables[$_.Name] = $null }
    $childVariables.HOME = $checkHome
    $childVariables.USERPROFILE = $checkHome
    $childVariables.GWT_BROWSER_URL_FILE = "$checkHome/url.txt"
    $childVariables.GWT_HOOK_BIN = $hookBinary
    $childVariables.GWT_DISABLE_BACKGROUND_INDEX = '1'
    $childVariables.GIT_TERMINAL_PROMPT = '0'
    $childVariables.GH_PROMPT_DISABLED = '1'
    Set-CheckEnvironment $childVariables
    $ownedProcess = Start-Process -FilePath $gwtBinary -ArgumentList '--no-tray','--no-open' -WorkingDirectory $checkout -WindowStyle Hidden -PassThru -RedirectStandardOutput "$checkHome/stdout.log" -RedirectStandardError "$checkHome/stderr.log"
    foreach ($key in $childVariables.Keys) { [Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key]) }
    Write-Output "Owned checkout PID: $($ownedProcess.Id); isolated HOME: $checkHome"
    $deadline = [DateTime]::UtcNow.AddSeconds(120)
    while (!(Test-Path "$checkHome/url.txt")) {
        if ($ownedProcess.HasExited -or [DateTime]::UtcNow -ge $deadline) { throw "Checkout startup failed: $checkHome" }
        Start-Sleep -Milliseconds 200
    }
    $baseUrl = (Get-Content "$checkHome/url.txt" -Raw).Trim()
    if ((Invoke-WebRequest -Method Head -Uri $baseUrl).StatusCode -ne 200) { throw 'Fresh server is not ready' }
    foreach ($asset in @('app.js','terminal-context-menu.js','terminal-attachments.js')) {
        $served = (Invoke-WebRequest -Uri ($baseUrl.TrimEnd('/') + '/' + $asset)).Content
        if ($served -cne [IO.File]::ReadAllText((Join-Path $checkout "crates/gwt/web/$asset"))) { throw "Stale embedded $asset" }
    }
    $keeper = [Net.WebSockets.ClientWebSocket]::new()
    $keeper.Options.SetRequestHeader('Origin', $baseUrl.TrimEnd('/'))
    $null = $keeper.ConnectAsync([Uri](($baseUrl -replace '^http','ws') + 'ws'), [Threading.CancellationToken]::None).GetAwaiter().GetResult()
    Assert-HookHealth ((Invoke-CheckoutEnvelope 'hook.health' @{expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}).output | ConvertFrom-Json)
    Set-CheckEnvironment @{GWT_PLAYWRIGHT_BASE_URL=$baseUrl;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;
        GWT_PLAYWRIGHT_NODE_MODULES="$deps/node_modules";NODE_PATH="$deps/node_modules";
        NODE_OPTIONS="--require $($deps.Replace('\','/'))/resolve-playwright.cjs"}
    & node "$deps/node_modules/@playwright/test/cli.js" test agent-window-scroll.spec.ts --config=crates/gwt/playwright/playwright.config.ts --headed --workers=1 "--output=$artifacts" @PlaywrightArguments
    if ($LASTEXITCODE -ne 0) { throw "Playwright failed: $LASTEXITCODE" }
} finally {
    try {
        if ($keeper) { $keeper.Dispose() }
        if ($ownedProcess -and !$ownedProcess.HasExited) { $null = $ownedProcess.WaitForExit(5000) }
        if ($ownedProcess -and !$ownedProcess.HasExited) {
            Stop-Process -Id $ownedProcess.Id
            if (!$ownedProcess.WaitForExit(5000)) { throw "Owned PID $($ownedProcess.Id) survived cleanup" }
        }
    } finally {
        foreach ($key in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key]) }
        Pop-Location
    }
}
