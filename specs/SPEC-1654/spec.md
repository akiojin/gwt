# Workspace Shell（Agent Canvas・Branch Browser・マルチウィンドウ実行セッション）

## Background

- `#1654` is the canonical workspace shell spec.
- The old `Sidebar + split tab layout` model mixed shell, branch inventory, worktree instances, and execution sessions into one surface.
- The new shell uses top-level `Agent Canvas` and `Branch Browser` tabs. `Assistant / worktree / agent / terminal` move to canvas tiles.
- The canonical shell uses tab switching between full-window surfaces; persistent side-by-side detail panes and VS Code-style split work surfaces are out of scope.
- Ref and worktree domain truth is delegated to `#1644`; this spec defines shell composition, execution-session projection, restore, and window-local shell behavior.
- Shell-triggered inventory caching, invalidation, and local Git backend performance fixes remain `#1644` even when surfaced through Branch Browser or canvas-adjacent flows.
- `#1648` remains the persistence boundary, `#1636` remains the Assistant behavior boundary, and `#1313` / `#1329` / `#1309` remain the multi-window implementation foundation.

## User Scenarios

### User Story 1 - Agent Canvas is the primary work surface

**Priority**: P0

開発者として、window を開いた瞬間に Sidebar ではなく Agent Canvas を中心に worktree / agent / terminal の関係を見渡したい。

**Independent Test**: project open 直後に shell を描画し、`Agent Canvas` と `Branch Browser` が top-level tab に存在し、どの top-level tab も window 全体を使うことを確認できる。

**Acceptance Scenarios**:

1. **Given** project を開いた window がある、**When** shell を表示する、**Then** top-level tab に `Agent Canvas` と `Branch Browser` が存在し、Sidebar は存在しない
2. **Given** Agent Canvas を開いている、**When** assistant / worktree / agent / terminal tile を表示する、**Then** tile は同じ canvas 上に自由配置される
3. **Given** non-canvas tab を開く、**When** Issues / PR / Settings / Version History / Project Index / Issue Spec を切り替える、**Then** split group ではなく flat top-level tab として切り替わり、tab content は window 全体を使う
4. **Given** Agent Canvas または Branch Browser を開いている、**When** shell を表示する、**Then** persistent right-side detail paneや side-by-side shell split は存在しない

### User Story 2 - Branch Browser materializes worktrees from refs

**Priority**: P0

開発者として、local / remote / all refs を Branch Browser で検索し、必要な branch を worktree 化して Agent Canvas に出したい。

**Independent Test**: Branch Browser を `Local`, `Remote`, `All` で切り替え、selected ref に対して create/focus action が適切に分岐することを確認できる。

**Acceptance Scenarios**:

1. **Given** remote-only branch が存在する、**When** Branch Browser でその ref を選ぶ、**Then** worktree create action を起動できる
2. **Given** local branch に既存 worktree がある、**When** Branch Browser からその branch を開く、**Then** 既存 worktree tile が Agent Canvas で focus される
3. **Given** `Local / Remote / All` mode を切り替える、**When** inventory を再描画する、**Then** projection は `#1644` の ref/worktree domain truth と整合する
4. **Given** Branch Browser を表示している、**When** branch list と selected branch detail を使う、**Then** list/detail は one-surface full-window layout で表示され、左右分割された固定幅 pane にはならない

### User Story 3 - Worktree tiles own execution relationships

**Priority**: P0

開発者として、どの terminal / agent がどの worktree から起動されたかを relation edge で常時把握したい。

**Independent Test**: worktree tile から agent/terminal を起動し、worktree -> child edge が常時表示され、tile close 時に edge も整理されることを確認できる。

**Acceptance Scenarios**:

1. **Given** worktree tile から agent を起動した、**When** launch が成功する、**Then** Agent Canvas に agent tile が追加され、worktree tile から edge で接続される
2. **Given** worktree tile から terminal を開いた、**When** terminal tile が表示される、**Then** 同じ worktree tile から edge で接続される
3. **Given** viewport 外に terminal tile がある、**When** canvas を縮小またはパンする、**Then** live terminal mount は止めても tile identity と edge は維持される
4. **Given** assistant / worktree / agent / terminal の詳細を開く、**When** interaction を行う、**Then** detail は popup / overlay に表示され、top-level tab の恒常的な横分割にはならない

### User Story 4 - Shell state restores per window

**Priority**: P0

開発者として、複数 window で別 project / 別 canvas を使っていても、restore 後に tile 配置と session 関係が window ごとに戻ってほしい。

**Independent Test**: 複数 window で別 project を開いた状態を保存して再起動し、window ごとに Agent Canvas / Branch Browser state が混線せず復元されることを確認できる。

**Acceptance Scenarios**:

1. **Given** 複数 window で Agent Canvas を使っている、**When** アプリを再起動する、**Then** window ごとに保存された shell state と canvas state が混線せずに復元される
2. **Given** Window A/B で異なる project が開いている、**When** Window A の agent/terminal/worktree state が変わる、**Then** Window B の shell state は変わらない
3. **Given** restore 時に stale な split-tab metadata や旧 agent/terminal tab 保存データが存在する、**When** shell state を復元する、**Then** canonical shell model に migrate または prune され、app 全体は起動継続する

## Edge Cases

- Branch Browser は remote-only ref を inventory として表示しても、worktree create までは Agent Canvas tile を作らない
- worktree tile が `gone` になっても child agent/terminal session がまだ生きている場合、worktree tile は stale/gone projection で残し、child close 後に整理する
- live terminal mount は viewport 外 tile で止めても、tile geometry / relation edge / selection state を失ってはならない
- restore payload に split-tab group metadata が残っていても、shell 正本は flat top-level tabs + canvas/browser state を優先する
- `Cmd+\`` / `Ctrl+\`` は canvas focus や xterm focus に奪われず window navigation として機能しなければならない

## Functional Requirements

- **FR-001**: top-level shell は flat tab bar を持ち、少なくとも `agentCanvas`, `branchBrowser`, `settings`, `issues`, `prs`, `versionHistory`, `projectIndex`, `issueSpec` を扱わなければならない
- **FR-002**: Sidebar は shell から除去されなければならない
- **FR-003**: Agent Canvas は `assistant`, `worktree`, `agent`, `terminal` tile を保持しなければならない
- **FR-004**: Assistant は canvas 上の常設 tile でなければならない
- **FR-005**: worktree 起点で生成された `agent` / `terminal` tile は `worktree -> child` の relation edge を常時表示しなければならない
- **FR-006**: agent launch と terminal launch は top-level tab を追加せず、Agent Canvas 上の tile を追加しなければならない
- **FR-007**: worktree tile クリックで worktree detail popup を開けなければならない
- **FR-008**: Branch Browser は `#1644` の ref/worktree domain projection を利用し、`local / remote / all` inventory を表示できなければならない
- **FR-009**: canvas interaction は自由配置、パン、ズームを提供しなければならない
- **FR-010**: live terminal rendering は viewport 内 tile に限定して良いが、tile 自体は canvas state に常に存在しなければならない
- **FR-011**: shell state は window-local に保存・復元されなければならない
- **FR-012**: `Cmd+\`` / `Ctrl+\`` による window cycling は維持されなければならない
- **FR-013**: split-tab group tree は shell 正本から除去されなければならない
- **FR-014**: old agent/terminal tab persistence は canonical canvas tile model に migrate されなければならない
- **FR-015**: すべての top-level tab content は window 全体を使う単一 surface として表示されなければならない
- **FR-016**: `Agent Canvas` と `Branch Browser` は persistent side-by-side detail pane を持ってはならない
- **FR-017**: Assistant / worktree / session detail が必要な場合は popup または overlay で表示してよいが、top-level tab の恒常レイアウトを分割してはならない

## Non-Functional Requirements

- **NFR-001**: restore は best-effort かつ non-blocking であり、1 window の shell restore failure が他 window を壊してはならない
- **NFR-002**: canvas / browser shell state は project path と window label の両方に対して局所的でなければならない
- **NFR-003**: branch/ref inventory の正本を shell 内で重複保持してはならず、`#1644` の domain projection を参照すること
- **NFR-004**: visible-tile-only terminal mount でも relation edge と layout が破綻してはならない
- **NFR-005**: Git/cache/PR/index などの重い処理は UI thread で実行してはならず、OS native background threads で実行しなければならない
- **NFR-006**: project open は cache-first かつ non-blocking でなければならず、issue cache warmup やその他の heavyweight refresh が first paint を待たせてはならない
- **NFR-007**: maximize/restore/resize は layout-only イベントとして扱われなければならず、branch inventory、PR status、issue cache refresh を直接または間接に起動してはならない
- **NFR-008**: responsive-performance E2E は startup interactive <= 1000ms、maximize/restore interactive <= 300ms を継続的に検証しなければならない
- **NFR-009**: backend hot paths for shell inventory and worktree listing must remain benchmarked so regressions are detectable without relying on frontend symptoms alone

## Success Criteria

- **SC-001**: project open 後に `Agent Canvas` と `Branch Browser` が表示され、Sidebar が消えている
- **SC-002**: すべての top-level tab が window 全体を使い、右半分が未使用になる固定レイアウトが存在しない
- **SC-003**: Branch Browser から remote/local ref を辿って worktree create/focus ができる
- **SC-004**: worktree 起点の agent/terminal tile に relation edge が表示される
- **SC-005**: multi-window restore 後に window ごとの shell/canvas state が混線しない
- **SC-006**: shell regressions が frontend unit test と e2e で検証される
- **SC-007**: slow issue-cache warmup を模擬しても、project open 後 1000ms 以内に主要 shell surface が操作可能になる
- **SC-008**: maximize/restore 後 300ms 以内に主要 shell surface が再操作可能になる
- **SC-009**: maximize/restore 前後で branch inventory、PR status、issue cache warmup の heavy refresh 呼び出し数が増えない
