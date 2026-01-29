# 調査: セッション変換一覧の内容表示とSpaceプレビュー

## 既存コード調査

- セッション変換UIは `crates/gwt-cli/src/tui/screens/wizard.rs` の Execution Mode: Convert に実装済み。
- 一覧表示は `ConvertSessionEntry.display` を使って描画している（`render_convert_session_select_step`）。
- キー処理は `crates/gwt-cli/src/tui/app.rs` の `handle_key_event` が中心。Wizard可視時はWizard専用のキー分岐が優先される。
- セッション解析は `crates/gwt-core/src/ai/session_parser/` にあり、`SessionParser::parse` が `ParsedSession.messages` を返す。
- `SessionListEntry` は `list_sessions` で取得可能だが、開始ユーザーメッセージは含まれない。

## 既存パターン/制約

- UI文言は英語のみ（CLAUDE.md）
- ASCIIのみのUI記号
- Wizardはポップアップ描画で `render_wizard` が中心

## 技術的決定

- 一覧の開始ユーザーメッセージは `SessionParser::parse` で取得する。
- プレビューはWizard内部のモーダル表示として追加する。
- キー処理はWizard表示中にSpace/Escを優先してハンドリングする。

## 依存関係・影響範囲

- 変更範囲は `gwt-cli` のWizard/UIと `app.rs` のキー処理に限定。
- `gwt-core` 側のAPI変更は不要（既存パーサーを利用）。
