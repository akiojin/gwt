param(
    [switch]$Force,
    [string]$SpecId,
    [string]$Agent
)

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$bashScript = Join-Path $scriptDir "../bash/update-agent-context.sh"

$bash = Get-Command bash -ErrorAction SilentlyContinue
if (-not $bash) {
    Write-Error "bash が見つかりません。bash スクリプトの実行が必要です。"
    exit 1
}

$argsList = @()
if ($Force) { $argsList += "--force" }
if ($SpecId) {
    $argsList += "--spec-id"
    $argsList += $SpecId
}
if ($Agent) { $argsList += $Agent }

& bash $bashScript @argsList
