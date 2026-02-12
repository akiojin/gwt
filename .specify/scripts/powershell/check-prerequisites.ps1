param(
    [switch]$Json,
    [switch]$RequireTasks,
    [switch]$IncludeTasks,
    [string]$SpecId
)

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$bashScript = Join-Path $scriptDir "../bash/check-prerequisites.sh"

$bash = Get-Command bash -ErrorAction SilentlyContinue
if (-not $bash) {
    Write-Error "bash が見つかりません。bash スクリプトの実行が必要です。"
    exit 1
}

$argsList = @()
if ($Json) { $argsList += "--json" }
if ($RequireTasks) { $argsList += "--require-tasks" }
if ($IncludeTasks) { $argsList += "--include-tasks" }
if ($SpecId) {
    $argsList += "--spec-id"
    $argsList += $SpecId
}

& bash $bashScript @argsList
