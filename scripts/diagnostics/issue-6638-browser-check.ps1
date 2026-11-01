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
$checkHome = Join-Path $checkout ('target/issue-6638-home-' + [Guid]::NewGuid().ToString('N'))
$realProfile = $env:USERPROFILE
$process = $null
$keeper = $null
$savedEnvironment = @{}

function Invoke-CheckoutEnvelope($operation, $parameters) {
    $callerBin = $env:GWT_BIN_PATH
    try {
        if ($operation -eq 'hook.doctor') { $env:GWT_BIN_PATH = $null }
        $result = @{schema_version=1;operation=$operation;params=$parameters} |
            ConvertTo-Json -Depth 8 -Compress | & $gwtdBinary | ConvertFrom-Json
    } finally {
        $env:GWT_BIN_PATH = $callerBin
    }
    if (!$result.ok) { throw "$operation failed: $($result.error) $($result.output)" }
    return $result
}
function Assert-HookPreflight($doctor) {
    $p = $doctor.preflight
    if ($p.schema_version -ne 1 -or $p.context -ne 'current' -or $p.result -ne 'proceed' -or
        $p.normal_completion -ne 'normal' -or $p.convergence -ne 'converged' -or
        $null -eq $p.reasons -or @($p.reasons).Count -ne 0 -or $null -eq $p.diagnostics) {
        throw "Managed browser preflight failed: $($p | ConvertTo-Json -Depth 6 -Compress)"
    }
    foreach ($diagnostic in $p.diagnostics) {
        if (!$diagnostic.id -or $diagnostic.classification -notin @('recovered','fail_open','foreign')) {
            throw 'Managed browser diagnostic is missing or unclassified.'
        }
    }
}

try {
    Set-Location $checkout
    if (!(Test-Path $gwtBinary) -or !(Test-Path $gwtdBinary)) { throw 'Build this checkout gwt and gwtd first.' }
    if (!$deps) { throw "Install pinned Playwright $version and its resolver in the local gwt-playwright cache first." }
    $hook = (Invoke-CheckoutEnvelope 'hook.doctor' @{purpose='managed_browser'}).output | ConvertFrom-Json
    $hookBinary = $hook.managed_hook_bin
    if (!$hookBinary -or $hookBinary -match '[/\\]target[/\\].*[/\\]gwtd(\.exe)?$') { throw 'Stable hook fallback missing.' }
    $doctorParams = @{purpose='managed_browser';repair=$true;expected_hook_bin=$hookBinary}
    Assert-HookPreflight ((Invoke-CheckoutEnvelope 'hook.doctor' $doctorParams).output | ConvertFrom-Json)

    New-Item -ItemType Directory -Path "$checkHome/.gwt" -Force | Out-Null
    if (Test-Path "$realProfile/.gwt/runtime") {
        New-Item -ItemType Junction -Path "$checkHome/.gwt/runtime" -Target "$realProfile/.gwt/runtime" | Out-Null
    }
    $monitor = Invoke-CheckoutEnvelope 'issue.monitor.status' @{}
    $repoHash = $monitor.project_store.hash
    if (!$repoHash) { throw 'Canonical project_store.hash missing.' }
    $projectState = "$checkHome/.gwt/projects/$repoHash/project-state"
    New-Item -ItemType Directory -Path $projectState -Force | Out-Null
    '{"settings":{"auto_start":false}}' | Set-Content "$projectState/pm.json" -Encoding utf8NoBOM
    '{"enabled":false,"max_active_agents":1,"priority_order":[]}' | Set-Content "$projectState/issue-monitor.json" -Encoding utf8NoBOM
    @{active_tab_id='usage-6638';tabs=@(@{id='usage-6638';title='Usage';project_root=$checkout;kind='git'});recent_projects=@()} |
        ConvertTo-Json -Depth 6 | Set-Content "$checkHome/.gwt/session.json" -Encoding utf8NoBOM

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
    Assert-HookPreflight ((Invoke-CheckoutEnvelope 'hook.doctor' @{purpose='managed_browser';expected_hook_bin=$hookBinary}).output | ConvertFrom-Json)
    $testVariables = @{GWT_PLAYWRIGHT_BASE_URL=$baseUrl;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;
        GWT_PLAYWRIGHT_PID=[string]$process.Id;GWT_PLAYWRIGHT_CHECKOUT_ROOT=$checkout;
        GWT_PLAYWRIGHT_PROJECT_ROOT=$checkout;GWT_PLAYWRIGHT_NODE_MODULES="$deps/node_modules";
        NODE_OPTIONS="--require $($deps.Replace('\','/'))/resolve-playwright.cjs";NODE_PATH="$deps/node_modules"}
    foreach ($key in $testVariables.Keys) {
        if (!$savedEnvironment.ContainsKey($key)) { $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key) }
        [Environment]::SetEnvironmentVariable($key, $testVariables[$key])
    }
    & node "$deps/node_modules/@playwright/test/cli.js" test --config crates/gwt/playwright/playwright.config.ts --workers=1 provider-usage.spec.ts @PlaywrightArguments
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
