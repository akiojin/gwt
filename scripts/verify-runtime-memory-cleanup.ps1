# Exercise the fixture's actual finally block: a GUI timeout must not orphan PTYs.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This contract requires Windows and PowerShell 7.' }

class TimeoutGui {
  [int]$Id = -1
  [bool]$HasExited = $false
  [void] Kill([bool]$Tree) {}
  [bool] WaitForExit([int]$Timeout) { return $false }
  [void] Dispose() {}
}

$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile(
  (Join-Path $PSScriptRoot 'verify-runtime-memory-budget.ps1'), [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw 'Fixture parse failed.' }
$blocks = @($ast.FindAll({ param($node)
  $node -is [Management.Automation.Language.TryStatementAst] -and $node.Finally -and
    $node.Finally.Extent.Text.Contains('shutdown.json')
}, $true))
if ($blocks.Count -ne 1) { throw 'Expected one owned-process cleanup block.' }
$text = $blocks[0].Finally.Extent.Text
$cleanup = [scriptblock]::Create($text.Substring(1, $text.Length - 2))
$checkHome = Join-Path ([IO.Path]::GetTempPath()) ('gwt-cleanup-contract-' + [guid]::NewGuid())
[void](New-Item -ItemType Directory -Path "$checkHome/workload" -Force)
$start = [Diagnostics.ProcessStartInfo]::new((Get-Command node -CommandType Application).Source)
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
$start.ArgumentList.Add('-e')
$start.ArgumentList.Add('setInterval(() => {}, 1000)')
$producer = [Diagnostics.Process]::Start($start)
try {
  [IO.File]::WriteAllText("$checkHome/workload/producer.json", (@{
    pid=$producer.Id
    started_at=([DateTimeOffset]$producer.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
  } | ConvertTo-Json))
  $gui = [TimeoutGui]::new()
  $anchor = $null
  $savedRunner = @{}
  $stdout = $null
  $stderr = $null
  $exitCode = 0
  $failure = $null
  try { & $cleanup } catch { $failure = $_.Exception.Message }
  if ($failure -notlike '*Owned GUI PID -1 remains*') { throw "Missing GUI timeout failure: $failure" }
  if (-not $producer.HasExited) { throw 'GUI timeout skipped owned producer cleanup.' }
  $shutdown = [IO.File]::ReadAllText("$checkHome/shutdown.json") | ConvertFrom-Json
  if ($shutdown.owned_gui_stopped -or -not $shutdown.cleanup_errors.Count) {
    throw 'Shutdown evidence must preserve the GUI cleanup failure.'
  }
  Write-Output 'PASS: GUI timeout still stops the real owned producer and writes failure evidence.'
} finally {
  if (-not $producer.HasExited) { $producer.Kill($true) }
  if (-not $producer.WaitForExit(10000)) { throw 'Contract-owned producer remains.' }
  $producer.Dispose()
}
