param(
    [switch]$Json,
    [switch]$Force,
    [string]$SpecId
)

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$bashScript = Join-Path $scriptDir "../bash/setup-plan.sh"

$bash = Get-Command bash -ErrorAction SilentlyContinue
if (-not $bash) {
    Write-Error "bash が見つかりません。bash スクリプトの実行が必要です。"
    exit 1
}

$argsList = @()
if ($Json) { $argsList += "--json" }
if ($Force) { $argsList += "--force" }
if ($SpecId) {
    $argsList += "--spec-id"
    $argsList += $SpecId
}

& bash $bashScript @argsList
