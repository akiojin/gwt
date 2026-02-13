param(
    [switch]$Json,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$Args
)

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$bashScript = Join-Path $scriptDir "../bash/create-new-feature.sh"

$bash = Get-Command bash -ErrorAction SilentlyContinue
if (-not $bash) {
    Write-Error "bash が見つかりません。bash スクリプトの実行が必要です。"
    exit 1
}

$argsList = @()
if ($Json) { $argsList += "--json" }

if (-not $Args -or $Args.Count -eq 0) {
    Write-Error "機能説明が空です"
    exit 1
}

$argsList += $Args

& bash $bashScript @argsList
