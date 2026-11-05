$ErrorActionPreference = 'Stop'
$tokens = $null
$parseErrors = $null
$runner = Join-Path $PSScriptRoot 'issue-5658-browser-check.ps1'
$ast = [Management.Automation.Language.Parser]::ParseFile($runner, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw $parseErrors[0] }
$assertion = $ast.Find({ param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Assert-HookHealth'
}, $true)
. ([scriptblock]::Create($assertion.Extent.Text))

function New-Doctor {
    @{status='ready';issues=@();preflight=@{schema_version=1;result='proceed';context='current';normal_completion='normal';
        convergence='converged';reasons=@();diagnostics=@(@{id='retained';classification='recovered'})}}
}
Assert-HookHealth (New-Doctor)
$rejected = 0
foreach ($case in @('missing', 'unknown', 'ledger', 'diagnostics-missing', 'unattributed', 'schema-string', 'enum-case')) {
    $doctor = New-Doctor
    switch ($case) {
        'missing' { $doctor.Remove('preflight') }
        'unknown' { $doctor.preflight.result = 'unknown' }
        'ledger' { $doctor.preflight.reasons = @('ledger_unavailable') }
        'diagnostics-missing' { $doctor.preflight.Remove('diagnostics') }
        'unattributed' { $doctor.preflight.diagnostics[0].classification = 'unattributed' }
        'schema-string' { $doctor.preflight.schema_version = '1' }
        'enum-case' { $doctor.preflight.result = 'PROCEED' }
    }
    $failed = $false
    try { Assert-HookHealth $doctor } catch { $failed = $true }
    if (!$failed) { throw "Managed-browser preflight accepted $case evidence" }
    $rejected++
}
Write-Output "Managed-browser preflight: normal accepted; $rejected invalid observations rejected."

$resolver = $ast.Find({ param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Resolve-CheckoutBuildDirectory'
}, $true)
if (!$resolver) { throw 'Checkout build directory resolver is missing.' }
. ([scriptblock]::Create($resolver.Extent.Text))
$fixtureCheckout = [IO.Path]::GetFullPath((Join-Path ([IO.Path]::GetTempPath()) 'gwt-build-checkout'))
if ((Resolve-CheckoutBuildDirectory $fixtureCheckout 'target' 'debug') -ne (Join-Path $fixtureCheckout 'target/debug')) {
    throw 'Default checkout debug directory changed.'
}
$scratch = Join-Path $fixtureCheckout 'target/measurement'
if ((Resolve-CheckoutBuildDirectory $fixtureCheckout $scratch 'release') -ne (Join-Path $scratch 'release')) {
    throw 'Checkout-local scratch target was not selected.'
}
$outside = $false
try { Resolve-CheckoutBuildDirectory $fixtureCheckout '../gwt-build-checkout-other/target' 'debug' | Out-Null } catch { $outside = $true }
if (!$outside) { throw 'Build target escaped the current checkout.' }
Write-Output 'Checkout target: default and local scratch accepted; sibling refused.'

if ($IsWindows) {
    $snapshots = @(& pwsh -NoProfile -File (Join-Path $PSScriptRoot 'issue-5658-cpu.ps1') -TargetProcessId $PID -SampleMilliseconds 1000 |
        ForEach-Object { $_ | ConvertFrom-Json })
    if ($snapshots.Count -ne 2 -or $snapshots[0].phase -ne 'start' -or $snapshots[1].phase -ne 'end') {
        throw 'CPU sampler did not produce its two process snapshots.'
    }
    foreach ($snapshot in $snapshots) {
        if ($snapshot.process_id -ne $PID) { throw 'Snapshot process identity changed.' }
        foreach ($field in @('working_set_bytes', 'private_memory_bytes', 'peak_working_set_bytes')) {
            if ($snapshot.$field -isnot [ValueType] -or $snapshot.$field -le 0) { throw "Missing positive native memory field: $field" }
        }
    }
    Write-Output 'Native memory: both snapshots retain process identity and positive byte fields.'
}
