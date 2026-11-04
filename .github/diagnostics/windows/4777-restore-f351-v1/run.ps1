# This wrapper owns the diagnostic. Historical source receives no runner credentials.
# A missing/partial result never establishes NoStartVerified or cleanup success.
param([switch]$SelfTest)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Get-BytesHash([byte[]]$Bytes) {
    $algorithm = [Security.Cryptography.SHA256]::Create()
    try { return -join ($algorithm.ComputeHash($Bytes) | ForEach-Object { $_.ToString('x2') }) }
    finally { $algorithm.Dispose() }
}

function New-ChildEnvironment([string]$PrivateRoot) {
    $values = [Collections.Generic.Dictionary[string,string]]::new([StringComparer]::OrdinalIgnoreCase)
    # Build/runtime infrastructure only. Credentials, Actions metadata, provider
    # identities, arbitrary GWT settings and the caller's HOME are absent.
    foreach ($name in @('PATH','SystemRoot','WINDIR','COMSPEC','PATHEXT',
        'PROCESSOR_ARCHITECTURE','PROCESSOR_IDENTIFIER','NUMBER_OF_PROCESSORS',
        'ProgramFiles','ProgramFiles(x86)','ProgramData','CommonProgramFiles',
        'CommonProgramFiles(x86)','RUSTUP_HOME','VSINSTALLDIR','VCINSTALLDIR',
        'VCToolsInstallDir','WindowsSdkDir','WindowsSDKVersion','UCRTVersion',
        'UniversalCRTSdkDir','INCLUDE','LIB','LIBPATH')) {
        $value = [Environment]::GetEnvironmentVariable($name)
        if ($null -ne $value) { $values[$name] = $value }
    }
    foreach ($pair in @{
        HOME = 'home'; USERPROFILE = 'home'; APPDATA = 'home/AppData/Roaming';
        LOCALAPPDATA = 'home/AppData/Local'; TEMP = 'temp'; TMP = 'temp';
        CARGO_HOME = 'home/.cargo'; CARGO_TARGET_DIR = 'target'
    }.GetEnumerator()) {
        $path = Join-Path $PrivateRoot $pair.Value
        [void][IO.Directory]::CreateDirectory($path)
        $values[$pair.Key] = $path
    }
    $values['CARGO_PROFILE_DEV_DEBUG'] = '0'
    return ,$values
}

function Initialize-OwnedLauncher {
    if ('GwtFixedDiagnostic.OwnedJob' -as [type]) { return }
    # Microsoft CreateProcess contract: explicit handle list, suspended creation,
    # assignment before the first instruction, retained handles and job drain.
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
namespace GwtFixedDiagnostic {
public sealed class OwnedJob : IDisposable {
    const uint CREATE_SUSPENDED = 0x4, CREATE_UNICODE_ENVIRONMENT = 0x400;
    const uint EXTENDED_STARTUPINFO_PRESENT = 0x80000, CREATE_NO_WINDOW = 0x8000000;
    const int PROC_THREAD_ATTRIBUTE_HANDLE_LIST = 0x20002;
    IntPtr job, process;
    bool disposed;
    public int? ExitCode { get; private set; }
    [StructLayout(LayoutKind.Sequential)] struct Security { public int Length; public IntPtr Descriptor; [MarshalAs(UnmanagedType.Bool)] public bool Inherit; }
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] struct Startup {
        public int Size; public string Reserved, Desktop, Title;
        public uint X,Y,XSize,YSize,XChars,YChars,Fill,Flags; public ushort Show,Reserved2;
        public IntPtr ReservedPointer,Input,Output,Error;
    }
    [StructLayout(LayoutKind.Sequential)] struct StartupEx { public Startup Startup; public IntPtr Attributes; }
    [StructLayout(LayoutKind.Sequential)] struct ProcessInfo { public IntPtr Process,Thread; public uint ProcessId,ThreadId; }
    [StructLayout(LayoutKind.Sequential)] struct Limits {
        public long ProcessTime,JobTime; public uint Flags;
        public UIntPtr MinWorking,MaxWorking; public uint ActiveLimit; public UIntPtr Affinity;
        public uint Priority,Scheduling;
    }
    [StructLayout(LayoutKind.Sequential)] struct IoCounters { public ulong ReadOps,WriteOps,OtherOps,ReadBytes,WriteBytes,OtherBytes; }
    [StructLayout(LayoutKind.Sequential)] struct ExtendedLimits { public Limits Basic; public IoCounters Io; public UIntPtr ProcessMemory,JobMemory,PeakProcessMemory,PeakJobMemory; }
    [StructLayout(LayoutKind.Sequential)] struct Accounting {
        public long User,Kernel,PeriodUser,PeriodKernel; public uint PageFaults,TotalProcesses,ActiveProcesses,Terminated;
    }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr CreateJobObjectW(IntPtr attributes,string name);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool SetInformationJobObject(IntPtr job,int kind,ref ExtendedLimits limits,uint size);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool QueryInformationJobObject(IntPtr job,int kind,out Accounting info,uint size,IntPtr returned);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool AssignProcessToJobObject(IntPtr job,IntPtr process);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool TerminateJobObject(IntPtr job,uint exit);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool TerminateProcess(IntPtr process,uint exit);
    [DllImport("kernel32.dll", SetLastError=true)] static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32.dll", SetLastError=true)] static extern uint WaitForSingleObject(IntPtr handle,uint milliseconds);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetExitCodeProcess(IntPtr process,out uint exit);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool InitializeProcThreadAttributeList(IntPtr list,int count,uint flags,ref IntPtr size);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool UpdateProcThreadAttribute(IntPtr list,uint flags,IntPtr attribute,IntPtr value,IntPtr size,IntPtr previous,IntPtr returned);
    [DllImport("kernel32.dll")] static extern void DeleteProcThreadAttributeList(IntPtr list);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr CreateFileW(string name,uint access,uint share,ref Security attributes,uint creation,uint flags,IntPtr template);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool CreateProcessW(string application,StringBuilder command,IntPtr processAttributes,IntPtr threadAttributes,bool inherit,uint flags,IntPtr environment,string directory,ref StartupEx startup,out ProcessInfo info);
    static Exception Unavailable() { return new InvalidOperationException("owned diagnostic process unavailable"); }
    static string Quote(string value) {
        var result = new StringBuilder("\""); int slashes = 0;
        foreach (char c in value) {
            if (c == '\\') { slashes++; continue; }
            if (c == '"') { result.Append('\\', slashes * 2 + 1); result.Append(c); }
            else { result.Append('\\', slashes); result.Append(c); }
            slashes = 0;
        }
        result.Append('\\', slashes * 2); return result.Append('"').ToString();
    }
    // Construct a complete EnvironmentBlock; CreateProcess never inherits the
    // wrapper's environment. Only explicit stdout/stderr/NUL handles are passed.
    public OwnedJob(string executable,string[] arguments,string directory,IDictionary<string,string> environment,string rawPath) {
        IntPtr output = IntPtr.Zero, input = IntPtr.Zero, attributes = IntPtr.Zero, handles = IntPtr.Zero, block = IntPtr.Zero;
        ProcessInfo info = new ProcessInfo(); bool initialized = false;
        try {
            job = CreateJobObjectW(IntPtr.Zero,null); if (job == IntPtr.Zero) throw Unavailable();
            var limits = new ExtendedLimits(); limits.Basic.Flags = 0x2000; // KILL_ON_JOB_CLOSE; no breakaway.
            if (!SetInformationJobObject(job,9,ref limits,(uint)Marshal.SizeOf(limits))) throw Unavailable();
            var security = new Security { Length=Marshal.SizeOf(typeof(Security)),Inherit=true };
            output = CreateFileW(rawPath,0x40000000,1,ref security,2,0x80,IntPtr.Zero);
            input = CreateFileW("NUL",0x80000000,1|2,ref security,3,0x80,IntPtr.Zero);
            if (output == new IntPtr(-1) || input == new IntPtr(-1)) throw Unavailable();
            IntPtr size = IntPtr.Zero; InitializeProcThreadAttributeList(IntPtr.Zero,1,0,ref size);
            attributes = Marshal.AllocHGlobal(size);
            if (!InitializeProcThreadAttributeList(attributes,1,0,ref size)) throw Unavailable();
            initialized = true; handles = Marshal.AllocHGlobal(IntPtr.Size*2);
            Marshal.WriteIntPtr(handles,0,input); Marshal.WriteIntPtr(handles,IntPtr.Size,output);
            if (!UpdateProcThreadAttribute(attributes,0,new IntPtr(PROC_THREAD_ATTRIBUTE_HANDLE_LIST),handles,new IntPtr(IntPtr.Size*2),IntPtr.Zero,IntPtr.Zero)) throw Unavailable();
            var startup = new StartupEx(); startup.Startup.Size=Marshal.SizeOf(startup);
            startup.Startup.Flags=0x100; startup.Startup.Input=input; startup.Startup.Output=output; startup.Startup.Error=output; startup.Attributes=attributes;
            string text = String.Join("\0",environment.OrderBy(p=>p.Key,StringComparer.OrdinalIgnoreCase).Select(p=>p.Key+"="+p.Value))+"\0\0";
            block=Marshal.StringToHGlobalUni(text);
            var command=new StringBuilder(Quote(executable)+" "+String.Join(" ",arguments.Select(Quote)));
            if (!CreateProcessW(executable,command,IntPtr.Zero,IntPtr.Zero,true,CREATE_SUSPENDED|CREATE_UNICODE_ENVIRONMENT|EXTENDED_STARTUPINFO_PRESENT|CREATE_NO_WINDOW,block,directory,ref startup,out info)) throw Unavailable();
            process=info.Process;
            if (!AssignProcessToJobObject(job,process)) { TerminateProcess(process,1); WaitForSingleObject(process,5000); throw Unavailable(); }
            if (ResumeThread(info.Thread)==0xffffffff) { TerminateJobObject(job,1); throw Unavailable(); }
        } catch { Dispose(); throw Unavailable(); }
        finally {
            if (info.Thread!=IntPtr.Zero) CloseHandle(info.Thread);
            if (output!=IntPtr.Zero && output!=new IntPtr(-1)) CloseHandle(output);
            if (input!=IntPtr.Zero && input!=new IntPtr(-1)) CloseHandle(input);
            if (initialized) DeleteProcThreadAttributeList(attributes);
            if (attributes!=IntPtr.Zero) Marshal.FreeHGlobal(attributes);
            if (handles!=IntPtr.Zero) Marshal.FreeHGlobal(handles);
            if (block!=IntPtr.Zero) Marshal.FreeHGlobal(block);
        }
    }
    public bool HasExited() {
        uint state=WaitForSingleObject(process,0); if (state==0x102) return false;
        if (state!=0) throw Unavailable(); uint code;
        if (!GetExitCodeProcess(process,out code)) throw Unavailable(); ExitCode=unchecked((int)code); return true;
    }
    public bool Cleanup(int maximumMilliseconds=300000) {
        if (job==IntPtr.Zero || !TerminateJobObject(job,1)) return false;
        var elapsed=Stopwatch.StartNew();
        while (elapsed.ElapsedMilliseconds<maximumMilliseconds) {
            Accounting info;
            if (!QueryInformationJobObject(job,1,out info,(uint)Marshal.SizeOf(typeof(Accounting)),IntPtr.Zero)) return false;
            if (info.ActiveProcesses==0) return true;
            Thread.Sleep(100);
        }
        return false;
    }
    public void Dispose() {
        if (disposed) return; disposed=true;
        if (job!=IntPtr.Zero) { CloseHandle(job); job=IntPtr.Zero; }
        if (process!=IntPtr.Zero) { CloseHandle(process); process=IntPtr.Zero; }
    }
}
}
'@ | Out-Null
}

function Wait-OwnedJobs([object[]]$Jobs, [int]$Minutes) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($true) {
        $done = $true
        foreach ($job in $Jobs) { if (-not $job.HasExited()) { $done = $false } }
        if ($done) { return $true }
        if ($watch.Elapsed.TotalMinutes -ge $Minutes) { return $false }
        Start-Sleep -Milliseconds 100
    }
}

function Write-DurableJson([string]$Path, [object]$Value) {
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($Value | ConvertTo-Json -Depth 12 -Compress))
    $stream = [IO.FileStream]::new($Path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None,4096,[IO.FileOptions]::WriteThrough)
    try { $stream.Write($bytes,0,$bytes.Length); $stream.Flush($true) }
    finally { $stream.Dispose() }
}

function Get-CaseOutcomes([string]$Provider, [string]$Raw, $ExitCode) {
    # exit_code is the shared provider/libtest process exit. A known PASS may
    # carry 101 when its sibling failed; no per-case OS exit is invented.
    if ($Provider -notin @('codex','claude') -or ($null -ne $ExitCode -and $ExitCode -isnot [int])) { throw 'invalid case projection' }
    $summary=[Regex]::Matches($Raw,'(?m)^test result: (?:ok|FAILED)\. ([0-2]) passed; ([0-2]) failed; 0 ignored;')
    $complete=$summary.Count -eq 1 -and ([int]$summary[0].Groups[1].Value+[int]$summary[0].Groups[2].Value) -eq 2
    $failed=@()
    foreach ($name in @('windows_official_provider_launch_uses_installed_direct_runner','windows_startup_restored_provider_tree_closes_through_public_gui')) {
        if ($Raw -match ('(?m)^    '+[Regex]::Escape($name)+'\s*$')) { $failed += $name }
    }
    foreach ($kind in @('official_launch','restored_close')) {
        $name=if ($kind -eq 'official_launch') {'windows_official_provider_launch_uses_installed_direct_runner'} else {'windows_startup_restored_provider_tree_closes_through_public_gui'}
        $outcome='unknown'
        if ($complete -and $ExitCode -eq 0 -and [int]$summary[0].Groups[1].Value -eq 2) { $outcome='pass' }
        elseif ($complete -and $ExitCode -ne 0 -and $failed.Count -eq [int]$summary[0].Groups[2].Value) {
            $outcome=if ($name -in $failed) {'fail'} else {'pass'}
        }
        [ordered]@{provider=$Provider;case=$kind;outcome=$outcome;exit_code=$ExitCode}
    }
}

function Complete-ProviderObservation([bool]$AllFinished, [object[]]$Providers, [object]$Result) {
    $Result.reason=if ($AllFinished) {'Completed'} else {'WrapperTimeout'}
    foreach ($entry in $Providers) {
        try {
            # Query the retained exact process object before reading an outcome.
            # A peer timeout does not erase an already finished provider.
            if (-not $entry.job.HasExited()) { continue }
            foreach ($case in $Result.cases | Where-Object { $_.provider -eq $entry.provider }) { $case.exit_code=$entry.job.ExitCode }
            $raw=[IO.File]::ReadAllText($entry.raw)
            foreach ($observed in @(Get-CaseOutcomes $entry.provider $raw $entry.job.ExitCode)) {
                foreach ($case in $Result.cases | Where-Object { $_.provider -eq $observed.provider -and $_.case -eq $observed.case }) {
                    $case.outcome=$observed.outcome; $case.exit_code=$observed.exit_code
                }
            }
        } catch {
            # Keep this provider's unobserved fields unknown; continue collecting
            # its peer without exposing raw errors or declaring NoStart.
        }
    }
    if ($AllFinished -and @($Result.cases | Where-Object { $_.outcome -ne 'pass' -or $_.exit_code -ne 0 }).Count -gt 0) { $Result.reason='CasesFailed' }
}

function Read-StaticCommand([string]$Executable, [string[]]$Arguments, [string]$Directory, [string]$RawPath, [int]$MaximumMilliseconds=300000) {
    # These are wrapper-owned metadata queries, never historical build/test code.
    $start = [Diagnostics.ProcessStartInfo]::new($Executable)
    $start.UseShellExecute = $false; $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true; $start.RedirectStandardError = $true
    $start.WorkingDirectory = $Directory
    foreach ($argument in $Arguments) { [void]$start.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new(); $process.StartInfo = $start
    try {
        if ($MaximumMilliseconds -le 0) { throw 'metadata deadline elapsed' }
        [void]$process.Start(); $stdout = $process.StandardOutput.ReadToEndAsync(); $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($MaximumMilliseconds)) { $process.Kill(); throw 'metadata unavailable' }
        [IO.File]::WriteAllText($RawPath,$stdout.GetAwaiter().GetResult()+$stderr.GetAwaiter().GetResult())
        if ($process.ExitCode -ne 0) { throw 'metadata unavailable' }
        return $stdout.GetAwaiter().GetResult().Trim()
    } finally { $process.Dispose() }
}

function Read-SetupCommand([string]$Executable, [string[]]$Arguments, [string]$Directory, [string]$RawPath, [Diagnostics.Stopwatch]$Watch) {
    $remaining=[Math]::Max(0,300000-[int]$Watch.ElapsedMilliseconds)
    return Read-StaticCommand $Executable $Arguments $Directory $RawPath $remaining
}

function Set-DiagnosticFailure([object]$Result) {
    $Result.reason=switch ($Result.phase) {
        'compile' {'CompileFailed'}
        'contracts' {'ContractsFailed'}
        'providers' {'CasesFailed'}
        default {'SetupFailed'}
    }
}

function Test-FixedSourceInputs([object]$Profile, [string]$Head, [string]$Tree, [byte[]]$Before, [byte[]]$Lock, [byte[]]$Patch) {
    return $Head -ceq $Profile.source_commit -and $Tree -ceq $Profile.source_tree -and $Before.Length -eq $Profile.before.bytes -and (Get-BytesHash $Before) -ceq $Profile.before.sha256 -and $Lock.Length -eq $Profile.source_lock.bytes -and (Get-BytesHash $Lock) -ceq $Profile.source_lock.sha256 -and $Patch.Length -eq $Profile.patch.bytes -and (Get-BytesHash $Patch) -ceq $Profile.patch.sha256
}

function Complete-DiagnosticCleanup([object]$Result, [object[]]$Jobs, [string]$Target, [byte[]]$Original, [bool]$Modified, [string]$Git, [string]$Source, [object]$Profile, [string]$Private) {
    $clean=$true
    $phaseBeforeRestore=$result.phase
    $cleanupWatch=[Diagnostics.Stopwatch]::StartNew()
    foreach ($owned in $jobs) {
        try {
            $remaining=[Math]::Max(0,300000-[int]$cleanupWatch.ElapsedMilliseconds)
            if (-not $owned.Cleanup($remaining)) { $clean=$false }
        }
        catch { $clean=$false }
        finally {
            try { $owned.Dispose() }
            catch { $clean=$false }
        }
    }
    if ($modified) {
        $result.phase='restore'
        try {
            $oldMtime=[IO.File]::GetLastWriteTimeUtc($target)
            [IO.File]::WriteAllBytes($target,$original)
            $fresh=[DateTime]::UtcNow
            if ($fresh -le $oldMtime) { $fresh=$oldMtime.AddTicks(1) }
            [IO.File]::SetLastWriteTimeUtc($target,$fresh)
            $restored=[IO.File]::ReadAllBytes($target)
            $result.restore.after_bytes=$restored.Length; $result.restore.after_sha256=Get-BytesHash $restored
            if ($restored.Length -ne $original.Length -or $result.restore.after_sha256 -cne $result.restore.before_sha256 -or [IO.File]::GetLastWriteTimeUtc($target) -le $oldMtime) { $clean=$false }
            if ((Get-BytesHash ([IO.File]::ReadAllBytes((Join-Path $source 'Cargo.lock')))) -cne $profile.source_lock.sha256) { $clean=$false }
            $remaining=[Math]::Max(0,300000-[int]$cleanupWatch.ElapsedMilliseconds)
            [void](Read-StaticCommand $git @('reset','-q','HEAD','--',$profile.target_path) $source (Join-Path $private 'reset.raw') $remaining)
            $remaining=[Math]::Max(0,300000-[int]$cleanupWatch.ElapsedMilliseconds)
            $restoredTree=Read-StaticCommand $git @('write-tree') $source (Join-Path $private 'restored-tree.raw') $remaining
            if ($restoredTree -cne $profile.source_tree) { $clean=$false }
            $remaining=[Math]::Max(0,300000-[int]$cleanupWatch.ElapsedMilliseconds)
            [void](Read-StaticCommand $git @('diff','--quiet','HEAD','--') $source (Join-Path $private 'restored-diff.raw') $remaining)
        } catch { $clean=$false }
    }
    if ($cleanupWatch.ElapsedMilliseconds -ge 300000) { $clean=$false }
    if ($result.no_start_verified -and -not $modified -and $jobs.Count -eq 0) { $result.cleanup='NotStarted' }
    elseif ($clean -and $modified) { $result.cleanup='ObservedClean' }
    else { $result.cleanup='Unobserved'; $result.no_start_verified=$false }
    if ($result.cleanup -eq 'Unobserved') { $result.reason='CleanupUnobserved'; $result.phase='restore' }
    elseif ($result.reason -eq 'Completed' -or $result.reason -eq 'CasesFailed') { $result.phase='result' }
    else { $result.phase=$phaseBeforeRestore }
}

function Test-NoStartBoundaries([string]$Root) {
    # Synthetic fixtures only: these files are private SelfTest output, never
    # provider artifacts, canonical receipts or authority for a live request.
    $names=@('GWT_DIAG_REQUEST_ID','GWT_DIAG_EXPECTED_PROFILE','GWT_DIAG_EXPECTED_WORKFLOW',
        'GWT_DIAG_SOURCE_ROOT','GWT_DIAG_RESULT_DIR','RUNNER_TEMP','GITHUB_REPOSITORY',
        'GITHUB_SHA','GITHUB_RUN_ID','GITHUB_RUN_ATTEMPT')
    $saved=@{}
    foreach ($name in $names) { $saved[$name]=[Environment]::GetEnvironmentVariable($name) }
    try {
        $profileHash=Get-BytesHash ([IO.File]::ReadAllBytes((Join-Path $PSScriptRoot 'profile.json')))
        foreach ($kind in @('RefDrift','InputMismatch','Unobserved')) {
            $directory=Join-Path $Root ('synthetic-'+$kind)
            [void][IO.Directory]::CreateDirectory($directory)
            foreach ($pair in @{
                GWT_DIAG_REQUEST_ID='00000000-0000-4000-8000-000000000001';
                GWT_DIAG_EXPECTED_PROFILE=$profileHash; GWT_DIAG_EXPECTED_WORKFLOW=('a'*40);
                GWT_DIAG_SOURCE_ROOT=(Join-Path $directory 'missing-source');
                GWT_DIAG_RESULT_DIR=$directory; RUNNER_TEMP=$directory;
                GITHUB_REPOSITORY='akiojin/gwt'; GITHUB_SHA=('a'*40);
                GITHUB_RUN_ID='1'; GITHUB_RUN_ATTEMPT='1'
            }.GetEnumerator()) { [Environment]::SetEnvironmentVariable($pair.Key,$pair.Value) }
            if ($kind -eq 'RefDrift') { [Environment]::SetEnvironmentVariable('GWT_DIAG_EXPECTED_WORKFLOW',('b'*40)) }
            if ($kind -eq 'InputMismatch') { [Environment]::SetEnvironmentVariable('GWT_DIAG_EXPECTED_PROFILE',('0'*64)) }
            Invoke-Diagnostic
            $receipt=[IO.File]::ReadAllText((Join-Path $directory 'result.json')) | ConvertFrom-Json
            if ($receipt.observation_started -or $receipt.cases.Count -ne 4 -or @($receipt.cases | Where-Object { $_.outcome -ne 'not_run' -or $null -ne $_.exit_code }).Count -ne 0) { throw 'synthetic start boundary failed' }
            if ($kind -eq 'Unobserved') {
                if ($receipt.no_start_verified -or $receipt.cleanup -ne 'Unobserved') { throw 'missing observation became NoStart' }
            } elseif (-not $receipt.no_start_verified -or $receipt.reason -ne $kind -or $receipt.cleanup -ne 'NotStarted') { throw 'explicit NoStart boundary failed' }
        }
    } finally {
        foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name,$saved[$name]) }
    }
}

function Test-CleanupRestoreFixtures([string]$Root, [object]$NativeJob) {
    # A private repository exercises the production index/bytes/mtime restore.
    # Fault objects replace only the native API result, never the cleanup logic.
    $source=Join-Path $Root 'cleanup-fixture'
    $hooks=Join-Path $source 'empty-hooks'
    [void][IO.Directory]::CreateDirectory($hooks)
    $git=(Get-Command git -CommandType Application).Source
    $original=[Text.Encoding]::UTF8.GetBytes("fixture original`n")
    $overlay=[Text.Encoding]::UTF8.GetBytes("fixture overlay`n")
    $lock=[Text.Encoding]::UTF8.GetBytes("fixture lock`n")
    $target=Join-Path $source 'fixture.rs'
    [IO.File]::WriteAllBytes($target,$original)
    [IO.File]::WriteAllBytes((Join-Path $source 'Cargo.lock'),$lock)
    [void](Read-StaticCommand $git @('init','--quiet',('--template='+$hooks)) $source (Join-Path $Root 'fixture-init.raw'))
    [void](Read-StaticCommand $git @('add','--','fixture.rs','Cargo.lock') $source (Join-Path $Root 'fixture-add.raw'))
    [void](Read-StaticCommand $git @('-c',('core.hooksPath='+$hooks),'-c','commit.gpgsign=false','-c','user.name=DiagnosticFixture','-c','user.email=fixture@example.invalid','commit','-qm','fixture') $source (Join-Path $Root 'fixture-commit.raw'))
    $head=Read-StaticCommand $git @('rev-parse','HEAD') $source (Join-Path $Root 'fixture-head.raw')
    $tree=Read-StaticCommand $git @('rev-parse','HEAD^{tree}') $source (Join-Path $Root 'fixture-tree.raw')
    $definition=@{source_commit=$head;source_tree=$tree;target_path='fixture.rs';before=@{bytes=$original.Length;sha256=(Get-BytesHash $original)};source_lock=@{bytes=$lock.Length;sha256=(Get-BytesHash $lock)};patch=@{bytes=$overlay.Length;sha256=(Get-BytesHash $overlay)}}
    if (-not (Test-FixedSourceInputs $definition $head $tree $original $lock $overlay)) { throw 'legal source fixture rejected' }
    $wrongBefore=[byte[]]$original.Clone(); $wrongBefore[0]=$wrongBefore[0] -bxor 1
    if (Test-FixedSourceInputs $definition $head $tree $wrongBefore $lock $overlay) { throw 'same-length before mismatch accepted' }
    $sentinel='private-cleanup-sentinel'
    foreach ($scenario in @('setup','compile','contracts','providers','cleanup_false','cleanup_throw','dispose_throw','restore_failure','deadline_cancel')) {
        if ([IO.Directory]::Exists($target)) { [IO.Directory]::Delete($target) }
        [IO.File]::WriteAllBytes($target,$overlay)
        $old=[DateTime]::new(2020,1,1,0,0,0,[DateTimeKind]::Utc)
        [IO.File]::SetLastWriteTimeUtc($target,$old)
        [void](Read-StaticCommand $git @('add','--','fixture.rs') $source (Join-Path $Root 'fixture-overlay.raw'))
        $phase=if ($scenario -in @('setup','compile','contracts','providers')) {$scenario} else {'providers'}
        $result=@{phase=$phase;reason='WrapperTimeout';cleanup='Unobserved';no_start_verified=$false;observation_started=($phase -ne 'setup');restore=@{before_bytes=$original.Length;before_sha256=(Get-BytesHash $original);after_bytes=$null;after_sha256=$null}}
        $jobs=@()
        if ($scenario -in @('setup','compile','contracts','providers')) {
            try { throw [InvalidOperationException]::new($sentinel) }
            catch { Set-DiagnosticFailure $result }
        }
        if ($scenario -in @('cleanup_false','cleanup_throw','dispose_throw')) {
            $fault=[pscustomobject]@{Mode=$scenario}
            $fault | Add-Member ScriptMethod Cleanup {param($milliseconds) if ($this.Mode -eq 'cleanup_throw') {throw 'private-cleanup-sentinel'}; return $this.Mode -ne 'cleanup_false'}
            $fault | Add-Member ScriptMethod Dispose {if ($this.Mode -eq 'dispose_throw') {throw 'private-cleanup-sentinel'}}
            $jobs=@($fault)
        }
        if ($scenario -eq 'restore_failure') {
            [IO.File]::Delete($target)
            [void][IO.Directory]::CreateDirectory($target)
        }
        if ($scenario -eq 'deadline_cancel' -and $null -ne $NativeJob) { $jobs=@($NativeJob) }
        Complete-DiagnosticCleanup $result $jobs $target $original $true $git $source $definition $Root
        $failed=$scenario -in @('cleanup_false','cleanup_throw','dispose_throw','restore_failure')
        if ($failed) {
            if ($result.cleanup -ne 'Unobserved' -or $result.reason -ne 'CleanupUnobserved') { throw 'cleanup fault became success' }
        } else {
            $expected=switch ($scenario) {'setup' {'SetupFailed'} 'compile' {'CompileFailed'} 'contracts' {'ContractsFailed'} 'providers' {'CasesFailed'} default {'WrapperTimeout'}}
            if ($result.cleanup -ne 'ObservedClean' -or $result.reason -ne $expected) { throw 'stage fault lost its safe outcome' }
        }
        if ($result.no_start_verified) { throw 'cleanup fault became NoStart' }
        if ($scenario -ne 'restore_failure') {
            if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($target)) -cne [Convert]::ToBase64String($original) -or [IO.File]::GetLastWriteTimeUtc($target) -le $old -or $result.restore.after_sha256 -cne $definition.before.sha256) { throw 'original bytes or fresh mtime were not restored' }
        } elseif ($null -ne $result.restore.after_sha256) { throw 'failed restore invented a measurement' }
        $receipt=Join-Path $Root ('synthetic-cleanup-'+$scenario+'.json')
        Write-DurableJson $receipt $result
        $safe=[IO.File]::ReadAllText($receipt)
        $saved=$safe | ConvertFrom-Json
        if ($saved.cleanup -ne $result.cleanup -or $saved.reason -ne $result.reason -or $saved.no_start_verified -or $saved.restore.after_sha256 -cne $result.restore.after_sha256) { throw 'saved cleanup outcome changed' }
        if ($safe.Contains($sentinel) -or $safe.Contains($source) -or $safe.Contains($source.Replace('\','\\'))) { throw 'cleanup fault leaked private diagnostics' }
    }
}

function Test-Wrapper {
    $root = Join-Path ([IO.Path]::GetTempPath()) ('gwt-diagnostic-selftest-'+[Guid]::NewGuid().ToString())
    [void][IO.Directory]::CreateDirectory($root)
    $job = $null
    try {
        Initialize-OwnedLauncher
        $environment = New-ChildEnvironment $root
        if ($environment.ContainsKey('GWT_DIAGNOSTIC_SECRET_SENTINEL') -or $environment.ContainsKey('GITHUB_TOKEN') -or $environment.ContainsKey('ACTIONS_RUNTIME_TOKEN')) { throw 'unsafe environment' }
        $command = Join-Path $env:SystemRoot 'System32/cmd.exe'
        $job = [GwtFixedDiagnostic.OwnedJob]::new($command,@('/d','/c','if defined GWT_DIAGNOSTIC_SECRET_SENTINEL (exit 13) else (echo private-diagnostic-sentinel & echo test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; & exit 0)'),$root,$environment,(Join-Path $root 'private.raw'))
        if (-not (Wait-OwnedJobs @($job) 1) -or $job.ExitCode -ne 0 -or -not $job.Cleanup()) { throw 'owned process failed' }
        # Public output is constructed from a closed projection; neither raw
        # diagnostics nor arbitrary exceptions are serialized into the receipt.
        $sentinel = 'private-diagnostic-sentinel'
        $raw=[IO.File]::ReadAllText((Join-Path $root 'private.raw'))
        if (-not $raw.Contains($sentinel)) { throw 'raw sentinel fixture missing' }
        $cases=@(Get-CaseOutcomes 'codex' $raw ([int]0))
        $candidate = [ordered]@{ schema_version=1; reason='Completed'; cleanup='ObservedClean';cases=$cases;diagnostic_only=$true;ready_evidence=$false }
        Write-DurableJson (Join-Path $root 'result.json') $candidate
        $safe=[IO.File]::ReadAllText((Join-Path $root 'result.json'))
        if (@($cases | Where-Object { $_.outcome -eq 'pass' }).Count -ne 2) { throw 'case projection failed' }
        if ($safe.Contains($sentinel) -or $safe.Contains($root)) { throw 'unsafe result' }
        $failed=@(Get-CaseOutcomes 'claude' ($sentinel+"`n    windows_startup_restored_provider_tree_closes_through_public_gui`ntest result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 1 filtered out;`n") ([int]101))
        if ($failed[0].outcome -ne 'pass' -or $failed[1].outcome -ne 'fail') { throw 'failure projection failed' }
        $finishedJob=$job
        # An infinite controlled child makes the deadline branch independent of
        # runner scheduling. The same retained job must drain after termination.
        $job=[GwtFixedDiagnostic.OwnedJob]::new([Environment]::ProcessPath,@('-NoLogo','-NoProfile','-NonInteractive','-Command','while ($true) { Start-Sleep -Seconds 1 }'),$root,$environment,(Join-Path $root 'deadline.raw'))
        try {
            if (Wait-OwnedJobs @($job) 0) { throw 'deadline branch did not run' }
            $partial=@{reason='WrapperTimeout';observation_started=$true;no_start_verified=$false;cases=@()}
            foreach ($provider in @('codex','claude')) {
                foreach ($case in @('official_launch','restored_close')) { $partial.cases += @{provider=$provider;case=$case;outcome='unknown';exit_code=$null} }
            }
            Complete-ProviderObservation $false @(@{provider='codex';job=$finishedJob;raw=(Join-Path $root 'private.raw')},@{provider='claude';job=$job;raw=(Join-Path $root 'deadline.raw')}) $partial
            if (@($partial.cases | Where-Object { $_.provider -eq 'codex' -and $_.outcome -eq 'pass' -and $_.exit_code -eq 0 }).Count -ne 2) { throw 'finished provider measurements were discarded' }
            if (@($partial.cases | Where-Object { $_.provider -eq 'claude' -and $_.outcome -eq 'unknown' -and $null -eq $_.exit_code }).Count -ne 2 -or $partial.reason -ne 'WrapperTimeout' -or $partial.no_start_verified) { throw 'partial timeout boundary changed' }
            if (($partial | ConvertTo-Json -Depth 6 -Compress).Contains($sentinel)) { throw 'partial projection leaked raw output' }
        } finally { $finishedJob.Dispose() }
        Test-CleanupRestoreFixtures $root $job
        Test-NoStartBoundaries $root
        'Windows diagnostic SelfTest PASS'
    } finally {
        if ($null -ne $job) { $job.Dispose() }
        [IO.Directory]::Delete($root,$true)
    }
}

function Invoke-Diagnostic {
    $setupWatch=[Diagnostics.Stopwatch]::StartNew()
    $profilePath = Join-Path $PSScriptRoot 'profile.json'
    $profileBytes = [IO.File]::ReadAllBytes($profilePath)
    $profile = [Text.Encoding]::UTF8.GetString($profileBytes) | ConvertFrom-Json
    $resultDirectory = $env:GWT_DIAG_RESULT_DIR
    [void][IO.Directory]::CreateDirectory($resultDirectory)
    $private = Join-Path $env:RUNNER_TEMP ('gwt-fixed-diagnostic-'+[Guid]::NewGuid().ToString())
    [void][IO.Directory]::CreateDirectory($private)
    $jobs = [Collections.Generic.List[object]]::new()
    $original = $null; $target = $null; $modified = $false; $git=$null; $source=$null
    $result = [ordered]@{
        schema_version=1; request_id=$null; profile_id='windows-4777-restore-f351-v1';
        profile_digest=(Get-BytesHash $profileBytes); repository='akiojin/gwt';
        workflow_commit=$null; run_id=$null; run_attempt=$null;
        source_commit=$null; source_tree=$null; source_lock_sha256=$null; overlay_tree=$null;
        observation_started=$false; phase='setup'; reason='SetupFailed'; cleanup='Unobserved';
        restore=@{before_bytes=$null;before_sha256=$null;after_bytes=$null;after_sha256=$null};
        cases=@(); binaries=@(); image=@{os='windows';arch=$null;rust_version=$null;node_major=$null;node_version=$null;runner_label='windows-latest';image_version=$null;os_build=$null};
        no_start_verified=$false; diagnostic_only=$true; ready_evidence=$false
    }
    foreach ($provider in @('codex','claude')) {
        foreach ($case in @('official_launch','restored_close')) {
            $result.cases += [ordered]@{provider=$provider;case=$case;outcome='not_run';exit_code=$null}
        }
    }
    try {
        $id = [Guid]::Empty
        if (-not [Guid]::TryParseExact($env:GWT_DIAG_REQUEST_ID,'D',[ref]$id) -or $id.ToString() -cne $env:GWT_DIAG_REQUEST_ID) { $result.reason='InputMismatch'; $result.no_start_verified=$true; $result.cleanup='NotStarted'; return }
        $result.request_id=$id.ToString()
        if ($env:GITHUB_SHA -cmatch '^[0-9a-f]{40}$') { $result.workflow_commit=$env:GITHUB_SHA }
        if ($env:GITHUB_RUN_ID -match '^[1-9][0-9]{0,18}$') { $result.run_id=[long]$env:GITHUB_RUN_ID }
        if ($env:GITHUB_RUN_ATTEMPT -match '^[1-9][0-9]{0,8}$') { $result.run_attempt=[int]$env:GITHUB_RUN_ATTEMPT }
        if ($env:GITHUB_REPOSITORY -cne 'akiojin/gwt' -or $result.run_attempt -ne 1 -or $null -eq $result.run_id -or $null -eq $result.workflow_commit) { $result.reason='InputMismatch'; $result.no_start_verified=$true; $result.cleanup='NotStarted'; return }
        if ($env:GWT_DIAG_EXPECTED_WORKFLOW -cne $result.workflow_commit) { $result.reason='RefDrift'; $result.no_start_verified=$true; $result.cleanup='NotStarted'; return }
        if ($env:GWT_DIAG_EXPECTED_PROFILE -cne $result.profile_digest) { $result.reason='InputMismatch'; $result.no_start_verified=$true; $result.cleanup='NotStarted'; return }
        $source = [IO.Path]::GetFullPath($env:GWT_DIAG_SOURCE_ROOT)
        $git = (Get-Command git.exe -CommandType Application).Source
        $head = Read-SetupCommand $git @('rev-parse','HEAD') $source (Join-Path $private 'head.raw') $setupWatch
        $tree = Read-SetupCommand $git @('rev-parse','HEAD^{tree}') $source (Join-Path $private 'tree.raw') $setupWatch
        $target = Join-Path $source $profile.target_path
        $original = [IO.File]::ReadAllBytes($target)
        $lock = [IO.File]::ReadAllBytes((Join-Path $source 'Cargo.lock'))
        $patchPath = Join-Path $PSScriptRoot 'diagnostic.patch'
        $patch = [IO.File]::ReadAllBytes($patchPath)
        if (-not (Test-FixedSourceInputs $profile $head $tree $original $lock $patch)) { $result.reason='InputMismatch'; $result.no_start_verified=$true; $result.cleanup='NotStarted'; return }
        $result.source_commit=$head; $result.source_tree=$tree; $result.source_lock_sha256=Get-BytesHash $lock
        $result.restore.before_bytes=$original.Length; $result.restore.before_sha256=Get-BytesHash $original
        $rustup = (Get-Command rustup.exe -CommandType Application).Source
        $cargo = Read-SetupCommand $rustup @('which','--toolchain','1.99.1','cargo') $source (Join-Path $private 'cargo-path.raw') $setupWatch
        $rustc = Read-SetupCommand $rustup @('which','--toolchain','1.99.1','rustc') $source (Join-Path $private 'rustc-path.raw') $setupWatch
        $rustVersion = Read-SetupCommand $rustc @('-vV') $source (Join-Path $private 'rust-version.raw') $setupWatch
        $node = (Get-Command node.exe -CommandType Application).Source
        $nodeVersion = Read-SetupCommand $node @('--version') $source (Join-Path $private 'node-version.raw') $setupWatch
        if ($rustVersion -notmatch '^rustc 1\.99\.1 ' -or $nodeVersion -cnotmatch '^v22\.[0-9]+\.[0-9]+$') { $result.reason='SetupNotStarted'; $result.no_start_verified=$true; $result.cleanup='NotStarted'; return }
        $result.image.rust_version='1.99.1'; $result.image.node_major=22; $result.image.node_version=$nodeVersion.Substring(1)
        $architecture = [Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
        if ($architecture -eq 'X64') { $result.image.arch='x86_64' }
        elseif ($architecture -eq 'Arm64') { $result.image.arch='aarch64' }
        else { $result.reason='SetupNotStarted'; $result.no_start_verified=$true; $result.cleanup='NotStarted'; return }
        $hostMatch=[Regex]::Matches($rustVersion,'(?m)^host: ((?:x86_64|aarch64)-pc-windows-msvc)\r?$')
        if ($hostMatch.Count -ne 1) { $result.reason='SetupNotStarted'; $result.no_start_verified=$true; $result.cleanup='NotStarted'; return }
        $targetTriple=$hostMatch[0].Groups[1].Value
        $result.image.os_build=[Environment]::OSVersion.Version.Build
        if ($env:ImageVersion -match '^[0-9]+(\.[0-9]+){1,5}$' -and $env:ImageVersion.Length -le 64) { $result.image.image_version=$env:ImageVersion }
        Initialize-OwnedLauncher
        $environment = New-ChildEnvironment $private
        $environment['PATH'] = (Split-Path $cargo)+';'+$environment['PATH']
        [void](Read-SetupCommand $git @('apply','--check',$patchPath) $source (Join-Path $private 'patch-check.raw') $setupWatch)
        $modified=$true
        [void](Read-SetupCommand $git @('apply',$patchPath) $source (Join-Path $private 'patch.raw') $setupWatch)
        $patched=[IO.File]::ReadAllBytes($target)
        if ($patched.Length -ne $profile.after.bytes -or (Get-BytesHash $patched) -cne $profile.after.sha256) { throw 'fixed patch mismatch' }
        # The scratch checkout is wholly owned. Its temporary index records the
        # exact overlay tree while HEAD remains the historical source commit.
        [void](Read-SetupCommand $git @('add','--',$profile.target_path) $source (Join-Path $private 'index.raw') $setupWatch)
        $overlay=Read-SetupCommand $git @('write-tree') $source (Join-Path $private 'overlay.raw') $setupWatch
        if ($overlay -cnotmatch '^[0-9a-f]{40}$') { throw 'overlay unavailable' }
        $result.overlay_tree=$overlay
        if ($setupWatch.ElapsedMilliseconds -ge 300000) { $result.reason='WrapperTimeout'; return }
        # Monotonic marker is durably committed BEFORE the first source command.
        $result.observation_started=$true
        Write-DurableJson (Join-Path $private 'observation-started.json') @{observation_started=$true}
        $result.phase='compile'
        $compile=[GwtFixedDiagnostic.OwnedJob]::new($cargo,@('test','-p','gwt','--test','windows_agent_launch_e2e','--no-run'),$source,$environment,(Join-Path $private 'compile.raw'))
        $jobs.Add($compile)
        if (-not (Wait-OwnedJobs @($compile) 30)) { $result.reason='WrapperTimeout'; return }
        if ($compile.ExitCode -ne 0) { $result.reason='CompileFailed'; return }
        foreach ($kind in @('gwt','gwtd','test')) {
            if ($kind -eq 'test') {
                $files=@([IO.Directory]::GetFiles((Join-Path $private 'target/debug/deps'),'windows_agent_launch_e2e-*.exe'))
                if ($files.Count -ne 1) { throw 'test binary unavailable' }
                $binary=$files[0]
            } else { $binary=Join-Path $private ('target/debug/'+$kind+'.exe') }
            $result.binaries += [ordered]@{kind=$kind;sha256=(Get-BytesHash ([IO.File]::ReadAllBytes($binary)));source_commit=$head;source_tree=$tree;overlay_tree=$overlay;target=$targetTriple;profile='test'}
        }
        $result.phase='contracts'
        $contracts=[GwtFixedDiagnostic.OwnedJob]::new($cargo,@('test','-p','gwt','--test','windows_agent_launch_e2e'),$source,$environment,(Join-Path $private 'contracts.raw'))
        $jobs.Add($contracts)
        if (-not (Wait-OwnedJobs @($contracts) 5)) { $result.reason='WrapperTimeout'; return }
        if ($contracts.ExitCode -ne 0) { $result.reason='ContractsFailed'; return }
        if ([IO.File]::ReadAllText((Join-Path $private 'contracts.raw')) -notmatch '(?m)^test result: ok\. 1 passed; 0 failed; 2 ignored;') { $result.reason='ContractsFailed'; return }
        $result.phase='providers'
        $providers=@()
        foreach ($provider in @('codex','claude')) {
            $providerEnvironment=[Collections.Generic.Dictionary[string,string]]::new($environment,[StringComparer]::OrdinalIgnoreCase)
            $providerEnvironment['GWT_WINDOWS_AGENT_E2E_CREDENTIAL_FREE']='1'
            $providerEnvironment['GWT_WINDOWS_AGENT_PROVIDER']=$provider
            $raw=Join-Path $private ($provider+'.raw')
            foreach ($case in $result.cases | Where-Object { $_.provider -eq $provider }) { $case.outcome='unknown' }
            $owned=[GwtFixedDiagnostic.OwnedJob]::new($cargo,@('test','-p','gwt','--test','windows_agent_launch_e2e','--','--ignored','--test-threads=1','--nocapture'),$source,$providerEnvironment,$raw)
            $jobs.Add($owned); $providers += @{provider=$provider;job=$owned;raw=$raw}
        }
        # Both start before either is awaited. Each keeps the original libtest
        # filter/order, one thread, zero retries, and original internal budgets.
        Complete-ProviderObservation (Wait-OwnedJobs @($providers | ForEach-Object { $_.job }) 60) $providers $result
    } catch {
        # Exception strings, arbitrary paths and raw stdout never enter public
        # output. A missing measurement stays null; no false/default proves NoStart.
        Set-DiagnosticFailure $result
    } finally {
        Complete-DiagnosticCleanup $result $jobs $target $original $modified $git $source $profile $private
        Write-DurableJson (Join-Path $resultDirectory 'result.json') $result
        # Raw scratch remains runner-local until its ephemeral disposal. Never
        # upload it, copy it to another host, or print it to diagnose this job.
    }
}

try {
    if ($SelfTest) { Test-Wrapper }
    else { Invoke-Diagnostic }
} catch {
    [Console]::Error.WriteLine('fixed diagnostic wrapper failed; outcome is unobserved')
    exit 1
}
