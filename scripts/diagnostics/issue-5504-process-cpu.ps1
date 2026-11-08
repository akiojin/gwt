param([Parameter(Mandatory = $true)][int]$ProcessId)
$ErrorActionPreference = 'Stop'
$process = Get-Process -Id $ProcessId
@{ pid=$process.Id; process_name=$process.ProcessName;
    cpu_seconds=$process.TotalProcessorTime.TotalSeconds;
    captured_at=[DateTime]::UtcNow.ToString('o') } | ConvertTo-Json -Compress
