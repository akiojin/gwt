[CmdletBinding()]
param(
  [ValidateSet("all", "codex", "claude")]
  [string]$Provider = "all",
  [ValidateSet("all", "latest", "exact")]
  [string]$Selector = "all",
  [string]$CodexExactVersion = "",
  [string]$ClaudeExactVersion = "",
  [string]$OutputDirectory = "",
  [ValidateRange(30, 1800)]
  [int]$TurnTimeoutSeconds = 600,
  [Parameter(DontShow = $true)]
  [string]$InternalHookReceiptPath = "",
  [Parameter(DontShow = $true)]
  [ValidateSet("", "codex", "claude")]
  [string]$InternalHookProvider = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-ObjectString {
  param(
    [Parameter(Mandatory = $true)]$InputObject,
    [Parameter(Mandatory = $true)][string[]]$Names
  )

  foreach ($name in $Names) {
    $property = $InputObject.PSObject.Properties[$name]
    if ($null -ne $property -and $property.Value -is [string] -and
        -not [string]::IsNullOrWhiteSpace($property.Value)) {
      return $property.Value
    }
  }
  return $null
}

function Write-InternalSessionStartReceipt {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)][string]$ProviderName
  )

  $inputText = [Console]::In.ReadToEnd()
  if ([string]::IsNullOrWhiteSpace($inputText)) {
    throw "SessionStart hook input was empty."
  }
  $payload = $inputText | ConvertFrom-Json
  $eventName = Get-ObjectString -InputObject $payload -Names @(
    "hook_event_name", "hookEventName", "event_name", "event"
  )
  if ($null -ne $eventName -and $eventName -ne "SessionStart") {
    throw "Expected SessionStart hook input."
  }
  $providerSessionId = Get-ObjectString -InputObject $payload -Names @(
    "session_id", "sessionId", "thread_id", "threadId", "conversation_id", "conversationId"
  )
  if ([string]::IsNullOrWhiteSpace($providerSessionId)) {
    throw "SessionStart hook input did not include a provider Session id."
  }

  # Intentionally retain only non-secret identity fields. Hook payloads can
  # include cwd/config data and must never be copied into verification output.
  $receipt = [ordered]@{
    source_event = "SessionStart"
    provider = $ProviderName
    provider_session_id = $providerSessionId
  }
  $line = ($receipt | ConvertTo-Json -Compress) + [Environment]::NewLine
  [IO.File]::AppendAllText($Path, $line, [Text.UTF8Encoding]::new($false))
}

if (-not [string]::IsNullOrWhiteSpace($InternalHookReceiptPath)) {
  if ([string]::IsNullOrWhiteSpace($InternalHookProvider)) {
    throw "InternalHookProvider is required in hook capture mode."
  }
  Write-InternalSessionStartReceipt `
    -Path $InternalHookReceiptPath `
    -ProviderName $InternalHookProvider
  exit 0
}

if (-not $IsWindows) {
  throw "windows-agent-launch-smoke.ps1 requires Windows and PowerShell 7."
}

function Resolve-ExecutablePath {
  param([Parameter(Mandatory = $true)][string]$FilePath)

  if ([IO.Path]::IsPathFullyQualified($FilePath)) {
    return $FilePath
  }
  $command = Get-Command `
    -Name $FilePath `
    -CommandType Application `
    -ErrorAction Stop |
    Select-Object -First 1
  if ($null -eq $command -or [string]::IsNullOrWhiteSpace($command.Source)) {
    throw "Required application is unavailable: $FilePath"
  }
  return $command.Source
}

function Invoke-CapturedProcess {
  param(
    [Parameter(Mandatory = $true)][string]$FilePath,
    [Parameter(Mandatory = $true)][AllowEmptyString()][string[]]$Arguments,
    [Parameter(Mandatory = $true)][string]$WorkingDirectory,
    [Parameter(Mandatory = $true)][int]$TimeoutSeconds,
    [hashtable]$EnvironmentOverrides = @{},
    [string]$Purpose = "official-provider command"
  )

  $startInfo = [Diagnostics.ProcessStartInfo]::new()
  $startInfo.FileName = Resolve-ExecutablePath -FilePath $FilePath
  $startInfo.WorkingDirectory = $WorkingDirectory
  $startInfo.UseShellExecute = $false
  $startInfo.CreateNoWindow = $true
  $startInfo.RedirectStandardInput = $true
  $startInfo.RedirectStandardOutput = $true
  $startInfo.RedirectStandardError = $true
  foreach ($argument in $Arguments) {
    [void]$startInfo.ArgumentList.Add($argument)
  }
  foreach ($name in $EnvironmentOverrides.Keys) {
    $startInfo.Environment[$name] = $EnvironmentOverrides[$name]
  }

  $process = [Diagnostics.Process]::new()
  $process.StartInfo = $startInfo
  if (-not $process.Start()) {
    throw "Failed to start an official-provider smoke process."
  }
  $process.StandardInput.Close()
  $stdoutTask = $process.StandardOutput.ReadToEndAsync()
  $stderrTask = $process.StandardError.ReadToEndAsync()
  if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
    try {
      $process.Kill($true)
    } catch {
      # Preserve the original timeout as the actionable failure.
    }
    throw "$Purpose timed out after $TimeoutSeconds seconds."
  }
  $stdout = $stdoutTask.GetAwaiter().GetResult()
  $stderr = $stderrTask.GetAwaiter().GetResult()
  $exitCode = $process.ExitCode
  $process.Dispose()
  if ($exitCode -ne 0) {
    # Raw provider output is deliberately not printed: it can contain account
    # metadata. The temporary copy is removed by the outer finally block.
    throw "$Purpose failed with exit code $exitCode; raw output was discarded."
  }
  return [pscustomobject]@{
    Stdout = $stdout
    Stderr = $stderr
  }
}

function Test-SemanticVersion {
  param([Parameter(Mandatory = $true)][string]$Version)

  return $Version -match '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$'
}

function Resolve-LatestExactVersion {
  param(
    [Parameter(Mandatory = $true)][string]$Package,
    [Parameter(Mandatory = $true)][string]$WorkingDirectory
  )

  $result = Invoke-CapturedProcess `
    -FilePath "npm.cmd" `
    -Arguments @("view", "$Package@latest", "version", "--json") `
    -WorkingDirectory $WorkingDirectory `
    -TimeoutSeconds 15 `
    -Purpose "npm metadata lookup"
  $value = $result.Stdout | ConvertFrom-Json
  if ($value -isnot [string] -or -not (Test-SemanticVersion -Version $value)) {
    throw "npm metadata did not resolve $Package@latest to exactly one semantic version."
  }
  return $value
}

function Get-Sha256 {
  param([Parameter(Mandatory = $true)][string]$Value)

  $bytes = [Text.Encoding]::UTF8.GetBytes($Value)
  $hash = [Security.Cryptography.SHA256]::HashData($bytes)
  return [Convert]::ToHexString($hash).ToLowerInvariant()
}

function ConvertFrom-JsonLines {
  param([Parameter(Mandatory = $true)][string]$Text)

  $events = @()
  foreach ($line in ($Text -split "`r?`n")) {
    if ([string]::IsNullOrWhiteSpace($line)) {
      continue
    }
    try {
      $events += ,($line | ConvertFrom-Json)
    } catch {
      throw "Official provider emitted a non-JSON stdout record."
    }
  }
  return $events
}

function Assert-CodexTurn {
  param(
    [Parameter(Mandatory = $true)]$Events,
    [Parameter(Mandatory = $true)][string]$ExpectedMarker,
    [string]$ExpectedSessionId = ""
  )

  $sessionId = $null
  $turnCompleted = $false
  $assistantMarkerObserved = $false
  foreach ($event in $Events) {
    $eventType = Get-ObjectString -InputObject $event -Names @("type")
    if ($eventType -eq "thread.started") {
      $sessionId = Get-ObjectString -InputObject $event -Names @("thread_id", "session_id")
    }
    if ($eventType -eq "turn.completed") {
      $turnCompleted = $true
    }
    $itemProperty = $event.PSObject.Properties["item"]
    if ($null -ne $itemProperty -and $null -ne $itemProperty.Value) {
      $itemType = Get-ObjectString -InputObject $itemProperty.Value -Names @("type")
      if ($itemType -in @("command_execution", "mcp_tool_call", "web_search", "file_change")) {
        throw "Codex used a tool during the no-tools smoke turn."
      }
      if ($itemType -eq "agent_message") {
        $assistantText = Get-ObjectString `
          -InputObject $itemProperty.Value `
          -Names @("text", "content")
        if ($null -ne $assistantText -and
            $assistantText.Contains($ExpectedMarker, [StringComparison]::Ordinal)) {
          $assistantMarkerObserved = $true
        }
      }
    }
  }
  if ([string]::IsNullOrWhiteSpace($sessionId) -or -not $turnCompleted) {
    throw "Codex did not emit an authenticated completed turn with a Session id."
  }
  if (-not [string]::IsNullOrWhiteSpace($ExpectedSessionId) -and $sessionId -ne $ExpectedSessionId) {
    throw "Codex resume did not keep the same provider Session."
  }
  if (-not $assistantMarkerObserved) {
    throw "Codex did not return the expected no-tools marker."
  }
  return $sessionId
}

function Assert-ClaudeTurn {
  param(
    [Parameter(Mandatory = $true)]$Events,
    [Parameter(Mandatory = $true)][string]$ExpectedMarker,
    [string]$ExpectedSessionId = ""
  )

  $sessionId = $null
  $success = $false
  $assistantMarkerObserved = $false
  foreach ($event in $Events) {
    $candidate = Get-ObjectString -InputObject $event -Names @("session_id", "sessionId")
    if (-not [string]::IsNullOrWhiteSpace($candidate)) {
      $sessionId = $candidate
    }
    $eventType = Get-ObjectString -InputObject $event -Names @("type")
    if ($eventType -eq "result") {
      $isErrorProperty = $event.PSObject.Properties["is_error"]
      $success = $null -eq $isErrorProperty -or $isErrorProperty.Value -ne $true
    }
    $messageProperty = $event.PSObject.Properties["message"]
    if ($null -ne $messageProperty -and $null -ne $messageProperty.Value) {
      $contentProperty = $messageProperty.Value.PSObject.Properties["content"]
      if ($null -ne $contentProperty -and $contentProperty.Value -is [array]) {
        foreach ($block in $contentProperty.Value) {
          $blockType = Get-ObjectString -InputObject $block -Names @("type")
          if ($blockType -eq "tool_use") {
            throw "Claude used a tool during the no-tools smoke turn."
          }
          if ($eventType -eq "assistant" -and $blockType -eq "text") {
            $assistantText = Get-ObjectString -InputObject $block -Names @("text")
            if ($null -ne $assistantText -and
                $assistantText.Contains($ExpectedMarker, [StringComparison]::Ordinal)) {
              $assistantMarkerObserved = $true
            }
          }
        }
      }
    }
  }
  if ([string]::IsNullOrWhiteSpace($sessionId) -or -not $success) {
    throw "Claude did not emit an authenticated successful turn with a Session id."
  }
  if (-not [string]::IsNullOrWhiteSpace($ExpectedSessionId) -and $sessionId -ne $ExpectedSessionId) {
    throw "Claude resume did not keep the same provider Session."
  }
  if (-not $assistantMarkerObserved) {
    throw "Claude did not return the expected no-tools marker."
  }
  return $sessionId
}

function Assert-ProviderCredential {
  param([Parameter(Mandatory = $true)][string]$ProviderName)

  $userProfile = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
  if ($ProviderName -eq "codex") {
    $codexHome = if ([string]::IsNullOrWhiteSpace($env:CODEX_HOME)) {
      Join-Path $userProfile ".codex"
    } else {
      $env:CODEX_HOME
    }
    $present = -not [string]::IsNullOrWhiteSpace($env:OPENAI_API_KEY) -or
      -not [string]::IsNullOrWhiteSpace($env:CODEX_API_KEY) -or
      (Test-Path -LiteralPath (Join-Path $codexHome "auth.json"))
    if (-not $present) {
      throw "Missing Codex credential. Run 'codex login' or set OPENAI_API_KEY; authenticated smoke cannot be skipped."
    }
    return
  }

  $claudeHome = Join-Path $userProfile ".claude"
  $present = -not [string]::IsNullOrWhiteSpace($env:ANTHROPIC_API_KEY) -or
    -not [string]::IsNullOrWhiteSpace($env:CLAUDE_CODE_OAUTH_TOKEN) -or
    (Test-Path -LiteralPath (Join-Path $claudeHome ".credentials.json"))
  if (-not $present) {
    throw "Missing Claude credential. Run 'claude auth login' or set ANTHROPIC_API_KEY; authenticated smoke cannot be skipped."
  }
}

function Get-HookCommand {
  param(
    [Parameter(Mandatory = $true)][string]$ScriptPath,
    [Parameter(Mandatory = $true)][string]$ReceiptPath,
    [Parameter(Mandatory = $true)][string]$ProviderName
  )

  $pwshPath = (Get-Command pwsh -ErrorAction Stop).Source
  $quotedPwsh = $pwshPath.Replace("'", "''")
  $quotedScript = $ScriptPath.Replace("'", "''")
  $quotedReceipt = $ReceiptPath.Replace("'", "''")
  # Claude Code and Codex both execute Windows command hooks through the
  # system command processor. Keep the explicit PowerShell wrapper used by the
  # production gwt hook generator, then call this PowerShell 7 script inside it.
  return "powershell.exe -NoProfile -NonInteractive -Command `"& { & '$quotedPwsh' -NoProfile -NonInteractive -ExecutionPolicy Bypass -File '$quotedScript' -InternalHookReceiptPath '$quotedReceipt' -InternalHookProvider '$ProviderName' }`""
}

function Protect-CurrentUserDirectory {
  param([Parameter(Mandatory = $true)][string]$Path)

  $currentUser = [Security.Principal.WindowsIdentity]::GetCurrent().User
  if ($null -eq $currentUser) {
    throw "Unable to resolve the current Windows user SID."
  }
  $acl = [Security.AccessControl.DirectorySecurity]::new()
  $acl.SetOwner($currentUser)
  $acl.SetAccessRuleProtection($true, $false)
  $inheritance = [Security.AccessControl.InheritanceFlags]::ContainerInherit -bor
    [Security.AccessControl.InheritanceFlags]::ObjectInherit
  $rule = [Security.AccessControl.FileSystemAccessRule]::new(
    $currentUser,
    [Security.AccessControl.FileSystemRights]::FullControl,
    $inheritance,
    [Security.AccessControl.PropagationFlags]::None,
    [Security.AccessControl.AccessControlType]::Allow
  )
  [void]$acl.AddAccessRule($rule)
  Set-Acl -LiteralPath $Path -AclObject $acl

  $applied = Get-Acl -LiteralPath $Path
  if (-not $applied.AreAccessRulesProtected) {
    throw "Temporary credential directory still inherits access rules."
  }
  $unexpectedAllowRules = @($applied.Access | Where-Object {
      if ($_.AccessControlType -ne [Security.AccessControl.AccessControlType]::Allow) {
        return $false
      }
      try {
        $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]) -ne $currentUser
      } catch {
        return $true
      }
    })
  if ($unexpectedAllowRules.Count -ne 0) {
    throw "Temporary credential directory grants access outside the current user."
  }
}

function Read-HookReceipts {
  param([Parameter(Mandatory = $true)][string]$Path)

  if (-not (Test-Path -LiteralPath $Path)) {
    return @()
  }
  return @(ConvertFrom-JsonLines -Text (Get-Content -LiteralPath $Path -Raw))
}

function Invoke-OfficialCase {
  param(
    [Parameter(Mandatory = $true)]$Case,
    [Parameter(Mandatory = $true)][string]$RepoRoot,
    [Parameter(Mandatory = $true)][string]$CaseRoot,
    [Parameter(Mandatory = $true)][string]$ScriptPath,
    [Parameter(Mandatory = $true)][int]$TimeoutSeconds
  )

  Assert-ProviderCredential -ProviderName $Case.Provider
  New-Item -ItemType Directory -Force -Path $CaseRoot | Out-Null
  $gitResult = Invoke-CapturedProcess `
    -FilePath "git.exe" `
    -Arguments @("init", "--quiet") `
    -WorkingDirectory $CaseRoot `
    -TimeoutSeconds 30 `
    -Purpose "$($Case.Name) git init"
  $null = $gitResult

  $resolvedVersion = if (-not [string]::IsNullOrWhiteSpace($Case.ExactVersion)) {
    if (-not (Test-SemanticVersion -Version $Case.ExactVersion)) {
      throw "$($Case.Name) exact version is not semantic."
    }
    $Case.ExactVersion
  } else {
    Resolve-LatestExactVersion -Package $Case.Package -WorkingDirectory $CaseRoot
  }
  $requestedSelector = if ($Case.Selector -eq "latest") { "latest" } else { $resolvedVersion }

  $probe = Invoke-CapturedProcess `
    -FilePath "npx.cmd" `
    -Arguments @("--yes", "$($Case.Package)@$resolvedVersion", "--version") `
    -WorkingDirectory $CaseRoot `
    -TimeoutSeconds 120 `
    -Purpose "$($Case.Name) exact package probe"
  if (-not $probe.Stdout.Contains($resolvedVersion, [StringComparison]::Ordinal)) {
    throw "$($Case.Name) exact package probe did not report $resolvedVersion."
  }

  $receiptPath = Join-Path $CaseRoot "session-start-receipts.jsonl"
  $hookCommand = Get-HookCommand `
    -ScriptPath $ScriptPath `
    -ReceiptPath $receiptPath `
    -ProviderName $Case.Provider
  $hookGroup = @(@{
      matcher = "*"
      hooks = @(@{
          type = "command"
          command = $hookCommand
          timeout = 30
        })
    })

  $freshMarker = "GWT_SMOKE_FRESH_OK"
  $resumeMarker = "GWT_SMOKE_RESUME_OK"
  $freshPrompt = "Reply with exactly $freshMarker. Do not call or use tools."
  $resumePrompt = "Reply with exactly $resumeMarker. Do not call or use tools."
  $prefix = @("--yes", "$($Case.Package)@$resolvedVersion")

  if ($Case.Provider -eq "codex") {
    # A fresh project is intentionally untrusted. Current Codex versions do
    # not discover project-local hooks until project trust is granted, and
    # --dangerously-bypass-hook-trust only bypasses per-hook trust. Use a
    # case-local CODEX_HOME so the official smoke neither mutates ambient
    # project trust nor depends on it.
    $userProfile = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
    $ambientCodexHome = if ([string]::IsNullOrWhiteSpace($env:CODEX_HOME)) {
      Join-Path $userProfile ".codex"
    } else {
      $env:CODEX_HOME
    }
    $codexHome = Join-Path $CaseRoot "codex-home"
    New-Item -ItemType Directory -Force -Path $codexHome | Out-Null
    $authSource = Join-Path $ambientCodexHome "auth.json"
    if (Test-Path -LiteralPath $authSource) {
      Copy-Item -LiteralPath $authSource -Destination (Join-Path $codexHome "auth.json")
    }
    @{ hooks = @{ SessionStart = $hookGroup } } |
      ConvertTo-Json -Depth 12 |
      Set-Content -LiteralPath (Join-Path $codexHome "hooks.json") -Encoding utf8NoBOM
    $codexEnvironment = @{ CODEX_HOME = $codexHome }

    $fresh = Invoke-CapturedProcess `
      -FilePath "npx.cmd" `
      -Arguments ($prefix + @(
          "exec", "--enable", "hooks", "--skip-git-repo-check", "--sandbox", "read-only",
          "--ignore-rules", "--dangerously-bypass-hook-trust",
          "--json", $freshPrompt
        )) `
      -WorkingDirectory $CaseRoot `
      -TimeoutSeconds $TimeoutSeconds `
      -EnvironmentOverrides $codexEnvironment `
      -Purpose "$($Case.Name) fresh turn"
    $freshEvents = ConvertFrom-JsonLines -Text $fresh.Stdout
    $sessionId = Assert-CodexTurn -Events $freshEvents -ExpectedMarker $freshMarker

    $resume = Invoke-CapturedProcess `
      -FilePath "npx.cmd" `
      -Arguments ($prefix + @(
          "exec", "--enable", "hooks", "resume", "--skip-git-repo-check", "--ignore-rules",
          "--dangerously-bypass-hook-trust", "--json",
          $sessionId, $resumePrompt
        )) `
      -WorkingDirectory $CaseRoot `
      -TimeoutSeconds $TimeoutSeconds `
      -EnvironmentOverrides $codexEnvironment `
      -Purpose "$($Case.Name) resume turn"
    $resumeEvents = ConvertFrom-JsonLines -Text $resume.Stdout
    $resumeSessionId = Assert-CodexTurn `
      -Events $resumeEvents `
      -ExpectedMarker $resumeMarker `
      -ExpectedSessionId $sessionId
  } else {
    $claudeSettings = Join-Path $CaseRoot "claude-smoke-settings.json"
    @{ hooks = @{ SessionStart = $hookGroup } } |
      ConvertTo-Json -Depth 12 |
      Set-Content -LiteralPath $claudeSettings -Encoding utf8NoBOM
    $claudeBase = @(
      "-p", "--output-format", "stream-json", "--verbose", "--include-hook-events",
      "--tools", "", "--disable-slash-commands", "--strict-mcp-config",
      "--mcp-config", '{"mcpServers":{}}', "--setting-sources", "project",
      "--settings", $claudeSettings, "--permission-mode", "dontAsk"
    )

    $fresh = Invoke-CapturedProcess `
      -FilePath "npx.cmd" `
      -Arguments ($prefix + $claudeBase + @($freshPrompt)) `
      -WorkingDirectory $CaseRoot `
      -TimeoutSeconds $TimeoutSeconds `
      -Purpose "$($Case.Name) fresh turn"
    $freshEvents = ConvertFrom-JsonLines -Text $fresh.Stdout
    $sessionId = Assert-ClaudeTurn -Events $freshEvents -ExpectedMarker $freshMarker

    $resume = Invoke-CapturedProcess `
      -FilePath "npx.cmd" `
      -Arguments ($prefix + $claudeBase + @("--resume", $sessionId, $resumePrompt)) `
      -WorkingDirectory $CaseRoot `
      -TimeoutSeconds $TimeoutSeconds `
      -Purpose "$($Case.Name) resume turn"
    $resumeEvents = ConvertFrom-JsonLines -Text $resume.Stdout
    $resumeSessionId = Assert-ClaudeTurn `
      -Events $resumeEvents `
      -ExpectedMarker $resumeMarker `
      -ExpectedSessionId $sessionId
  }

  if ($resumeSessionId -ne $sessionId) {
    throw "$($Case.Name) did not resume the same provider Session."
  }
  $receipts = @(Read-HookReceipts -Path $receiptPath)
  $matchingReceipts = @($receipts | Where-Object {
      $_.source_event -eq "SessionStart" -and
      $_.provider -eq $Case.Provider -and
      $_.provider_session_id -eq $sessionId
    })
  if ($matchingReceipts.Count -lt 2) {
    throw "$($Case.Name) did not record authenticated SessionStart for fresh and resume (matching=$($matchingReceipts.Count), total=$($receipts.Count))."
  }

  return [ordered]@{
    schema_version = 1
    case = $Case.Name
    provider = $Case.Provider
    official_package = $Case.Package
    requested_selector = $requestedSelector
    resolved_exact_version = $resolvedVersion
    runner_kind = "npx"
    authenticated_provider_identity = $true
    authenticated_session_start = $true
    session_start_receipt_count = $matchingReceipts.Count
    same_provider_session_resume = $true
    fresh_no_tools = $true
    resume_no_tools = $true
    session_fingerprint_sha256 = Get-Sha256 -Value $sessionId
  }
}

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$scriptPath = $MyInvocation.MyCommand.Path
$checkoutGwt = Join-Path $repoRoot "target/debug/gwt.exe"
$checkoutGwtd = Join-Path $repoRoot "target/debug/gwtd.exe"
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
  $OutputDirectory = Join-Path $repoRoot "target/verification/windows-agent-launch-smoke"
}

foreach ($requiredCommand in @("cargo", "git.exe", "npm.cmd", "npx.cmd", "powershell.exe", "pwsh")) {
  if ($null -eq (Get-Command $requiredCommand -ErrorAction SilentlyContinue)) {
    throw "Required command is unavailable: $requiredCommand"
  }
}

# Always materialize this checkout's binaries; never use a production GWT.app,
# installed gwt.exe, GWT_BIN_PATH, or another already-running instance.
$build = Invoke-CapturedProcess `
  -FilePath "cargo" `
  -Arguments @("build", "-p", "gwt", "--bin", "gwt", "--bin", "gwtd") `
  -WorkingDirectory $repoRoot `
  -TimeoutSeconds 1200 `
  -Purpose "checkout gwt/gwtd build"
$null = $build
foreach ($checkoutBinary in @($checkoutGwt, $checkoutGwtd)) {
  if (-not (Test-Path -LiteralPath $checkoutBinary)) {
    throw "Checkout build did not produce $checkoutBinary"
  }
}

$caseMatrix = @(
  [pscustomobject]@{
    Name = "codex/latest"; Provider = "codex"; Selector = "latest"
    Package = "@openai/codex"; ExactVersion = ""
  },
  [pscustomobject]@{
    Name = "codex/exact"; Provider = "codex"; Selector = "exact"
    Package = "@openai/codex"; ExactVersion = $CodexExactVersion
  },
  [pscustomobject]@{
    Name = "claude/latest"; Provider = "claude"; Selector = "latest"
    Package = "@anthropic-ai/claude-code"; ExactVersion = ""
  },
  [pscustomobject]@{
    Name = "claude/exact"; Provider = "claude"; Selector = "exact"
    Package = "@anthropic-ai/claude-code"; ExactVersion = $ClaudeExactVersion
  }
)
$selectedCases = @($caseMatrix | Where-Object {
    ($Provider -eq "all" -or $_.Provider -eq $Provider) -and
    ($Selector -eq "all" -or $_.Selector -eq $Selector)
  })
if ($selectedCases.Count -eq 0) {
  throw "No official-provider smoke cases were selected."
}

$temporaryRoot = Join-Path ([IO.Path]::GetTempPath()) ("gwt-windows-agent-smoke-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force -Path $temporaryRoot | Out-Null
Protect-CurrentUserDirectory -Path $temporaryRoot
try {
  $results = @()
  foreach ($case in $selectedCases) {
    Write-Host "Running authenticated official-provider smoke: $($case.Name)"
    $caseRoot = Join-Path $temporaryRoot ($case.Name.Replace("/", "-"))
    $results += ,(Invoke-OfficialCase `
        -Case $case `
        -RepoRoot $repoRoot `
        -CaseRoot $caseRoot `
        -ScriptPath $scriptPath `
        -TimeoutSeconds $TurnTimeoutSeconds)
  }

  New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
  $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
  $evidencePath = Join-Path $OutputDirectory "official-provider-smoke-$stamp.json"
  [ordered]@{
    schema_version = 1
    generated_at_utc = (Get-Date).ToUniversalTime().ToString("o")
    checkout_build = $true
    cases = $results
  } |
    ConvertTo-Json -Depth 12 |
    Set-Content -LiteralPath $evidencePath -Encoding utf8NoBOM
  Write-Host "Authenticated official-provider smoke PASS: $evidencePath"
} finally {
  # Provider JSON streams and hook receipts never survive the run. Only the
  # curated evidence above is retained, with Session ids reduced to SHA-256.
  if (Test-Path -LiteralPath $temporaryRoot) {
    Remove-Item -LiteralPath $temporaryRoot -Recurse -Force
  }
}
