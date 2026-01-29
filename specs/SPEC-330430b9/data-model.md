# データモデル: セッション変換一覧の内容表示とSpaceプレビュー

## 追加/変更エンティティ

### ConvertSessionEntry (既存)

- **目的**: セッション変換一覧の表示用データ
- **追加/更新する属性**:
  - `first_user_message: Option<String>`
  - `display: String`（`<snippet> | updated YYYY-MM-DD HH:MM` を格納）

### WizardState (既存)

- **目的**: Wizard UI 状態の保持
- **追加/更新する属性**:
  - `convert_preview_open: bool`
  - `convert_preview_lines: Vec<String>`
  - `convert_preview_scroll: u16`
  - `convert_preview_error: Option<String>`
  - `convert_preview_view_height: u16`（スクロール上限計算用）

## 生成ルール

- `display` は開始ユーザーメッセージ抜粋と更新日時のみを含む。
- 抜粋取得に失敗した場合は `Unavailable`、ユーザーメッセージが無い場合は `No user message`。
- プレビューは先頭10メッセージ（User/Assistant）を ASCII ラベルで整形して `convert_preview_lines` に格納。
