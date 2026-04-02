# Quickstart: SPEC-1776 — Parent UX Validation

## Parent Artifact Validation

- `SPEC-1776/spec.md` が parent UX spec になっていること
- `research.md` に cross-spec comparison matrix があること
- `tasks.md` に child sync task が存在すること

## Manual Validation Flow for the Rebuild

### Branch-First Entry

- `cargo run -p gwt-tui`
- 起動直後に `Branches` が primary entry として見える
- ブランチ行に起動中セッション件数が表示される

### Enter Flow

- セッションが無いブランチで `Enter` すると `Wizard` が開く
- セッションが 1 件だけあるブランチで `Enter` すると、そのセッションへ入る
- セッションが複数あるブランチで `Enter` すると selector が開き、`既存へ入る / 追加起動 / フルWizard` を選べる

### Permanent Multi-Mode

- 4 件以上のセッションを起動し、通常時は均等グリッドで見える
- フォーカス中セッションを最大化できる
- 最大化時にタブで他セッションへ移動できる
- 管理画面を開閉しても、セッション領域は直前レイアウトへ戻る

### Management Tabs

- 管理画面は `Branches / SPECs / Issues / Profiles` の 4 タブを持つ
- `SPECs` と `Issues` は一覧・詳細・launch entry まで使える
- `Profiles` で env profile を作成・編集・削除・切替できる
- `Profiles` で OS 環境変数参照・置換が使える

### Integration Safety

- `Quick Start` が Branches flow から使える
- hooks confirm と skill registration が launch flow に残る
- `native PTY` の terminal behavior が維持される

## Verification Commands

- `cargo test -p gwt-core -p gwt-tui`
- `cargo clippy --all-targets --all-features -- -D warnings`
