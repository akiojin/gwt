[CmdletBinding(PositionalBinding = $false)]
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArguments)

$ErrorActionPreference = 'Stop'
$checkout = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$gwtBinary = Join-Path $checkout 'target/debug/gwt.exe'
$gwtdBinary = Join-Path $checkout 'target/debug/gwtd.exe'
$version = (Get-Content (Join-Path $checkout 'scripts/playwright-version.txt') -Raw).Trim()
$deps = Join-Path "$env:LOCALAPPDATA/Temp" "gwt-playwright-win-$version"
$realProfile = $env:USERPROFILE
# The PM's instruction runtime is intentionally outside its Git worktree.
# Keep HOME outside this checkout too, or Codex would discover this checkout
# as the runtime's ancestor repository and ask to trust unrelated source.
$checkHome = Join-Path ([IO.Path]::GetTempPath()) ('gwt-issue-3431-' + [Guid]::NewGuid().ToString('N'))
$fixture = Join-Path $checkHome 'project'
$evidenceDirectory = Join-Path $checkout ('target/issue-3431-evidence/' + [IO.Path]::GetFileName($checkHome))
$projectState = $null
$process = $null
$keeper = $null
$savedEnvironment = @{}

function Invoke-CheckoutEnvelope($operation, $parameters) {
    $request = @{schema_version=1;operation=$operation;params=$parameters} | ConvertTo-Json -Depth 12 -Compress
    $response = $request | & $gwtdBinary | ConvertFrom-Json
    if (!$response.ok) { throw "$operation failed: $($response.error) $($response.output)" }
    return $response
}
function Assert-HookHealth($health) {
    $blocking = @($health.issues | Where-Object { $_ -notmatch '^managed hook failure: .* state=fail-open( |$)' })
    if ($health.status -eq 'inactive' -or $blocking.Count) { throw "Hook convergence failed: $($blocking -join ', ')" }
}
function Set-CheckEnvironment($values) {
    foreach ($key in $values.Keys) {
        if (!$savedEnvironment.ContainsKey($key)) { $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key) }
        [Environment]::SetEnvironmentVariable($key, $values[$key])
    }
}

try {
    Set-Location $checkout
    if (!(Test-Path $gwtBinary) -or !(Test-Path $gwtdBinary)) { throw 'Build this checkout gwt and gwtd first.' }
    if (!(Test-Path "$deps/node_modules/@playwright/test/cli.js") -or !(Test-Path "$deps/resolve-playwright.cjs")) {
        throw "Install pinned Playwright $version and its resolver at $deps first."
    }
    $hook = (Invoke-CheckoutEnvelope 'hook.doctor' @{}).output | ConvertFrom-Json
    $hookBinary = $hook.managed_hook_bin
    if (!$hookBinary -or $hookBinary -match '[/\\]target[/\\].*[/\\]gwtd(\.exe)?$') { throw 'Stable hook fallback missing.' }
    New-Item -ItemType Directory -Path "$checkHome/.gwt","$checkHome/.codex","$fixture/.codex" -Force | Out-Null
    # Reuse the checkout's actual IPC publisher. The helper is a fixture
    # producer for one owned daemon inbox, never a receipt/ACK/turn writer.
    $publisherSource = @'
use std::{fs,path::PathBuf,time::Duration};
fn main() -> Result<(),Box<dyn std::error::Error>> {
    let mut args=std::env::args().skip(1);
    let home=fs::canonicalize(PathBuf::from(args.next().ok_or("home missing")?))?;
    let project=fs::canonicalize(PathBuf::from(args.next().ok_or("project missing")?))?;
    let input=fs::canonicalize(PathBuf::from(args.next().ok_or("inbox missing")?))?;
    if !project.starts_with(&home) || !input.starts_with(&home) {return Err("fixture escaped owned HOME".into());}
    for (key,_) in std::env::vars_os().collect::<Vec<_>>() {
        if key.to_string_lossy().starts_with("GWT_") {std::env::remove_var(key);}
    }
    std::env::set_var("HOME",&home); std::env::set_var("USERPROFILE",&home);
    let items:Vec<gwt::IssueMonitorInboxItem>=serde_json::from_slice(&fs::read(input)?)?;
    let identity=gwt::runtime_daemon_events::ProjectStoreIdentity::from_project_root(&project);
    let payload=gwt::runtime_daemon_events::issue_monitor_payload_with_project_store("inbox",
        serde_json::to_value(&items)?,std::process::id(),&identity);
    gwt::daemon_publisher::publish_event_with_timeout(&project,"issue_monitor",payload,Duration::from_secs(5))?;
    println!("published owned fixture inbox: {} rows",items.len());
    Ok(())
}
'@
    $publisherSource | Set-Content "$checkHome/signal-publisher.rs" -Encoding utf8NoBOM
    $buildDeps = (Resolve-Path "$checkout/target/debug/deps").Path
    $gwtLib = (Get-ChildItem -LiteralPath $buildDeps -Filter 'libgwt-*.rlib' | Sort-Object LastWriteTime -Descending | Select-Object -First 1).FullName
    $gwtArtifact = [IO.Path]::GetFileNameWithoutExtension($gwtLib).Substring(3)
    $gwtFingerprint = Get-Content "$checkout/target/debug/.fingerprint/$gwtArtifact/lib-gwt.json" -Raw | ConvertFrom-Json
    $jsonDependency = $gwtFingerprint.deps | Where-Object { $_[1] -eq 'serde_json' }
    $jsonFingerprint = [BitConverter]::ToString([BitConverter]::GetBytes([UInt64]$jsonDependency[3])).Replace('-','').ToLowerInvariant()
    # Workspace tests may build another serde feature set more recently.
    # Link the exact serde_json dependency of the selected checkout gwt rlib.
    $jsonLib = Get-ChildItem "$checkout/target/debug/.fingerprint" -Directory -Filter 'serde_json-*' | Where-Object {
        $fingerprint = Join-Path $_.FullName 'lib-serde_json'
        (Test-Path $fingerprint) -and (Get-Content $fingerprint -Raw).Trim() -eq $jsonFingerprint
    } | ForEach-Object { Join-Path $buildDeps ('lib' + $_.Name + '.rlib') } | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (!$gwtLib -or !$jsonLib) { throw 'Build the checkout libraries before this fixture.' }
    $nativeLinkArguments = @()
    foreach ($registry in Get-ChildItem "$realProfile/.cargo/registry/src" -Directory) {
        foreach ($windowsTarget in Get-ChildItem $registry.FullName -Directory -Filter 'windows_x86_64_msvc-*') {
            $nativeLinkArguments += @('-L', ('native=' + (Join-Path $windowsTarget.FullName 'lib')))
        }
    }
    & rustc --edition=2021 "$checkHome/signal-publisher.rs" --extern "gwt=$gwtLib" --extern "serde_json=$jsonLib" -L "dependency=$buildDeps" @nativeLinkArguments -o "$checkHome/signal-publisher.exe"
    if ($LASTEXITCODE -ne 0) { throw 'Owned inbox publisher compilation failed.' }
    if (Test-Path "$realProfile/.gwt/runtime") {
        New-Item -ItemType Junction -Path "$checkHome/.gwt/runtime" -Target "$realProfile/.gwt/runtime" | Out-Null
    }
    if (!(Test-Path "$realProfile/.codex/auth.json")) { throw 'The real Codex provider needs existing authentication.' }
    # Read credentials/config into this owned HOME. Never let provider writes
    # or hook-trust updates modify the user's Codex configuration.
    Copy-Item -LiteralPath "$realProfile/.codex/auth.json" -Destination "$checkHome/.codex/auth.json"
    $providerConfig = Get-Content "$realProfile/.codex/config.toml"
    $configPrefix = [System.Collections.Generic.List[string]]::new()
    $inFeatures = $false
    foreach ($line in $providerConfig) {
        if ($line -match '^\[') {
            if ($line -eq '[features]') { $inFeatures = $true } else { break }
        }
        $configPrefix.Add($line)
    }
    if (!$inFeatures) { throw 'Copy the existing provider features before running this real-provider acceptance.' }
    $configPrefix | Set-Content "$checkHome/.codex/config.toml" -Encoding utf8NoBOM
    # This is an independent, remote-free fixture repository, never the user's
    # repository or an implementation worktree. No GitHub credential is bridged.
    git -C $fixture init --quiet
    if ($LASTEXITCODE -ne 0) { throw 'Fixture git init failed.' }
    'This repository is a terminal transport fixture. Never contact GitHub, invoke Issue/PR/Board mutations, inspect other projects, or delegate work. Follow only the supplied harmless diagnostic prompt.' |
        Set-Content "$fixture/AGENTS.md" -Encoding utf8NoBOM
    @'
# PM transport fixture policy

Policy sentinel: GWT_PM_TRANSPORT_POLICY_3431

This explicitly opted-in policy belongs only to the remote-free Issue #3431
terminal transport fixture. The session remains the registered resident PM.

A task-notification with status diagnostic and task-id 3431-busy-* or
3431-queued-* is automated fixture setup, not human intake or a PM supervision
cycle. For these diagnostic turns, do not initialize PM state, inventory the
fleet or PRs, read unrelated skills, inspect other projects, delegate, or post
reports. Read this selected policy, execute only the supplied harmless
PowerShell command, poll only that command if it is still running, wait for its
real completion, and return the requested exact reply. A queued diagnostic
needs only its requested reply. Never contact GitHub during diagnostic setup.

Actual scheduled supervision and Monitor activity wake prompts are not covered
by that diagnostic exception. Follow the normal gwt-pm contract for those turns.
All existing hooks, Stop behavior, runtime state, reporting gates, authority,
and delivery checks remain active. The external test harness owns Pause/Resume
and fixture cleanup; do not change those controls yourself.

Never alter hooks, receipts, the delivery ledger, provider history, runtime
sidecars, or any gate. Never manufacture an acknowledgement, turn event, or
completion. Do not modify repository files or operate on another project.
'@ | Set-Content "$fixture/PM-TRANSPORT.md" -Encoding utf8NoBOM
    git -C $fixture add -- AGENTS.md PM-TRANSPORT.md
    git -C $fixture -c user.name='gwt fixture' -c user.email='fixture@example.invalid' commit --quiet -m 'test: terminal transport fixture'
    if ($LASTEXITCODE -ne 0) { throw 'Fixture initial commit failed.' }
    '{"hooks":{}}' | Set-Content "$fixture/.codex/hooks.json" -Encoding utf8NoBOM

    $isolated = @{}
    Get-ChildItem Env:GWT_* | ForEach-Object { $isolated[$_.Name] = $null }
    $isolated.HOME = $checkHome
    $isolated.USERPROFILE = $checkHome
    $isolated.CODEX_HOME = "$checkHome/.codex"
    $isolated.CODEX_THREAD_ID = $null
    $isolated.CODEX_INTERNAL_ORIGINATOR_OVERRIDE = $null
    $isolated.GH_TOKEN = $null
    $isolated.GITHUB_TOKEN = $null
    $isolated.GWT_HOOK_BIN = $hookBinary
    $isolated.GWT_DISABLE_BACKGROUND_INDEX = '1'
    $isolated.GWT_BROWSER_URL_FILE = "$checkHome/url.txt"
    $isolated.RUST_LOG = 'gwt=info,warn'
    $isolated.GIT_TERMINAL_PROMPT = '0'
    $isolated.GH_PROMPT_DISABLED = '1'
    Set-CheckEnvironment $isolated
    Push-Location $fixture
    try {
        $monitor = Invoke-CheckoutEnvelope 'issue.monitor.status' @{}
        $repoHash = $monitor.project_store.hash
        if (!$repoHash) { throw 'Canonical fixture project_store.hash missing.' }
        $projectState = "$checkHome/.gwt/projects/$repoHash/project-state"
        New-Item -ItemType Directory -Path $projectState -Force | Out-Null
        $pmRuntime = "$checkHome/.gwt/projects/$repoHash/pm/runtime"
        New-Item -ItemType Directory -Path $pmRuntime -Force | Out-Null
        @{settings=@{auto_start=$false;paused=$true;loop_interval_secs=10;launch_profile=@{agent_id='codex'};
            project_policy_files=@('PM-TRANSPORT.md')}} |
            ConvertTo-Json -Depth 8 | Set-Content "$projectState/pm.json" -Encoding utf8NoBOM
        # Enabled snapshots can wake the PM; autonomous mode and an empty
        # profile pool prevent this fixture from launching implementation agents.
        @{enabled=$true;autonomous_mode=$false;max_active_agents=1;priority_order=@();launch_profiles=@()} |
            ConvertTo-Json -Depth 8 | Set-Content "$projectState/issue-monitor.json" -Encoding utf8NoBOM
        Invoke-CheckoutEnvelope 'concern.create' @{summary='Owned terminal transport fixture';
            symptom_measurement=@{kind='shell_command';command='Write-Output ''{"count":1}'''};
            baseline=@{count=1};verification_predicate=@{pointer='/count';op='eq';expected=0};
            owner_issues=@();escalate_after_cycles=1000} | Out-Null
        $params = @{repair=$true;expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}
        Invoke-CheckoutEnvelope 'hook.doctor' $params | Out-Null
        Invoke-CheckoutEnvelope 'hook.register_codex_managed_hook_trust' @{codex_config="$checkHome/.codex/config.toml";
            codex_hook_discovery='both';project_root=$fixture} | Out-Null
        # Register only this owned runtime before launch. Codex's folder trust
        # screen can otherwise arrive after its first empty composer render.
        Invoke-CheckoutEnvelope 'hook.register_codex_managed_project_trust' @{codex_config="$checkHome/.codex/config.toml";
            project_root=$pmRuntime} | Out-Null
        Assert-HookHealth ((Invoke-CheckoutEnvelope 'hook.health' @{expected_hook_bin=$hookBinary;
            runtime_state_path="$checkHome/.gwt/missing-runtime.json"}).output | ConvertFrom-Json)
    } finally { Pop-Location }
    @{active_tab_id='wake-3431';tabs=@(@{id='wake-3431';title='PM wake diagnostic';project_root=$fixture;kind='git'});recent_projects=@()} |
        ConvertTo-Json -Depth 8 | Set-Content "$checkHome/.gwt/session.json" -Encoding utf8NoBOM
    $process = Start-Process -FilePath $gwtBinary -ArgumentList '--no-tray','--no-open' -WorkingDirectory $checkout -WindowStyle Hidden -PassThru -RedirectStandardOutput "$checkHome/stdout.log" -RedirectStandardError "$checkHome/stderr.log"
    New-Item -ItemType Directory -Path "$checkout/target/issue-3431-evidence" -Force | Out-Null
    @{home=$checkHome;fixture=$fixture;project_state=$projectState;pid=$process.Id;gwt=$gwtBinary;gwtd=$gwtdBinary;evidence=$evidenceDirectory} |
        ConvertTo-Json | Set-Content "$checkout/target/issue-3431-evidence/browser-latest.json" -Encoding utf8NoBOM
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
    $null = $keeper.ConnectAsync([Uri](($baseUrl -replace '^http','ws') + 'ws'), [Threading.CancellationToken]::None).GetAwaiter().GetResult()
    Push-Location $fixture
    try {
        Assert-HookHealth ((Invoke-CheckoutEnvelope 'hook.health' @{expected_hook_bin=$hookBinary;runtime_state_path="$checkHome/.gwt/missing-runtime.json"}).output | ConvertFrom-Json)
    } finally { Pop-Location }
    # Restore the launching execution's environment before Playwright. Its
    # child provider already inherited only the isolated GUI environment.
    foreach ($key in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($key,$savedEnvironment[$key]) }
    Set-CheckEnvironment @{GWT_PLAYWRIGHT_BASE_URL=$baseUrl;GWT_PLAYWRIGHT_CHECK_HOME=$checkHome;
        GWT_PLAYWRIGHT_PID=[string]$process.Id;GWT_PLAYWRIGHT_CHECKOUT_ROOT=$checkout;
        GWT_PLAYWRIGHT_PROJECT_ROOT=$fixture;GWT_PLAYWRIGHT_PROJECT_STATE=$projectState;
        GWT_PLAYWRIGHT_SIGNAL_PUBLISHER="$checkHome/signal-publisher.exe";
        GWT_PLAYWRIGHT_REAL_PM_WAKE='1';GWT_PLAYWRIGHT_NODE_MODULES="$deps/node_modules";
        NODE_OPTIONS="--require $($deps.Replace('\','/'))/resolve-playwright.cjs";NODE_PATH="$deps/node_modules"}
    & node "$deps/node_modules/@playwright/test/cli.js" test --config crates/gwt/playwright/playwright.config.ts --workers=1 pm-wake-submit-live.spec.ts @PlaywrightArguments
    if ($LASTEXITCODE -ne 0) { throw "Playwright failed with exit $LASTEXITCODE" }
} finally {
    try {
        if ($keeper) { $keeper.Dispose() }
        if ($process -and !$process.HasExited) {
            & taskkill.exe /PID $process.Id /T /F 2>$null | Out-Null
            $null = $process.WaitForExit(10000)
            if (!$process.HasExited) { throw "Owned checkout PID $($process.Id) survived cleanup." }
        }
        $ownedAuth = Join-Path $checkHome '.codex/auth.json'
        if (Test-Path -LiteralPath $ownedAuth) { Remove-Item -LiteralPath $ownedAuth }
        # verify.run removes its temporary HOME after this wrapper returns.
        # Export only these non-secret diagnostic files, never a whole HOME.
        if (Test-Path -LiteralPath $checkHome) {
            New-Item -ItemType Directory -Path $evidenceDirectory -Force | Out-Null
            foreach ($artifact in Get-ChildItem -LiteralPath $checkHome -File | Where-Object { $_.Name -match '^wake-.*\.(json|jsonl|png)$' }) {
                Copy-Item -LiteralPath $artifact.FullName -Destination $evidenceDirectory
            }
            if ($projectState) {
                $receiptLedger = Join-Path $projectState 'pm-delivery-receipts.jsonl'
                if (Test-Path -LiteralPath $receiptLedger) { Copy-Item -LiteralPath $receiptLedger -Destination $evidenceDirectory }
            }
            Write-Output "Preserved wake evidence: $evidenceDirectory"
        }
    } finally {
        foreach ($key in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($key,$savedEnvironment[$key]) }
        Set-Location $checkout
    }
}
