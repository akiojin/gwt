# クイックスタート: プロジェクトモード（Project Mode）

**仕様ID**: `SPEC-ba3f610c` | **日付**: 2026-02-27

## 前提

- GUI版 gwt が起動できること
- AI設定が有効であること（endpoint + model が設定済み）
- Worker用エージェント（Claude Code等）がインストール済みであること（ペルソナ設定は任意）
- `gh` CLI が認証済みであること（GitHub Issue/PR機能を使う場合）

## 基本フロー

### 1. プロジェクトモードを開く

1. gwt GUI を起動
2. タブバーの `Project Mode` を選択
3. Leadチャット画面が表示される（右カラム）+ ダッシュボード（左カラム）

### 2. プロジェクトを開始する

1. Leadチャットに要件を入力（例: "認証機能とダッシュボードを実装して"）
2. Leadがclarify質問をする → 回答する
3. LeadがGitHub Issueに仕様を記録（spec → plan → tasks → tdd）
4. Leadが計画全体を提示する
5. 承認する（Enter or "y"）

### 3. 自律実行を見守る

- Leadが各IssueにCoordinatorを起動
- CoordinatorがWorkerを起動し実装を開始
- ダッシュボード（左カラム）でIssue/Task/Workerの進捗を確認
- Leadが2分間隔で進捗をチャットに報告

### 4. 結果を確認する

- テストパス → PR自動作成
- CI失敗 → 自律修正ループ（最大3回）
- 全Issue完了 → プロジェクト完了

## 手動テスト手順

### Project Modeタブ切り替え

1. Branch Modeタブが表示されている状態で `Project Mode` タブをクリック
2. ダッシュボード + Leadチャット画面が表示される
3. 別タブ（Settings等）をクリック → Branch Modeに戻る

### Leadチャット

1. チャット入力欄にテキストを入力
2. Enter → 送信（送信ボタンにスピナー表示）
3. Shift+Enter → 改行（送信されない）
4. IME変換中のEnter → 送信されない

### ダッシュボード操作

1. Issue行をクリック → 展開（Task/Worker/Coordinator詳細）
2. Task行をクリック → Branch Modeの該当Worktreeにジャンプ
3. `[View Terminal]` → CoordinatorターミナルをBranch Mode側で表示

### AI設定未構成

1. AI設定を無効にしてProject Modeタブを開く
2. エラーメッセージ "AI settings are required" が表示される
3. 設定ウィザードへの導線が表示される
