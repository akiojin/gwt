[CmdletBinding(PositionalBinding = $false)]
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$PlaywrightArguments)

$ErrorActionPreference = 'Stop'
$checkout = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$version = (Get-Content (Join-Path $checkout 'scripts/playwright-version.txt') -Raw).Trim()
$deps = Join-Path $env:LOCALAPPDATA "Temp/gwt-playwright-win-$version"
$hookHome = Join-Path ([IO.Path]::GetTempPath()) ('gwt-5542-hook-' + [Guid]::NewGuid().ToString('N'))
$saved = @{}
try {
    foreach ($binary in @('gwt.exe', 'gwtd.exe')) {
        if (!(Test-Path -LiteralPath "$checkout/target/debug/$binary")) { throw "Build checkout $binary first" }
    }
    New-Item -ItemType Directory -Path $hookHome | Out-Null
    # Keep the current checkout's hook image stable through the owned fixture
    # lifetime, outside disposable target paths. The existing fixture audits it.
    Copy-Item -LiteralPath "$checkout/target/debug/gwtd.exe" -Destination "$hookHome/gwtd.exe"
    $variables = @{
        GWT_PLAYWRIGHT_CHECKOUT_ROOT=$checkout; GWT_PLAYWRIGHT_ISSUE_6842='1';
        GWT_HOOK_BIN="$hookHome/gwtd.exe"; NODE_PATH="$deps/node_modules";
        NODE_OPTIONS="--require $($deps.Replace('\','/'))/resolve-playwright.cjs"
    }
    foreach ($key in $variables.Keys) {
        $saved[$key] = [Environment]::GetEnvironmentVariable($key)
        [Environment]::SetEnvironmentVariable($key, $variables[$key])
    }
    # The existing fixture launches only this checkout's gwt, with a fresh HOME
    # and a throwaway repository/provider, and joins cleanup on every exit.
    & node "$deps/node_modules/@playwright/test/cli.js" test --config "$checkout/crates/gwt/playwright/playwright.config.ts" --workers=1 --retries=0 issue-6842-launch-recovery-live.spec.ts --grep 'Issue #5390 launch' @PlaywrightArguments
    if ($LASTEXITCODE -ne 0) { throw "Headed ancestry/Monitor regression failed: $LASTEXITCODE" }
} finally {
    foreach ($key in $saved.Keys) { [Environment]::SetEnvironmentVariable($key, $saved[$key]) }
    # Keep the exact verified hook image as local provenance with the fixture.
    Write-Output "Checkout hook image retained at $hookHome"
}
