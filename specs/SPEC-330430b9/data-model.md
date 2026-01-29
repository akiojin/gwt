# データモデル: セッション変換一覧の内容表示とSpaceプレビュー

## 追加/変更エンティティ

### ConvertSessionEntry (既存)

- **目的**: セッション変換一覧の表示用データ
- **追加/更新する属性**:
  - `first_user_message: Option<String>`
  - `session_name: Option<String>`（Worktree名・ブランチ名相当）
  - `name_unavailable: bool`（Worktree名が取得できない場合のフラグ）
  - `display: String`（`<name> | <snippet> | updated YYYY-MM-DD HH:MM` を格納）

### WizardState (既存)

- **目的**: Wizard UI 状態の保持
- **追加/更新する属性**:
  - `convert_preview_open: bool`
  - `convert_preview_lines: Vec<String>`
  - `convert_preview_scroll: u16`
  - `convert_preview_error: Option<String>`
  - `convert_preview_view_height: u16`（スクロール上限計算用）
  - `convert_in_progress: bool`
  - `convert_spinner_tick: usize`

## 生成ルール

- `display` はセッション名・開始ユーザーメッセージ抜粋・更新日時を含む。
- セッション名（Worktree名）が取得できない場合は `Unavailable`、未設定の場合は `No name` を表示する。
- 抜粋取得に失敗した場合は `Unavailable`、ユーザーメッセージが無い場合は `No user message`。
- プレビューは先頭10メッセージ（User/Assistant）を ASCII ラベルで整形して `convert_preview_lines` に格納。
