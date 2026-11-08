# Read-only observation of an existing GUI and its real implementation panes.
# Example: pwsh -File docs/perf/issue-5385/collect.ps1 -GuiProcessId 32708
param([Parameter(Mandatory)][int]$GuiProcessId)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$gwtExe = if ($env:GWT_BIN_PATH -and (Test-Path -LiteralPath $env:GWT_BIN_PATH)) {
    $env:GWT_BIN_PATH
} else { (Get-Command gwtd -CommandType Application).Source }
$samplesPath = Join-Path $PSScriptRoot 'samples.jsonl'
if (Test-Path -LiteralPath $samplesPath) { throw 'Refusing to overwrite existing observation.' }
$gui = Get-Process -Id $GuiProcessId
$startedAt = $gui.StartTime.ToUniversalTime().ToString('o')
$binary = $gui.Path
$context = [ordered]@{
    schema_version = 1
    issue = 5385
    gui_pid = $GuiProcessId
    gui_started_at = $startedAt
    gui_version = (Get-Item -LiteralPath $binary).VersionInfo.ProductVersion
    gui_sha256 = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant()
    release_tag = 'v9.121.1'
    release_commit = (& git rev-parse 'v9.121.1^{commit}').Trim()
    baseline_commit = (& git rev-parse origin/develop).Trim()
    os = [Environment]::OSVersion.VersionString
    logical_processors = [Environment]::ProcessorCount
    duration_seconds = 3600
    interval_seconds = 60
    rss_budget_bytes = 2147483648
    cpu_budget_cores = 1
    rss_definition = 'GUI process WorkingSet64 only; excludes agent and WebView child processes'
    cpu_definition = 'Delta GUI TotalProcessorTime.TotalSeconds / delta monotonic elapsed seconds; not divided by logical CPU count'
}
[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'context.json'), ($context | ConvertTo-Json -Depth 5) + "`n")
$clock = [Diagnostics.Stopwatch]::StartNew()
for ($minute = 0; $minute -le 60; $minute++) {
    $remaining = $minute * 60 - $clock.Elapsed.TotalSeconds
    if ($remaining -gt 0) { Start-Sleep -Milliseconds ([int][Math]::Ceiling($remaining * 1000)) }
    $gui = Get-Process -Id $GuiProcessId
    if ($gui.StartTime.ToUniversalTime().ToString('o') -ne $startedAt) { throw 'GUI identity changed.' }
    $row = [ordered]@{
        minute = $minute
        observed_at = [DateTime]::UtcNow.ToString('o')
        elapsed_seconds = $clock.Elapsed.TotalSeconds
        gui_pid = $GuiProcessId
        gui_started_at = $startedAt
        working_set_bytes = $gui.WorkingSet64
        private_bytes = $gui.PrivateMemorySize64
        cpu_seconds = $gui.TotalProcessorTime.TotalSeconds
    }
    $envelope = '{"schema_version":1,"operation":"pane.list","params":{}}' | & $gwtExe | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0 -or -not $envelope.ok) { throw 'pane.list failed; observation is incomplete.' }
    $lines = @($envelope.output -split "`n" | Where-Object { $_.Trim() })
    $row['panes'] = $lines
    $row['implementation_panes'] = @($lines | Where-Object { $_ -match '\trunning\t' -and $_ -match '\trole=implementation_agent(?:\t|$)' }).Count
    [IO.File]::AppendAllText($samplesPath, ($row | ConvertTo-Json -Depth 5 -Compress) + "`n", [Text.UTF8Encoding]::new($false))
    Write-Output ("minute={0} rss_gib={1:F3} cpu_seconds={2:F3} implementation_panes={3}" -f $minute, ($row.working_set_bytes / 1GB), $row.cpu_seconds, $row.implementation_panes)
}
Write-Output 'Observation complete: 61 samples over 60 real minutes. Budget verdicts must be calculated from samples.jsonl.'
