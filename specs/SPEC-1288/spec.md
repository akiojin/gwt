# バグ修正仕様: From Issue でブランチ名に prefix が二重表示される

**仕様ID**: `SPEC-1288`
**作成日**: 2026-02-27
**更新日**: 2026-02-27
**ステータス**: ドラフト
**カテゴリ**: GUI
**依存仕様**:

- SPEC-c6ba640a（GitHub Issue連携によるブランチ作成）
- SPEC-a2f8e3b1（From Issue ブランチプレフィックスAI判定）

**入力**: ユーザー説明: "Issue #1288を解決して"

## 背景

- Launch Agent の New Branch > From Issue では、prefix 選択ドロップダウンと readonly 入力欄の双方に prefix が含まれ、`bugfix` が二重に見える。
- 実際に作成される branch 名は正しいが、UI 表示が誤解を招く。

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - From Issue の branch 表示を正しく理解したい (優先度: P0)

ユーザーとして、From Issue の Branch Name で prefix と suffix の役割を明確に見分けたい。

**独立したテスト**: `bug` ラベル付き Issue 選択時、prefix セレクトは `bugfix/`、入力欄は `issue-<number>` のみになること。

**受け入れシナリオ**:

1. **前提条件** From Issue タブで `bug` ラベル付き Issue #99 を選択、**操作** Branch Name 表示を確認、**期待結果** prefix は `bugfix/`、入力欄は `issue-99` を表示する
2. **前提条件** From Issue タブで prefix を `feature/` から `hotfix/` に変更、**操作** 表示を確認、**期待結果** 入力欄は `issue-<number>` のままで、prefix 側のみ変化する

---

### ユーザーストーリー 2 - 起動時の branch 作成は従来どおり成功したい (優先度: P0)

ユーザーとして、表示修正後も launch payload は従来どおり `{prefix}issue-{number}` で送られてほしい。

**独立したテスト**: From Issue launch の request で `branch` と `createBranch.name` が full branch name であること。

**受け入れシナリオ**:

1. **前提条件** `bugfix/` + Issue #99 を選択済み、**操作** Launch 実行、**期待結果** request.branch は `bugfix/issue-99` で送信される
2. **前提条件** 同上、**操作** Launch 実行、**期待結果** request.createBranch.name は `bugfix/issue-99` で送信される

## エッジケース

- prefix 未選択（AI 判定失敗等）時は、入力欄に `issue-<number>` を表示したまま Launch を無効化する。
- Manual タブの Direct / AI Suggest の表示・送信ロジックには影響を与えない。

## 要件 *(必須)*

### 機能要件

- **FR-001**: From Issue の Branch Name 表示は、prefix セレクトと `issue-<number>` の suffix 表示を分離しなければならない。
- **FR-002**: From Issue の launch 時に送信する branch 名は、`{prefix}issue-{number}` 形式を維持しなければならない。
- **FR-003**: `request.branch` と `request.createBranch.name` は同一の full branch name を使用しなければならない。
- **FR-004**: prefix 未選択時の Launch disabled 制御は既存挙動を維持しなければならない。

### 非機能要件

- **NFR-001**: 変更範囲は `AgentLaunchForm` の UI 表示と launch 組み立てに限定し、Tauri command I/F を変更しない。
- **NFR-002**: 既存 From Issue 回帰を防ぐため、`AgentLaunchForm.test.ts` に表示と payload のテストを追加する。

## 制約と仮定

- ブランチ命名規約 `{prefix}issue-{number}` は既存仕様に従い変更しない。
- prefix 種別は既存 4 種（`feature/`, `bugfix/`, `hotfix/`, `release/`）を維持する。

## 成功基準 *(必須)*

- **SC-001**: From Issue 選択後、Branch Name の入力欄に prefix が重複表示されない。
- **SC-002**: Launch payload は表示変更後も `{prefix}issue-{number}` を保持する。
- **SC-003**: 追加したユニットテストが通過し、既存 From Issue テストに回帰がない。
