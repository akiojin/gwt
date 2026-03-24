# Local Git Backend Domain（Ref / Worktree・Inventory・Cleanup）

## Background

- `#1644` is the canonical local Git backend spec.
- This issue owns GitHub-free backend behavior for local repository state: git CLI wrapping, ref inventory, worktree projection, branch inventory snapshot/detail hydration, cache/invalidation, cleanup/protection/gone/divergence resolution, and worktree materialization/focus rules.
- The shell owner remains `#1654`; this issue defines backend truth rather than shell composition or UI-local merge logic.
- `#1647` owns project lifecycle orchestration only.
- `#1714` owns worktree-issue linkage and exact issue cache only.
- `#1643` owns GitHub integration only.
- `#1649` owns PR lifecycle only.

## Ownership Boundaries

### Owned Here

- local branch/ref/worktree inventory and projection semantics
- local Git state reads/writes that do not require GitHub APIs
- branch inventory snapshot/detail hydration, cache, and invalidate rules
- create/focus/ambiguity rules for worktree materialization
- cleanup safety, protection, gone/divergence, display-name fallback inputs, and stable worktree identity

### Not Owned Here

- GitHub Issue/Spec search, version history, release/tag retrieval (`#1643`)
- PR create/update/status/review lifecycle (`#1649`)
- Worktree-Issue linkage source of truth and exact issue cache persistence (`#1714`)
- project open/close/create/switch orchestration (`#1647`)
- workspace shell composition, canvas/browser layout, and window restore (`#1654`)

## User Scenarios

### User Story 1 - Browse local / remote / all refs as inventory

**Priority**: P0

開発者として、実体化済み worktree とは別に、local refs と remote-only refs を統一 inventory として検索・絞り込みしたい。

**Independent Test**: 同じ repo で `Local`, `Remote`, `All` を切り替え、projection ごとに意図した ref 集合だけが見えることを確認できる。

**Acceptance Scenarios**:

1. **Given** local refs と remote refs が存在する、**When** inventory mode を `Local` にする、**Then** local refs だけが表示される
2. **Given** local refs と remote refs が存在する、**When** inventory mode を `Remote` にする、**Then** remote-only refs を含む remote projection が表示される
3. **Given** 同じ canonical branch 名の local/remote ref がある、**When** inventory mode を `All` にする、**Then** identity を壊さない統合 projection として表示できる

### User Story 2 - Resolve refs to worktree actions

**Priority**: P0

開発者として、選択した ref に対して、新規 worktree を作るか既存 worktree を開くかを一貫したルールで決めたい。

**Independent Test**: remote-only ref、local ref without worktree、local ref with existing worktree の 3 パターンで、resolution が `create` / `focus` / explicit choice に分岐することを確認できる。

**Acceptance Scenarios**:

1. **Given** remote-only ref を選択した、**When** open action を行う、**Then** worktree create flow が提案される
2. **Given** local branch に既存 worktree instance がある、**When** open action を行う、**Then** 既存 worktree instance が返される
3. **Given** local branch はあるが realized worktree がない、**When** open action を行う、**Then** worktree create flow が提案される

### User Story 3 - Keep worktree meaning in the domain

**Priority**: P0

開発者として、display name、issue linkage、safety、tool usage などを shell ではなく worktree domain から一貫して引きたい。

**Independent Test**: worktree instance projection を読み出し、display-name fallback、divergence、tool usage、safety が同じ projection から得られることを確認できる。

**Acceptance Scenarios**:

1. **Given** worktree instance がある、**When** display name を解決する、**Then** `手動 display_name -> issue linkage -> AI summary -> branch name` の順で決まる
2. **Given** cleanup を検討する、**When** safety を計算する、**Then** protected/current/agent-running/change/unpushed/pr-state を含めて判定できる
3. **Given** issue linkage または tool usage が更新された、**When** worktree projection を再構築する、**Then** shell はこの domain 更新だけを見ればよい

### User Story 4 - Provide stable worktree identity for execution sessions

**Priority**: P0

開発者として、agent/terminal execution が branch 文字列ではなく worktree instance identity を参照できるようにしたい。

**Independent Test**: worktree から起動した agent/terminal session metadata が stable worktree identity を持ち、shell がその id だけで parent relation を解決できることを確認できる。

**Acceptance Scenarios**:

1. **Given** worktree から agent/terminal が起動される、**When** session metadata を記録する、**Then** 参照先 worktree instance を安定して特定できる
2. **Given** branch display name が変わる、**When** session relation を再評価する、**Then** branch label ではなく worktree identity を使うため親 relation は壊れない

### User Story 5 - Provide reusable local Git backend services to adjacent specs

**Priority**: P0

開発者として、shell・project lifecycle・PR/GitHub 連携が local Git backend の正本を再定義せず、同じ projection / invalidation ルールを再利用できるようにしたい。

**Independent Test**: Branch Browser 表示、project open、PR status 参照、issue cache 連携の代表変更を分類したとき、local Git backend 変更は常に `#1644` に帰属できることを確認できる。

**Acceptance Scenarios**:

1. **Given** Branch Browser が ref inventory を必要とする、**When** backend projection を参照する、**Then** shell ローカル merge ロジックではなく `#1644` の projection を使う
2. **Given** project open または manual refresh が repo state invalidation を起こす、**When** backend refresh policy を決める、**Then** invalidation reason と refresh boundary は `#1644` で定義される
3. **Given** PR/GitHub 機能が local branch/worktree context を必要とする、**When** local Git state を参照する、**Then** adjacent spec は `#1644` の振る舞いを消費し、再定義しない

## Edge Cases

- same-name local/remote ref がある場合、`All` projection は 1 つの canonical inventory entry で `hasLocal/hasRemote` を区別する
- upstream が gone の local branch は inventory と worktree instance の両方で `isGone` を保持するが、即 cleanup 候補とは同一視しない
- local branch に worktree が複数あるなど 1 ref -> multiple instances の状態を検出した場合、domain は ambiguity を明示し、暗黙に 1 つを選ばない
- remote-only ref は inventory に存在しても worktree instance projection へ昇格してはならない
- display name, issue linkage, tool usage が欠けても branch name fallback で projection を維持しなければならない
- shell maximize/restore や tab switching は local Git backend refresh policy の owner ではなく、この domain を request する consumer に留まる
- issue linkage / exact cache miss は `#1714` の責務であり、この domain が GitHub lookup semantics の owner になってはならない

## Functional Requirements

- **FR-001**: domain は `ref inventory` と `worktree instance` を別概念として持たなければならない
- **FR-002**: ref inventory は `local`, `remote`, `all` の projection を提供しなければならない
- **FR-003**: remote-only refs は worktree instance を持たず、inventory 上の存在として扱わなければならない
- **FR-004**: selected ref に対して `create worktree` / `focus existing worktree` の解決ルールを定義しなければならない
- **FR-005**: worktree instance は display name, issue linkage, tool usage, divergence, gone/current/protected/safety を保持または解決できなければならない
- **FR-006**: cleanup / branch protection / PR linkage の判定は worktree instance を単位に行わなければならない
- **FR-007**: shell (`#1654`) は branch truth を重複実装せず、本 domain projection を参照しなければならない
- **FR-008**: execution session から参照できる stable worktree identity を提供しなければならない
- **FR-009**: same-name local/remote refs の canonicalization ルールを projection に明示しなければならない
- **FR-010**: GitHub API を必要としない local Git backend responsibilities は本 spec で定義されなければならない
- **FR-011**: domain は inventory snapshot、worktree projection、action resolution、detail hydration、cache invalidation の reusable backend interface を定義しなければならない
- **FR-012**: `#1654`, `#1647`, `#1649`, `#1714`, `#1643` は adjacent owner または consumer として本 domain を参照し、local Git behavior を再定義してはならない

## Non-Functional Requirements

- **NFR-001**: domain projection は Sidebar の有無に依存してはならない
- **NFR-002**: local/remote/all projection は branch identity を壊さず、同名 ref の扱いを曖昧にしてはならない
- **NFR-003**: worktree instance projection は cleanup, popup detail, canvas card, session edge の共通正本にならなければならない
- **NFR-004**: ownership boundary は正確かつ非重複でなければならず、同じ local Git backend concern を複数 spec の canonical scope に置いてはならない
- **NFR-005**: local Git backend cache/invalidation は shell 固有の UI レイアウトや GitHub online availability を前提にしてはならない

## Success Criteria

- **SC-001**: Branch Browser が `#1644` の domain projection だけで `Local / Remote / All` を表示できる
- **SC-002**: remote-only ref と realized worktree instance を混同しない
- **SC-003**: worktree create/focus の判断ルールがテストで固定される
- **SC-004**: display name / safety / cleanup / linkage の正本が shell 側へ漏れない
- **SC-005**: 代表的な backend change の owner が一意に分類できる（branch inventory/perf/cache/invalidate = `#1644`, project open orchestration = `#1647`, issue cache sync = `#1714`, GitHub search/version history = `#1643`, PR lifecycle = `#1649`）
