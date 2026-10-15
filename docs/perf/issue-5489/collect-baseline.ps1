# Read-only counters from the existing GUI; does not launch or stop a GUI/agent.
param(
    [int]$GuiProcessId = 32708,
    [int]$DurationSeconds = 30,
    [string]$OutputName = 'baseline.json'
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$outputPath = Join-Path $PSScriptRoot $OutputName
if (Test-Path -LiteralPath $outputPath) { throw 'Refusing to overwrite an observation.' }
$gui = Get-Process -Id $GuiProcessId
$identity = $gui.StartTime.ToUniversalTime().ToString('o')
$binary = $gui.Path
$guiVersion = (Get-Item -LiteralPath $binary).VersionInfo.ProductVersion
$binaryHash = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant()
$roots = @((Get-Content -LiteralPath "$env:USERPROFILE/.gwt/session.json" -Raw | ConvertFrom-Json).tabs.project_root)
$runtimeDir = Join-Path $env:USERPROFILE '.gwt/sessions/runtime'
$runtimeFiles = @(Get-ChildItem -LiteralPath $runtimeDir -Recurse -File)
$runtime = [ordered]@{
    namespaces = @(Get-ChildItem -LiteralPath $runtimeDir -Directory).Count
    files = $runtimeFiles.Count
    bytes = ($runtimeFiles | Measure-Object Length -Sum).Sum
}
$paneResponse = '{"schema_version":1,"operation":"pane.list","params":{}}' | gwtd | ConvertFrom-Json
if (-not $paneResponse.ok) { throw 'pane.list failed.' }
$panesBefore = @($paneResponse.output -split "`n" | Where-Object { $_.Trim() })
$gui = Get-Process -Id $GuiProcessId
$threadsBefore = @{}
foreach ($thread in $gui.Threads) {
    try { $threadsBefore[$thread.Id] = $thread.TotalProcessorTime.TotalSeconds } catch {}
}
$cpuBefore = $gui.TotalProcessorTime.TotalSeconds
$observedAt = [DateTime]::UtcNow.ToString('o')
$clock = [Diagnostics.Stopwatch]::StartNew()
Start-Sleep -Seconds $DurationSeconds
$gui = Get-Process -Id $GuiProcessId
if ($gui.StartTime.ToUniversalTime().ToString('o') -ne $identity) { throw 'GUI identity changed.' }
$cpuAfter = $gui.TotalProcessorTime.TotalSeconds
$elapsed = $clock.Elapsed.TotalSeconds
$threadsAfter = @()
foreach ($thread in $gui.Threads) {
    try {
        if ($threadsBefore.ContainsKey($thread.Id)) {
            $cpu = $thread.TotalProcessorTime.TotalSeconds - $threadsBefore[$thread.Id]
            $threadsAfter += [ordered]@{ id = $thread.Id; cpu_seconds = $cpu }
        }
    } catch {}
}
$paneResponse = '{"schema_version":1,"operation":"pane.list","params":{}}' | gwtd | ConvertFrom-Json
if (-not $paneResponse.ok) { throw 'pane.list failed.' }
$panesAfter = @($paneResponse.output -split "`n" | Where-Object { $_.Trim() })
$row = [ordered]@{
    schema_version = 1
    issue = 5489
    observed_at = $observedAt
    gui_pid = $GuiProcessId
    gui_started_at = $identity
    gui_binary = $binary
    gui_version = $guiVersion
    gui_sha256 = $binaryHash
    roots = $roots
    runtime = $runtime
    elapsed_seconds = $elapsed
    gui_cpu_before_seconds = $cpuBefore
    gui_cpu_after_seconds = $cpuAfter
    gui_cpu_seconds = $cpuAfter - $cpuBefore
    gui_cores = ($cpuAfter - $cpuBefore) / $elapsed
    rss_bytes = $gui.WorkingSet64
    threads = @($threadsAfter | Sort-Object { $_['cpu_seconds'] } -Descending)
    panes_before = $panesBefore
    panes_after = $panesAfter
    implementation_panes_before = @($panesBefore | Where-Object { $_ -match '\trunning\t' -and $_ -match '\trole=implementation_agent(?:\t|$)' }).Count
    implementation_panes_after = @($panesAfter | Where-Object { $_ -match '\trunning\t' -and $_ -match '\trole=implementation_agent(?:\t|$)' }).Count
    method = 'GUI TotalProcessorTime and ProcessThread.TotalProcessorTime differences; Stopwatch seconds; core units are not divided by logical processors. Thread counters are read sequentially around the process interval; short-interval edge skew is retained.'
}
[IO.File]::WriteAllText($outputPath, ($row | ConvertTo-Json -Depth 6) + "`n", [Text.UTF8Encoding]::new($false))
Write-Output $outputPath
