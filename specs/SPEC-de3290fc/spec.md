# 機能仕様: PRタブへのWorkflow統合とブランチ状態表示

**仕様ID**: `SPEC-de3290fc`
**作成日**: 2026-02-22
**更新日**: 2026-02-22
**ステータス**: ドラフト
**カテゴリ**: GUI
**依存仕様**:

- `SPEC-7c0444a8`（Worktree Summary タブ構成）
- `SPEC-d6949f99`（PR/Workflow 表示）

**入力**: ユーザー説明: "PRタブにWorkflowタブを統合する。Required Checksの識別、Update Branch実行、Conflict状態表示を含む"

## 背景

- 現在 Worktree Summary は `Summary / Git / Issue / PR / Workflow / Docker` の6タブ構成だが、Workflow は PR に紐づくデータであり、別タブに分離されていると PR の全体像が把握しにくい。
- GitHub の Branch Protection Rules における Required Checks と任意の Checks の区別がなく、マージ可否の判断に必要な情報が不足している。
- ブランチが base ブランチより遅れている（behind）状態やコンフリクト状態が目立たず、Update Branch アクションが GUI から実行できない。

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - PRタブで Checks 状態を一覧できる (優先度: P0)

開発者として、PR タブ内で CI Checks の状態を確認したいので、Workflow 情報が PR メタ情報と統合されて表示されてほしい。

**独立したテスト**: PR タブを開くと Reviews/Comments/Changes に加えて折りたたみ式の Checks セクションが表示され、各 CheckRun のステータスとバッジが確認できること。

**受け入れシナリオ**:

1. **前提条件** PR に CheckRun が紐づいている、**操作** PR タブを開く、**期待結果** Checks セクションが折りたたみ式アコーディオンとして表示され、各 CheckRun のステータスアイコン・名前・結論が表示される。
2. **前提条件** Checks セクションが表示されている、**操作** アコーディオンの展開/折りたたみを切り替える、**期待結果** Checks リストの表示/非表示が切り替わる。
3. **前提条件** PR に CheckRun が 0 件、**操作** PR タブを開く、**期待結果** Checks セクションは「No checks」と表示される。
4. **前提条件** PR が存在しない、**操作** PR タブを開く、**期待結果** 従来通り「No PR」が表示される。

---

### ユーザーストーリー 2 - Required Checks を識別できる (優先度: P0)

開発者として、どの Check が Branch Protection で必須かを把握したいので、Required Checks に「required」バッジが付与されてほしい。

**独立したテスト**: Required に指定された CheckRun には「required」バッジが表示され、それ以外には表示されないこと。

**受け入れシナリオ**:

1. **前提条件** Branch Protection で Required Status Checks が設定されており、PR に required/non-required の CheckRun がある、**操作** PR タブの Checks セクションを展開する、**期待結果** Required な CheckRun には「required」バッジが表示され、そうでない CheckRun にはバッジが表示されない。
2. **前提条件** Branch Protection Rules が設定されていない、**操作** PR タブの Checks セクションを展開する、**期待結果** 全 CheckRun がバッジなしで表示される（エラーにならない）。

---

### ユーザーストーリー 3 - ブランチの behind/conflict 状態を確認し Update Branch を実行できる (優先度: P0)

開発者として、PR のマージ可否に関する全状態（mergeable、behind、conflict）を把握し、必要に応じてブランチ更新を実行したい。

**独立したテスト**: Merge メタ行に mergeable 状態・mergeStateStatus が表示され、BEHIND 時に Update Branch ボタンが表示されクリックで更新が実行されること。

**受け入れシナリオ**:

1. **前提条件** PR の head ブランチが base ブランチより遅れている（mergeStateStatus が BEHIND）、**操作** PR タブを開く、**期待結果** Merge 行に「Behind base」の表示と「Update Branch」ボタンが表示される。
2. **前提条件** 「Update Branch」ボタンが表示されている、**操作** ボタンをクリックする、**期待結果** `gh api` 経由でブランチ更新が実行され、成功時に表示が更新される。
3. **前提条件** PR がコンフリクト状態（mergeable が CONFLICTING）、**操作** PR タブを開く、**期待結果** Merge 行に「Conflicting」バッジが赤色で表示される。
4. **前提条件** PR が CLEAN 状態（マージ可能で最新）、**操作** PR タブを開く、**期待結果** Merge 行に「Mergeable」バッジが緑色で表示され、Update Branch ボタンは非表示。
5. **前提条件** ブランチ更新中にエラーが発生、**操作** Update Branch ボタンをクリック、**期待結果** エラーメッセージが表示され、再試行可能。

---

### ユーザーストーリー 4 - Workflow タブが削除され PR タブに統合されている (優先度: P1)

開発者として、タブ構成がシンプルであってほしいので、Workflow タブが削除され PR タブ内で全情報が確認できるようになっていてほしい。

**独立したテスト**: タブ列に Workflow タブが存在せず、PR タブ内に Checks セクションが含まれていること。

**受け入れシナリオ**:

1. **前提条件** Worktree Summary パネルを表示、**操作** タブ列を確認する、**期待結果** `Summary / Git / Issue / PR / Docker` の5タブが表示され、Workflow タブは存在しない。
2. **前提条件** PR タブを開く、**操作** PR 詳細を確認する、**期待結果** PR メタ情報・Merge状態・Checks・Reviews・Comments・Changes の全セクションが表示される。

## エッジケース

- GitHub CLI が未認証/未インストールの場合: 従来通りエラーメッセージを表示する。
- Branch Protection Rules が存在しない場合: `isRequired` が false として扱い、全 CheckRun をバッジなしで表示する。
- `mergeStateStatus` が取得できない場合（API 互換性）: `mergeable` フィールドのみで表示し、Update Branch ボタンは非表示にする。
- Update Branch 実行中にネットワークエラーが発生した場合: ローディング表示をクリアし、エラーメッセージを表示する。
- CheckRun が 50 件を超える場合: 先頭50件を表示する（現行の GraphQL ページネーション制限）。

## 要件 *(必須)*

### 機能要件

- **FR-001**: Worktree Summary のタブ構成を `Summary / Git / Issue / PR / Docker` の5タブに変更し、Workflow タブを削除しなければならない。
- **FR-002**: PR タブ内に折りたたみ式アコーディオンとして Checks セクションを追加し、PR に紐づく全 CheckRun を表示しなければならない。
- **FR-003**: 各 CheckRun に対して、GitHub Branch Protection の Required Status Checks に該当する場合は「required」バッジを表示しなければならない。
- **FR-004**: GraphQL クエリに `isRequired` フィールド（CheckRun 上）を追加し、Required 判定情報を取得しなければならない。
- **FR-005**: GraphQL クエリに `mergeStateStatus` フィールド（PullRequest 上）を追加し、BEHIND/BLOCKED/CLEAN/DIRTY/DRAFT/HAS_HOOKS/UNKNOWN/UNSTABLE の状態を取得しなければならない。
- **FR-006**: PR メタ情報の Merge 行を拡張し、`mergeable` + `mergeStateStatus` に基づく状態表示を行わなければならない。
- **FR-007**: `mergeStateStatus` が BEHIND の場合、「Update Branch」ボタンを表示し、クリックで GitHub API 経由のブランチ更新を実行しなければならない。
- **FR-008**: ブランチ更新は `gh api` の `PUT /repos/{owner}/{repo}/pulls/{pull_number}/update-branch` を使用しなければならない。
- **FR-009**: ブランチ更新の実行中はローディング表示を行い、成功/失敗を UI に反映しなければならない。

### 非機能要件

- **NFR-001**: Checks セクションの折りたたみ状態はセッション中保持される（ページ遷移やタブ切替で状態を維持）。
- **NFR-002**: Update Branch API 呼び出しのタイムアウトは30秒とする。

## 制約と仮定

- GitHub CLI（gh）が認証済みであること。
- GitHub GraphQL API の `isRequired` フィールドが利用可能であること（GitHub Enterprise でも対応）。
- `mergeStateStatus` は GitHub GraphQL API v4 で提供される PullRequest フィールドであること。

## 成功基準 *(必須)*

- **SC-001**: Workflow タブが削除され、PR タブ内の折りたたみ式 Checks セクションで全 CheckRun が確認できる。
- **SC-002**: Required Checks に「required」バッジが正しく表示される（Branch Protection 設定に基づく）。
- **SC-003**: Merge 行で behind/conflicting/clean 状態が視覚的に区別でき、behind 時に Update Branch ボタンが機能する。
- **SC-004**: 既存の PR タブの機能（メタ情報、Reviews、Comments、Changes）が全て維持されている。
- **SC-005**: Workflow タブへの参照が全てのコード・テストから除去されている。
