[CmdletBinding(PositionalBinding = $false)]
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArguments)

$ErrorActionPreference = 'Stop'
$checkout = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$gwtBinary = Join-Path $checkout 'target/debug/gwt.exe'
$gwtdBinary = Join-Path $checkout 'target/debug/gwtd.exe'
$version = (Get-Content (Join-Path $checkout 'scripts/playwright-version.txt') -Raw).Trim()
$deps = @('gwt-playwright-win-', 'gwt-playwright-') | ForEach-Object {
    Join-Path "$env:LOCALAPPDATA/Temp" ($_ + $version)
} | Where-Object {
    (Test-Path "$_/node_modules/@playwright/test/cli.js") -and (Test-Path "$_/resolve-playwright.cjs")
} | Select-Object -First 1
$realProfile = $env:USERPROFILE
$checkHome = Join-Path $checkout ('target/issue-6235-home-' + [Guid]::NewGuid().ToString('N'))
$process = $null
$keeper = $null
$savedEnvironment = @{}

function Invoke-CheckoutEnvelope($operation, $parameters) {
    $envelope = @{schema_version=1;operation=$operation;params=$parameters} | ConvertTo-Json -Depth 8 -Compress
    $result = $envelope | & $gwtdBinary | ConvertFrom-Json
    if (!$result.ok) { throw "$operation failed: $($result.error) $($result.output)" }
    return $result
}
function Assert-HookHealth($health) {
    $blocking = @($health.issues | Where-Object { $_ -notmatch '^managed hook failure: .* state=fail-open( |$)' })
    if ($health.status -eq 'inactive' -or $blocking.Count) { throw "Hook convergence failed: $($blocking -join ', ')" }
}

try {
    Set-Location $checkout
    if (!(Test-Path $gwtBinary) -or !(Test-Path $gwtdBinary)) { throw 'Build this checkout gwt and gwtd first.' }
    if (!$deps) { throw "Install pinned Playwright $version and its resolver in the local gwt-playwright cache first." }
    New-Item -ItemType Directory -Path "$checkHome/.gwt" -Force | Out-Null
    if (Test-Path "$realProfile/.gwt/runtime") {
        New-Item -ItemType Junction -Path "$checkHome/.gwt/runtime" -Target "$realProfile/.gwt/runtime" | Out-Null
    }
    $monitor = Invoke-CheckoutEnvelope 'issue.monitor.status' @{}
    $repoHash = $monitor.project_store.hash
    if (!$repoHash) { throw 'Canonical project_store.hash missing.' }
    $project = "$checkHome/.gwt/projects/$repoHash"
    $projectState = "$project/project-state"
    New-Item -ItemType Directory -Path $projectState -Force | Out-Null
    $timestamp = '2026-10-11T00:00:00Z'
    @{settings=@{auto_start=$false};registration=@{session_id='pm-reports-fixture';agent_id='codex';worktree_path=$checkout;created_at=$timestamp}} |
        ConvertTo-Json -Depth 6 | Set-Content "$projectState/pm.json" -Encoding utf8NoBOM
    '{"enabled":false,"max_active_agents":1,"priority_order":[]}' | Set-Content "$projectState/issue-monitor.json" -Encoding utf8NoBOM
    @{active_tab_id='pm-6235';tabs=@(@{id='pm-6235';title='PM Reports';project_root=$checkout;kind='git'});recent_projects=@()} |
        ConvertTo-Json -Depth 6 | Set-Content "$checkHome/.gwt/session.json" -Encoding utf8NoBOM
    @{viewport=@{x=0;y=0;zoom=1};windows=@(@{id='pm-reports';title='PM';preset='agent';geometry=@{x=30;y=30;width=1050;height=720};z_index=1;status='stopped';session_id='pm-reports-fixture';agent_id='codex';persist=$true});next_z_index=2} |
        ConvertTo-Json -Depth 8 | Set-Content "$project/workspace.json" -Encoding utf8NoBOM
    New-Item -ItemType Directory -Path "$checkHome/.gwt/sessions" -Force | Out-Null
    $fixtureRoot = $checkout.Replace('\', '/')
    @"
id = "pm-reports-fixture"
worktree_path = "$fixtureRoot"
branch = "work/issue-6235"
agent_id = { type = "Codex" }
status = "Stopped"
created_at = "$timestamp"
updated_at = "$timestamp"
last_activity_at = "$timestamp"
"@ | Set-Content "$checkHome/.gwt/sessions/pm-reports-fixture.toml" -Encoding utf8NoBOM
    @{id='persisted-before-restart';kind='decision';body="# Persisted before restart`n`nThis report survives the application restart.";created_at=$timestamp} |
        ConvertTo-Json -Compress | Set-Content "$projectState/pm-reports.jsonl" -Encoding utf8NoBOM

    $hook = (Invoke-CheckoutEnvelope 'hook.doctor' @{}).output | ConvertFrom-Json
    $hookBinary = $hook.managed_hook_bin
    if (!$hookBinary -or $hookBinary -match '[/\\]target[/\\].*[/\\]gwtd(\.exe)?$') { throw 'Stable hook fallback missing.' }
    $hookParams = @{repair=$true;expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}
    Assert-HookHealth ((Invoke-CheckoutEnvelope 'hook.doctor' $hookParams).output | ConvertFrom-Json).health
    $childVariables = @{}
    Get-ChildItem Env:GWT_* | ForEach-Object { $childVariables[$_.Name] = $null }
    $childVariables.HOME = $checkHome
    $childVariables.USERPROFILE = $checkHome
    $childVariables.GWT_BROWSER_URL_FILE = "$checkHome/url.txt"
    $childVariables.GWT_HOOK_BIN = $hookBinary
    $childVariables.GWT_DISABLE_BACKGROUND_INDEX = '1'
    $childVariables.GIT_TERMINAL_PROMPT = '0'
    $childVariables.GH_PROMPT_DISABLED = '1'
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
    if ((Invoke-WebRequest -Method Head -Uri $baseUrl).StatusCode -ne 200) { throw 'Fresh checkout URL is not ready.' }
    $keeper = [System.Net.WebSockets.ClientWebSocket]::new()
    $keeper.Options.SetRequestHeader('Origin', $baseUrl.TrimEnd('/'))
    $null = $keeper.ConnectAsync([Uri](($baseUrl -replace '^http', 'ws') + 'ws'), [Threading.CancellationToken]::None).GetAwaiter().GetResult()
    $healthParams = @{expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}
    Assert-HookHealth ((Invoke-CheckoutEnvelope 'hook.health' $healthParams).output | ConvertFrom-Json)
    $testVariables = @{GWT_PLAYWRIGHT_BASE_URL=$baseUrl;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;
        GWT_PLAYWRIGHT_PID=[string]$process.Id;GWT_PLAYWRIGHT_CHECKOUT_ROOT=$checkout;
        GWT_PLAYWRIGHT_PROJECT_ROOT=$checkout;GWT_PLAYWRIGHT_NODE_MODULES="$deps/node_modules";
        NODE_OPTIONS="--require $($deps.Replace('\','/'))/resolve-playwright.cjs";NODE_PATH="$deps/node_modules"}
    foreach ($key in $testVariables.Keys) {
        if (!$savedEnvironment.ContainsKey($key)) { $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key) }
        [Environment]::SetEnvironmentVariable($key, $testVariables[$key])
    }
    & node "$deps/node_modules/@playwright/test/cli.js" test --config crates/gwt/playwright/playwright.config.ts --workers=1 pm-chat-live.spec.ts --grep 'Issue #6235' @PlaywrightArguments
    if ($LASTEXITCODE -ne 0) { throw "Playwright failed with exit $LASTEXITCODE" }
} finally {
    if ($keeper) { $keeper.Dispose() }
    if ($process -and !$process.HasExited) { $null = $process.WaitForExit(12000) }
    if ($process -and !$process.HasExited) {
        & taskkill.exe /PID $process.Id /T /F 2>$null | Out-Null
        $null = $process.WaitForExit(5000)
        if (!$process.HasExited) { throw "Owned checkout PID $($process.Id) survived cleanup." }
    }
    foreach ($key in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key]) }
}
