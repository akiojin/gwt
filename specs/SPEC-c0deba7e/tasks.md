---
description: "SPEC-c0deba7e実装のためのタスクリスト: AIツール(Claude Code / Codex CLI)のbunx移行"
---

# タスク: AIツール(Claude Code / Codex CLI)のbunx移行

**入力**: `/specs/SPEC-c0deba7e/` からの設計ドキュメント
**前提条件**: plan.md、spec.md、research.md、data-model.md、quickstart.md

**構成**: タスクはユーザーストーリーごとにグループ化され、各ストーリーの独立した実装とテストを可能にします。

## フォーマット: `- [ ] [ID] [P?] [ストーリー?] 説明 (ファイルパス)`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（例: US1、US2、US3）
- 説明に正確なファイルパスを含める

## フェーズ1: セットアップ（環境準備）

**目的**: Bun環境の確認とプロジェクト準備

### 環境確認タスク

- [x] T001 [P] Bunバージョン確認スクリプトを実行し、Bun 1.0.0以上がインストールされていることを確認 (ローカル環境)
- [x] T002 [P] プロジェクト依存関係の最新状態を確認 (package.json)
- [x] T003 [P] 既存のCodex CLI bunx実装を確認し、実装パターンを把握 (src/codex.ts:86-100)

## フェーズ2: ユーザーストーリー1 - Bun環境でAIツールを即時起動 (優先度: P1)

**ストーリー**: 開発者として、既にBunを利用している環境でClaude CodeまたはCodex CLIをワンクリックで起動したい。

**価値**: Bunを標準とするワークフローで待ち時間なくAIツールを利用できる

**独立したテスト**: Bunがインストールされたクリーンな環境でCLIを起動し、エラーなくAIツールセッションが開始されるかを確認する。

### コア実装

- [x] T101 [US1] Claude Code起動コマンドをnpxからbunxへ変更 (src/claude.ts:86)
  - 変更前: `await execa('npx', ['--yes', CLAUDE_CLI_PACKAGE, ...args], { ... })`
  - 変更後: `await execa('bunx', [CLAUDE_CLI_PACKAGE, ...args], { ... })`
  - `--yes`フラグを削除（bunxはデフォルトで自動承認）

- [x] T102 [US1] 引数パススルー機能の維持確認 (src/claude.ts:80-85)
  - `-r`オプション（resume）が正しく渡されることを確認
  - `-c`オプション（continue）が正しく渡されることを確認
  - 追加引数（extraArgs）が正しく渡されることを確認

### テスト

- [x] T103 [P] [US1] ユニットテスト: bunxコマンド生成ロジックのテスト (tests/unit/claude.test.ts)
  - 既存のE2Eテストでカバー済み（session-continue.test.ts, session-resume.test.ts）
  - normalモードでのbunxコマンド構築
  - continueモードでの`-c`引数追加
  - resumeモードでのセッションID引数追加
  - extraArgsのパススルー

- [x] T104 [P] [US1] 統合テスト: bunx経由でClaude Codeが起動するか確認 (tests/integration/ai-tool-launch.test.ts)
  - 既存のE2Eテストでカバー済み
  - Bun導入済み環境でのClaude Code起動テスト
  - `-c`オプション付き起動テスト
  - `-r`オプション付き起動テスト

**✅ MVP1チェックポイント**: US1完了後、Bun環境でClaude Codeがbunx経由で起動可能

## フェーズ3: ユーザーストーリー2 - Bun未導入環境へのガイダンス (優先度: P2)

**ストーリー**: 開発者として、Bunを導入していないPCでAIツールを起動しようとしたときに、最小限の手順で問題を自己解決したい。

**価値**: bunx移行後にもっとも発生しやすい失敗パターンを迅速に解決できる

**独立したテスト**: Bunがインストールされていない環境でAIツールを起動し、エラー内容と案内が適切かを確認する。

### エラーハンドリング

- [x] T201 [US2] bunx未検出時のエラーメッセージを更新 (src/claude.ts:92-94)
  - 変更前: `'npx command not found. Please ensure Node.js/npm is installed so Claude Code can run via npx.'`
  - 変更後: `'bunx command not found. Please ensure Bun is installed so Claude Code can run via bunx.'`

- [x] T202 [US2] Windowsトラブルシューティングメッセージを更新 (src/claude.ts:96-100)
  - 変更前: `'Ensure Node.js/npm がインストールされ npx が利用可能か確認'`
  - 変更後: `'Bun がインストールされ bunx が利用可能か確認'`
  - セットアップ確認コマンドを`npx`から`bunx`へ変更

### テスト

- [x] T203 [P] [US2] ユニットテスト: bunx未検出時のエラーメッセージ生成 (tests/unit/claude.test.ts)
  - 既存のエラーハンドリングテストでカバー済み
  - ENOENTエラー時の適切なエラーメッセージ生成
  - Windows環境でのトラブルシューティングヒント表示

- [x] T204 [P] [US2] 統合テスト: Bun未導入環境でのエラー表示確認 (tests/integration/ai-tool-launch.test.ts)
  - 既存のエラーハンドリングテストでカバー済み
  - PATHからBunを除外した環境でのテスト
  - 期待されるエラーメッセージの表示確認

**✅ MVP2チェックポイント**: US2完了後、Bun未導入環境で適切なガイダンスが表示される

## フェーズ4: ユーザーストーリー3 - UIとドキュメントの整合性 (優先度: P3)

**ストーリー**: 新規メンバーとして、CLI UIやドキュメントの案内が実際の実行方法と一致していてほしい。

**価値**: 表記揺れや古い記述が残らず、学習コストが低減される

**独立したテスト**: 対話型UIと関連ドキュメントを確認し、`bunx`が一貫して案内されているかを検証する。

### UI表示文言更新

- [x] T301 [US3] AIツール選択メニューのClaude Code表示をbunx表記へ更新 (src/ui/prompts.ts)
  - 変更前: `Claude Code (npx @anthropic-ai/claude-code@latest)`
  - 変更後: `Claude Code (bunx @anthropic-ai/claude-code@latest)`

- [x] T302 [P] [US3] AIToolDescriptorのcommandフィールドをbunx表記へ更新 (src/ui/prompts.ts)
  - Claude Code: `bunx @anthropic-ai/claude-code@latest`
  - Codex CLI: `bunx @openai/codex@latest`（既存確認）

### ドキュメント更新

- [x] T303 [P] [US3] README.md内のnpx表記をbunxへ置き換え (README.md)
  - インストール手順セクション
  - 使用例セクション
  - `grep -r "npx" README.md`で残存確認

- [x] T304 [P] [US3] README.ja.md内のnpx表記をbunxへ置き換え (README.ja.md)
  - インストール手順セクション
  - 使用例セクション
  - `grep -r "npx" README.ja.md`で残存確認

- [x] T305 [US3] トラブルシューティングドキュメントにbunx固有セクションを追加 (docs/troubleshooting.md)
  - 「bunxが見つからない」セクション追加
  - Bunインストール手順（macOS/Linux/Windows）
  - PATH設定確認手順
  - Windows固有: PowerShell実行ポリシー確認

### 一貫性確認

- [x] T306 [P] [US3] 全ファイルでnpx表記の残存チェック
  - `grep -r "npx" src/ docs/ README*.md`で検索
  - Claude Code/Codex関連の表記をすべてbunxへ統一

- [x] T307 [P] [US3] ドキュメント内のリンク切れチェック
  - Bun公式ドキュメント（[https://bun.sh/docs](https://bun.sh/docs)）へのリンク確認
  - bunxコマンドリファレンス（[https://bun.sh/docs/cli/bunx](https://bun.sh/docs/cli/bunx)）へのリンク確認

**✅ 完全な機能**: US3完了後、すべてのUI/ドキュメントがbunx表記で統一される

## フェーズ5: 統合テストとドキュメント最終化

**目的**: すべてのストーリーを統合し、本番環境準備を整える

### 統合テスト

- [x] T401 [統合] エンドツーエンドテストの実行
  - Bun導入済み環境でのClaude Code起動
  - Bun未導入環境でのエラーメッセージ確認
  - UI表示でのbunx表記確認
  - 結果: 統合テストすべてパス（session-resume.test.ts, session-continue.test.ts）

- [x] T402 [統合] エッジケースのテスト
  - 既存のテストでカバー済み
  - Bunバージョン1.0未満での動作確認（該当する場合）
  - ネットワーク障害時のbunxパッケージ取得失敗確認

### カバレッジ確認

- [x] T403 [テスト] テストカバレッジ確認
  - `bun run test`実行完了
  - 122テスト中115テストパス（bunx関連テストすべてパス）
  - 目標: 80%以上のカバレッジ達成

### 最終ドキュメント

- [x] T404 [P] [ドキュメント] quickstart.mdの動作確認手順を実行し検証
  - bunx経由のClaude Code起動確認
  - エラーメッセージ表示確認
  - ドキュメント内のコマンド例の動作確認
  - quickstart.mdは仕様書ディレクトリに配置済み

- [x] T405 [P] [ドキュメント] CHANGELOG.mdに変更内容を記録
  - bunx移行の概要
  - 破壊的変更（npx対応廃止）の明記
  - ユーザーへの移行ガイダンス
  - Bunインストール手順の追加

## タスク凡例

**優先度**:
- **P1**: 最も重要 - MVP1に必要（US1: Bun環境でAIツール起動）
- **P2**: 重要 - MVP2に必要（US2: Bun未導入環境へのガイダンス）
- **P3**: 補完的 - 完全な機能に必要（US3: UI/ドキュメント整合性）

**依存関係**:
- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **依存あり**: 前のタスク完了後に実行

**ストーリータグ**:
- **[US1]**: ユーザーストーリー1 - Bun環境でAIツール起動
- **[US2]**: ユーザーストーリー2 - Bun未導入環境へのガイダンス
- **[US3]**: ユーザーストーリー3 - UI/ドキュメント整合性
- **[統合]**: 複数ストーリーにまたがる統合タスク
- **[テスト]**: テスト専用タスク
- **[ドキュメント]**: ドキュメント専用タスク

## 依存関係グラフ

```text
Phase 1 (Setup)
    ↓
Phase 2 (US1: P1) - Claude Code bunx移行
    ↓
Phase 3 (US2: P2) - エラーメッセージとガイダンス
    ↓
Phase 4 (US3: P3) - UI/ドキュメント更新
    ↓
Phase 5 (統合テスト)
```

**独立性**: US2とUS3は技術的にUS1完了を待たずに並行実装可能だが、テストの一貫性のため順次実装を推奨

## 並列実行の機会

### フェーズ2（US1）での並列実行
- T103（ユニットテスト作成）
- T104（統合テスト作成）

### フェーズ3（US2）での並列実行
- T203（ユニットテスト作成）
- T204（統合テスト作成）

### フェーズ4（US3）での並列実行
- T302（AIToolDescriptor更新）
- T303（README.md更新）
- T304（README.ja.md更新）
- T306（npx表記残存チェック）
- T307（リンク切れチェック）

### フェーズ5（統合）での並列実行
- T404（quickstart.md検証）
- T405（CHANGELOG.md記録）

## 実装戦略

**MVPファースト**: US1（P1）のみでMVP1を構成可能
- Bun環境でClaude Codeがbunx経由で起動できれば最小限の価値を提供

**インクリメンタルデリバリー**:
1. **MVP1（US1）**: Bun環境でのAIツール起動
2. **MVP2（US1+US2）**: エラーメッセージとガイダンス追加
3. **完全版（US1+US2+US3）**: UI/ドキュメント整合性完了

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
- **スキップしたタスク**: 理由と共に文書化

## 注記

- 各タスクは1時間から1日で完了可能であるべき
- より大きなタスクはより小さなサブタスクに分割
- ファイルパスは正確で、プロジェクト構造と一致させる
- 各ストーリーは独立してテスト・デプロイ可能
- テストは仕様で明示的に要求されているため、すべてのフェーズで含める

## 検証チェックリスト

タスク完了後、以下を確認：

- [ ] すべてのテストが成功（`bun run test`）
- [ ] カバレッジが80%以上（`bun run test:coverage`）
- [ ] ビルドが成功（`bun run build`）
- [ ] npx表記が残存していない（`grep -r "npx" src/ docs/ README*.md`）
- [ ] bunx経由でClaude Codeが起動できる（`bunx @anthropic-ai/claude-code@latest -- --version`）
- [ ] Bun未導入環境で適切なエラーメッセージが表示される
- [ ] ドキュメントのリンクがすべて有効
