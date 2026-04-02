# Tasks: SPEC-1776 — 旧TUI UX を基準にした ratatui TUI 再設計

## Phase 0: Parent SPEC Reset

- [x] T001: `SPEC-1776` を「全面移植仕様」から「parent UX spec」へ書き換える
- [x] T002: `research.md` に cross-spec comparison matrix を追加する
- [x] T003: `data-model.md` / `quickstart.md` を新しい shell model に合わせて更新する
- [x] T004: 主要 sync 対象 SPEC (`SPEC-1654`, `SPEC-1770`, `SPEC-1777`, `SPEC-1782`) の差分メモを作る
- [x] T005: workflow / persistence / integration 監査対象 (`SPEC-1579`, `SPEC-1787`, `SPEC-1714`, `SPEC-1354`, `SPEC-1643`, `SPEC-1786`, `SPEC-1542`, `SPEC-1656`, `SPEC-1648`, `SPEC-1646`, `SPEC-1644`, `SPEC-1636`, `SPEC-1779`) を `sync required / reference only / deferred` に分類する
- [x] T006: `gwt-spec-ops` 系 skill contract が `SPEC-1579` / `SPEC-1787` 側で parent UX と矛盾しないか監査メモを残す

## Phase 1: Branch-First Entry

- [x] T100: [TDD] `Branches` が primary entry として振る舞う state tests を追加する
- [x] T101: ブランチ行の session count 表示を実装する
- [x] T102: `Enter` の 3 分岐 (`no session / one session / many sessions`) を実装する
- [x] T103: `many sessions` 時の selector UI を実装する

## Phase 2: Permanent Multi-Mode Session Workspace

- [x] T200: [TDD] `equal grid` layout model の tests を追加する
- [x] T201: `4件以上` を前提にした session grid を実装する
- [x] T202: focus session の maximize toggle を実装する
- [x] T203: maximize 時の tab switch を実装する
- [x] T204: 管理画面開閉後の layout restore を実装する
- [x] T205: `hidden pane` 依存を削除する

## Phase 3: Management Workspace Core

- [x] T300: 管理画面タブを `Branches / SPECs / Issues / Profiles` に整理する
- [x] T301: `SPECs` 一覧・詳細・launch entry を parent flow に同期する
- [x] T302: `Issues` 一覧・詳細・launch entry を parent flow に同期する
- [x] T303: `Profiles` タブを env profile 専用 UI として作り直す
- [x] T304: `Profiles` で env profile の作成・編集・削除・切替を実装する
- [x] T305: `Profiles` で OS 環境変数参照・置換を実装する

## Phase 4: Launch Flow Integration

- [x] T400: branch enter selector と `Quick Start` の接続方針をテストで固定する
- [x] T401: `既存へ入る / 追加起動 / フルWizard` を実装する
- [x] T402: hooks confirm (`SPEC-1786`) を新 launch flow に再接続する
- [x] T403: skill registration と session persistence を新 launch flow に再接続する

## Phase 5: Child SPEC Synchronization

- [x] T500: `SPEC-1654` を新 shell model に同期する
- [x] T501: `SPEC-1770` を新 shortcut / layout policy に同期する
- [x] T502: `SPEC-1777` を parent navigation に同期する
- [x] T503: `SPEC-1782` を `1ブランチ = Nセッション` 前提へ同期する
- [x] T504: `SPEC-1579` / `SPEC-1787` の workflow entry contract を監査し、必要なら wording を同期する
- [x] T505: `SPEC-1714` / `SPEC-1354` / `SPEC-1643` の Issue list/detail/linkage contract を監査する
- [x] T506: `SPEC-1786` の hooks confirm が新 launch flow と矛盾しないか確認する
- [x] T507: `SPEC-1542` / `SPEC-1656` の `Profiles = Env profiles` wording を同期する
- [x] T508: `SPEC-1644` / `SPEC-1646` / `SPEC-1648` / `SPEC-1636` / `SPEC-1779` を reference-only 監査し、親SPECとの矛盾有無を記録する

## Deferred

- [ ] T900: `Settings` を戻す
- [x] T901: `Logs` を戻す
- [x] T902: `Versions` を戻す
- [ ] T903: `AI summary` を再導入する
- [ ] T904: custom agent UI を再設計する
