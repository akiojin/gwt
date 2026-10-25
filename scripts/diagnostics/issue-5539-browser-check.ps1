[CmdletBinding(PositionalBinding = $false)]
param(
    [ValidateSet('baseline', 'after')][string]$Label = 'after',
    [switch]$AllowBaselineLatencyFailure,
    [string]$DependencyPython = '',
    [string]$RepoHash = '',
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArguments
)

$ErrorActionPreference = 'Stop'
$checkout = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$gwtBinary = Join-Path $checkout 'target/debug/gwt.exe'
$gwtdBinary = Join-Path $checkout 'target/debug/gwtd.exe'
$deps = Join-Path $env:LOCALAPPDATA 'Temp/gwt-playwright-win-1.49.1'
$realProfile = $env:USERPROFILE
$runId = [Guid]::NewGuid().ToString('N')
$checkHome = Join-Path $checkout "target/issue-5539-home-$runId"
$artifacts = Join-Path $checkout "crates/gwt/playwright/test-results/issue-5539-$Label-$runId"
$process = $null
$warmWorker = $null
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
function Get-DigestRecord([string]$path) {
    $item = Get-Item -LiteralPath $path
    return @{path=$item.FullName;sha256=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant();
        bytes=$item.Length;last_write_utc=$item.LastWriteTimeUtc.ToString('o')}
}
try {
    Set-Location $checkout
    if ($AllowBaselineLatencyFailure -and $Label -ne 'baseline') { throw 'Only baseline may waive the p95 assertion.' }
    if (!(Test-Path $gwtBinary) -or !(Test-Path $gwtdBinary)) { throw 'Build checkout gwt and gwtd before browser-check.' }
    if (!(Test-Path "$deps/node_modules/@playwright/test/cli.js") -or !(Test-Path "$deps/resolve-playwright.cjs")) {
        throw "Install pinned Playwright dependencies and resolver at $deps before browser-check."
    }
    $realRuntime = Join-Path $realProfile '.gwt/runtime'
    $manifest = Get-Content (Join-Path $realRuntime 'project_index_runtime_manifest.json') -Raw | ConvertFrom-Json
    $python = if ($DependencyPython) { (Resolve-Path -LiteralPath $DependencyPython).Path } else {
        Join-Path (Join-Path $realRuntime $manifest.venv.path) 'Scripts/python.exe'
    }
    if (!(Test-Path $python)) { throw 'The current runtime manifest does not resolve an installed Python.' }
    $modelCache = if ($env:HF_HOME) { $env:HF_HOME } else { Join-Path $realProfile '.cache/huggingface' }
    if (!(Test-Path $modelCache)) { throw 'The real embedding model must already be cached for this offline browser-check.' }
    $runtime = Join-Path $checkHome '.gwt/runtime'
    New-Item -ItemType Directory -Path $runtime, $artifacts -Force | Out-Null
    # Share installed dependencies only. A runtime-root junction also shares
    # Rust's fixed coordinator path and resident workers with production.
    # The runtime may rebuild a failed probe. Keep its writable venv private;
    # only import dependencies from the supplied or installed environment.
    $requirements = "$checkout/crates/gwt-core/runtime/project_index_requirements.txt"
    $requirementsHash = (Get-FileHash -LiteralPath $requirements -Algorithm SHA256).Hash.Substring(0,16).ToLowerInvariant()
    $privateVenv = "$runtime/venvs/chroma-venv-$requirementsHash"
    & $python -m venv --without-pip $privateVenv
    if ($LASTEXITCODE -ne 0) { throw 'Private browser-check venv creation failed' }
    $dependencySite = Join-Path (Split-Path (Split-Path $python -Parent) -Parent) 'Lib/site-packages'
    $siteLiteral = ($dependencySite.Replace('\','/') | ConvertTo-Json -Compress)
    "import site; site.addsitedir($siteLiteral)" | Set-Content "$privateVenv/Lib/site-packages/gwt-dependencies.pth" -Encoding utf8NoBOM
    Copy-Item -LiteralPath $requirements -Destination "$runtime/project_index_requirements.txt"
    $python = (Resolve-Path -LiteralPath "$privateVenv/Scripts/python.exe").Path
    $repoHash = $RepoHash
    if (!$repoHash) {
        $monitor = Invoke-CheckoutEnvelope 'issue.monitor.status' @{}
        $repoHash = $monitor.project_store.hash
    }
    if (!$repoHash) { throw 'project_store.hash missing' }
    $projectHome = "$checkHome/.gwt/projects/$repoHash"
    New-Item -ItemType Directory -Path "$projectHome/project-state", "$projectHome/work-notes" -Force | Out-Null
    '{"settings":{"auto_start":false}}' | Set-Content "$projectHome/project-state/pm.json" -Encoding utf8NoBOM
    '{"enabled":false,"max_active_agents":1,"priority_order":[]}' | Set-Content "$projectHome/project-state/issue-monitor.json" -Encoding utf8NoBOM
    @'
# Memory

## 2026-10-16 — Issue 5539 search acceptance

Search route responsiveness acceptance. Eight concurrent semantic searches must keep system settings, preset lists, and browser controls responsive. Warmed search returns this memory document.
'@ | Set-Content "$projectHome/work-notes/memory.md" -Encoding utf8NoBOM
    @{tabs=@(@{id='search-route-5539';title='Search route';project_root=$checkout;kind='git'});recent_projects=@()} |
        ConvertTo-Json -Depth 8 | Set-Content "$checkHome/.gwt/session.json" -Encoding utf8NoBOM
    $hook = (Invoke-CheckoutEnvelope 'hook.doctor' @{}).output | ConvertFrom-Json
    $hookBinary = $hook.managed_hook_bin
    if (!$hookBinary -or $hookBinary -match '[/\\]target[/\\].*[/\\]gwtd(\.exe)?$') { throw 'Stable hook fallback missing' }
    $hookParams = @{repair=$true;expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}
    $repaired = (Invoke-CheckoutEnvelope 'hook.doctor' $hookParams).output | ConvertFrom-Json
    Assert-HookHealth $repaired.health
    $childVariables = @{HOME=$checkHome;USERPROFILE=$checkHome;GWT_BROWSER_URL_FILE="$checkHome/url.txt";
        GWT_HOOK_BIN=$hookBinary;GIT_TERMINAL_PROMPT='0';GH_PROMPT_DISABLED='1';GWT_BIN_PATH=$null;
        GWT_DISABLE_BACKGROUND_INDEX='1';GWT_INDEX_FAKE_EMBEDDING='0';
        GWT_INDEX_COORDINATOR_ROOT="$runtime/index-coordinator";
        HF_HOME=$modelCache;HF_HUB_OFFLINE='1';TRANSFORMERS_OFFLINE='1'}
    foreach ($key in $childVariables.Keys) {
        $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key)
        [Environment]::SetEnvironmentVariable($key, $childVariables[$key])
    }
    # Materialize the checkout binary's embedded runner, even when its source
    # differs from the working tree used for the baseline measurement.
    $runtimeStatus = Invoke-CheckoutEnvelope 'index.status' @{}
    $runtimeStatus.output | Set-Content "$artifacts/runtime-status.txt" -Encoding utf8NoBOM
    $activeManifest = Get-Content "$runtime/project_index_runtime_manifest.json" -Raw | ConvertFrom-Json
    $python = (Get-Item (Join-Path (Join-Path $runtime $activeManifest.venv.path) 'Scripts/python.exe')).FullName
    $runner = (Get-Item (Join-Path $runtime $activeManifest.runner.path)).FullName
    # Build a real, embedding-ready memory index in the isolated HOME.
    $indexOutput = & $python $runner --action index-memory --repo-hash $repoHash --project-root $checkout --mode full --qos interactive
    if ($LASTEXITCODE -ne 0) { throw "Real memory indexing failed with exit $LASTEXITCODE" }
    $indexOutput | Set-Content "$artifacts/index-memory.json" -Encoding utf8NoBOM
    $indexResult = $indexOutput | Select-Object -Last 1 | ConvertFrom-Json
    if (!$indexResult.ok -or $indexResult.indexed -ne 1) { throw "Real memory indexing did not produce exactly one document: $($indexOutput -join ' ')" }
    $warmArguments = @($runner, '--action', 'search-multi', '--repo-hash', $repoHash,
        '--project-root', $checkout, '--scopes', 'memory', '--query', 'Search route responsiveness acceptance',
        '--n-results', '50', '--match-mode', 'semantic', '--file-index-protocol', 'v2', '--no-auto-build', '--qos', 'interactive')
    # Match Rust's native worker key so prewarming does not create a second model.
    # Older baseline runners accept this flag and keep their existing default key.
    $identity = ConvertTo-Json -InputObject @($runner, $python, '0', 'interactive') -Compress
    $digest = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($identity))).ToLowerInvariant()
    $warmArguments += @('--worker-dir', (Join-Path $runtime "search-workers/$digest"))
    Write-Output 'Preparing the real resident model before the warmed GUI measurements.'
    $warmOutput = & $python @warmArguments
    $warmExit = $LASTEXITCODE
    $warmOutput | Set-Content "$artifacts/model-prewarm.json" -Encoding utf8NoBOM
    $warmResult = $warmOutput | Select-Object -Last 1 | ConvertFrom-Json
    if ($warmResult.worker_pid) { $warmWorker = Get-Process -Id $warmResult.worker_pid -ErrorAction SilentlyContinue }
    if ($warmExit -ne 0 -or !$warmResult.ok -or $warmResult.memoryResults.Count -ne 1) { throw 'Real resident model prewarm did not return the memory fixture.' }
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
    # Keep --no-tray alive across dark/light browser context replacement.
    $keeper = [System.Net.WebSockets.ClientWebSocket]::new()
    $keeper.Options.SetRequestHeader('Origin', $baseUrl.TrimEnd('/'))
    $null = $keeper.ConnectAsync([Uri](($baseUrl -replace '^http', 'ws') + 'ws'), [Threading.CancellationToken]::None).GetAwaiter().GetResult()
    $healthParams = @{expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}
    $health = (Invoke-CheckoutEnvelope 'hook.health' $healthParams).output | ConvertFrom-Json
    Assert-HookHealth $health
    $activeManifest = Get-Content (Join-Path $runtime 'project_index_runtime_manifest.json') -Raw | ConvertFrom-Json
    @{label=$Label;captured_utc=[DateTime]::UtcNow.ToString('o');checkout=$checkout;
        head=(& git rev-parse HEAD);process_id=$process.Id;runtime_manifest=$activeManifest;
        source_binary_provenance='Working-tree source digests are informational and do not prove binary/source correspondence. The baseline binary digest identifies the measured pre-fix executable.';
        binaries=@((Get-DigestRecord $gwtBinary), (Get-DigestRecord $gwtdBinary));
        sources=@((Get-DigestRecord "$checkout/crates/gwt/src/index_search.rs"),
            (Get-DigestRecord "$checkout/crates/gwt/src/index_worker.rs"),
            (Get-DigestRecord "$checkout/crates/gwt-core/runtime/chroma_index_runner.py"));
        harness=@((Get-DigestRecord "$checkout/crates/gwt/playwright/tests/search-route-live.spec.ts"),
            (Get-DigestRecord $PSCommandPath));
        runtime_runner=(Get-DigestRecord $runner)} |
        ConvertTo-Json -Depth 12 | Set-Content "$artifacts/checkout-evidence.json" -Encoding utf8NoBOM
    $testVariables = @{GWT_PLAYWRIGHT_BASE_URL=$baseUrl;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;
        GWT_PLAYWRIGHT_PID=[string]$process.Id;GWT_PLAYWRIGHT_CHECKOUT_ROOT=$checkout;
        GWT_PLAYWRIGHT_PROJECT_ROOT=$checkout;GWT_SEARCH_CPU_LABEL=$Label;
        GWT_SEARCH_ALLOW_BASELINE_LATENCY_FAILURE=$(if ($AllowBaselineLatencyFailure) { '1' } else { '0' });
        GWT_PLAYWRIGHT_NODE_MODULES="$deps/node_modules";
        NODE_OPTIONS="--require $($deps.Replace('\','/'))/resolve-playwright.cjs";NODE_PATH="$deps/node_modules"}
    foreach ($key in $testVariables.Keys) {
        $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key)
        [Environment]::SetEnvironmentVariable($key, $testVariables[$key])
    }
    & node "$deps/node_modules/@playwright/test/cli.js" test --config crates/gwt/playwright/playwright.config.ts --output "$artifacts/browser" --workers=1 --headed search-route-live.spec.ts @PlaywrightArguments
    if ($LASTEXITCODE -ne 0) { throw "Playwright failed with exit $LASTEXITCODE" }
    Write-Output "Browser evidence: $artifacts"
} finally {
    if ($keeper) { $keeper.Dispose() }
    if ($process -and !$process.HasExited) { $null = $process.WaitForExit(12000) }
    if ($process -and !$process.HasExited) {
        # Terminate only this exact owned process and its descendants.
        & taskkill.exe /PID $process.Id /T /F 2>$null | Out-Null
        $null = $process.WaitForExit(5000)
        if (!$process.HasExited) { throw "Owned checkout PID $($process.Id) survived cleanup" }
    }
    if ($warmWorker -and !$warmWorker.HasExited) {
        Stop-Process -InputObject $warmWorker
        if (!$warmWorker.WaitForExit(5000)) { throw "Owned prewarmed worker PID $($warmWorker.Id) survived cleanup" }
    }
    foreach ($key in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key]) }
}
