# 実装計画: Codex CLI skills フラグ互換

**仕様ID**: `SPEC-3b0ed29b` | **日付**: 2026-01-10 | **仕様書**: [spec.md](spec.md)
**入力**: `specs/SPEC-3b0ed29b/spec.md` の追記 (Codex CLI v0.80.0+ の skills フラグ無効化)

## 概要

- Codex CLI v0.80.0+ では `--enable skills` が未知フラグとなるため、起動引数から除外する。
- v0.79.x 以前では `--enable skills` を付与してスキルを利用可能にする。
- CLI/TUI起動とWeb UI起動の両方で同じ判定を適用する。

## 変更範囲

- `src/shared/codingAgentConstants.ts`: 互換判定ヘルパー追加
- `src/codex.ts`: バージョンに応じたフラグ付与
- `src/services/codingAgentResolver.ts`: installed/bunxでの付与切り替え
- `tests/unit/codex.test.ts`
- `tests/unit/codingAgentResolver.test.ts`
- `tests/unit/services/codingAgentResolver.test.ts`

## テスト戦略

- v0.80.0+ (latest) では `--enable skills` が含まれないこと
- installed v0.79.x では `--enable skills` が含まれること
- 既存の引数順序と起動経路が維持されること

## リスクと緩和

- **バージョン判定失敗**: 旧CLIでスキルが無効になる可能性
  - **緩和**: 判定不能時は付与しない方針を明記し、必要なら `extraArgs` で上書きできる
