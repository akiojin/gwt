# Read-only OS observer, deliberately outside the measured Agent Job.
param(
    [Parameter(Mandatory = $true)][string]$FixtureHome,
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [ValidateRange(100, 10000)][int]$IntervalMilliseconds = 500
)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
public static class Issue6422Os {
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
    public struct Entry {
        public uint size, usage, pid; public UIntPtr heap;
        public uint module, threads, parent; public int priority; public uint flags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=260)] public string name;
    }
    [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr CreateToolhelp32Snapshot(uint flags, uint pid);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] static extern bool Process32FirstW(IntPtr snapshot, ref Entry entry);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] static extern bool Process32NextW(IntPtr snapshot, ref Entry entry);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr OpenJobObjectW(uint access, bool inherit, string name);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool IsProcessInJob(IntPtr process, IntPtr job, out bool member);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool QueryInformationJobObject(IntPtr job, int infoClass, byte[] info, uint size, IntPtr returned);
    public static Entry[] Tree() {
        var rows = new List<Entry>(); var snapshot = CreateToolhelp32Snapshot(2, 0);
        if (snapshot == new IntPtr(-1)) throw new Win32Exception();
        try {
            var entry = new Entry { size=(uint)Marshal.SizeOf(typeof(Entry)) };
            if (!Process32FirstW(snapshot, ref entry)) throw new Win32Exception();
            do { rows.Add(entry); } while (Process32NextW(snapshot, ref entry));
        } finally { CloseHandle(snapshot); }
        return rows.ToArray();
    }
    public static object Job(int pid, string name) {
        if (String.IsNullOrEmpty(name)) name = null;
        var job = name == null ? IntPtr.Zero : OpenJobObjectW(4, false, name);
        if (name != null && job == IntPtr.Zero) return new { name, error=Marshal.GetLastWin32Error() };
        try {
            using (var process = Process.GetProcessById(pid)) {
                bool member;
                if (!IsProcessInJob(process.Handle, job, out member)) return new { name, error=Marshal.GetLastWin32Error() };
                if (!member) return new { name, member=false };
                // NULL queries this observer's current Job, never the remote process's Job.
                if (name == null && pid != Process.GetCurrentProcess().Id) return new { name, member=true };
                var cpu = new byte[8]; var limits = new byte[144];
                var cpuOk = QueryInformationJobObject(job, 15, cpu, 8, IntPtr.Zero);
                int cpuError = cpuOk ? 0 : Marshal.GetLastWin32Error();
                var limitsOk = QueryInformationJobObject(job, 9, limits, 144, IntPtr.Zero);
                int limitsError = limitsOk ? 0 : Marshal.GetLastWin32Error();
                return new { name, member=true, cpu_query_error=cpuError, limit_query_error=limitsError,
                    cpu_flags=BitConverter.ToUInt32(cpu,0), cpu_rate=BitConverter.ToUInt32(cpu,4),
                    limit_flags=BitConverter.ToUInt32(limits,16) };
            }
        } finally { if (job != IntPtr.Zero) CloseHandle(job); }
    }
    public static object Read(int pid, uint parent, string name, string[] jobs) {
        try {
            using (var process = Process.GetProcessById(pid)) {
                var threads = new List<object>();
                foreach (ProcessThread thread in process.Threads) {
                    try {
                        var state = thread.ThreadState;
                        threads.Add(new { tid=thread.Id, state=state.ToString(),
                            started_at=thread.StartTime.ToUniversalTime().ToString("o"),
                            wait=state == System.Diagnostics.ThreadState.Wait ? thread.WaitReason.ToString() : null,
                            priority=thread.CurrentPriority, base_priority=thread.BasePriority,
                            cpu_ms=thread.TotalProcessorTime.TotalMilliseconds });
                    } catch (Exception e) { threads.Add(new { tid=thread.Id, error=e.GetType().Name }); }
                }
                var memberships = new List<object>();
                foreach (string job in jobs) {
                    // Keep nonmembership too: it proves which tracked Jobs were queried.
                    memberships.Add(Job(pid, job));
                }
                return new { pid, parent, name, started_at=process.StartTime.ToUniversalTime().ToString("o"),
                    cpu_ms=process.TotalProcessorTime.TotalMilliseconds, priority=process.PriorityClass.ToString(),
                    base_priority=process.BasePriority, in_job=Job(pid,null), jobs=memberships, threads };
            }
        } catch (Exception e) { return new { pid, parent, name, error=e.GetType().Name }; }
    }
}
'@
if (-not [Environment]::Is64BitProcess) { throw 'The native extended-limit layout requires 64-bit PowerShell.' }
$fixture = Get-Content -LiteralPath "$FixtureHome/fixture.json" -Raw | ConvertFrom-Json
$selfJob = [Issue6422Os]::Job($PID, $null)
$ready = @{pid=$PID;priority=(Get-Process -Id $PID).PriorityClass.ToString();current_job=$selfJob;started_at=[DateTime]::UtcNow.ToString('o');interval_ms=$IntervalMilliseconds;method='Toolhelp process tree + System.Diagnostics.ProcessThread + query-only named Jobs; no stack trace or continuous scheduler trace'}
$ready | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath "$OutputDirectory/observer-ready.json" -Encoding utf8
$writer = [IO.StreamWriter]::new("$OutputDirectory/processes.jsonl", $false, [Text.UTF8Encoding]::new($false))
$clock = [Diagnostics.Stopwatch]::StartNew()
$previous = 0.0
try {
    while (-not (Test-Path -LiteralPath "$FixtureHome/observer-stop") -and $clock.Elapsed.TotalSeconds -lt 1500) {
        $started = [DateTime]::UtcNow.ToString('o')
        $elapsed = $clock.Elapsed.TotalMilliseconds
        $tree = [Issue6422Os]::Tree()
        $owned = [Collections.Generic.HashSet[int]]::new()
        foreach ($root in @($PID, $fixture.gui_pid, $fixture.cargo_load.cargo_pid)) { [void]$owned.Add([int]$root) }
        do {
            $changed = $false
            foreach ($entry in $tree) { if ($owned.Contains([int]$entry.parent) -and $owned.Add([int]$entry.pid)) { $changed = $true } }
        } while ($changed)
        # Include the control runner itself, without adding every Chromium
        # descendant's threads to this deliberately bounded observer.
        if (Test-Path -LiteralPath "$FixtureHome/observer-runner-pid") {
            [void]$owned.Add([int][IO.File]::ReadAllText("$FixtureHome/observer-runner-pid"))
        }
        $registryPath = "$FixtureHome/.gwt/runtime/windows-pane-jobs"
        $registryErrors = [Collections.Generic.List[string]]::new()
        $descriptors = @()
        $registryMissing = -not (Test-Path -LiteralPath $registryPath)
        if (-not $registryMissing) {
            try { $descriptors = @(Get-ChildItem -LiteralPath $registryPath -Filter '*.json' -File -ErrorAction Stop) }
            catch { $registryErrors.Add("enumerate: $($_.Exception.GetType().Name)") }
        }
        $jobs = @($descriptors | ForEach-Object {
            try { $descriptor = Get-Content -LiteralPath $_.FullName -Raw -ErrorAction Stop | ConvertFrom-Json; if ($descriptor.owner_pid -eq $fixture.gui_pid) { $descriptor.name } }
            catch { $registryErrors.Add("$($_.Exception.GetType().Name): descriptor read failed") }
        })
        $rows = @($tree | Where-Object { $owned.Contains([int]$_.pid) } | ForEach-Object { [Issue6422Os]::Read([int]$_.pid, $_.parent, $_.name, [string[]]$jobs) })
        $sample = @{started_at=$started;completed_at=[DateTime]::UtcNow.ToString('o');elapsed_ms=$elapsed;start_gap_ms=$elapsed-$previous;sample_ms=$clock.Elapsed.TotalMilliseconds-$elapsed;registry_missing=$registryMissing;descriptor_count=$descriptors.Count;registry_errors=@($registryErrors);queried_jobs=$jobs;processes=$rows}
        $writer.WriteLine(($sample | ConvertTo-Json -Depth 10 -Compress))
        $writer.Flush()
        $previous = $elapsed
        Start-Sleep -Milliseconds $IntervalMilliseconds
    }
} finally { $writer.Dispose() }
