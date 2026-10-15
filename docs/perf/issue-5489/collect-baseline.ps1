# Read-only counters from the existing GUI; does not launch or stop a GUI/agent.
param(
    [int]$GuiProcessId = 32708,
    [int]$DurationSeconds = 30,
    [string]$OutputName = 'baseline.json'
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class MonitorThreadCounters {
    [DllImport("kernel32.dll")] public static extern IntPtr OpenThread(uint access, bool inherit, uint id);
    [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll")] static extern int GetThreadDescription(IntPtr handle, out IntPtr description);
    [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr memory);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetThreadTimes(IntPtr handle, out long created, out long exited, out long kernel, out long user);
    public static string Name(IntPtr handle) {
        IntPtr text;
        if (GetThreadDescription(handle, out text) < 0) return null;
        try { return Marshal.PtrToStringUni(text); } finally { LocalFree(text); }
    }
    public static double CpuSeconds(IntPtr handle) {
        long created, exited, kernel, user;
        if (!GetThreadTimes(handle, out created, out exited, out kernel, out user))
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        if (exited != 0) throw new InvalidOperationException("Monitor thread exited during observation.");
        return (kernel + user) / 10000000.0;
    }
}
'@
$outputPath = Join-Path $PSScriptRoot $OutputName
if (Test-Path -LiteralPath $outputPath) { throw 'Refusing to overwrite an observation.' }
$gui = Get-Process -Id $GuiProcessId
$identity = $gui.StartTime.ToUniversalTime().ToString('o')
$binary = $gui.Path
$guiVersion = (Get-Item -LiteralPath $binary).VersionInfo.ProductVersion
$binaryHash = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant()
$releaseTag = "v$guiVersion"
$releaseCommit = (& git rev-parse --verify "$releaseTag^{commit}").Trim()
if ($LASTEXITCODE -ne 0) { throw 'GUI release tag is unavailable; fetch tags before observation.' }
$fixCommit = '2b6ca0eca084db53ed381a09a85c7d8eb4e2acf0'
& git merge-base --is-ancestor $fixCommit $releaseCommit
if ($LASTEXITCODE -notin 0, 1) { throw 'Cannot establish release ancestry.' }
$includesCensusFix = $LASTEXITCODE -eq 0
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
$monitorHandle = [IntPtr]::Zero
$monitorId = 0
foreach ($thread in $gui.Threads) {
    $handle = [MonitorThreadCounters]::OpenThread(0x0800, $false, $thread.Id)
    if ($handle -eq [IntPtr]::Zero) { continue }
    if ([MonitorThreadCounters]::Name($handle) -eq 'gwt-monitor-runtime') {
        $monitorHandle = $handle
        $monitorId = $thread.Id
        break
    }
    [void][MonitorThreadCounters]::CloseHandle($handle)
}
if ($monitorHandle -eq [IntPtr]::Zero) { throw 'Named Monitor thread not found.' }
try {
$threadsBefore = @{}
foreach ($thread in $gui.Threads) {
    try { $threadsBefore[$thread.Id] = $thread.TotalProcessorTime.TotalSeconds } catch {}
}
$cpuBefore = $gui.TotalProcessorTime.TotalSeconds
$monitorBefore = [MonitorThreadCounters]::CpuSeconds($monitorHandle)
$observedAt = [DateTime]::UtcNow.ToString('o')
$clock = [Diagnostics.Stopwatch]::StartNew()
Start-Sleep -Seconds $DurationSeconds
$gui = Get-Process -Id $GuiProcessId
if ($gui.StartTime.ToUniversalTime().ToString('o') -ne $identity) { throw 'GUI identity changed.' }
$cpuAfter = $gui.TotalProcessorTime.TotalSeconds
$elapsed = $clock.Elapsed.TotalSeconds
$monitorAfter = [MonitorThreadCounters]::CpuSeconds($monitorHandle)
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
    release_tag = $releaseTag
    release_commit = $releaseCommit
    census_fix_commit = $fixCommit
    includes_census_fix = $includesCensusFix
    roots = $roots
    runtime = $runtime
    elapsed_seconds = $elapsed
    gui_cpu_before_seconds = $cpuBefore
    gui_cpu_after_seconds = $cpuAfter
    gui_cpu_seconds = $cpuAfter - $cpuBefore
    gui_cores = ($cpuAfter - $cpuBefore) / $elapsed
    monitor = [ordered]@{
        id = $monitorId
        name = [MonitorThreadCounters]::Name($monitorHandle)
        cpu_before_seconds = $monitorBefore
        cpu_after_seconds = $monitorAfter
        cpu_seconds = $monitorAfter - $monitorBefore
        cores = ($monitorAfter - $monitorBefore) / $elapsed
        method = 'GetThreadDescription and GetThreadTimes on one retained thread handle; exited thread refuses observation.'
    }
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
} finally {
    [void][MonitorThreadCounters]::CloseHandle($monitorHandle)
}
