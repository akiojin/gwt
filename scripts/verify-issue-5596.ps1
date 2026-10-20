# Offline owned-process acceptance harness; additional arguments reach Playwright unchanged.
[CmdletBinding(PositionalBinding = $false)]
param(
  [ValidateSet('baseline','after')][string]$Phase = 'after',
  [string]$EvidenceDirectory,
  [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArgs
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This harness requires Windows and PowerShell 7.' }
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$gwt = Join-Path $repoRoot 'target/debug/gwt.exe'
$gwtd = Join-Path $repoRoot 'target/debug/gwtd.exe'
foreach ($binary in @($gwt,$gwtd)) { if (-not (Test-Path -LiteralPath $binary)) { throw 'Build this checkout: cargo build -p gwt --bin gwt --bin gwtd' } }
$checkHome = Join-Path ([IO.Path]::GetTempPath()) ('gwt-launch-5596-' + [guid]::NewGuid())
if (-not $EvidenceDirectory) { $EvidenceDirectory = Join-Path $repoRoot "target/verification/issue-5596/$Phase" }
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
$environment.RUST_LOG = 'info,gwt.frontend.timing=debug'
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
$anchor = $null
$exitCode = 1
$streams = @{}
try {
  # Compile only the local command fixture, never an agent/provider package.
  $deps = Join-Path $repoRoot 'target/debug/deps'
  $jsonLibrary = Get-ChildItem -LiteralPath $deps -Filter 'libserde_json-*.rlib' | Sort-Object LastWriteTime -Descending | Select-Object -First 1
  if (-not $jsonLibrary) { throw 'Build checkout gwt/gwtd first (serde_json rlib missing).' }
  $rustArgs = @('--edition=2021','--crate-name','issue_monitor_launch_provider',"$repoRoot/crates/gwt/playwright/fixtures/issue-monitor-launch-provider.rs",'-L',"dependency=$deps",'--extern',"serde_json=$($jsonLibrary.FullName)",'-o',"$checkHome/bin/provider.exe")
  $cargoConfig = [IO.File]::ReadAllText("$repoRoot/.cargo/config.toml")
  if ($cargoConfig -match '(?s)\[target\.x86_64-pc-windows-msvc\].*?linker\s*=\s*"([^"]+)"') { $rustArgs += @('-C',"linker=$($Matches[1])") }
  [void](Invoke-Owned $rustc $rustArgs)
  foreach ($command in @('codex','gh','git')) { Copy-Item -LiteralPath "$checkHome/bin/provider.exe" -Destination "$checkHome/bin/$command.exe" }
  # Only this disposable repository is materialized. Git network protocols are disabled.
  foreach ($gitArgs in @(@('init','--quiet','-b','develop','--separate-git-dir',"$checkHome/project.git"),@('commit','--allow-empty','--quiet','-m','chore: initialize offline fixture'),@('init','--bare','--quiet',"$checkHome/origin.git"),@('remote','add','origin',"$checkHome/origin.git"),@('push','--quiet','origin','develop'),@('symbolic-ref','refs/remotes/origin/HEAD','refs/remotes/origin/develop'))) {
    [void](Invoke-Owned $realGit $gitArgs)
  }
  $environment.PATH = "$checkHome/bin;$($environment.PATH)"
  $environment.GWT_TEST_GH = "$checkHome/bin/gh.exe"
  $environment.GWT_TEST_GH_SANDBOX = '1'
  $issues = @(1..12 | ForEach-Object { @{number=(559600+$_);title="Offline launch fixture $_";state='OPEN';body="## Acceptance Criteria`n- [ ] AC-1: Record local provider handoff.";labels=@(@{name='gwt-queued'});comments=@();updatedAt='2026-10-16T00:00:00Z';url="https://github.com/fixture/launch-perf/issues/$(559600+$_)"} })
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
  [IO.File]::WriteAllText("$state/issue-monitor.json", (@{enabled=$true;autonomous_mode=$false;max_active_agents=12;max_active_agents_mode='manual';priority_order=@();terminal_queue_auto_refill=$false;auto_apply_updates=$false;launch_auto=$false;launch_profile=@{agent_id='codex';runtime_target='Host';skip_permissions=$true};pending_launch_deliveries=$deliveries} | ConvertTo-Json -Depth 8))
  $start = New-OwnedProcess $gwt @('--no-tray','--no-open')
  # The agent launch pipeline injects checkout gwtd; the GUI keeps only the
  # stable public hook fallback audited above.
  $start.Environment['GWT_BROWSER_URL_FILE'] = "$checkHome/url"
  $gui = [Diagnostics.Process]::Start($start)
  $streams.guiOut = $gui.StandardOutput.ReadToEndAsync()
  $streams.guiErr = $gui.StandardError.ReadToEndAsync()
  $sourceRevision = (& $realGit -C $repoRoot rev-parse HEAD).Trim()
  $sourceSnapshot = @('crates/gwt/src/main.rs','crates/gwt/src/app_runtime/mod.rs','crates/gwt/src/app_runtime/wizard.rs','crates/gwt/src/app_runtime/launch.rs','crates/gwt/src/launch_runtime.rs','crates/gwt/src/issue_monitor_worker.rs','crates/gwt-config/src/home.rs','crates/gwt-core/src/logging/writer.rs','crates/gwt/src/perf/dispatch.rs',
    'scripts/verify-issue-5596.ps1','crates/gwt/playwright/tests/issue-monitor-launch-perf-live.spec.ts','crates/gwt/playwright/fixtures/issue-monitor-launch-provider.rs') | ForEach-Object {
    $fromCommit = $Phase -eq 'baseline' -and $_.StartsWith('crates/gwt/src/')
    $blobSha = if ($fromCommit) { (& $realGit -C $repoRoot rev-parse "${sourceRevision}:$_").Trim() } else { (& $realGit -C $repoRoot hash-object -- $_).Trim() }
    @{path=$_;blob_sha=$blobSha;source= $(if ($fromCommit) { $sourceRevision } else { 'working-tree' })}
  }
  $expectedDeliveries = @($deliveries | ForEach-Object { @{issue_number=$_.issue_number;delivery_id=$_.delivery_id;claim_id=$_.claim_id} })
  $fixture = @{phase=$Phase;gui_pid=$gui.Id;gui_started_at=$gui.StartTime.ToUniversalTime().ToString('o');checkout_binary=$gwt;binary_sha256=(Get-FileHash -LiteralPath $gwt -Algorithm SHA256).Hash;binary_modified_at=(Get-Item -LiteralPath $gwt).LastWriteTimeUtc.ToString('o');source_snapshot=@($sourceSnapshot);checkout=$repoRoot;checkout_sha=(& $realGit -C $repoRoot rev-parse HEAD).Trim();isolated_home=$checkHome;project=$project;project_key=$key;issues=@($issues.number);expected_deliveries=$expectedDeliveries;evidence_directory=$EvidenceDirectory}
  $fixture.checkout_daemon = $gwtd
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
  $version = [IO.File]::ReadAllText("$repoRoot/scripts/playwright-version.txt").Trim()
  $depsRoot = if ($env:GWT_PLAYWRIGHT_DEPS_DIR) { $env:GWT_PLAYWRIGHT_DEPS_DIR } else { Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) "Temp/gwt-playwright-$version" }
  $cli = Join-Path $depsRoot 'node_modules/playwright/cli.js'
  $resolver = Join-Path $depsRoot 'resolve-playwright.cjs'
  if (-not (Test-Path -LiteralPath $cli) -or -not (Test-Path -LiteralPath $resolver)) { throw "Pinned Playwright $version missing; bootstrap existing visual tests first." }
  # An absent remaining argument must not become an empty test filter: an
  # empty filter matches the whole suite instead of only the named file.
  $extraPlaywrightArgs = @($PlaywrightArgs | Where-Object { -not [string]::IsNullOrEmpty($_) })
  $start = New-OwnedProcess $node (@($cli,'test','--config',"$repoRoot/crates/gwt/playwright/playwright.config.ts",'--workers=1','--retries=0','--timeout','600000','--output',"$EvidenceDirectory/playwright",'issue-monitor-launch-perf-live.spec.ts') + $extraPlaywrightArgs) $repoRoot
  $start.Environment['GWT_PLAYWRIGHT_BASE_URL'] = $url
  $start.Environment['GWT5596_HOME'] = $checkHome
  $start.Environment['GWT_PLAYWRIGHT_NODE_MODULES'] = Join-Path $depsRoot 'node_modules'
  $start.Environment['NODE_OPTIONS'] = "--require `"$($resolver.Replace('\','/'))`""
  if ($env:GWT_HEADED_E2E_REPORT) { $start.Environment['GWT_HEADED_E2E_REPORT'] = $env:GWT_HEADED_E2E_REPORT }
  $runner = [Diagnostics.Process]::Start($start)
  $streams.runnerOut = $runner.StandardOutput.ReadToEndAsync()
  $streams.runnerErr = $runner.StandardError.ReadToEndAsync()
  $runner.WaitForExit()
  $exitCode = $runner.ExitCode
  Write-Output $streams.runnerOut.GetAwaiter().GetResult()
  Write-Output $streams.runnerErr.GetAwaiter().GetResult()
} finally {
  $cleanupErrors = [Collections.Generic.List[string]]::new()
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
