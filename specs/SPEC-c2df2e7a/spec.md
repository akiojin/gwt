# バグ修正仕様: From Issue の Branch Exists 誤判定（stale remote-tracking ref）

**仕様ID**: `SPEC-c2df2e7a`
**作成日**: 2026-02-25
**更新日**: 2026-02-25
**ステータス**: 承認済み
**カテゴリ**: Core / Git Issue
**依存仕様**:

- `SPEC-c6ba640a`（From Issue UI）
- `SPEC-rb01a2f3`（remote branch 検出）

**入力**: ユーザー説明: "Issue #1231: Branch Exists false positive caused by stale remote-tracking branch for From Issue"

## 背景

- `find_branch_for_issue()` は remote-tracking branch（`git branch -r`）をそのまま「存在」とみなしている
- 実リモートから既に削除された stale ref が残っていると、`Branch Exists` が誤表示される
- 結果として、Issue #1029 のように実ブランチが無いのに From Issue が選択不可になる

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - stale remote-tracking を Exists に含めない (優先度: P0)

開発者として、実リモートに存在しない stale remote-tracking ref では `Branch Exists` 判定にならないようにしたい。

**独立したテスト**: `find_branch_for_issue()` が stale remote-tracking ref のみを持つ状態で `None` を返すこと

**受け入れシナリオ**:

1. **前提条件** `origin/bugfix/issue-1029` が remote-tracking に残り、実リモートには存在しない、**操作** `find_branch_for_issue(repo, 1029)` を呼ぶ、**期待結果** `None` が返る
2. **前提条件** stale ref が存在する、**操作** Launch Agent > From Issue を開く、**期待結果** Issue は `Branch Exists` で無効化されない

---

### ユーザーストーリー 2 - 実リモート branch は引き続き検出する (優先度: P0)

開発者として、リモートに実在する Issue branch は従来通り `Branch Exists` として検出したい。

**独立したテスト**: 実リモートに `bugfix/issue-1029` があるとき `Some("bugfix/issue-1029")` を返すこと

**受け入れシナリオ**:

1. **前提条件** ローカルには branch がなく、リモートには branch がある、**操作** `find_branch_for_issue(repo, 1029)` を呼ぶ、**期待結果** remote branch 名を返す
2. **前提条件** remote-tracking と実リモートが一致する、**操作** 判定APIを呼ぶ、**期待結果** 既存 branch として扱う

---

### ユーザーストーリー 3 - ローカル branch 優先は維持する (優先度: P1)

開発者として、ローカル branch がある場合は追加の remote 検証なしで即時検出したい。

**独立したテスト**: ローカル `feature/issue-1029` があるとき `Some("feature/issue-1029")` を返すこと

**受け入れシナリオ**:

1. **前提条件** ローカル branch が存在する、**操作** `find_branch_for_issue(repo, 1029)` を呼ぶ、**期待結果** ローカル branch を返す
2. **前提条件** 同名 remote branch も存在する、**操作** 判定APIを呼ぶ、**期待結果** ローカル branch 優先を維持する

## エッジケース

- `origin/HEAD -> origin/main` のような symbolic ref 行は branch 候補から除外する
- 同じ branch 候補が重複して出ても `ls-remote` は重複実行しない
- `git ls-remote` が失敗した場合は曖昧に成功扱いせずエラーとして呼び出し側へ返す

## 要件 *(必須)*

### 機能要件

- **FR-001**: `find_branch_for_issue()` はローカル branch 検出を最優先で実行しなければならない
- **FR-002**: remote-tracking branch 候補は remote 名と branch 名へ分離し、symbolic ref を除外しなければならない
- **FR-003**: remote-tracking branch 候補は `git ls-remote --heads <remote> <branch>` で実在確認しなければならない
- **FR-004**: 実在確認で空結果だった候補は stale とみなし、`Branch Exists` に含めてはならない
- **FR-005**: `git ls-remote` 実行失敗時はエラーを返し、誤って `Some(...)` を返してはならない
- **FR-006 (TDD)**: `find_branch_for_issue()` 修正前に再現テスト（stale/実在/ローカル優先）を追加し、RED→GREEN の順で完了しなければならない

### 非機能要件

- **NFR-001**: 既存のローカル branch 判定の性能特性に退行を生じさせない（local hit 時に追加ネットワーク確認を行わない）
- **NFR-002**: `cargo test -p gwt-core git::issue::tests::` と `cargo clippy -p gwt-core --all-targets -- -D warnings` を通過する

## 制約と仮定

- `Branch Exists` 判定は `worktree + local + 実在remote` を対象とし、stale remote-tracking は対象外とする
- remote 実在確認には Git の標準コマンド（`ls-remote --heads`）のみを使用する

## 成功基準 *(必須)*

- **SC-001**: stale remote-tracking のみ存在するケースで `find_branch_for_issue()` が `None` を返す
- **SC-002**: 実在 remote branch のケースで `find_branch_for_issue()` が `Some(branch)` を返す
- **SC-003**: ローカル branch 優先ケースで既存動作を維持する
- **SC-004**: 追加テストを含む `git::issue::tests::` が全件成功する
