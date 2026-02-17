# バグ修正仕様: ベアリポジトリでリモートブランチからワークツリー作成時に E1003 エラー

**仕様ID**: `SPEC-bare-wt01`
**作成日**: 2026-02-17
**更新日**: 2026-02-17
**ステータス**: 承認済み
**カテゴリ**: Core / Worktree
**依存仕様**:

- SPEC-a70a1ece（ベアリポジトリ対応）

**入力**: ユーザー説明: "ベアリポジトリで origin/develop からワークツリーを作成すると [E1003] Branch not found が発生する"

## 背景

- gwt はベアリポジトリ（`git clone --bare`）を使用する
- ベアリポジトリでは `refs/remotes/origin/*` が存在しない
- `git fetch --all` は `refs/heads/*` にブランチを格納する
- 初期クローンではデフォルトブランチ（`main`）のみが `refs/heads/` に存在
- `origin/main` は成功するが `origin/develop` は失敗する

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - ベアリポジトリでリモートブランチからワークツリー作成 (優先度: P0)

ユーザーとして、ベアリポジトリで `origin/develop` のようなリモートブランチからワークツリーを作成したい。

**独立したテスト**: ベアリポジトリで未フェッチのリモートブランチからワークツリーを作成できる

**受け入れシナリオ**:

1. **前提条件** ベアリポジトリがクローンされ、リモートに `develop` ブランチが存在、**操作** `create_for_branch("origin/develop")` を呼ぶ、**期待結果** ワークツリーが正常に作成される
2. **前提条件** ベアリポジトリで `main` がデフォルトブランチ、**操作** `create_for_branch("origin/main")` を呼ぶ、**期待結果** ワークツリーが正常に作成される（既存動作の回帰なし）

---

### ユーザーストーリー 2 - 新規ブランチ作成時のリモートベースブランチ解決 (優先度: P0)

ユーザーとして、ベアリポジトリで `origin/develop` をベースに新規ブランチを作成したい。

**独立したテスト**: ベアリポジトリで未フェッチのリモートブランチをベースに新規ブランチを作成できる

**受け入れシナリオ**:

1. **前提条件** ベアリポジトリでリモートに `develop` ブランチが存在するがローカル未フェッチ、**操作** `create_new_branch("feature/new", Some("origin/develop"))` を呼ぶ、**期待結果** ワークツリーが正常に作成され、HEAD が `develop` のコミットを指す

---

### ユーザーストーリー 3 - ls-remote のタイムアウト保護 (優先度: P1)

開発者として、`remote_exists` の `ls-remote` 呼び出しがタイムアウトとプロンプト抑制を持つようにしたい。

**独立したテスト**: `remote_exists` が `run_git_with_timeout` を使用し、タイムアウト時に `false` を返す

**受け入れシナリオ**:

1. **前提条件** リモートが応答しない状態、**操作** `remote_exists` を呼ぶ、**期待結果** タイムアウト後に `Ok(false)` を返す

## エッジケース

- ネステッドブランチ名（`origin/feature/foo`）での動作
- リモートが到達不能な場合のタイムアウト処理
- `fetch_all` 後にブランチが `refs/heads/*` に作成されるケース

## 要件 *(必須)*

### 機能要件

- **FR-001**: `Branch::remote_exists` が `run_git_with_timeout` を使用し、`GIT_TERMINAL_PROMPT=0` とタイムアウトを適用する
- **FR-002**: `create_for_branch` でリモートブランチ指定時、ローカル `refs/heads/*` を先行チェックする
- **FR-003**: `create_for_branch` で `fetch_all` 後に `resolve_remote_branch` が失敗した場合、`refs/heads/*` をフォールバックチェックする
- **FR-004**: `create_new_branch` で `remote_exists` 失敗時に `fetch_all` + ローカル存在チェックをフォールバックとして実行する

### 非機能要件

- **NFR-001**: `ls-remote` のタイムアウトは5秒（既存の `LS_REMOTE_TIMEOUT` 定数を使用）
- **NFR-002**: 既存テストが全て通過する（回帰なし）

## 制約と仮定

- ベアリポジトリでは `refs/remotes/origin/*` が存在しない前提
- `git fetch --all` により `refs/heads/*` にブランチが作成される前提

## 成功基準 *(必須)*

- **SC-001**: ベアリポジトリで `origin/develop`（未フェッチ）からワークツリーを作成できる
- **SC-002**: 既存テストが全て通過する
- **SC-003**: `cargo clippy --all-targets --all-features -- -D warnings` が通過する
