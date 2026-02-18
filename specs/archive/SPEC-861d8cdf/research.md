# 調査メモ: エージェント状態の可視化（Hook再登録の自動化）

**仕様ID**: `SPEC-861d8cdf`
**日付**: 2026-01-21

## 既存挙動の整理

- Hook登録/解除は `crates/gwt-core/src/config/claude_hooks.rs` に集約されている
- 登録関数は「gwt hook が既に存在する場合は追加しない」ため、既存のgwtパスは更新されない
- Hook未登録時の提案は `crates/gwt-cli/src/tui/app.rs` の起動フローで行われ、tmux multi のときのみ確認ダイアログが出る
- settings.json は新/旧フォーマットの両方を扱うため、再登録の際も互換性を維持する必要がある

## 技術的判断

- 起動時の再登録は「既存のgwt hookを除去 → 追加」の順で実施するのが最短
- 非gwt hookを保持する必要があるため、解除処理はgwt hookのみに限定する

## リスク

- settings.json 書き込み不可時に再登録が失敗する可能性がある
- 既存hook配列の重複・混在を正しく扱わないと設定破損の恐れがある
