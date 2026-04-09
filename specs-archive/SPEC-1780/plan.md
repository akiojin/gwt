# Plan: SPEC-1780 — ファイル貼り付け（クリップボードから PTY へ）

## Summary

通常のテキスト paste と分離した file clipboard 導線を設計し、OS ごとの取得方法と PTY 挿入形式を固定する。

## Technical Context

- `crates/gwt-tui/src/app.rs` の paste 経路
- `SPEC-1770` が input interaction を担当。
- macOS / Linux の clipboard adapter が必要。

## Phased Implementation

### Phase 1: Clipboard Contract

1. ファイル clipboard と通常テキスト clipboard の判定契約を定義する。

### Phase 2: OS Adapters

1. macOS と Linux の取得手段を定める。

### Phase 3: Verification

1. 単一/複数ファイル・text-only fallback を検証項目にする。
