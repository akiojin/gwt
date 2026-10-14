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
# Keep the observer outside tracked source. Both arms use this same proxy;
# Rust preserves argv/cwd/env/stdio without shell parsing or another crate.
$realGit = (Get-Command git -CommandType Application | Select-Object -First 1).Source
$proxyDir = Join-Path $repoRoot 'target/startup-common-dir-observer'
$proxyBinary = Join-Path $proxyDir 'git.exe'
if ($realGit -eq $proxyBinary) { throw 'The observer must resolve the real Git executable before changing PATH.' }
[void](New-Item -ItemType Directory -Path $proxyDir -Force)
$proxySource = @'
use std::{env, fs::OpenOptions, io::{self, Write}, process::Command, time::Instant};
use std::os::windows::process::CommandExt;

fn main() {
    let real = env::var_os("GWT_REAL_GIT").expect("Git observer requires GWT_REAL_GIT");
    let args: Vec<_> = env::args_os().skip(1).collect();
    let observed = args.iter().any(|arg| arg == "rev-parse")
        && args.iter().any(|arg| arg == "--git-common-dir");
    let started = Instant::now();
    let status = Command::new(real).args(&args).creation_flags(0x08000000).status();
    let duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    if observed {
        let write_receipt = || -> io::Result<()> {
            let path = env::var_os("GWT_COMMON_DIR_RECORD_FILE")
                .ok_or_else(|| io::Error::other("Git observer requires a census file"))?;
            let success = status.as_ref().is_ok_and(|status| status.success());
            let receipt = format!(
                "{{\"success\":{},\"count\":1,\"real_git_duration_ms\":{:.3}}}\n",
                success, duration_ms);
            // One append write keeps concurrent command receipts together.
            let mut file = OpenOptions::new().append(true).open(path)?;
            file.write_all(receipt.as_bytes())?;
            file.sync_data()
        };
        if write_receipt().is_err() {
            eprintln!("Git observer could not write its census receipt");
            std::process::exit(125);
        }
    }
    std::process::exit(status.ok().and_then(|status| status.code()).unwrap_or(125));
}
'@
$proxySourcePath = Join-Path $proxyDir 'git-observer.rs'
[IO.File]::WriteAllText($proxySourcePath, $proxySource)
$rustc = (Get-Command rustc -CommandType Application).Source
& $rustc --edition=2021 -C opt-level=1 $proxySourcePath -o $proxyBinary
if ($LASTEXITCODE -ne 0) { throw 'Standalone Git observer compilation failed.' }

$runnerEnvironment = @{
  GWT_PLAYWRIGHT_STARTUP_COMMON_DIR='1'; GWT_PLAYWRIGHT_CHECKOUT_ROOT=$repoRoot;
  GWT_COMMON_DIR_CONTROL_SHA=$ControlSha; GWT_COMMON_DIR_TREATMENT_SHA=(& git rev-parse HEAD).Trim();
  GWT_PLAYWRIGHT_NODE_MODULES=(Join-Path $deps 'node_modules');
  GWT_COMMON_DIR_PROXY_DIR=$proxyDir; GWT_COMMON_DIR_REAL_GIT=$realGit;
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
