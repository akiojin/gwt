# 機能仕様: Windows 移行プロジェクトで Docker 起動時に mount エラーを回避する

**仕様ID**: `SPEC-4e2f1028`
**作成日**: 2026-02-13
**更新日**: 2026-02-13
**ステータス**: ドラフト
**カテゴリ**: GUI
**依存仕様**:

- specs/archive/SPEC-f5f5657e/spec.md

**入力**: ユーザー説明: "Issue 1028: Docker Image のビルドに失敗（too many colons）"

## 背景

- Windows で旧 gwt から移行したプロジェクトにおいて、Docker 起動時の bind mount 文字列が `too many colons` で失敗する。
- 現行実装は `HOST_GIT_COMMON_DIR` / `HOST_GIT_WORKTREE_DIR` を短縮記法でそのまま `source:target` 同値マウントしており、ドライブレター付きパスが target 側に入ると不正になり得る。

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - Docker 起動が失敗せず継続できる (優先度: P0)

Windows ユーザーとして、Build Images / Force Recreate を有効にして Launch しても、mount 文字列不正で失敗せず Docker 起動を継続したい。

**独立したテスト**: mixed path（`/Repository/...` と `D:/Repository/...`）で override 生成と mount 計画を検証し、不正な `source:target` 文字列が生成されないことを確認する。

**受け入れシナリオ**:

1. **前提条件** `HOST_GIT_COMMON_DIR=/Repository/repo.git` と `HOST_GIT_WORKTREE_DIR=D:/Repository/repo.git/worktrees/feature`、**操作** Docker override を生成、**期待結果** drive-letter を target に持つ不正 mount が生成されない。
2. **前提条件** worktree gitdir が common dir 配下、**操作** bind mount 計画を生成、**期待結果** 重複した worktree mount を追加しない。

---

### ユーザーストーリー 2 - 既存 Linux/macOS 挙動を壊さない (優先度: P1)

開発者として、通常の POSIX パス環境では既存の Docker 起動フローを維持したい。

**独立したテスト**: POSIX パス入力で既存どおりの mount target になることを単体テストで確認する。

**受け入れシナリオ**:

1. **前提条件** composeサービスが `/workspace` を持たない、**操作** Docker起動後に `docker compose exec` を実行、**期待結果** 固定 `-w /workspace` を強制せず起動できる。
---

## エッジケース

- `HOST_GIT_*` のいずれかが未設定の場合、override で空 mount を生成しない。
- バックスラッシュを含む Windows パスを受け取っても `/` 正規化して扱う。

## 要件 *(必須)*

### 機能要件

- **FR-001**: システムは Git 用 bind mount 生成時に、container target へ Windows ドライブレター付きパスを直接使用しない。
- **FR-002**: システムは common dir 配下にある worktree gitdir の重複マウントを追加しない。
- **FR-003**: システムは compose override を long syntax bind mount で生成し、短縮記法でのコロン解釈破綻を回避する。
- **FR-004**: システムは通常の compose 起動時に、workdir が未確定なら `docker compose exec` へ `-w` を付与しない。

### 非機能要件

- **NFR-001**: 追加ロジックは `crates/gwt-tauri/src/commands/terminal.rs` のユニットテストで再現ケースをカバーする。

## 制約と仮定

- Linux コンテナ前提（Windows コンテナは対象外）。
- 既存の Docker 起動フロー（compose up/exec/down）と UI 入力は変更しない。

## 成功基準 *(必須)*

- **SC-001**: Issue 1028 の再現条件（mixed path）で、`too many colons` を誘発する mount 記述が生成されない。
- **SC-002**: 追加したユニットテストが成功し、既存の関連テストも回帰しない。
