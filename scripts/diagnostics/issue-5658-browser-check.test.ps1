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
