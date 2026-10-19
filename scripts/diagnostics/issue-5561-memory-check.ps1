# Reuse the owned checkout/fresh-HOME fixture. Deterministic PTYs are not a
# substitute for Issue #5561's same-condition real model-agent observation.
[CmdletBinding(DefaultParameterSetName = 'Measure')]
param(
  [Parameter(Mandatory, ParameterSetName = 'Check')][string]$EvidenceDirectory,
  [Parameter(ParameterSetName = 'Measure')]
  [ValidateSet('baseline', 'after')][string]$Label = 'after',
  [Parameter(ParameterSetName = 'Measure')]
  [ValidateRange(1, 14400)][int]$Seconds = 14400,
  [Parameter(ValueFromRemainingArguments = $true, ParameterSetName = 'Measure')][string[]]$PlaywrightArgs
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PlaywrightArgs = @($PlaywrightArgs | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
$checkout = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$sources = @('crates/gwt/src/work_events_ingest.rs', 'crates/gwt-core/src/work_events_intake.rs', 'crates/gwt-git/src/blob.rs')
function Get-ProductSourceHash([string]$Relative) {
  $text = [IO.File]::ReadAllText((Join-Path $checkout $Relative)).Replace("`r`n", "`n")
  # Exclude only the exact temporary measurement hook removed before delivery.
  $text = $text.Replace('#[cfg(not(test))] include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/issue-5561-source-probe.rs"));' + "`n", '')
  return [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($text))).ToLowerInvariant()
}
if ($PSCmdlet.ParameterSetName -eq 'Check') {
  $shutdown = Get-Content -LiteralPath (Join-Path $EvidenceDirectory 'after/shutdown.json') -Raw | ConvertFrom-Json
  if (!$shutdown.owned_gui_stopped -or $shutdown.test_exit_code -ne 0 -or $shutdown.cleanup_errors.Count) {
    throw 'The measured workload or its owned-process cleanup failed.'
  }
  $metadata = Get-Content -LiteralPath (Join-Path $EvidenceDirectory 'after/metadata.json') -Raw | ConvertFrom-Json
  if (@($metadata.source_sha256.PSObject.Properties).Count -ne $sources.Count -or
      @($sources | Where-Object { $_ -notin $metadata.source_sha256.PSObject.Properties.Name }).Count) {
    throw 'Missing measured product source hashes.'
  }
  foreach ($source in $metadata.source_sha256.PSObject.Properties) {
    if ((Get-ProductSourceHash $source.Name) -ne $source.Value) { throw "Measured product source changed: $($source.Name)" }
  }
  foreach ($theme in @('dark', 'light')) {
    $result = Get-Content -LiteralPath (Join-Path $EvidenceDirectory "after/runtime-memory-$theme.json") -Raw | ConvertFrom-Json
    $minimumSeconds = if ($theme -eq 'dark') { 14400 } else { 60 }
    if (!$result.passed -or $result.browser_errors.Count -or $result.pane_ids.Count -ne 12 -or
        $result.initial_producers.Count -ne 12 -or $result.final_producers.Count -ne 12 -or
        $result.requested_duration_seconds -lt $minimumSeconds -or
        $result.rss_samples[-1].elapsed_ms -lt $minimumSeconds * 1000 -or
        @($result.rss_samples | Where-Object rss_bytes -gt 1500000000).Count) {
      throw "Missing passing twelve-PTY $theme observation within 1.5GB."
    }
    Write-Output "PASS: $theme twelve-PTY observation, $($result.rss_samples[-1].elapsed_ms)ms, all RSS samples <=1500000000 bytes, browser errors 0."
  }
  $old = Get-Content -LiteralPath (Join-Path $EvidenceDirectory 'allocation/old.json') -Raw | ConvertFrom-Json
  $new = Get-Content -LiteralPath (Join-Path $EvidenceDirectory 'allocation/new.json') -Raw | ConvertFrom-Json
  $oldFinal = $old.phases | Where-Object phase -eq 'after_final'
  $newFinal = $new.phases | Where-Object phase -eq 'after_final'
  if ($old.scanned_source_count -le 0 -or $old.scanned_source_count -ne $new.scanned_source_count -or
      $newFinal.live_bytes -ge $oldFinal.live_bytes) { throw 'Same-input source-map retention did not decrease.' }
  Write-Output "PASS: $($new.scanned_source_count) same-input sources, retained live allocation $($oldFinal.live_bytes) -> $($newFinal.live_bytes) bytes."
  Write-Output 'Scope: deterministic PTYs and a metadata-only allocation slice; real model-agent same-condition AC-2 remains separate.'
  exit 0
}
$runDirectory = Join-Path $checkout ('target/issue-5561/' + $Label + '-' + [guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $runDirectory -Force)
$hashes = @{}
foreach ($relative in $sources) {
  $hashes[$relative] = Get-ProductSourceHash $relative
}
[IO.File]::WriteAllText((Join-Path $runDirectory 'source.json'), (@{
  label=$Label; started_at=[DateTime]::UtcNow.ToString('o'); checkout=$checkout;
  head=(& git -C $checkout rev-parse HEAD).Trim(); source_sha256=$hashes;
  source_hash_rule='UTF-8/LF, excluding the exact temporary source-probe include line';
  binary_sha256=(Get-FileHash -LiteralPath (Join-Path $checkout 'target/debug/gwt.exe') -Algorithm SHA256).Hash.ToLowerInvariant();
  requested_seconds=$Seconds; pane_count=12; budget_bytes=1500000000;
  limitations='Deterministic command Agent PTYs, not real model inference/hooks/builds.'
} | ConvertTo-Json -Depth 5))
$settings = @{
  GWT_PLAYWRIGHT_MEMORY_SECONDS=[string]$Seconds;
  GWT_PLAYWRIGHT_MEMORY_PANES='12';
  GWT_PLAYWRIGHT_MEMORY_BUDGET_BYTES='1500000000';
  GWT_PLAYWRIGHT_MEMORY_ISSUE='5561';
  GWT_PLAYWRIGHT_MEMORY_RECEIPT=(Join-Path $runDirectory 'fixture-home.txt')
}
$previous = @{}
foreach ($key in $settings.Keys) {
  $previous[$key] = [Environment]::GetEnvironmentVariable($key)
  [Environment]::SetEnvironmentVariable($key, $settings[$key])
}
try {
  & (Join-Path $checkout 'scripts/verify-runtime-memory-budget.ps1') @PlaywrightArgs
  $result = $LASTEXITCODE
} finally {
  foreach ($key in $previous.Keys) { [Environment]::SetEnvironmentVariable($key, $previous[$key]) }
}
Write-Output "Issue #5561 run metadata: $runDirectory"
exit $result
