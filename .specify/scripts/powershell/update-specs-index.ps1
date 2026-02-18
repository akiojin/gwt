$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$bashScript = Join-Path $scriptDir "../bash/update-specs-index.sh"

$bash = Get-Command bash -ErrorAction SilentlyContinue
if (-not $bash) {
    Write-Error "bash が見つかりません。bash スクリプトの実行が必要です。"
    exit 1
}

& bash $bashScript
