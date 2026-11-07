$ErrorActionPreference = 'Stop'
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot 'issue-6809-browser-check.ps1'), [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw 'Browser check script did not parse.' }
$client = $ast.Find({ param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Invoke-SourceHookAudit'
}, $false)
if (!$client) { throw 'Source hook audit rendezvous is missing.' }
. ([ScriptBlock]::Create($client.Extent.Text))
$SourceAuditDirectory = Join-Path ([IO.Path]::GetTempPath()) ('gwt-scan-audit-test-' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $SourceAuditDirectory
$broker = Start-Job -ArgumentList $SourceAuditDirectory -ScriptBlock {
    param($directory)
    for ($index = 0; $index -lt 2; $index++) {
        $deadline = [DateTime]::UtcNow.AddSeconds(30)
        do {
            $request = Get-ChildItem -LiteralPath $directory -Filter '*.request.json' |
                Where-Object { !(Test-Path -LiteralPath ($_.FullName -replace '\.request\.json$', '.response.json')) } | Select-Object -First 1
            if ([DateTime]::UtcNow -ge $deadline) { throw 'Audit test request timed out.' }
            if (!$request) { Start-Sleep -Milliseconds 100 }
        } while (!$request)
        $message = Get-Content -LiteralPath $request.FullName -Raw | ConvertFrom-Json
        if ($message.operation -cne 'hook.doctor' -or $message.params.purpose -cne 'managed_browser') { throw 'Audit request changed.' }
        $answer = @{ok=($index -eq 0);output=if ($index -eq 0) { 'fresh caller observation' } else { 'source audit refused' }}
        $response = $request.FullName -replace '\.request\.json$', '.response.json'
        [IO.File]::WriteAllText("$response.tmp", ($answer | ConvertTo-Json))
        [IO.File]::Move("$response.tmp", $response)
    }
}
try {
    $answer = Invoke-SourceHookAudit @{purpose='managed_browser'}
    if ($answer.output -cne 'fresh caller observation') { throw 'Audit response changed.' }
    $refused = $false
    try { Invoke-SourceHookAudit @{purpose='managed_browser'} | Out-Null }
    catch { $refused = $_.Exception.Message -like '*source audit refused*' }
    if (!$refused) { throw 'Source audit refusal was ignored.' }
    $null = Wait-Job $broker
    Receive-Job $broker -ErrorAction Stop | Out-Null
} finally { Stop-Job $broker; Remove-Job $broker -Force }
Write-Output 'PASS: fresh source response and refusal propagation.'
