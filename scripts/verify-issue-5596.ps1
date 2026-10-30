# Offline owned-process acceptance harness; additional arguments reach Playwright unchanged.
[CmdletBinding(PositionalBinding = $false)]
param(
  [ValidateSet('baseline','after')][string]$Phase = 'after',
  [string]$EvidenceDirectory,
  [switch]$Sequential,
  [ValidateRange(1,10)][int]$LaunchCount = 10,
  [switch]$CargoLoad,
  [switch]$PostAckDiagnostics,
  [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArgs
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This harness requires Windows and PowerShell 7.' }
if (($CargoLoad -or $PSBoundParameters.ContainsKey('LaunchCount')) -and -not $Sequential) { throw 'LaunchCount and CargoLoad require Sequential.' }
if ($PostAckDiagnostics -and (-not $Sequential -or -not $CargoLoad -or $LaunchCount -ne 2)) { throw 'PostAckDiagnostics requires Sequential, CargoLoad and LaunchCount 2; run each theme in a fresh fixture.' }
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$gwt = Join-Path $repoRoot 'target/debug/gwt.exe'
$gwtd = Join-Path $repoRoot 'target/debug/gwtd.exe'
foreach ($binary in @($gwt,$gwtd)) { if (-not (Test-Path -LiteralPath $binary)) { throw 'Build this checkout: cargo build -p gwt --bin gwt --bin gwtd' } }
$checkHome = Join-Path ([IO.Path]::GetTempPath()) ('gwt-launch-5596-' + [guid]::NewGuid())
if (-not $EvidenceDirectory) {
  $owner = if ($Sequential) { 'issue-6096' } else { 'issue-5596' }
  $EvidenceDirectory = Join-Path $repoRoot "target/verification/$owner/$Phase"
}
$EvidenceDirectory = [IO.Path]::GetFullPath($EvidenceDirectory)
foreach ($folder in @($checkHome,$EvidenceDirectory,"$checkHome/.gwt","$checkHome/.codex","$checkHome/tmp","$checkHome/bin","$checkHome/project")) {
  [void](New-Item -ItemType Directory -Path $folder -Force)
}
$project = "$checkHome/project"
[void](New-Item -ItemType Directory -Path "$project/.codex" -Force)
[IO.File]::WriteAllText("$project/.codex/hooks.json", '{}')
$node = (Get-Command node -CommandType Application).Source
$realGit = (Get-Command git -CommandType Application).Source
# Resolve the selected compiler before fresh HOME hides rustup's toolchain state.
$rustc = (& rustup which rustc).Trim()
if ($LASTEXITCODE -or -not (Test-Path -LiteralPath $rustc -PathType Leaf)) { throw 'Cannot resolve the active Rust compiler.' }
$cargo = (& rustup which cargo).Trim()
if ($LASTEXITCODE -or -not (Test-Path -LiteralPath $cargo -PathType Leaf)) { throw 'Cannot resolve the active Cargo executable.' }
$environment = @{}
foreach ($entry in [Environment]::GetEnvironmentVariables().GetEnumerator()) {
  if ($entry.Key -notmatch '^(GWT_|CODEX_|CLAUDE_|GH_TOKEN$|GITHUB_TOKEN$|OPENAI_API_KEY$|ANTHROPIC_API_KEY$)') { $environment[$entry.Key] = [string]$entry.Value }
}
$environment.HOME = $checkHome
$environment.USERPROFILE = $checkHome
$environment.CODEX_HOME = "$checkHome/.codex"
$environment.TEMP = "$checkHome/tmp"
$environment.TMP = "$checkHome/tmp"
$environment.GIT_CONFIG_GLOBAL = "$checkHome/.gitconfig"
$environment.GIT_CONFIG_NOSYSTEM = '1'
$environment.GIT_TERMINAL_PROMPT = '0'
$environment.GH_PROMPT_DISABLED = '1'
$environment.GWT5596_REAL_GIT = $realGit
$environment.RUST_LOG = 'info,gwt.frontend.timing=debug,gwt_input_trace=debug'
[IO.File]::WriteAllText("$checkHome/.gitconfig", "[user]`nname = GwtFixture`nemail = fixture@example.invalid`n[protocol]`nallow = never`n[protocol `"file`"]`nallow = always`n")
[IO.File]::WriteAllText("$checkHome/.codex/config.toml", "approval_policy = 'never'`nsuppress_unstable_features_warning = true`n")
[IO.File]::WriteAllText("$checkHome/.codex/auth.json", '{"OPENAI_API_KEY":"fixture-only"}')
[IO.File]::WriteAllText("$checkHome/.gwt/config.toml", "auto_apply_updates = false`n[perf]`nenabled = true`n")
$realRuntime = Join-Path ([Environment]::GetFolderPath('UserProfile')) '.gwt/runtime'
if (Test-Path -LiteralPath $realRuntime) { [void](New-Item -ItemType Junction -Path "$checkHome/.gwt/runtime" -Target $realRuntime) }
function New-OwnedProcess([string]$File, [string[]]$Arguments, [string]$WorkingDirectory = $project) {
  $start = [Diagnostics.ProcessStartInfo]::new($File)
  $start.WorkingDirectory = $WorkingDirectory
  $start.UseShellExecute = $false
  $start.CreateNoWindow = $true
  $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
  $start.Environment.Clear()
  foreach ($key in $environment.Keys) { $start.Environment[$key] = $environment[$key] }
  foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
  $start.RedirectStandardOutput = $true
  $start.RedirectStandardError = $true
  return $start
}
function Invoke-Owned([string]$File, [string[]]$Arguments) {
  $process = [Diagnostics.Process]::Start((New-OwnedProcess $File $Arguments))
  try {
    $output = $process.StandardOutput.ReadToEndAsync()
    $errors = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(120000)) { $process.Kill($true); throw 'Fixture command timed out.' }
    if ($process.ExitCode) { throw "Fixture command failed: $($errors.GetAwaiter().GetResult())" }
    return $output.GetAwaiter().GetResult().Trim()
  } finally { $process.Dispose() }
}
function Invoke-IsolatedGwtd([string]$Operation, [hashtable]$Parameters = @{}) {
  $start = New-OwnedProcess $gwtd @()
  $start.RedirectStandardInput = $true
  $start.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
  $process = [Diagnostics.Process]::Start($start)
  try {
    $process.StandardInput.WriteLine((@{schema_version=1;operation=$Operation;params=$Parameters} | ConvertTo-Json -Depth 9 -Compress))
    $process.StandardInput.Close()
    $output = $process.StandardOutput.ReadToEndAsync()
    $errors = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(60000)) { $process.Kill($true); throw "$Operation timed out." }
    $envelope = $output.GetAwaiter().GetResult() | ConvertFrom-Json
    if ($process.ExitCode -or -not $envelope.ok) { throw "$Operation failed (diagnostic output withheld)." }
    [void]$errors.GetAwaiter().GetResult()
    return $envelope
  } finally { $process.Dispose() }
}
function Assert-Hooks($Envelope) {
  $health = $Envelope.output | ConvertFrom-Json
  [IO.File]::WriteAllText("$EvidenceDirectory/hook-health-latest.json", ($health | ConvertTo-Json -Depth 8))
  $blocking = @($health.issues | Where-Object { $_ -notmatch '^managed hook failure: .* state=fail-open( |$)' })
  if ($health.status -eq 'inactive' -or $blocking.Count) { throw 'Managed-hook convergence failed.' }
  return $health
}
$gui = $null
$github = $null
$runner = $null
$cargoLoadProcess = $null
$observer = $null
$anchor = $null
$state = $null
$exitCode = 1
$streams = @{}
try {
  # Compile only the local command fixture, never an agent/provider package.
  # Keep the caller's HOME and Cargo caches for this host build. Its artifact
  # messages bind both extern crates to one dependency graph, including features.
  $artifactLines = & $cargo build --offline -p gwt --lib --message-format=json --target-dir "$repoRoot/target" --manifest-path "$repoRoot/Cargo.toml"
  if ($LASTEXITCODE) { throw 'Cannot resolve fixture libraries from the host Cargo build.' }
  $artifacts = @($artifactLines | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object reason -EQ 'compiler-artifact')
  $libraries = @{}
  foreach ($crateName in @('gwt','serde_json')) {
    $paths = @($artifacts | Where-Object { $_.target.name -eq $crateName -and $_.target.kind -contains 'lib' } | ForEach-Object {
      $_.filenames | Where-Object { $_.EndsWith('.rlib') }
    } | Sort-Object -Unique)
    if ($paths.Count -ne 1 -or -not (Test-Path -LiteralPath $paths[0] -PathType Leaf)) { throw "Missing or ambiguous $crateName rlib in the host Cargo artifact messages." }
    $libraries[$crateName] = Get-Item -LiteralPath $paths[0]
  }
  $deps = Join-Path $repoRoot 'target/debug/deps'
  $jsonLibrary = $libraries['serde_json']
  $gwtLibrary = $libraries['gwt']
  $rustArgs = @('--edition=2021','--crate-name','issue_monitor_launch_provider',"$repoRoot/crates/gwt/playwright/fixtures/issue-monitor-launch-provider.rs",'-L',"dependency=$deps",'--extern',"serde_json=$($jsonLibrary.FullName)",'-o',"$checkHome/bin/provider.exe")
  $cargoConfig = [IO.File]::ReadAllText("$repoRoot/.cargo/config.toml")
  if ($cargoConfig -match '(?s)\[target\.x86_64-pc-windows-msvc\].*?linker\s*=\s*"([^"]+)"') { $rustArgs += @('-C',"linker=$($Matches[1])") }
  [void](Invoke-Owned $rustc $rustArgs)
  foreach ($command in @('codex','gh','git')) { Copy-Item -LiteralPath "$checkHome/bin/provider.exe" -Destination "$checkHome/bin/$command.exe" }
  if ($Sequential) {
    # Use the production prefs lock and atomic mutation rather than racing a
    # read/replace against the daemon. This helper accepts only this fixture's
    # already-confirmed delivery identities; it never creates upstream claims.
    $enqueueSource = @'
use std::{io::{self, Read}, path::PathBuf};
fn request_scan(project: &std::path::Path) -> io::Result<()> {
    let payload = gwt::runtime_daemon_events::issue_monitor_payload(
        "control", serde_json::json!({"scan_now":{}}), std::process::id(),
    );
    gwt::daemon_publisher::publish_durable_issue_monitor_control(project, payload)
        .map_err(|error| io::Error::other(error.to_string()))
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = PathBuf::from(std::env::var_os("HOME").ok_or("missing fixture HOME")?);
    let fixture: serde_json::Value = serde_json::from_slice(&std::fs::read(home.join("fixture.json"))?)?;
    let project = home.join("project");
    if fixture["sequential"] != true || PathBuf::from(fixture["project"].as_str().ok_or("missing fixture project")?) != project {
        return Err("enqueue requires the isolated sequential fixture".into());
    }
    if std::env::args().any(|argument| argument == "--ready") {
        request_scan(&project)?;
        return Ok(());
    }
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let delivery: gwt::issue_monitor::PendingIssueMonitorLaunchDelivery = serde_json::from_str(&input)?;
    let expected = fixture["expected_deliveries"].as_array().ok_or("missing fixture deliveries")?;
    let index = expected.iter().position(|row| row["issue_number"] == delivery.issue_number
        && row["delivery_id"] == delivery.delivery_id && row["claim_id"] == delivery.claim_id)
        .ok_or("delivery is not owned by this fixture")?;
    if delivery.claim_owner != "fixture" || delivery.branch_name != format!("work/issue-{}", delivery.issue_number) {
        return Err("foreign delivery refused".into());
    }
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&project);
    gwt::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        if !prefs.pending_launch_deliveries.is_empty() || !prefs.failed_issues.is_empty()
            || prefs.launch_confirmations.len() != index
            || expected[..index].iter().any(|row| {
                let number = row["issue_number"].as_u64().unwrap();
                prefs.launch_confirmations.get(&number).is_none_or(|ack|
                    ack.delivery_id.as_deref() != row["delivery_id"].as_str()
                    || ack.claim_id.as_deref() != row["claim_id"].as_str())
            }) {
            return Err(io::Error::other("previous fixture delivery is not durably acknowledged"));
        }
        // Match the durable state produced by apply_confirmed_claim. A live
        // daemon rebase imports active reservations from launching_issues.
        prefs.launching_issues.push(gwt::issue_monitor::IssueMonitorLaunchingIssue {
            issue_number: delivery.issue_number,
            claimed_at: Some(delivery.created_at.clone()),
        });
        prefs.claim_identities.push(gwt::issue_monitor::IssueMonitorClaimIdentity {
            issue_number: delivery.issue_number,
            claim_id: delivery.claim_id.clone(),
            owner: delivery.claim_owner.clone(),
        });
        prefs.pending_launch_deliveries.push(delivery.clone());
        Ok(())
    })?;
    // Match the ordinary CLI path: wake the owning daemon and require its ACK.
    // A publish error is returned; do not replay or silently wait for a tick.
    request_scan(&project)?;
    println!("{}", serde_json::json!({"issue_number": delivery.issue_number, "enqueued_at": delivery.created_at, "scan_acknowledged":true}));
    Ok(())
}
'@
    [IO.File]::WriteAllText("$checkHome/enqueue.rs", $enqueueSource)
    $enqueueArgs = @('--edition=2021','--crate-name','issue_monitor_fixture_enqueue',"$checkHome/enqueue.rs",'-L',"dependency=$deps",'--extern',"serde_json=$($jsonLibrary.FullName)",'--extern',"gwt=$($gwtLibrary.FullName)",'-o',"$checkHome/bin/enqueue.exe")
    # rustc does not replay Cargo build-script metadata from an rlib. Reuse
    # the native search paths recorded by this checkout's compiled dependencies.
    $nativePaths = Get-ChildItem -LiteralPath (Join-Path $repoRoot 'target/debug/build') -Filter output -File -Recurse | ForEach-Object {
      foreach ($line in [IO.File]::ReadAllLines($_.FullName)) {
        if ($line -match '^cargo::?rustc-link-search=native=(.+)$' -and (Test-Path -LiteralPath $Matches[1] -PathType Container)) { $Matches[1] }
      }
    } | Sort-Object -Unique
    foreach ($nativePath in $nativePaths) { $enqueueArgs += @('-L',"native=$nativePath") }
    if ($cargoConfig -match '(?s)\[target\.x86_64-pc-windows-msvc\].*?linker\s*=\s*"([^"]+)"') { $enqueueArgs += @('-C',"linker=$($Matches[1])") }
    [void](Invoke-Owned $rustc $enqueueArgs)
  }
  # Only this disposable repository is materialized. Git network protocols are disabled.
  foreach ($gitArgs in @(@('init','--quiet','-b','develop','--separate-git-dir',"$checkHome/project.git"),@('commit','--allow-empty','--quiet','-m','chore: initialize offline fixture'),@('init','--bare','--quiet',"$checkHome/origin.git"),@('remote','add','origin',"$checkHome/origin.git"),@('push','--quiet','origin','develop'),@('symbolic-ref','refs/remotes/origin/HEAD','refs/remotes/origin/develop'))) {
    [void](Invoke-Owned $realGit $gitArgs)
  }
  $environment.PATH = "$checkHome/bin;$($environment.PATH)"
  $environment.GWT_TEST_GH = "$checkHome/bin/gh.exe"
  $environment.GWT_TEST_GH_SANDBOX = '1'
  $fixtureCount = if ($Sequential) { $LaunchCount } else { 12 }
  $issues = @(1..$fixtureCount | ForEach-Object { @{number=(559600+$_);title="Offline launch fixture $_";state='OPEN';body="## Acceptance Criteria`n- [ ] AC-1: Record local provider handoff.";labels=@(@{name='gwt-queued'});comments=@();updatedAt='2026-10-16T00:00:00Z';url="https://github.com/fixture/launch-perf/issues/$(559600+$_)"} })
  [IO.File]::WriteAllText("$checkHome/github-issues.json", ($issues | ConvertTo-Json -Depth 5 -AsArray))
  # Read-only loopback owner API. Every unsupported/mutating request fails locally.
  $serverSource = @'
const fs=require('node:fs'),http=require('node:http'),path=require('node:path');
const home=process.argv[2];
const connection=nodes=>({nodes,totalCount:nodes.length,pageInfo:{hasNextPage:false,endCursor:null}});
const server=http.createServer(async(req,res)=>{try{
 let text='';for await(const chunk of req)text+=chunk;const input=text?JSON.parse(text):{};
 const issues=JSON.parse(fs.readFileSync(path.join(home,'github-issues.json'),'utf8'));
 fs.appendFileSync(path.join(home,'github-requests.jsonl'),JSON.stringify({method:req.method,url:req.url})+'\n');
 if(req.method!=='GET'&&!(req.method==='POST'&&req.url==='/graphql'&&!/\bmutation\b/.test(input.query||'')))throw Error('mutations forbidden');
 const graph=i=>i&&({...i,labels:connection(i.labels),comments:connection(i.comments),timelineItems:connection([])});
 let result;
 if(req.url==='/graphql')result={data:{repository:{issue:graph(issues.find(i=>i.number===Number(input.variables?.number))),issues:connection(issues.map(graph))},rateLimit:{remaining:5000,resetAt:'2099-01-01T00:00:00Z',cost:1}}};
 else if(/^\/repos\/fixture\/launch-perf\/issues\/\d+(?:\?.*)?$/.test(req.url)){const i=issues.find(i=>i.number===Number(req.url.match(/issues\/(\d+)/)[1]));result={...i,state:'open',html_url:i.url,updated_at:i.updatedAt};}
 else if(/\/issues\/\d+\/comments(?:\?|$)/.test(req.url))result=issues.find(i=>i.number===Number(req.url.match(/issues\/(\d+)/)[1]))?.comments||[];
 else if(/\/(pulls|commits)(?:\?|$)/.test(req.url))result=[];
 else if(/^\/repos\/fixture\/launch-perf\/issues(?:\?|$)/.test(req.url))result=issues;
 else throw Error('unsupported read endpoint');
 res.writeHead(200,{'content-type':'application/json'});res.end(JSON.stringify(result));
 }catch(e){res.writeHead(500);res.end(JSON.stringify({message:String(e)}));}});
server.listen(0,'127.0.0.1',()=>fs.writeFileSync(path.join(home,'github-url'),'http://127.0.0.1:'+server.address().port));
'@
  [IO.File]::WriteAllText("$checkHome/github.cjs", $serverSource)
  $github = [Diagnostics.Process]::Start((New-OwnedProcess $node @("$checkHome/github.cjs",$checkHome)))
  $streams.githubOut = $github.StandardOutput.ReadToEndAsync()
  $streams.githubErr = $github.StandardError.ReadToEndAsync()
  $deadline = [DateTime]::UtcNow.AddSeconds(30)
  while (-not (Test-Path -LiteralPath "$checkHome/github-url")) { if ($github.HasExited -or [DateTime]::UtcNow -gt $deadline) { throw 'Loopback fixture did not start.' }; Start-Sleep -Milliseconds 100 }
  $endpoint = [IO.File]::ReadAllText("$checkHome/github-url").Trim()
  $environment.GWT_OWNER_GITHUB_TEST_MODE = 'loopback-v1'
  $environment.GWT_OWNER_GITHUB_REST_BASE = $endpoint
  $environment.GWT_OWNER_GITHUB_GRAPHQL_URL = "$endpoint/graphql"
  $environment.GWT_OWNER_GITHUB_TOKEN = 'fixture-only'
  # Admission now reads the machine policy, rather than legacy project fields.
  # Seed only this disposable HOME before the first status/migration read.
  [void](New-Item -ItemType Directory -Path "$checkHome/.gwt/machine-state" -Force)
  [IO.File]::WriteAllText("$checkHome/.gwt/machine-state/agent-settings.json", '{"mode":"manual","manual_limit":12,"last_change":null}')
  $status = Invoke-IsolatedGwtd 'issue.monitor.status'
  $key = $status.project_store.hash
  if ($key -notmatch '^[a-zA-Z0-9_-]+$') { throw 'Canonical fixture ProjectKey unavailable.' }
  $state = "$checkHome/.gwt/projects/$key/project-state"
  [void](New-Item -ItemType Directory -Path $state -Force)
  [IO.File]::WriteAllText("$state/pm.json", '{"settings":{"auto_start":false}}')
  [IO.File]::WriteAllText("$checkHome/.gwt/session.json", (@{active_tab_id='launch-perf';tabs=@(@{id='launch-perf';title='Offline launch fixture';project_root=$project;kind='git'});recent_projects=@()} | ConvertTo-Json -Depth 6))
  [void](Invoke-IsolatedGwtd 'hook.register_codex_managed_project_trust' @{codex_config="$checkHome/.codex/config.toml";project_root=$project})
  $hookBin = ((Invoke-IsolatedGwtd 'hook.doctor').output | ConvertFrom-Json).managed_hook_bin
  if (-not $hookBin -or $hookBin -match '(?i)[/\\]target[/\\].*[/\\]gwtd(\.exe)?$') { throw 'Stable/public hook fallback unavailable.' }
  $environment.GWT_HOOK_BIN = $hookBin
  $hookParams = @{expected_hook_bin=$hookBin;runtime_state_path="$checkHome/.gwt/browser-check-missing-runtime-state.json"}
  [void](Invoke-IsolatedGwtd 'hook.doctor' ($hookParams + @{repair=$true}))
  [void](Invoke-IsolatedGwtd 'hook.register_codex_managed_hook_trust' @{codex_config="$checkHome/.codex/config.toml";codex_hook_discovery='both';project_root=$project})
  $health = Assert-Hooks (Invoke-IsolatedGwtd 'hook.health' $hookParams)
  [IO.File]::WriteAllText("$EvidenceDirectory/hook-audit.json", ($health | ConvertTo-Json -Depth 6))
  # Hook convergence can take a minute. Start the existing 120-second delivery
  # deadline only when the GUI can consume this already-admitted replay.
  $deliveries = @($issues | ForEach-Object { @{delivery_id=[guid]::NewGuid().ToString();issue_number=$_.number;branch_name="work/issue-$($_.number)";linked_issue_kind='issue';claim_id=[guid]::NewGuid().ToString();claim_owner='fixture';launch_session_strategy='fresh_required';created_at=[DateTime]::UtcNow.ToString('o')} })
  # Keep the replay's already-confirmed claims visible to ordinary background
  # reconciliation. Empty comments would describe lost upstream ownership.
  foreach ($delivery in $deliveries) {
    $claim = @{claim_id=$delivery.claim_id;owner=$delivery.claim_owner;issue_number=$delivery.issue_number;status='active';heartbeat_at='2099-01-01T00:00:00Z';expires_at='2099-01-01T01:00:00Z'} | ConvertTo-Json -Compress
    ($issues | Where-Object number -EQ $delivery.issue_number).comments = @(@{id="IC_fixture_$($delivery.issue_number)";databaseId=$delivery.issue_number;body="<!-- gwt-auto-improve-claim v1 -->`n$claim`n<!-- /gwt-auto-improve-claim -->";createdAt='2026-10-16T00:00:00Z';updatedAt='2026-10-16T00:00:00Z';author=@{login='fixture'}})
  }
  [IO.File]::WriteAllText("$checkHome/github-issues.json", ($issues | ConvertTo-Json -Depth 8 -AsArray))
  # Queue admission and upstream claim creation are outside this acceptance run.
  $initialDeliveries = if ($Sequential) { @() } else { $deliveries }
  $initialPrefs = @{enabled=$true;autonomous_mode=$false;max_active_agents=12;max_active_agents_mode='manual';priority_order=@();terminal_queue_auto_refill=$false;auto_apply_updates=$false;launch_auto=$false;launch_profile=@{agent_id='codex';runtime_target='Host';skip_permissions=$true};pending_launch_deliveries=@($initialDeliveries)}
  $initialPrefs.launching_issues = @($initialDeliveries | ForEach-Object { @{issue_number=$_.issue_number;claimed_at=$_.created_at} })
  $initialPrefs.claim_identities = @($initialDeliveries | ForEach-Object { @{issue_number=$_.issue_number;claim_id=$_.claim_id;owner=$_.claim_owner} })
  [IO.File]::WriteAllText("$state/issue-monitor.json", ($initialPrefs | ConvertTo-Json -Depth 8))
  $start = New-OwnedProcess $gwt @('--no-tray','--no-open')
  # The agent launch pipeline injects checkout gwtd; the GUI keeps only the
  # stable public hook fallback audited above.
  $start.Environment['GWT_BROWSER_URL_FILE'] = "$checkHome/url"
  $gui = [Diagnostics.Process]::Start($start)
  $streams.guiOut = $gui.StandardOutput.ReadToEndAsync()
  $streams.guiErr = $gui.StandardError.ReadToEndAsync()
  $sourceRevision = (& $realGit -C $repoRoot rev-parse HEAD).Trim()
  $sourcePaths = @('crates/gwt/src/main.rs','crates/gwt/src/app_runtime/mod.rs','crates/gwt/src/app_runtime/wizard.rs','crates/gwt/src/app_runtime/launch.rs','crates/gwt/src/launch_runtime.rs','crates/gwt/src/launch_runtime/base_freshness.rs','crates/gwt/src/cli/gwtd_resolver.rs','crates/gwt/src/issue_monitor_worker.rs','crates/gwt-config/src/home.rs','crates/gwt-core/src/paths.rs','crates/gwt-core/src/logging/writer.rs','crates/gwt/src/perf/dispatch.rs',
    'crates/gwt/web/app.js','crates/gwt/web/terminal-viewport-reflow.js',
    'scripts/verify-issue-5596.ps1','crates/gwt/playwright/tests/issue-monitor-launch-perf-live.spec.ts','crates/gwt/playwright/fixtures/issue-monitor-launch-provider.rs')
  if ($PostAckDiagnostics) {
    $sourcePaths += @('scripts/diagnostics/issue-6422-processes.ps1','crates/gwt/playwright/tests/issue-6422-post-ack-live.spec.ts',
      'crates/gwt-terminal/src/pty.rs','crates/gwt-terminal/src/pty/process_group.rs','crates/gwt-core/src/process_tree.rs','crates/gwt-core/src/process_tree/job_cpu.rs')
  }
  $sourceSnapshot = $sourcePaths | ForEach-Object {
    $fromCommit = $Phase -eq 'baseline' -and ($_.StartsWith('crates/gwt/src/') -or $_.StartsWith('crates/gwt-core/src/') -or $_.StartsWith('crates/gwt/web/'))
    $blobSha = if ($fromCommit) { (& $realGit -C $repoRoot rev-parse "${sourceRevision}:$_").Trim() } else { (& $realGit -C $repoRoot hash-object -- $_).Trim() }
    @{path=$_;blob_sha=$blobSha;source= $(if ($fromCommit) { $sourceRevision } else { 'working-tree' })}
  }
  $expectedDeliveries = @($deliveries | ForEach-Object { @{issue_number=$_.issue_number;delivery_id=$_.delivery_id;claim_id=$_.claim_id} })
  $fixture = @{phase=$Phase;sequential=[bool]$Sequential;launch_count=$fixtureCount;gui_pid=$gui.Id;gui_started_at=$gui.StartTime.ToUniversalTime().ToString('o');checkout_binary=$gwt;binary_sha256=(Get-FileHash -LiteralPath $gwt -Algorithm SHA256).Hash;binary_modified_at=(Get-Item -LiteralPath $gwt).LastWriteTimeUtc.ToString('o');source_snapshot=@($sourceSnapshot);checkout=$repoRoot;checkout_sha=(& $realGit -C $repoRoot rev-parse HEAD).Trim();isolated_home=$checkHome;project=$project;project_key=$key;issues=@($issues.number);expected_deliveries=$expectedDeliveries;evidence_directory=$EvidenceDirectory}
  $fixture.checkout_daemon = $gwtd
  $fixture.hook_generation_fallback = $hookBin
  $fixture.daemon_sha256 = (Get-FileHash -LiteralPath $gwtd -Algorithm SHA256).Hash
  [IO.File]::WriteAllText("$checkHome/fixture.json", ($fixture | ConvertTo-Json -Depth 5))
  [IO.File]::WriteAllText("$EvidenceDirectory/fixture.json", ($fixture | ConvertTo-Json -Depth 5))
  Write-Output "Owned checkout PID: $($gui.Id); retained evidence: $EvidenceDirectory; isolated HOME: $checkHome"
  $url = ''
  $deadline = [DateTime]::UtcNow.AddSeconds(120)
  do {
    if ($gui.HasExited) { throw 'Owned checkout exited during startup.' }
    if (Test-Path -LiteralPath "$checkHome/url") {
      $candidate = [IO.File]::ReadAllText("$checkHome/url").Trim()
      try { if ((Invoke-WebRequest -Uri $candidate -Method Head -TimeoutSec 2).StatusCode -eq 200) { $url=$candidate; break } } catch {}
    }
    Start-Sleep -Milliseconds 200
  } while ([DateTime]::UtcNow -lt $deadline)
  if (-not $url) { throw 'Fresh checkout readiness timed out.' }
  $anchor = [Net.WebSockets.ClientWebSocket]::new()
  $anchorUrl = [UriBuilder]::new($url)
  $anchorUrl.Scheme = 'ws'; $anchorUrl.Path = '/ws'
  $anchor.ConnectAsync($anchorUrl.Uri, [Threading.CancellationToken]::None).GetAwaiter().GetResult() | Out-Null
  [void](Assert-Hooks (Invoke-IsolatedGwtd 'hook.health' $hookParams))
  # The GUI must supervise this checkout's companion, independently of the
  # stable hook-generation fallback. Observe the authority PID before timing.
  $daemonDeadline = [DateTime]::UtcNow.AddSeconds(30)
  $daemonProof = $null
  $daemonMatches = $false
  do {
    $fencePath = "$state/issue-monitor.shutdown-revoke"
    if (Test-Path -LiteralPath $fencePath) {
      $fence = [IO.File]::ReadAllText($fencePath) | ConvertFrom-Json
      if ($fence -and $fence.PSObject.Properties['pid']) {
        $daemonProcess = Get-Process -Id $fence.pid -ErrorAction SilentlyContinue
        if ($daemonProcess) {
          $daemonMatches = [string]::Equals($daemonProcess.Path, $gwtd, [StringComparison]::OrdinalIgnoreCase)
          $daemonProof = @{pid=$daemonProcess.Id;actual_binary=$daemonProcess.Path;expected_binary=$gwtd;matches_checkout=$daemonMatches;observed_at=[DateTime]::UtcNow.ToString('o')}
          [IO.File]::WriteAllText("$EvidenceDirectory/daemon-authority.json", ($daemonProof | ConvertTo-Json))
          if ($daemonMatches) { break }
        }
      }
    }
    Start-Sleep -Milliseconds 100
  } while ([DateTime]::UtcNow -lt $daemonDeadline)
  if (-not $daemonProof) { throw 'Owned checkout daemon authority readiness timed out.' }
  if (-not $daemonMatches) { throw 'Owned GUI selected a daemon outside this checkout; see daemon-authority.json.' }
  if ($CargoLoad) {
    # A real, dependency-free cargo build runs CPU work in its build script.
    # Leave two logical cores available while exercising a genuinely busy host.
    $loadRoot = "$checkHome/cargo-load"
    [void](New-Item -ItemType Directory -Path $loadRoot -Force)
    [IO.File]::WriteAllText("$loadRoot/Cargo.toml", "[package]`nname='gwt-launch-load'`nversion='0.0.0'`nedition='2021'`n[workspace]`n[lib]`npath='lib.rs'`n")
    [IO.File]::WriteAllText("$loadRoot/lib.rs", '')
    $loadSource = @'
use std::{fs::{self, File}, io::Write, path::PathBuf, sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}}, thread, time::{Duration, Instant}};
fn main() -> std::io::Result<()> {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let workers = thread::available_parallelism().map_or(2, usize::from).saturating_sub(2).max(1);
    let stop = Arc::new(AtomicBool::new(false));
    let active = Arc::new(AtomicBool::new(false));
    let work = Arc::new(AtomicU64::new(0));
    let handles: Vec<_> = (0..workers).map(|worker| {
        let (stop, active, work) = (stop.clone(), active.clone(), work.clone());
        thread::spawn(move || {
            let mut value = worker as u64 + 1;
            while !stop.load(Ordering::Relaxed) {
                if !active.load(Ordering::Relaxed) { thread::sleep(Duration::from_millis(200)); continue; }
                for _ in 0..100_000 { value = std::hint::black_box(value.wrapping_mul(6364136223846793005).wrapping_add(1)); }
                work.fetch_add(100_000, Ordering::Relaxed);
            }
        })
    }).collect();
    let started = Instant::now();
    let mut progress = File::create(root.join("progress.jsonl"))?;
    while !root.join("stop").exists() && started.elapsed() < Duration::from_secs(1500) {
        let enabled = root.join("start").exists();
        active.store(enabled, Ordering::Relaxed);
        writeln!(progress, "{{\"pid\":{},\"workers\":{},\"elapsed_ms\":{},\"work_units\":{}}}", std::process::id(), if enabled { workers } else { 0 }, started.elapsed().as_millis(), work.load(Ordering::Relaxed))?;
        progress.flush()?;
        thread::sleep(Duration::from_millis(200));
    }
    stop.store(true, Ordering::Relaxed);
    for handle in handles { handle.join().expect("CPU workload worker"); }
    fs::write(root.join("finished"), b"complete\n")
}
'@
    [IO.File]::WriteAllText("$loadRoot/build.rs", $loadSource)
    $start = New-OwnedProcess $cargo @('build','--offline','--manifest-path',"$loadRoot/Cargo.toml",'--target-dir',"$loadRoot/target") $loadRoot
    $start.Environment['RUSTC'] = $rustc
    $start.Environment['RUSTFLAGS'] = '-C linker=rust-lld'
    $cargoLoadProcess = [Diagnostics.Process]::Start($start)
    $streams.cargoOut = $cargoLoadProcess.StandardOutput.ReadToEndAsync()
    $streams.cargoErr = $cargoLoadProcess.StandardError.ReadToEndAsync()
    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    do {
      if ($cargoLoadProcess.HasExited) { throw 'Cargo load exited before launch measurement.' }
      if (Test-Path -LiteralPath "$loadRoot/progress.jsonl") { break }
      Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    if (-not (Test-Path -LiteralPath "$loadRoot/progress.jsonl")) { throw 'Cargo load readiness timed out.' }
    $fixture.cargo_load = @{cargo_pid=$cargoLoadProcess.Id;progress_path="$loadRoot/progress.jsonl";workload='bounded parallel CPU work in a real cargo build script; active only after browser/page readiness';logical_cores=[Environment]::ProcessorCount}
    [IO.File]::WriteAllText("$checkHome/fixture.json", ($fixture | ConvertTo-Json -Depth 5))
    [IO.File]::WriteAllText("$EvidenceDirectory/fixture.json", ($fixture | ConvertTo-Json -Depth 5))
  }
  if ($PostAckDiagnostics) {
    $fixture.post_ack_diagnostics = $true
    [IO.File]::WriteAllText("$checkHome/fixture.json", ($fixture | ConvertTo-Json -Depth 5))
    [IO.File]::WriteAllText("$EvidenceDirectory/fixture.json", ($fixture | ConvertTo-Json -Depth 5))
    # Start outside the Agent's policy Job, before either measured delivery.
    $observerStart = New-OwnedProcess (Get-Process -Id $PID).Path @('-NoProfile','-NonInteractive','-File',"$repoRoot/scripts/diagnostics/issue-6422-processes.ps1",'-FixtureHome',$checkHome,'-OutputDirectory',$EvidenceDirectory)
    $observer = [Diagnostics.Process]::Start($observerStart)
    $streams.observerOut = $observer.StandardOutput.ReadToEndAsync()
    $streams.observerErr = $observer.StandardError.ReadToEndAsync()
    $observerDeadline = [DateTime]::UtcNow.AddSeconds(30)
    while (-not (Test-Path -LiteralPath "$EvidenceDirectory/observer-ready.json")) {
      if ($observer.HasExited -or [DateTime]::UtcNow -gt $observerDeadline) { throw 'Post-ACK observer did not become ready.' }
      Start-Sleep -Milliseconds 100
    }
  }
  $version = [IO.File]::ReadAllText("$repoRoot/scripts/playwright-version.txt").Trim()
  $depsRoot = if ($env:GWT_PLAYWRIGHT_DEPS_DIR) { $env:GWT_PLAYWRIGHT_DEPS_DIR } else { Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) "Temp/gwt-playwright-$version" }
  $cli = Join-Path $depsRoot 'node_modules/playwright/cli.js'
  $resolver = Join-Path $depsRoot 'resolve-playwright.cjs'
  if (-not (Test-Path -LiteralPath $cli) -or -not (Test-Path -LiteralPath $resolver)) { throw "Pinned Playwright $version missing; bootstrap existing visual tests first." }
  # An absent remaining argument must not become an empty test filter: an
  # empty filter matches the whole suite instead of only the named file.
  $extraPlaywrightArgs = @($PlaywrightArgs | Where-Object { -not [string]::IsNullOrEmpty($_) })
  $testFile = if ($PostAckDiagnostics) { 'issue-6422-post-ack-live.spec.ts' } else { 'issue-monitor-launch-perf-live.spec.ts' }
  $start = New-OwnedProcess $node (@($cli,'test','--config',"$repoRoot/crates/gwt/playwright/playwright.config.ts",'--workers=1','--retries=0','--timeout','600000','--output',"$EvidenceDirectory/playwright",$testFile) + $extraPlaywrightArgs) $repoRoot
  $start.Environment['GWT_PLAYWRIGHT_BASE_URL'] = $url
  $start.Environment['GWT5596_HOME'] = $checkHome
  if ($Sequential) { $start.Environment['GWT5596_SEQUENTIAL'] = '1' }
  $start.Environment['GWT5596_LAUNCH_COUNT'] = [string]$fixtureCount
  $start.Environment['GWT_PLAYWRIGHT_NODE_MODULES'] = Join-Path $depsRoot 'node_modules'
  $start.Environment['NODE_OPTIONS'] = "--require `"$($resolver.Replace('\','/'))`""
  if ($env:GWT_HEADED_E2E_REPORT) { $start.Environment['GWT_HEADED_E2E_REPORT'] = $env:GWT_HEADED_E2E_REPORT }
  $runner = [Diagnostics.Process]::Start($start)
  if ($PostAckDiagnostics) { [IO.File]::WriteAllText("$checkHome/observer-runner-pid", [string]$runner.Id) }
  $streams.runnerOut = $runner.StandardOutput.ReadToEndAsync()
  $streams.runnerErr = $runner.StandardError.ReadToEndAsync()
  $runner.WaitForExit()
  $exitCode = $runner.ExitCode
  Write-Output $streams.runnerOut.GetAwaiter().GetResult()
  Write-Output $streams.runnerErr.GetAwaiter().GetResult()
} finally {
  $cleanupErrors = [Collections.Generic.List[string]]::new()
  if ($observer) {
    try {
      [IO.File]::WriteAllText("$checkHome/observer-stop", 'stop')
      if (-not $observer.WaitForExit(10000)) { $observer.Kill($true); [void]$observer.WaitForExit(10000); $cleanupErrors.Add('Observer did not stop normally.') }
      [IO.File]::WriteAllText("$EvidenceDirectory/observer-Out.log", $streams.observerOut.GetAwaiter().GetResult())
      [IO.File]::WriteAllText("$EvidenceDirectory/observer-Err.log", $streams.observerErr.GetAwaiter().GetResult())
      if ($observer.ExitCode) { $cleanupErrors.Add('Observer failed; retain partial samples and inspect observer-Err.log.') }
    } catch { $cleanupErrors.Add("Owned observer cleanup failed: $($_.Exception.Message)") }
    finally {
      try { if (-not $observer.HasExited) { $observer.Kill($true); [void]$observer.WaitForExit(10000) } }
      catch { $cleanupErrors.Add('Owned observer fallback termination failed.') }
      $observer.Dispose()
    }
  }
  $logSources = @("$checkHome/.gwt/logs")
  if ($state) { $logSources += Join-Path (Split-Path $state -Parent) 'logs' }
  foreach ($logSource in $logSources) {
    if (Test-Path -LiteralPath $logSource) {
      try { Copy-Item -LiteralPath $logSource -Destination "$EvidenceDirectory/startup-$([IO.Path]::GetFileName((Split-Path $logSource -Parent)))" -Recurse -Force -ErrorAction Stop }
      catch { $cleanupErrors.Add('Owned startup log preservation failed.') }
    }
  }
  if ($cargoLoadProcess) {
    [IO.File]::WriteAllText("$checkHome/cargo-load/stop", 'stop')
    try {
      if (-not $cargoLoadProcess.WaitForExit(10000)) { $cargoLoadProcess.Kill($true); [void]$cargoLoadProcess.WaitForExit(10000) }
      if ($cargoLoadProcess.ExitCode -ne 0) { $cleanupErrors.Add('Owned cargo load did not finish successfully.') }
      Copy-Item -LiteralPath "$checkHome/cargo-load/progress.jsonl" -Destination "$EvidenceDirectory/cargo-load-progress.jsonl" -ErrorAction Stop
      [IO.File]::WriteAllText("$EvidenceDirectory/cargo-Out.log", $streams.cargoOut.GetAwaiter().GetResult())
      [IO.File]::WriteAllText("$EvidenceDirectory/cargo-Err.log", $streams.cargoErr.GetAwaiter().GetResult())
    } catch { $cleanupErrors.Add("Owned cargo load cleanup failed: $($_.Exception.Message)") }
    $cargoLoadProcess.Dispose()
  }
  if ($state -and (Test-Path -LiteralPath "$state/issue-monitor.json")) {
    try { Copy-Item -LiteralPath "$state/issue-monitor.json" -Destination "$EvidenceDirectory/issue-monitor-final.json" -ErrorAction Stop }
    catch { $cleanupErrors.Add('Final Monitor prefs snapshot failed.') }
  }
  if ($anchor) { $anchor.Abort(); $anchor.Dispose() }
  foreach ($entry in @(@{name='runner';process=$runner},@{name='gui';process=$gui},@{name='github';process=$github})) {
    if (-not $entry.process) { continue }
    try {
      if (-not $entry.process.HasExited) { $entry.process.Kill($true) }
      if (-not $entry.process.WaitForExit(10000)) { $cleanupErrors.Add("Owned $($entry.name) PID remains.") }
      foreach ($suffix in @('Out','Err')) {
        $streamKey = $entry.name+$suffix
        if ($streams.ContainsKey($streamKey)) { [IO.File]::WriteAllText("$EvidenceDirectory/$($entry.name)-$suffix.log", $streams[$streamKey].GetAwaiter().GetResult()) }
      }
    } catch { $cleanupErrors.Add("Owned $($entry.name) cleanup failed.") }
    $entry.process.Dispose()
  }
  # Reap only exact recorder executable identities if a GUI crash orphaned them.
  foreach ($receiptFile in Get-ChildItem -LiteralPath $checkHome -Filter 'provider-launch-*.json' -File) {
    try {
      $receipt = [IO.File]::ReadAllText($receiptFile.FullName) | ConvertFrom-Json
      $child = Get-Process -Id $receipt.pid -ErrorAction SilentlyContinue
      if ($child -and $child.Path -eq "$checkHome/bin/codex.exe") { $child.Kill($true); if (-not $child.WaitForExit(10000)) { $cleanupErrors.Add("Owned provider PID $($receipt.pid) remains.") } }
    } catch { $cleanupErrors.Add("Owned provider receipt cleanup failed: $($receiptFile.Name)") }
  }
  if (Test-Path -LiteralPath "$checkHome/.codex/auth.json") { Remove-Item -LiteralPath "$checkHome/.codex/auth.json" -Force }
  [IO.File]::WriteAllText("$EvidenceDirectory/shutdown.json", (@{test_exit_code=$exitCode;cleanup_errors=@($cleanupErrors);isolated_home=$checkHome;stopped_at=[DateTime]::UtcNow.ToString('o')} | ConvertTo-Json -Depth 4))
  Write-Output "Retained evidence: $EvidenceDirectory"
  if ($cleanupErrors.Count) { throw ($cleanupErrors -join '; ') }
}
# Baseline preserves the actual failing Playwright exit code and threshold result.
exit $exitCode
