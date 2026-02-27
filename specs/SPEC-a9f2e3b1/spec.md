# 機能仕様: Worktree詳細 MERGE 状態判定の整理

**仕様ID**: `SPEC-a9f2e3b1`
**作成日**: 2026-02-26
**更新日**: 2026-02-27
**ステータス**: レビュー中
**カテゴリ**: GUI / Backend
**依存仕様**:

- SPEC-d6949f99（PRステータス取得）
- SPEC-merge-pr（マージ機能）

**入力**: ユーザー説明: 「MERGE表示の判定を整理したい。Blocked/Warning/Checking を明確化し、Unknown 表示を廃止したい」

## 背景

- GitHub GraphQL API は `mergeable` / `mergeStateStatus` で一時的に `UNKNOWN` を返すことがあり、従来 UI は `Unknown` 表示になる
- Worktree詳細ビューでは、マージ不可理由（必須条件での失敗）と、マージ自体は可能だが非必須チェック失敗がある状態が混在して見える
- サイドバーと詳細ビューで判定基準が分散し、表示の意味が一貫しない
- その結果、ユーザーが「今マージできない理由」を即座に判断しづらい

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - Unknown 表示の廃止と確認中表示 (優先度: P0)

ユーザーとして、`UNKNOWN` をそのまま見せるのではなく、システムが確認中であることを `Checking merge status...` として表示してほしい。

**独立したテスト**: `mergeable=UNKNOWN` または `mergeStateStatus=UNKNOWN` のとき、`Unknown` ではなく `checking` 表示になる

**受け入れシナリオ**:

1. **前提条件** `mergeable=UNKNOWN` が返る、**操作** PR ステータスを表示、**期待結果** MERGE 主バッジは `Checking merge status...` を表示する
2. **前提条件** `retrying=true` の PR がある、**操作** Worktree詳細・サイドバーを表示、**期待結果** `checking` 表示かつパルスアニメーションを適用する
3. **前提条件** `pr-status-updated` で解決済みステータスが通知される、**操作** 非同期更新を受信、**期待結果** `checking` から最終状態に遷移する

---

### ユーザーストーリー 2 - Blocked と Checks warning の分離 (優先度: P0)

ユーザーとして、必須条件でマージ不能な状態は `Blocked`、非必須チェック失敗は別の `Checks warning` として区別してほしい。

**独立したテスト**: 必須チェック失敗時は `Blocked`、非必須のみ失敗時は `Checks warning` が表示される

**受け入れシナリオ**:

1. **前提条件** 必須チェックが failure、**操作** PR ステータスを表示、**期待結果** MERGE 主バッジは `Blocked`
2. **前提条件** 非必須チェックのみ failure（必須は成功）、**操作** PR ステータスを表示、**期待結果** 主バッジは `Mergeable` 系を維持し、`Checks warning` を併記
3. **前提条件** 必須チェックと非必須チェックの両方が failure、**操作** PR ステータスを表示、**期待結果** `Blocked` を優先し、`Checks warning` は表示しない

---

### ユーザーストーリー 3 - 判定フローの一本化 (優先度: P1)

ユーザーとして、サイドバーと詳細ビューで同じ判定基準で状態が表示されてほしい。

**独立したテスト**: バックエンドで算出した `merge_ui_state` を両画面が優先利用し、同一PRで同じ意味の状態になる

**受け入れシナリオ**:

1. **前提条件** `merge_ui_state=blocked` が返る、**操作** サイドバー表示、**期待結果** `mergeable=UNKNOWN` でも `checking` ではなく `blocked` として表示される
2. **前提条件** `merge_ui_state` が未設定（旧キャッシュ等）、**操作** フロント表示、**期待結果** フロントエンドのフォールバック判定で同等の状態に解決する

## エッジケース

- `UNKNOWN` と必須チェック failure が同時に存在: `Blocked` を優先する
- PR が `MERGED` / `CLOSED` の場合: 他条件より終端状態表示を優先する
- リトライ中に `pr-status-updated` が `retrying=true` で届く場合: 進捗イベントとして扱い、既存表示を維持する
- `merge_ui_state` が未設定の古いデータ: 既存フィールドからフロント側で互換フォールバックする

## 要件 *(必須)*

### 機能要件

- **FR-001**: バックエンドは `PrStatusLiteSummary` / `PrDetailResponse` に `merge_ui_state` を含める
- **FR-002**: `merge_ui_state` は `merged | closed | checking | blocked | conflicting | mergeable` のいずれかである
- **FR-003**: `blocked` 判定は `mergeStateStatus=BLOCKED`、必須チェック failure、`CHANGES_REQUESTED` のいずれかで成立する
- **FR-004**: `checking` 判定は `retrying=true`、または `mergeable=UNKNOWN` / `mergeStateStatus=UNKNOWN`（ただし `blocked` 非該当時）で成立する
- **FR-005**: 非必須チェック警告は `non_required_checks_warning` として返却し、非必須 failure があり必須 failure がない場合のみ true とする
- **FR-006**: フロントエンドは MERGE 主バッジで `Unknown` 文言を表示しない。`UNKNOWN` 系は `Checking merge status...` として表示する
- **FR-007**: フロントエンドは `blocked` を MERGE 主バッジで表示し、`mergeStateStatus` 補助バッジの `Blocked` 表示は行わない
- **FR-008**: フロントエンドは `non_required_checks_warning=true` のとき `Checks warning` バッジを表示する
- **FR-009**: サイドバー PR バッジは `merge_ui_state` を優先し、`blocked`/`checking`/`conflicting` を判定する
- **FR-010**: `retrying=true` の間は `checking` 表示にパルスアニメーションを適用する
- **FR-011**: `pr-status-updated` イベントで状態を非同期更新し、`retrying=false` の解決イベントで通常表示へ遷移する
- **FR-012**: 既存 UNKNOWN リトライ（指数バックオフ 2s,4s,8s,16s,32s）とキャッシュ退行防止ロジックを維持する

### 非機能要件

- **NFR-001**: MERGE 判定ロジックはバックエンドに集約し、フロントは互換フォールバックのみを持つ
- **NFR-002**: UI更新は同期ブロックせず、イベント駆動で反映する
- **NFR-003**: 状態差分は既存テスト（Rust unit / Vitest / Playwright）で検証可能であること

## 制約と仮定

- GitHub API の `UNKNOWN` は一時的な可能性が高いが、永続するケースもある
- 必須/非必須の区別は GraphQL の `isRequired` を信頼する
- 古いキャッシュ・互換データでは `merge_ui_state` が欠落しうる

## 成功基準 *(必須)*

- **SC-001**: Worktree詳細ビューに `Unknown` 文言が残らない
- **SC-002**: 必須 failure のとき `Blocked` が表示され、非必須のみ failure のとき `Checks warning` が表示される
- **SC-003**: サイドバーと詳細ビューで同一PRの状態意味が一致する
- **SC-004**: `retrying` 開始から解決イベントまで `checking` の非同期遷移が確認できる
