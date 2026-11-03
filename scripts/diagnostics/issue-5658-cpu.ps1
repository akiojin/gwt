param(
    [Parameter(Mandatory = $true)][int]$TargetProcessId,
    [ValidateRange(1000, 1800000)][int]$SampleMilliseconds = 30000
)

$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
public static class Issue5658Threads {
    [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr OpenThread(uint access, bool inherit, uint id);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll")] static extern bool GetThreadTimes(IntPtr handle, out long created, out long exited, out long kernel, out long user);
    [DllImport("kernel32.dll")] static extern int GetThreadDescription(IntPtr handle, out IntPtr description);
    [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr memory);
    public static object[] Read(int pid) {
        var rows = new List<object>();
        using (var process = Process.GetProcessById(pid)) {
            foreach (ProcessThread thread in process.Threads) {
                var handle = OpenThread(0x0800, false, (uint)thread.Id);
                if (handle == IntPtr.Zero) continue;
                try {
                    long created, exited, kernel, user;
                    if (!GetThreadTimes(handle, out created, out exited, out kernel, out user) || exited != 0) continue;
                    string name = "";
                    IntPtr description;
                    if (GetThreadDescription(handle, out description) >= 0) {
                        try { name = Marshal.PtrToStringUni(description) ?? ""; }
                        finally { LocalFree(description); }
                    }
                    rows.Add(new { id = thread.Id, name, created_file_time = created.ToString(),
                        kernel_seconds = kernel / 10000000.0, user_seconds = user / 10000000.0 });
                } finally { CloseHandle(handle); }
            }
        }
        return rows.ToArray();
    }
}
'@

$process = Get-Process -Id $TargetProcessId
$identity = $process.StartTime.ToUniversalTime().ToString('o')
$binary = $process.Path
$binaryHash = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant()
$clock = [Diagnostics.Stopwatch]::StartNew()
try {
    foreach ($phase in @('start', 'end')) {
        if ($phase -eq 'end') { Start-Sleep -Milliseconds $SampleMilliseconds }
        $process.Refresh()
        if ($process.HasExited -or $process.StartTime.ToUniversalTime().ToString('o') -ne $identity) {
            throw 'Owned GUI process identity changed during CPU sampling.'
        }
        $cpu = $process.TotalProcessorTime.TotalSeconds
        $elapsed = $clock.Elapsed.TotalSeconds
        $capturedAt = [DateTime]::UtcNow.ToString('o')
        $snapshot = [ordered]@{
            phase = $phase; process_id = $TargetProcessId; process_started_at = $identity
            binary = $binary; binary_sha256 = $binaryHash; timestamp_utc = $capturedAt
            elapsed_seconds = $elapsed; process_cpu_seconds = $cpu
            threads = @([Issue5658Threads]::Read($TargetProcessId))
        }
        [Console]::Out.WriteLine(($snapshot | ConvertTo-Json -Depth 5 -Compress))
        [Console]::Out.Flush()
    }
} finally { $process.Dispose() }
