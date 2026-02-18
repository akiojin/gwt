# データモデル: OpenTUI 移行（CLI UI）

## データソース

- Git/Worktree 操作から得られるブランチ情報と状態
- CLI 実行時のメモリ内状態（画面遷移、選択、フィルタ）
- ログ/履歴の参照（read-only）

## 主要エンティティ（既存型に準拠）

- BranchInfo / BranchItem: ブランチ表示に必要なメタ情報
- WorktreeInfo / WorktreeConfig: worktree の状態と設定
- PullRequest / MergedPullRequest: PR 情報
- CleanupTarget / CleanupResult: クリーンアップ対象と結果
- ModelOption / CodingAgentId / InferenceLevel: モデル・エージェント選択

## UI 状態

- ScreenType / ScreenState / Screen: 画面遷移と可視状態
- SelectedBranchState: 選択中ブランチの詳細
- UIFilter: ブランチ一覧のフィルタ
- 一時入力状態（フィルタ入力、フォーム入力、選択位置）

## 関係/制約

- BranchItem は BranchInfo を拡張し、表示用の派生情報を保持する。
- WorktreeInfo は BranchInfo と関連し、ローカル/リモート表示の区別に影響する。
- PullRequest は BranchInfo と紐づき、表示上の状態（OPEN/MERGED 等）に影響する。
- UI 状態は ScreenType によって許容される入力/操作が変わる。

## 検証ルール（UI 層）

- ブランチ選択/削除など破壊操作は確認 UI を必須とする。
- フィルタ/入力は空入力を許容し、未入力時は既定値へフォールバックする。
- Windows ネイティブ環境での表示崩れや入力遅延が無いことを確認する。
