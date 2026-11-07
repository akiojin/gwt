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

$sourceAudit = $ast.Find({ param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Invoke-SourceHookAudit'
}, $true)
if (!$sourceAudit) { throw 'Source hook audit rendezvous is missing.' }
. ([scriptblock]::Create($sourceAudit.Extent.Text))
$SourceAuditDirectory = Join-Path ([IO.Path]::GetTempPath()) ('gwt-source-audit-test-' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $SourceAuditDirectory
$broker = Start-Job -ArgumentList $SourceAuditDirectory -ScriptBlock {
    param($directory)
    for ($index = 0; $index -lt 2; $index++) {
        do {
            $request = Get-ChildItem -LiteralPath $directory -Filter '*.request.json' |
                Where-Object { !(Test-Path -LiteralPath ($_.FullName -replace '\.request\.json$', '.response.json')) } | Select-Object -First 1
            if (!$request) { Start-Sleep -Milliseconds 100 }
        } while (!$request)
        $message = Get-Content -LiteralPath $request.FullName -Raw | ConvertFrom-Json
        $valid = $message.operation -eq 'hook.doctor' -and $message.params.purpose -eq 'managed_browser'
        $responsePath = $request.FullName -replace '\.request\.json$', '.response.json'
        @{ok=($valid -and $index -eq 0);output=if ($index -eq 0) { 'source caller accepted' } else { 'source audit refused' }} |
            ConvertTo-Json | Set-Content -LiteralPath "$responsePath.tmp" -Encoding utf8NoBOM
        [IO.File]::Move("$responsePath.tmp", $responsePath)
    }
}
try {
    if ((Invoke-SourceHookAudit @{purpose='managed_browser'}).output -ne 'source caller accepted') { throw 'Source audit response was lost.' }
    $refused = $false
    try { Invoke-SourceHookAudit @{purpose='managed_browser'} | Out-Null } catch { $refused = $_.Exception.Message -like '*source audit refused*' }
    if (!$refused) { throw 'Source audit refusal was not propagated.' }
    $null = Wait-Job $broker
    Receive-Job $broker -ErrorAction Stop | Out-Null
} finally { Stop-Job $broker; Remove-Job $broker -Force }
Write-Output 'Source hook audit: request/response preserved; refusal propagated.'

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
