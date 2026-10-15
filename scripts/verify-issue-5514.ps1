$ErrorActionPreference = 'Stop'

# Normal nextest runs fail on a failing test; stress mode can exit zero even
# when its JUnit report contains failures. Check every iteration's exit code.
$filter = 'test(pm_board_context_succeeds_for_one_hundred_prompts_with_eight_writers) | test(issue_5404_unchanged_projection_inputs_skip_rebuild_and_changed_work_refreshes)'
for ($iteration = 1; $iteration -le 20; $iteration++) {
    Write-Host "Issue #5514 acceptance iteration $iteration/20"
    cargo nextest run -p gwt --lib --bin gwt --profile gwt-verify --retries 0 -E $filter
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

Write-Host 'Issue #5514 acceptance: 20/20 iterations, 40 passed, 0 failed'
