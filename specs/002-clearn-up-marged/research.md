# 調査結果: Clean up merged PRs 機能の不具合

## 調査概要
「Clean up merged PRs」機能の動作不良に関する調査を実施しました。

## 現状分析

### 1. メニュー表示と実装の不整合
**決定**: メニュー表示に「(a) Account management」と記載されているが、実際の'a'キーハンドラーが未実装
- **根拠**: `src/ui/prompts.ts`の199行目でメニューに「(a) Account management」を表示しているが、キーハンドラーが存在しない
- **代替案**: 
  1. 'a'キーハンドラーを実装する
  2. メニューから「(a) Account management」を削除する

### 2. 'c'キーの処理は正常
**決定**: 'c'キーによる「Clean up merged PRs」機能自体は正しく実装されている
- **根拠**: 
  - `selectBranchWithShortcuts`関数の125-129行目で'c'キーを正しく処理
  - `handleCleanupMergedPRs`関数（786-900行目）が適切に実装されている
- **代替案**: なし（正常動作）

### 3. 機能の処理フロー
**決定**: Clean up merged PRs機能の処理フローは以下の通り
1. GitHub CLIの可用性チェック
2. GitHub認証チェック
3. リモートから最新の変更を取得
4. マージ済みPRのworktreeを検出
5. クリーンアップ対象を選択
6. クリーンアップ実行（worktree削除、ブランチ削除、必要に応じてリモートブランチ削除）

## 技術的詳細

### 関連ファイル
- `src/ui/prompts.ts`: メニュー表示とキーハンドリング
- `src/index.ts`: メインループとCleanup処理
- `src/worktree.ts`: worktree操作関連
- `src/github.ts`: GitHub API連携
- `src/ui/display.ts`: 表示関連
- `src/ui/types.ts`: 型定義

### 依存関係
- GitHub CLI（gh）: マージ済みPRの検出に必要
- Git: worktreeとブランチ管理
- inquirer/core: インタラクティブUI

## 修正方針

### 推奨される修正
1. **メニュー表示の修正**: 「(a) Account management」を削除
   - 理由: 機能が未実装であり、ユーザーの混乱を招く
   - 影響範囲: 最小限（表示のみ）

### 代替案
1. **Account management機能の実装**
   - 利点: メニューの一貫性を保つ
   - 欠点: 追加開発が必要、仕様が不明確

## 結論
「Clean up merged PRs」機能自体は正常に動作しており、問題はメニュー表示の不整合にあります。未実装の「(a) Account management」への言及を削除することで、ユーザーエクスペリエンスを改善できます。