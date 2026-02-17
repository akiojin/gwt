# 調査メモ: SPEC-8ad13230

**仕様ID**: `SPEC-8ad13230` | **日付**: 2026-02-17

## 決定事項

- Issue body には主要成果物（`spec/plan/tasks/tdd/research/data-model/quickstart`）を section として保持する。
- `contracts/` と `checklists/` はファイル単位管理が必要なため、Issue comment を artifact ストアとして使う。
- comment は marker を付与して識別する。
  - `<!-- GWT_SPEC_ARTIFACT:contract:<name> -->`
  - `<!-- GWT_SPEC_ARTIFACT:checklist:<name> -->`
- 既存運用との互換性維持のため、legacy 形式（`contract:<name>` / `checklist:<name>`）も読み取り可能にする。

## 代替案と却下理由

- body のみで contracts/checklists を全文管理: ファイル単位更新が困難で競合が増えるため却下。
- contract を append 専用に維持: 更新・削除不能で運用要件を満たさないため却下。

## リスク

- comment 取得件数（100件）を超えると一覧漏れの可能性がある。
- GraphQL mutation 失敗時に detail を取れない場合がある。

## 対応

- `etag` 比較で上書き競合を検知する。
- delete は idempotent（存在しない場合は `false`）にして安全側に倒す。
