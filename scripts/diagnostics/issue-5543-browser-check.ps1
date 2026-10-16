# Build this checkout's gwt and gwtd first. Reuse -SeedHome for before/after comparisons.
[CmdletBinding(PositionalBinding = $false)]
param(
    [ValidateSet('before', 'after')][string]$Label = 'after',
    [string]$ProjectRoot = 'E:\gwt',
    [string]$SeedHome = '',
    [ValidateRange(0, 7200)][int]$ObserveSeconds = 0,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArguments
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This fixture requires Windows and PowerShell 7.' }
$checkout = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$project = (Resolve-Path -LiteralPath $ProjectRoot).Path
$gwtBinary = Join-Path $checkout 'target/debug/gwt.exe'
$gwtdBinary = Join-Path $checkout 'target/debug/gwtd.exe'
foreach ($binary in @($gwtBinary, $gwtdBinary)) {
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw 'Build checkout gwt and gwtd before browser-check.' }
}
$realProfile = $env:USERPROFILE
$runId = [guid]::NewGuid().ToString('N')
$checkHome = Join-Path $checkout "target/issue-5543-home-$runId"
$artifacts = Join-Path $checkout "crates/gwt/playwright/test-results/issue-5543-$Label-$runId"
if (-not $SeedHome) { $SeedHome = Join-Path $checkout "target/issue-5543-seed-$runId" }
$SeedHome = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($SeedHome)
$productionState = [IO.Path]::GetFullPath((Join-Path $realProfile '.gwt')).TrimEnd('\', '/')
if ($SeedHome.TrimEnd('\', '/') -eq $productionState -or
    $SeedHome.StartsWith($productionState + '\', [StringComparison]::OrdinalIgnoreCase) -or
    $SeedHome.TrimEnd('\', '/') -eq $realProfile.TrimEnd('\', '/')) {
    throw 'SeedHome must be separate from the production HOME and .gwt state.'
}
foreach ($directory in @('.gwt', '.codex', 'tmp')) {
    [void](New-Item -ItemType Directory -Path (Join-Path $checkHome $directory) -Force)
}
[void](New-Item -ItemType Directory -Path $artifacts -Force)
[IO.File]::WriteAllText((Join-Path $checkHome '.codex/config.toml'), '')
# Runtime code is the only shared directory. All measured application state is copied.
$runtime = Join-Path $realProfile '.gwt/runtime'
if (Test-Path -LiteralPath $runtime -PathType Container) {
    [void](New-Item -ItemType Junction -Path (Join-Path $checkHome '.gwt/runtime') -Target $runtime)
}
$childEnvironment = @{}
foreach ($entry in [Environment]::GetEnvironmentVariables().GetEnumerator()) {
    if ($entry.Key -notmatch '^(GWT_|CODEX_|CLAUDE_|GH_TOKEN$|GITHUB_TOKEN$|OPENAI_API_KEY$|ANTHROPIC_API_KEY$)') {
        $childEnvironment[$entry.Key] = [string]$entry.Value
    }
}
$childEnvironment.HOME = $checkHome
$childEnvironment.USERPROFILE = $checkHome
$childEnvironment.CODEX_HOME = Join-Path $checkHome '.codex'
$childEnvironment.TEMP = Join-Path $checkHome 'tmp'
$childEnvironment.TMP = $childEnvironment.TEMP
$childEnvironment.GWT_PROJECT_ROOT = $project
$childEnvironment.GWT_DISABLE_BACKGROUND_INDEX = '1'
$childEnvironment.GWT_PLAYWRIGHT_ACTIVE_WORK_RENDEZVOUS = '1'
$childEnvironment.GIT_TERMINAL_PROMPT = '0'
$childEnvironment.GH_PROMPT_DISABLED = '1'
# Codex's inherited RUST_LOG selects only its own targets and would hide gwt's
# acceptance telemetry. Keep the isolated process's logging deterministic.
$childEnvironment.RUST_LOG = 'info'

function New-CheckoutStartInfo([string]$Binary) {
    $startInfo = [Diagnostics.ProcessStartInfo]::new($Binary)
    $startInfo.WorkingDirectory = $checkout
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
function Invoke-CheckoutEnvelope([string]$Operation, [hashtable]$Parameters) {
    $startInfo = New-CheckoutStartInfo $gwtdBinary
    $startInfo.RedirectStandardInput = $true
    $startInfo.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
    $child = [Diagnostics.Process]::Start($startInfo)
    try {
        $child.StandardInput.WriteLine((@{schema_version=1;operation=$Operation;params=$Parameters} | ConvertTo-Json -Depth 10 -Compress))
        $child.StandardInput.Close()
        $stdout = $child.StandardOutput.ReadToEndAsync()
        $stderr = $child.StandardError.ReadToEndAsync()
        if (-not $child.WaitForExit(60000)) { $child.Kill($true); throw "$Operation timed out." }
        $result = $stdout.GetAwaiter().GetResult() | ConvertFrom-Json
        if ($child.ExitCode -ne 0 -or -not $result.ok) { throw "$Operation failed: $($stderr.GetAwaiter().GetResult()); $($result.output)" }
        return $result
    } finally { $child.Dispose() }
}
function Assert-HookHealth($Health) {
    $blocking = @($Health.issues | Where-Object {
        $_ -notmatch '^managed hook failure: .* state=fail-open( |$)' -and
        -not ($hookBin -eq 'gwtd' -and -not $publicGwtd -and $_ -match '^managed hook binary missing: .* uses gwtd$')
    })
    if ($Health.status -eq 'inactive' -or $blocking.Count) { throw "Hook convergence failed: $($blocking -join '; ')" }
}
function Assert-NoReparsePoint([string]$Directory) {
    $links = @(@(Get-Item -LiteralPath $Directory -Force) + @(Get-ChildItem -LiteralPath $Directory -Recurse -Force) |
        Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint })
    if ($links.Count) { throw "Snapshot contains a reparse point: $($links[0].FullName)" }
}
function Copy-DirectorySnapshot([string]$Source, [string]$Destination) {
    & robocopy.exe $Source $Destination /E /NFL /NDL /NJH /NJS /NP /R:0 /W:0 | Out-Null
    if ($LASTEXITCODE -ge 8) { throw "Snapshot copy failed ($LASTEXITCODE): $Source" }
}

$repoHash = (Invoke-CheckoutEnvelope 'issue.monitor.status' @{}).project_store.hash
if ($repoHash -notmatch '^[0-9a-f]{16}$') { throw 'Missing canonical project_store.hash.' }
$seedManifestPath = Join-Path $SeedHome 'issue-5543-seed.json'
if (Test-Path -LiteralPath $SeedHome) {
    if (-not (Test-Path -LiteralPath $seedManifestPath -PathType Leaf)) { throw 'Existing SeedHome has no Issue #5543 snapshot manifest; refusing to overwrite it.' }
    $seed = Get-Content -LiteralPath $seedManifestPath -Raw | ConvertFrom-Json
    if ($seed.repo_hash -ne $repoHash) { throw 'SeedHome belongs to a different repository.' }
} else {
    [void](New-Item -ItemType Directory -Path $SeedHome)
    $copied = @()
    foreach ($relative in @(
        ".gwt/cache/issues/$repoHash", '.gwt/sessions',
        ".gwt/projects/$repoHash/project-state/works.json",
        ".gwt/projects/$repoHash/project-state/works.detachments.json",
        ".gwt/cache/issue-links/$repoHash.json"
    )) {
        $source = Join-Path $realProfile $relative
        if (Test-Path -LiteralPath $source) {
            $destination = Join-Path $SeedHome $relative
            [void](New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force)
            if (Test-Path -LiteralPath $source -PathType Container) {
                Assert-NoReparsePoint $source
                Copy-DirectorySnapshot $source $destination
            }
            else { Copy-Item -LiteralPath $source -Destination $destination }
            $files = @(Get-ChildItem -LiteralPath $destination -File -Recurse)
            $copied += @{path=$relative;files=$files.Count;bytes=($files | Measure-Object -Property Length -Sum).Sum}
        } else { $copied += @{path=$relative;files=0;bytes=0;absent=$true} }
    }
    if (-not (Test-Path -LiteralPath (Join-Path $SeedHome ".gwt/cache/issues/$repoHash") -PathType Container)) { throw 'The real profile has no Issue cache to measure.' }
    $seed = @{schema_version=1;seed_id=$runId;repo_hash=$repoHash;captured_at=[DateTime]::UtcNow.ToString('o');copied=$copied}
    $seed | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $seedManifestPath -Encoding utf8NoBOM
}
# Reusing a seed never changes it. Each GUI receives a new copy of its fixed data.
Assert-NoReparsePoint $SeedHome
foreach ($entry in Get-ChildItem -LiteralPath (Join-Path $SeedHome '.gwt') -Force) {
    $destination = Join-Path $checkHome ".gwt/$($entry.Name)"
    if ($entry.PSIsContainer) { Copy-DirectorySnapshot $entry.FullName $destination }
    else { Copy-Item -LiteralPath $entry.FullName -Destination $destination -Force }
}
$projectState = Join-Path $checkHome ".gwt/projects/$repoHash/project-state"
[void](New-Item -ItemType Directory -Path $projectState -Force)
[IO.File]::WriteAllText((Join-Path $projectState 'pm.json'), '{"settings":{"auto_start":false}}')
[IO.File]::WriteAllText((Join-Path $projectState 'issue-monitor.json'), '{"enabled":false,"max_active_agents":1,"priority_order":[]}')
@{active_tab_id='issue-load-5543';tabs=@(@{id='issue-load-5543';title='Issue load';project_root=$project;kind='git'});recent_projects=@()} |
    ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $checkHome '.gwt/session.json') -Encoding utf8NoBOM

$localHookPattern = '(?i)(^|[/\\])target[/\\](?:[^/\\]+[/\\])*(debug|release)[/\\]gwtd(\.exe)?$'
$hookBin = $env:GWT_HOOK_BIN
if ($hookBin -and ($hookBin -match $localHookPattern -or ($hookBin -ne 'gwtd' -and -not (Test-Path -LiteralPath $hookBin -PathType Leaf)))) { $hookBin = '' }
$publicGwtd = Get-Command gwtd -CommandType Application -ErrorAction SilentlyContinue
if (-not $hookBin) { $hookBin = ((Invoke-CheckoutEnvelope 'hook.doctor' @{}).output | ConvertFrom-Json).managed_hook_bin }
if (-not $hookBin -or $hookBin -match $localHookPattern) { throw 'Managed hook resolver did not provide a stable fallback.' }
$childEnvironment.GWT_HOOK_BIN = $hookBin
$hookParameters = @{expected_hook_bin=$hookBin;runtime_state_path=(Join-Path $checkHome '.gwt/missing-runtime-state.json')}
[void](Invoke-CheckoutEnvelope 'hook.doctor' ($hookParameters + @{repair=$true}))
[void](Invoke-CheckoutEnvelope 'hook.register_codex_managed_hook_trust' @{
    codex_config=(Join-Path $checkHome '.codex/config.toml');codex_hook_discovery='both';project_root=$checkout
})
Assert-HookHealth (((Invoke-CheckoutEnvelope 'hook.health' $hookParameters).output | ConvertFrom-Json))

$ownedGui = $null
$keeper = $null
$savedEnvironment = @{}
$exitCode = 1
try {
    $urlFile = Join-Path $checkHome 'url.txt'
    $startInfo = New-CheckoutStartInfo $gwtBinary
    $startInfo.ArgumentList.Add('--no-tray')
    $startInfo.ArgumentList.Add('--no-open')
    $startInfo.Environment['GWT_BROWSER_URL_FILE'] = $urlFile
    $ownedGui = [Diagnostics.Process]::Start($startInfo)
    $ownedPid = $ownedGui.Id
    $stdoutTask = $ownedGui.StandardOutput.ReadToEndAsync()
    $stderrTask = $ownedGui.StandardError.ReadToEndAsync()
    Write-Output "Owned checkout PID: $ownedPid; isolated HOME: $checkHome; fixed seed: $SeedHome"
    $deadline = [DateTime]::UtcNow.AddSeconds(120)
    $baseUrl = ''
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($ownedGui.HasExited) { throw 'Fresh checkout exited during startup.' }
        if (Test-Path -LiteralPath $urlFile -PathType Leaf) {
            $candidate = [IO.File]::ReadAllText($urlFile).Trim()
            try { if ($candidate -and (Invoke-WebRequest -Uri $candidate -Method Head -TimeoutSec 2).StatusCode -eq 200) { $baseUrl = $candidate; break } } catch { }
        }
        Start-Sleep -Milliseconds 200
    }
    if (-not $baseUrl) { throw 'Fresh checkout URL never became ready.' }
    $keeper = [Net.WebSockets.ClientWebSocket]::new()
    $keeper.Options.SetRequestHeader('Origin', ([Uri]$baseUrl).GetLeftPart([UriPartial]::Authority))
    $socketUrl = [UriBuilder]::new($baseUrl)
    $socketUrl.Scheme = 'ws'
    $socketUrl.Path = '/ws'
    $keeper.ConnectAsync($socketUrl.Uri, [Threading.CancellationToken]::None).GetAwaiter().GetResult() | Out-Null
    $health = (Invoke-CheckoutEnvelope 'hook.health' $hookParameters).output | ConvertFrom-Json
    Assert-HookHealth $health
    $fixture = @{issue=5543;label=$Label;pid=$ownedPid;project=$project;checkout=$checkout;repo_hash=$repoHash;
        seed_home=$SeedHome;seed=$seed;hook_health=$health;started_at=[DateTime]::UtcNow.ToString('o');
        runtime_log_filter=$childEnvironment.RUST_LOG;
        binary_sha256=(Get-FileHash -LiteralPath $gwtBinary -Algorithm SHA256).Hash;
        gwtd_sha256=(Get-FileHash -LiteralPath $gwtdBinary -Algorithm SHA256).Hash}
    $fixture | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $checkHome 'issue-5543-isolated.json') -Encoding utf8NoBOM
    Copy-Item -LiteralPath (Join-Path $checkHome 'issue-5543-isolated.json') -Destination $artifacts
    $version = [IO.File]::ReadAllText((Join-Path $checkout 'scripts/playwright-version.txt')).Trim()
    $deps = if ($env:GWT_PLAYWRIGHT_DEPS_DIR) { $env:GWT_PLAYWRIGHT_DEPS_DIR } else { Join-Path $env:LOCALAPPDATA "Temp/gwt-playwright-win-$version" }
    $cli = Join-Path $deps 'node_modules/@playwright/test/cli.js'
    if (-not (Test-Path -LiteralPath $cli -PathType Leaf)) { throw "Pinned Playwright $version is missing at $deps; set GWT_PLAYWRIGHT_DEPS_DIR to its installation." }
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
'@ | Set-Content -LiteralPath $resolver -Encoding utf8NoBOM
    $runnerEnvironment = @{GWT_PLAYWRIGHT_BASE_URL=$baseUrl;GWT_PLAYWRIGHT_PROJECT_KEY=$repoHash;
        GWT_KNOWLEDGE_LOAD_OBSERVE_SECONDS=[string]$ObserveSeconds;
        GWT_PLAYWRIGHT_PROJECT_ROOT=$project;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;GWT_PLAYWRIGHT_PID=[string]$ownedPid;
        GWT_PLAYWRIGHT_NODE_MODULES=(Join-Path $deps 'node_modules');
        NODE_OPTIONS="--require `"$($resolver.Replace('\', '/'))`" $env:NODE_OPTIONS"}
    foreach ($key in $runnerEnvironment.Keys) {
        $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key)
        [Environment]::SetEnvironmentVariable($key, $runnerEnvironment[$key])
    }
    Push-Location $checkout
    try {
        & node $cli test --config crates/gwt/playwright/playwright.config.ts --output $artifacts --workers=1 --headed knowledge-load-live.spec.ts @PlaywrightArguments
        $exitCode = $LASTEXITCODE
    } finally { Pop-Location }
} finally {
    foreach ($key in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key]) }
    if ($keeper) { $keeper.Abort(); $keeper.Dispose() }
    if ($ownedGui) {
        if (-not $ownedGui.HasExited) { $ownedGui.Kill($true) }
        if (-not $ownedGui.WaitForExit(10000)) { throw "Owned GUI PID $ownedPid remained after cleanup." }
        [IO.File]::WriteAllText((Join-Path $checkHome 'stdout.log'), $stdoutTask.GetAwaiter().GetResult())
        [IO.File]::WriteAllText((Join-Path $checkHome 'stderr.log'), $stderrTask.GetAwaiter().GetResult())
        $ownedGui.Dispose()
    }
    Write-Output "Evidence: $artifacts; isolated HOME: $checkHome; reusable seed: $SeedHome; owned process cleanup finished."
}
if ($exitCode -ne 0) { throw "Issue #5543 headed verification failed ($exitCode)." }
