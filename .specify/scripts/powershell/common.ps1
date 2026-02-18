function Get-RepoRoot {
    try {
        $root = git rev-parse --show-toplevel 2>$null
        if ($LASTEXITCODE -eq 0 -and $root) {
            return $root.Trim()
        }
    } catch {}

    $dir = Split-Path -Parent $MyInvocation.MyCommand.Path
    while ($dir -and $dir -ne [System.IO.Path]::GetPathRoot($dir)) {
        if (Test-Path (Join-Path $dir ".git") -or Test-Path (Join-Path $dir ".specify")) {
            return $dir
        }
        $dir = Split-Path -Parent $dir
    }
    return $null
}

function Normalize-SpecId([string]$SpecId) {
    if (-not $SpecId) { return $null }
    $upper = $SpecId.ToUpper()
    if ($upper.StartsWith("SPEC-")) {
        return "SPEC-$($upper.Substring(5).ToLower())"
    }
    return "SPEC-$($upper.ToLower())"
}

function Test-SpecId([string]$SpecId) {
    return $SpecId -match '^SPEC-[a-f0-9]{8}$'
}
