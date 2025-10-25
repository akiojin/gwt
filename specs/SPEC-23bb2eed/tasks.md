---
description: "SPEC-23bb2eed実装のためのタスクリスト: semantic-release自動リリース機能"
---

# タスク: semantic-releaseによる自動リリース機能の実装

**入力**: `/specs/SPEC-23bb2eed/` からの設計ドキュメント
**前提条件**: plan.md、spec.md

**構成**: タスクは実装の論理的な順序でグループ化され、各段階の独立した実装とテストを可能にします。

## フォーマット: `- [ ] [ID] [P?] [ストーリー?] 説明 (ファイルパス)`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（例: US1、US2、US3）
- 説明に正確なファイルパスを含める

## 背景

**問題**: semantic-release関連パッケージが未インストールのため、mainマージ時の自動リリースが動作していない

**実装内容**:
1. package.jsonにsemantic-release依存パッケージを追加
2. release.ymlから冗長なpublish-npmジョブを削除
3. 依存関係のインストールと動作確認
4. テストコードの実装

**目的**: コミットメッセージベースの自動バージョン決定とリリースを実現する

## フェーズ1: セットアップ（現状確認）

**目的**: 現在の設定を確認し、実装の準備を整える

### 環境確認タスク

- [x] T001 [P] .releaserc.json の存在確認（プロジェクトルート/.releaserc.json）
- [x] T002 [P] release.yml の現在の設定確認（.github/workflows/release.yml）
- [x] T003 [P] package.jsonの現在の依存関係確認（package.json）

## フェーズ2: ユーザーストーリー1 - semantic-release依存パッケージの追加 (優先度: P1)

**ストーリー**: 開発者として、semantic-releaseが正常に動作するために必要なパッケージをインストールしたい。

**価値**: 必要な依存パッケージがインストールされることで、自動リリース機能が動作可能になる

**独立したテスト**: package.jsonに依存が追加され、bun installで正常にインストールされることを確認する。

### 依存パッケージの追加

- [x] T101 [US1] package.jsonのdevDependenciesにsemantic-release本体を追加（package.json）
  - semantic-release: ^24.2.0 を追加

- [x] T102 [US1] package.jsonにsemantic-releaseプラグインを追加（package.json）
  - @semantic-release/commit-analyzer: ^13.0.0
  - @semantic-release/release-notes-generator: ^14.0.1
  - @semantic-release/changelog: ^6.0.3
  - @semantic-release/npm: ^12.0.1
  - @semantic-release/git: ^10.0.1
  - @semantic-release/github: ^11.0.1

### 構文確認

- [ ] T103 [US1] package.jsonのJSON構文確認（package.json）
  - JSON形式が正しいことを確認
  - すべての依存パッケージが正しく記述されていることを確認

**✅ MVP1チェックポイント**: US1完了後、semantic-release関連パッケージがpackage.jsonに追加される

## フェーズ3: ユーザーストーリー2 - release.ymlの最適化 (優先度: P1)

**ストーリー**: 開発者として、semantic-releaseがnpm公開を担当するため、冗長なpublish-npmジョブを削除したい。

**価値**: ワークフローがシンプルになり、二重公開のリスクが排除される

**独立したテスト**: release.ymlが正しく更新され、releaseジョブのみが存在することを確認する。

### ワークフロー修正

- [ ] T201 [US2] release.ymlからpublish-npmジョブを削除（.github/workflows/release.yml）
  - 54行目から81行目を削除
  - releaseジョブのみを維持

### 構文確認

- [ ] T202 [US2] release.ymlのYAML構文確認（.github/workflows/release.yml）
  - YAML形式が正しいことを確認
  - インデントが適切であることを確認

**✅ MVP2チェックポイント**: US2完了後、release.ymlが最適化される

## フェーズ4: ユーザーストーリー3 - 依存関係のインストールと動作確認 (優先度: P1)

**ストーリー**: 開発者として、追加した依存パッケージが正常にインストールされ、ビルドとテストが成功することを確認したい。

**価値**: 変更が既存の機能を破壊していないことを保証する

**独立したテスト**: bun install、bun run build、bun run testが成功することを確認する。

### 依存関係のインストール

- [ ] T301 [US3] bun installで依存パッケージをインストール（プロジェクトルート）
  - semantic-release関連パッケージがnode_modules/にインストールされることを確認
  - エラーがないことを確認

### ビルドとテストの実行

- [ ] T302 [US3] bun run buildでビルド成功確認（プロジェクトルート）
  - dist/ディレクトリが生成されることを確認
  - TypeScriptのコンパイルエラーがないことを確認

- [ ] T303 [US3] bun run testでテスト成功確認（プロジェクトルート）
  - 既存のテストスイート（122テスト）が成功することを確認
  - カバレッジが低下していないことを確認

**✅ MVP3チェックポイント**: US3完了後、すべての依存関係が正常にインストールされ、ビルドとテストが成功する

## フェーズ5: ユーザーストーリー4 - ワークフロー動作確認用のテストコード実装 (優先度: P2)

**ストーリー**: 開発者として、semantic-releaseが正常に動作することを自動テストで確認したい。

**価値**: CI/CDパイプラインでの自動検証により、リリースプロセスの信頼性が向上する

**独立したテスト**: テストコードが実装され、semantic-releaseの設定が正しいことを確認する。

### テストコード実装

- [ ] T401 [P] [US4] .releaserc.jsonの読み込みテストを実装（tests/release-config.test.ts）
  - .releaserc.jsonが存在することを確認
  - JSON形式が正しいことを確認
  - 必須フィールド（branches、plugins）が存在することを確認

- [ ] T402 [P] [US4] semantic-releaseプラグインの存在確認テストを実装（tests/release-config.test.ts）
  - package.jsonにsemantic-releaseが含まれることを確認
  - 6つのプラグインがすべて含まれることを確認

### テストの実行

- [ ] T403 [US4] 新規テストの実行確認（プロジェクトルート）
  - bun run testで新規テストが成功することを確認
  - カバレッジが向上していることを確認

**✅ MVP4チェックポイント**: US4完了後、semantic-release設定の自動検証が可能になる

## フェーズ6: 最終確認とコミット

**目的**: すべての変更を統合し、mainブランチへのマージ準備を整える

### 最終検証

- [ ] T501 [最終] すべての変更ファイルの確認
  - package.json が更新されていることを確認
  - release.yml が更新されていることを確認
  - テストコードが追加されていることを確認（オプション）

- [ ] T502 [最終] bun run buildの最終確認
  - エラーなくビルドが完了することを確認

- [ ] T503 [最終] bun run testの最終確認
  - すべてのテスト（既存+新規）が成功することを確認

### コミットと完了

- [ ] T504 [最終] 変更をステージングして確認
  - git statusで変更ファイルを確認
  - package.json、release.yml、bun.lockb（または類似のロックファイル）が変更されていることを確認
  - 意図しないファイル変更がないことを確認

- [ ] T505 [最終] Conventional Commits形式でコミット
  - コミットメッセージ: `feat: semantic-release自動リリース機能を実装`
  - 本文に変更内容の詳細を記載：
    - semantic-release関連パッケージをdevDependenciesに追加
    - release.ymlからpublish-npmジョブを削除
    - mainマージ時にfeat/fix/BREAKING CHANGEで自動リリース
  - Co-Authored-By: Claude <noreply@anthropic.com> を含める

- [ ] T506 [最終] ブランチにプッシュしてPR作成
  - git push origin hotfix/auto-release でブランチプッシュ
  - PRを作成してCIの成功を確認
  - レビュー依頼（該当する場合）

- [ ] T507 [最終] mainブランチにマージして自動リリースを確認
  - PRをmainにマージ
  - GitHub Actionsでreleaseワークフローが自動実行されることを確認
  - semantic-releaseがコミットを分析することを確認
  - （リリース対象コミットがある場合）npmとGitHub Releasesに公開されることを確認

## タスク凡例

**優先度**:
- **P1**: 最も重要 - MVPに必要（US1-US3）
- **P2**: 重要 - テスト自動化（US4）
- **最終**: 完了とデプロイメント

**依存関係**:
- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **依存あり**: 前のタスク完了後に実行

**ストーリータグ**:
- **[US1]**: ユーザーストーリー1 - semantic-release依存パッケージの追加
- **[US2]**: ユーザーストーリー2 - release.ymlの最適化
- **[US3]**: ユーザーストーリー3 - 依存関係のインストールと動作確認
- **[US4]**: ユーザーストーリー4 - ワークフロー動作確認用のテストコード実装
- **[最終]**: 最終確認とコミット

## 依存関係グラフ

```text
Phase 1 (Setup) - 現状確認
    ↓
Phase 2 (US1: P1) - package.json更新 ←┐
    ↓                                  │ (並列実行可能)
Phase 3 (US2: P1) - release.yml修正 ←┘
    ↓
Phase 4 (US3: P1) - 依存インストールと動作確認
    ↓
Phase 5 (US4: P2) - テストコード実装（オプション）
    ↓
Phase 6 (最終確認) - コミット＆プッシュ
```

**独立性**: US1とUS2は並列実施可能（異なるファイルを変更）

## 並列実行の機会

### フェーズ1（Setup）での並列実行
- T001, T002, T003（すべて並列実行可能）

### フェーズ2-3（US1-US2）での並列実行
- T101-T103（package.json更新）
- T201-T202（release.yml修正）
※異なるファイルのため並列実施可能

### フェーズ5（US4）での並列実行
- T401, T402（テストコード実装は複数ファイルに分割可能）

## 実装戦略

**MVPファースト**: US1+US2+US3でMVPを構成
- package.jsonとrelease.ymlの更新で自動リリース機能が動作可能
- ビルドとテストの成功で既存機能の維持を保証

**インクリメンタルデリバリー**:
1. **MVP（US1+US2+US3）**: semantic-releaseが動作する最小限の変更
2. **完全版（MVP+US4）**: テスト自動化でリリースプロセスの信頼性を保証

## 変更されるファイル

### 更新
- `package.json` - semantic-release関連パッケージをdevDependenciesに追加
- `.github/workflows/release.yml` - publish-npmジョブを削除
- `bun.lockb` （または類似のロックファイル） - 依存関係のロック

### 新規作成（オプション）
- `tests/release-config.test.ts` - semantic-release設定の自動検証テスト

### 参照のみ（変更なし）
- `.releaserc.json` - 既存の設定を使用（変更不要）

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
- **スキップしたタスク**: 理由と共に文書化

## 注記

- 各タスクは15分から1時間で完了可能であるべき
- より大きなタスクはより小さなサブタスクに分割
- ファイルパスは正確で、プロジェクト構造と一致させる
- 各ストーリーは独立してテスト・デプロイ可能
- テストは既存のテストスイート（122テスト）を維持

## 検証チェックリスト

タスク完了後、以下を確認：

- [x] package.jsonにsemantic-release関連パッケージが追加されている
- [ ] release.ymlからpublish-npmジョブが削除されている
- [ ] bun installが成功する
- [ ] bun run buildが成功する
- [ ] bun run testが成功する（既存の122テスト）
- [ ] （オプション）テストコードが実装され、設定が検証される
- [ ] Conventional Commits形式でコミットされている
- [ ] mainマージ時にsemantic-releaseが実行される

## 成功基準

このタスクリストの完了により、以下の成功基準を達成します：

1. ✅ semantic-release関連パッケージがインストールされる
2. ✅ release.ymlがシンプルかつ適切に設定される
3. ✅ mainマージ時にコミットメッセージから自動バージョン決定が行われる
4. ✅ npm registryとGitHub Releasesに自動公開される
5. ✅ CHANGELOG.mdとpackage.jsonが自動更新される
6. ✅ 既存のテストスイートがすべてパスする

## 参考資料

- [spec.md](./spec.md) - 機能仕様
- [plan.md](./plan.md) - 実装計画と技術コンテキスト
- [.releaserc.json](../../.releaserc.json) - semantic-release設定
- [semantic-release公式ドキュメント](https://semantic-release.gitbook.io/)
- [Conventional Commits仕様](https://www.conventionalcommits.org/)
