### 既存テスト（FR-001〜FR-047 カバー）

#### `AgentLaunchForm.test.ts`（28テスト）

| # | テストケース | 関連 FR |
|---|------------|---------|
| 1 | keeps selectedAgent empty when all agents are unavailable | FR-001 |
| 2 | shows only agent names in the agent dropdown | FR-001 |
| 3 | shows fallback hint only in Agent Version field | FR-003 |
| 4 | displays codex model options including gpt-5.4 | FR-002 |
| 5 | displays claude model options with 1M context and opusplan variants | FR-002 |
| 6 | displays only supported copilot model options | FR-002 |
| 7 | passes selected codex model to launch request | FR-002, FR-036 |
| 8 | shows Fast mode only when codex gpt-5.4 is selected | FR-005 |
| 9 | passes enabled Fast mode for codex gpt-5.4 launches | FR-005 |
| 10 | disables capitalization and completion helpers for text and textarea inputs | UI |
| 11 | forces host launch even when docker context is not detected | FR-029 |
| 12 | defers gh CLI check until osEnvReady is true | FR-023 |
| 13 | shows gh missing message only after osEnvReady | FR-023 |
| 14 | keeps issue selection disabled while duplicate-branch check is pending | FR-019 |
| 15 | keeps issue selection disabled when duplicate-branch check fails | FR-019 |
| 16 | does not auto-load issue list when opened with prefillIssue | FR-042 |
| 17 | keeps Launch disabled in fromIssue mode until a prefix is selected | FR-018 |
| 18 | does not link or rollback issue branch before async launch job completion | FR-045 |
| 19 | uses previous successful launch options as next defaults | FR-009, FR-010 |
| 20 | does not update defaults when closed without launching | FR-010 |
| 21 | does not update defaults when launch fails | FR-010 |
| 22 | keeps installed selection when preferred agent stays the same | FR-011, FR-012 |
| 23 | falls back when saved defaults contain unavailable agent or invalid runtime/version | FR-012 |
| 24 | does not persist new-branch input fields into next defaults | FR-009 |
| 25 | loads base branches and allows direct branch name entry | FR-014, FR-016 |
| 26 | shows gh unauthenticated warning when gh exists but auth is missing | FR-017 |
| 27 | renders issue labels, branch-exists state, search filter, and infinite scroll paging | FR-017, FR-019 |
| 28 | filters from-issue list by number tokens and mixed AND query | FR-017 |
| 29 | shows GitHub API rate-limit error on issue fetch failure | FR-017 |
| 30 | shows agent config and version loading warnings | FR-003, FR-008 |
| 31 | blocks docker launch when compose service is missing | FR-047 |
| 32 | includes docker compose launch options in the launch request | FR-024, FR-026, FR-027 |
| 33 | shows env override parse errors and handles Escape key modal behavior | FR-022 |
| 34 | hides shell dropdown when no shells are available | FR-020 |
| 35 | shows shell dropdown when shells are available | FR-020 |
| 36 | disables shell dropdown when docker runtime is selected | FR-020 |
| 37 | shows prefix and suffix inputs when Direct mode is selected | FR-014 |
| 38 | persists and restores branchNamingMode from localStorage | FR-009, FR-011 |

#### `AgentLaunchForm.glm.test.ts`（3テスト）

| # | テストケース | 関連 FR |
|---|------------|---------|
| 1 | injects GLM env vars on launch and prefers Advanced env overrides | FR-008, FR-022 |
| 2 | does not inject GLM env vars when provider is Anthropic | FR-008 |
| 3 | persists switching back to Anthropic before launch | FR-008 |

#### `agentLaunchFormHelpers.test.ts`（7テスト）

| # | テストケース | 関連 FR |
|---|------------|---------|
| 1 | detects model support by agent id | FR-002 |
| 2 | formats errors and parses args/env overrides | FR-021, FR-022 |
| 3 | builds and splits branch names | FR-014 |
| 4 | builds docker status hint labels | FR-028 |
| 5 | resolves docker context selection for host/compose/dockerfile paths | FR-026 |
| 6 | checks issue launch guards and pagination trigger | FR-017, FR-019 |
| 7 | handles issue prefix classification and stale request checks | FR-018 |

#### `agentLaunchDefaults.test.ts`（6テスト）

| # | テストケース | 関連 FR |
|---|------------|---------|
| 1 | returns null when defaults are not stored | FR-011 |
| 2 | persists and restores launch defaults | FR-009 |
| 3 | returns null for invalid JSON | FR-012 |
| 4 | returns null for unknown schema version | FR-012 |
| 5 | defaults fastMode to false when older stored data does not include it | FR-012 |
| 6 | sanitizes invalid values to safe defaults | FR-012 |

#### `agentUtils.test.ts`（2テスト）

| # | テストケース | 関連 FR |
|---|------------|---------|
| 1 | maps known agent names to canonical IDs | FR-001 |
| 2 | returns null for unknown agent names | FR-001 |

#### `LaunchProgressModal.test.ts`（12テスト）

| # | テストケース | 関連 FR |
|---|------------|---------|
| 1 | renders step markers in running state | FR-031 |
| 2 | shows error message in error state | FR-044 |
| 3 | calls onCancel when Cancel button is clicked | FR-032 |
| 4 | calls onCancel on Escape key while running | FR-032 |
| 5 | calls onClose on Escape key when not running | FR-032 |
| 6 | renders nothing when open is false | UI |
| 7 | shows detail text when running | FR-031 |
| 8 | shows 'Use Existing Branch' button on E1004 error when onUseExisting is provided | FR-044 |
| 9 | does not show 'Use Existing Branch' button on non-E1004 errors | FR-044 |
| 10 | calls onUseExisting when 'Use Existing Branch' button is clicked | FR-044 |
| 11 | does not show 'Use Existing Branch' button on E1004 when onUseExisting is not provided | FR-044 |
| 12 | includes 'Registering skills' step in step list | FR-035 |

### FR-048 検証（Implemented）

#### `agentLaunchFormHelpers.test.ts`

- `classifyIssuePrefix()` が `ai-not-configured` / `error` / 無効 prefix の各ケースで `feature/` を返すことを確認
- `status: ok` の有効 prefix がそのまま適用されることを確認
- stale request 判定が引き続き機能することを確認

#### `AgentLaunchForm.test.ts`

- From Issue 選択時に prefix ドロップダウンがレンダリングされないことを確認
- ラベルベース自動分類で `bugfix/issue-99` が生成されることを確認
- AI 分類失敗時に `feature/issue-42` フォールバックで Launch できることを確認

---
