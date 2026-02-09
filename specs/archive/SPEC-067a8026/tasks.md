# タスク定義: LLMベースリリースワークフロー

**仕様ID**: `SPEC-067a8026`
**作成日**: 2026-01-21

## 実装タスク一覧

### Phase 1: 環境準備

#### Task 1.1: cliff.toml作成

**依存**: なし
**成果物**: `/cliff.toml`
**内容**:
- Conventional Commits解析ルールの設定
- バージョンbumpルール設定（chore/docsでもpatch）
- CHANGELOG出力フォーマット設定
- タグパターン設定（`v*`）

**受け入れ条件**:
- `git-cliff --bumped-version`でバージョンが正しく判定される
- `git-cliff --unreleased`でCHANGELOG差分が正しく生成される

---

#### Task 1.2: Dockerfile更新

**依存**: なし
**成果物**: Dockerfile（または開発環境設定ファイル）の更新
**内容**:
- `cargo install git-cliff`を追加

**受け入れ条件**:
- 開発環境で`git-cliff`コマンドが利用可能

---

### Phase 2: スキル実装

#### Task 2.1: /releaseスキル基本構造

**依存**: Task 1.1
**成果物**: `.claude/commands/release.md`の更新
**内容**:
- 新しいスキル定義（処理フロー記述）
- ブランチ確認ロジック
- エラーハンドリング方針

**受け入れ条件**:
- `/release`実行でスキルが認識される

---

#### Task 2.2: ブランチ確認・事前チェック

**依存**: Task 2.1
**成果物**: スキル内の事前チェックロジック
**内容**:
- 現在ブランチがdevelopか確認
- main同期チェック（`git merge-base --is-ancestor`）
- 既存リリースコミット確認

**受け入れ条件**:
- develop以外でエラー
- main未同期でエラー
- 既存リリースコミットでエラー

---

#### Task 2.3: バージョン判定

**依存**: Task 1.1, Task 2.2
**成果物**: スキル内のバージョン判定ロジック
**内容**:
- `git-cliff --bumped-version`実行
- バージョン取得・検証

**受け入れ条件**:
- feat→minor, fix→patch, !→majorが正しく判定される
- chore/docsのみでもpatchが返る

---

#### Task 2.4: ファイル更新

**依存**: Task 2.3
**成果物**: スキル内のファイル更新ロジック
**内容**:
- Cargo.toml更新（ルート + crates/配下）
- package.json更新
- Cargo.lock更新（`cargo update -w`）
- CHANGELOG.md更新（`git-cliff --unreleased --prepend`）

**受け入れ条件**:
- すべてのファイルが正しいバージョンに更新される
- CHANGELOG.mdに今回リリース分が追加される

---

#### Task 2.5: コミット・プッシュ

**依存**: Task 2.4
**成果物**: スキル内のコミット・プッシュロジック
**内容**:
- `chore(release): vX.Y.Z`形式でコミット
- developへプッシュ（リトライ付き）

**受け入れ条件**:
- コミットが`chore(release):`形式
- プッシュ失敗時に最大3回リトライ

---

#### Task 2.6: PR作成・更新

**依存**: Task 2.5
**成果物**: スキル内のPR作成ロジック
**内容**:
- 既存PR確認（`gh pr list --base main --head develop`）
- 既存PRがなければ新規作成
- `release`ラベル付与
- PR bodyはLLMが生成

**受け入れ条件**:
- 既存PRがあれば新規作成しない
- PRに`release`ラベルが付与される

---

### Phase 3: クリーンアップ

#### Task 3.1: prepare-release.yml削除

**依存**: Task 2.6（全スキル実装完了後）
**成果物**: `.github/workflows/prepare-release.yml`の削除
**内容**:
- ワークフローファイルを削除

**受け入れ条件**:
- ファイルが削除されている

---

## 依存関係図

```
Task 1.1 (cliff.toml) ─────────────┐
                                   │
Task 1.2 (Dockerfile) ─────────────┤
                                   │
                                   ▼
                          Task 2.1 (スキル基本)
                                   │
                                   ▼
                          Task 2.2 (事前チェック)
                                   │
                                   ▼
                          Task 2.3 (バージョン判定)
                                   │
                                   ▼
                          Task 2.4 (ファイル更新)
                                   │
                                   ▼
                          Task 2.5 (コミット・プッシュ)
                                   │
                                   ▼
                          Task 2.6 (PR作成)
                                   │
                                   ▼
                          Task 3.1 (旧WF削除)
```

## 並列実行可能なタスク

- Task 1.1 と Task 1.2 は並列実行可能
