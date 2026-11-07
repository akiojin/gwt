param(
    [Parameter(Mandatory)][int]$TargetProcessId,
    [Parameter(Mandatory)][string]$ExpectedBinary,
    [ValidateRange(1, 120)][int]$Seconds = 30,
    [Parameter(Mandatory)][string]$OutputFile
)

$ErrorActionPreference = 'Stop'
$process = Get-Process -Id $TargetProcessId
$binary = (Resolve-Path -LiteralPath $ExpectedBinary).Path
if ($process.Path -ne $binary -or $binary -notlike '*\target\*') {
    throw 'Sampling is restricted to the explicitly owned checkout executable.'
}
$started = $process.StartTime.ToUniversalTime().ToString('o')
# Sampling briefly suspends one owned thread at a time to obtain a consistent
# native stack. Run separately from idle CPU acceptance. Never target installed gwt.
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;
public static class Gwt6743Sampling {
    [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr OpenThread(uint access, bool inherit, uint id);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll")] static extern uint SuspendThread(IntPtr thread);
    [DllImport("kernel32.dll", SetLastError=true)] static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32.dll", SetLastError=true)] static extern uint GetProcessIdOfThread(IntPtr thread);
    [DllImport("kernel32.dll")] static extern bool GetThreadContext(IntPtr thread, IntPtr context);
    [DllImport("dbghelp.dll", CharSet=CharSet.Ansi, SetLastError=true)] static extern bool SymInitialize(IntPtr process, string path, bool invade);
    [DllImport("dbghelp.dll")] static extern bool SymCleanup(IntPtr process);
    [DllImport("dbghelp.dll")] static extern uint SymSetOptions(uint options);
    [DllImport("dbghelp.dll", SetLastError=true)] static extern bool SymFromAddr(IntPtr process, ulong address, out ulong displacement, IntPtr symbol);
    [DllImport("kernel32.dll", CharSet=CharSet.Ansi)] static extern IntPtr LoadLibraryA(string name);
    [DllImport("kernel32.dll", CharSet=CharSet.Ansi)] static extern IntPtr GetProcAddress(IntPtr module, string name);
    [DllImport("kernel32.dll")] static extern bool FreeLibrary(IntPtr module);
    [DllImport("dbghelp.dll", SetLastError=true)] static extern bool StackWalk64(uint machine, IntPtr process, IntPtr thread, IntPtr frame,
        IntPtr context, IntPtr readMemory, IntPtr functionTable, IntPtr moduleBase, IntPtr translate);
    public sealed class Sample { public int thread_id; public double elapsed_seconds; public ulong[] addresses; }
    public sealed class Symbol { public string address; public string name; public ulong displacement; }
    public sealed class Result { public List<Sample> samples = new List<Sample>(); public List<Symbol> symbols = new List<Symbol>(); public int context_failures; }
    static void Address(IntPtr frame, int offset, long value) { Marshal.WriteInt64(frame, offset, value); Marshal.WriteInt32(frame, offset+12, 3); }
    public static Result Run(int pid, int seconds, string binary, long startedFileTime) {
        if (RuntimeInformation.ProcessArchitecture != Architecture.X64) throw new Exception("AMD64 sampler requires x64 PowerShell.");
        var result = new Result();
        using (var process = Process.GetProcessById(pid)) {
            var handle = process.Handle;
            if (process.StartTime.ToUniversalTime().ToFileTimeUtc() != startedFileTime
                || !String.Equals(process.MainModule.FileName, binary, StringComparison.OrdinalIgnoreCase))
                throw new Exception("Owned process identity changed before sampling.");
            SymSetOptions(0x10 | 0x4 | 0x200 | 0x80000); // lines, deferred, fail-critical-errors, no prompts
            if (!SymInitialize(handle, System.IO.Path.GetDirectoryName(binary), true)) throw new Exception("SymInitialize: " + Marshal.GetLastWin32Error());
            var dbghelp = LoadLibraryA("dbghelp.dll");
            var functionTable = GetProcAddress(dbghelp, "SymFunctionTableAccess64");
            var moduleBase = GetProcAddress(dbghelp, "SymGetModuleBase64");
            var rawContext = Marshal.AllocHGlobal(1250);
            var context = new IntPtr((rawContext.ToInt64()+15) & ~15L);
            var frame = Marshal.AllocHGlobal(1024);
            try {
                var clock = Stopwatch.StartNew();
                while (clock.Elapsed.TotalSeconds < seconds) {
                    process.Refresh();
                    foreach (ProcessThread observed in process.Threads) {
                        // Waiting stacks are not CPU samples. Snapshot races are
                        // retained as a limitation, rather than weighted as CPU time.
                        try { if (observed.ThreadState != System.Diagnostics.ThreadState.Running) continue; }
                        catch (InvalidOperationException) { continue; } // transient thread exited
                        var thread = OpenThread(0x0002 | 0x0008 | 0x0040, false, (uint)observed.Id);
                        if (thread == IntPtr.Zero) continue;
                        try {
                            // Holding the handle closes TID reuse after this check.
                            if (GetProcessIdOfThread(thread) != (uint)pid) continue;
                            for (int i=0;i<1232;i++) Marshal.WriteByte(context,i,0);
                            Marshal.WriteInt32(context,48,0x0010000b); // AMD64 CONTEXT_FULL
                            if (SuspendThread(thread) == uint.MaxValue) continue;
                            try {
                                if (!GetThreadContext(thread, context)) { result.context_failures++; continue; }
                                for (int i=0;i<1024;i++) Marshal.WriteByte(frame,i,0);
                                Address(frame,0,Marshal.ReadInt64(context,248));
                                Address(frame,32,Marshal.ReadInt64(context,160));
                                Address(frame,48,Marshal.ReadInt64(context,152));
                                var addresses = new List<ulong>();
                                addresses.Add((ulong)Marshal.ReadInt64(context,248));
                                for (int depth=0;depth<40;depth++) {
                                    if (!StackWalk64(0x8664,handle,thread,frame,context,IntPtr.Zero,functionTable,moduleBase,IntPtr.Zero)) break;
                                    ulong pc=(ulong)Marshal.ReadInt64(frame,0);
                                    if (pc==0 || (addresses.Count>1 && pc==addresses[addresses.Count-1])) break;
                                    addresses.Add(pc);
                                }
                                result.samples.Add(new Sample { thread_id=observed.Id, elapsed_seconds=clock.Elapsed.TotalSeconds, addresses=addresses.ToArray() });
                            } finally {
                                if (ResumeThread(thread) == uint.MaxValue)
                                    throw new Exception("ResumeThread failed: " + Marshal.GetLastWin32Error());
                            }
                        } finally { CloseHandle(thread); }
                    }
                    Thread.Sleep(10);
                }
                var unique = new HashSet<ulong>();
                foreach(var sample in result.samples) foreach(var address in sample.addresses) unique.Add(address);
                var symbol = Marshal.AllocHGlobal(88+2048);
                try {
                    foreach(var address in unique) {
                        Marshal.WriteInt32(symbol,0,88); Marshal.WriteInt32(symbol,80,2048);
                        ulong displacement;
                        var ok=SymFromAddr(handle,address,out displacement,symbol);
                        result.symbols.Add(new Symbol { address=address.ToString("x"), displacement=displacement,
                            name=ok ? Marshal.PtrToStringAnsi(IntPtr.Add(symbol,84),Marshal.ReadInt32(symbol,76)) : "unresolved" });
                    }
                } finally { Marshal.FreeHGlobal(symbol); }
            } finally { Marshal.FreeHGlobal(frame); Marshal.FreeHGlobal(rawContext); SymCleanup(handle); FreeLibrary(dbghelp); }
        }
        return result;
    }
}
'@
$clock = [Diagnostics.Stopwatch]::StartNew()
$result = [Gwt6743Sampling]::Run($TargetProcessId, $Seconds, $binary, $process.StartTime.ToUniversalTime().ToFileTimeUtc())
$process.Refresh()
if ($process.HasExited -or $process.StartTime.ToUniversalTime().ToString('o') -ne $started) {
    throw 'Owned process identity changed during sampling.'
}
@{pid=$TargetProcessId;started_at=$started;binary=$binary;binary_sha256=(Get-FileHash -LiteralPath $binary).Hash.ToLowerInvariant();
    captured_at=[DateTime]::UtcNow.ToString('o');requested_seconds=$Seconds;elapsed_seconds=$clock.Elapsed.TotalSeconds;
    method='AMD64 running-thread snapshot + SuspendThread/GetThreadContext/StackWalk64; symbol resolution after capture';
    limitation='Running-state snapshot races and profiler overhead; sample counts are not CPU seconds. Excluded from ten-minute CPU acceptance.';
    result=$result} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $OutputFile -Encoding utf8NoBOM
Write-Output "Native samples: $($result.samples.Count); symbols: $($result.symbols.Count); evidence: $OutputFile"
