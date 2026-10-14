# Issue #4415: frozen-base / checkout headed startup measurements. Build gwt/gwtd first.
param(
  [string]$ControlSha = '7b3c545a48db35f27e52cb5b64bf0985f1791955',
  [Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArguments
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This fixture requires Windows and PowerShell 7.' }
$repoRoot = (& git rev-parse --show-toplevel).Trim()
foreach ($relative in @('target/startup-common-dir-control.exe', 'target/debug/gwt.exe', 'target/debug/gwtd.exe')) {
  if (-not (Test-Path -LiteralPath (Join-Path $repoRoot $relative) -PathType Leaf)) {
    throw 'Build the checkout gwt/gwtd and preserve the control gwt.exe before running this fixture.'
  }
}
$version = [IO.File]::ReadAllText((Join-Path $repoRoot 'scripts/playwright-version.txt')).Trim()
$deps = if ($env:GWT_PLAYWRIGHT_DEPS_DIR) { $env:GWT_PLAYWRIGHT_DEPS_DIR } else { Join-Path ([IO.Path]::GetTempPath()) "gwt-playwright-$version" }
$cli = Join-Path $deps 'node_modules/playwright/cli.js'
$resolver = Join-Path $deps 'resolve-playwright.cjs'
if (-not (Test-Path -LiteralPath $cli) -or -not (Test-Path -LiteralPath $resolver)) {
  throw "Pinned Playwright $version is missing; run scripts/run-visual-tests.sh --list once."
}
$node = (Get-Command node -CommandType Application).Source
$runnerEnvironment = @{
  GWT_PLAYWRIGHT_STARTUP_COMMON_DIR='1'; GWT_PLAYWRIGHT_CHECKOUT_ROOT=$repoRoot;
  GWT_COMMON_DIR_CONTROL_SHA=$ControlSha; GWT_COMMON_DIR_TREATMENT_SHA=(& git rev-parse HEAD).Trim();
  GWT_PLAYWRIGHT_NODE_MODULES=(Join-Path $deps 'node_modules');
  NODE_OPTIONS="--require `"$($resolver.Replace('\', '/'))`" $env:NODE_OPTIONS"
}
$savedEnvironment = @{}
foreach ($key in $runnerEnvironment.Keys) {
  $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key)
  [Environment]::SetEnvironmentVariable($key, $runnerEnvironment[$key])
}
$artifacts = Join-Path $repoRoot ('crates/gwt/playwright/test-results/startup-common-dir-' + [guid]::NewGuid())
try {
  Push-Location $repoRoot
  try {
    # Forward canonical verify.run's --headed / reporter arguments unchanged.
    & $node $cli test --config crates/gwt/playwright/playwright.config.ts --workers=1 --output $artifacts crates/gwt/playwright/tests/startup-common-dir.spec.ts @PlaywrightArguments
    $exitCode = $LASTEXITCODE
  } finally { Pop-Location }
} finally {
  foreach ($key in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key]) }
}
Write-Output "Startup comparison evidence: $artifacts"
exit $exitCode
