param(
    [Parameter(Mandatory = $true)][int]$TargetProcessId,
    [ValidateRange(1000, 120000)][int]$SampleMilliseconds = 10000
)

$ErrorActionPreference = 'Stop'

# GetThreadTimes measures CPU consumed by the owned GUI, excluding this sampler
# and the eight child shells. Descriptions identify the Rust publish worker.
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;

public static class Issue5501ThreadCpu {
    [StructLayout(LayoutKind.Sequential)]
    public struct FileTime { public uint Low; public uint High; }
    public sealed class ThreadSample {
        public int id;
        public string name;
        public int description_hresult;
        public string created_file_time;
        public double kernel_seconds;
        public double user_seconds;
    }
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr OpenThread(uint access, bool inheritHandle, uint threadId);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetThreadTimes(IntPtr thread, out FileTime created,
        out FileTime exited, out FileTime kernel, out FileTime user);
    [DllImport("kernel32.dll")]
    static extern int GetThreadDescription(IntPtr thread, out IntPtr description);
    [DllImport("kernel32.dll")]
    static extern IntPtr LocalFree(IntPtr memory);
    [DllImport("kernel32.dll")]
    static extern bool CloseHandle(IntPtr handle);
    static long Ticks(FileTime time) { return ((long)time.High << 32) | time.Low; }

    public static ThreadSample[] Read(int processId) {
        var samples = new List<ThreadSample>();
        using (var process = Process.GetProcessById(processId)) {
            foreach (ProcessThread thread in process.Threads) {
                IntPtr handle = OpenThread(0x0800, false, (uint)thread.Id);
                if (handle == IntPtr.Zero) continue; // A transient thread may exit.
                try {
                    FileTime created, exited, kernel, user;
                    if (!GetThreadTimes(handle, out created, out exited, out kernel, out user)) continue;
                    string name = "";
                    IntPtr description;
                    int descriptionResult = GetThreadDescription(handle, out description);
                    if (descriptionResult >= 0) {
                        try { name = Marshal.PtrToStringUni(description) ?? ""; }
                        finally { LocalFree(description); }
                    }
                    samples.Add(new ThreadSample {
                        id = thread.Id, name = name, description_hresult = descriptionResult,
                        created_file_time = Ticks(created).ToString(),
                        kernel_seconds = Ticks(kernel) / 10000000.0,
                        user_seconds = Ticks(user) / 10000000.0
                    });
                } finally { CloseHandle(handle); }
            }
        }
        return samples.ToArray();
    }
}
'@

foreach ($phase in @('start', 'end')) {
    if ($phase -eq 'end') { Start-Sleep -Milliseconds $SampleMilliseconds }
    $threads = [Issue5501ThreadCpu]::Read($TargetProcessId)
    $snapshot = [ordered]@{
        phase = $phase
        process_id = $TargetProcessId
        timestamp_utc = [DateTime]::UtcNow.ToString('o')
        threads = @($threads | Sort-Object created_file_time, id)
    }
    [Console]::Out.WriteLine(($snapshot | ConvertTo-Json -Depth 5 -Compress))
    [Console]::Out.Flush()
}
