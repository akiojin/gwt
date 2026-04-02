# Tasks: SPEC-1642 — Docker 実行ターゲットとサービス検出

## Phase 1: Scope

- [ ] T001: `SPEC-1552` と重複する lifecycle / 監視要件を削り、launch target 導線へ絞る。
- [ ] T002: compose / devcontainer 検出条件を明文化する。

## Phase 2: Implementation

- [ ] T003: サービス選択 UI と launch config の受け渡しを定義する。
- [ ] T004: Docker 失敗時の host fallback とエラー表示を定義する。

## Phase 3: Verification

- [ ] T005: Docker あり/なし/失敗の 3 系統を acceptance に落とし込む。
