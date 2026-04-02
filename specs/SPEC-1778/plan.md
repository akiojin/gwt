# Plan: SPEC-1778 — 音声入力

## Summary

既存の voice runtime / settings / indicator を土台に、gwt-tui の end-to-end voice input 導線を完成させる。

## Technical Context

- `crates/gwt-core/src/voice/*`
- `crates/gwt-tui/src/input/voice.rs`
- `SPEC-1645` が Settings 導線を担当。

## Phased Implementation

### Phase 1: Inventory

1. 現在ある runtime / settings / indicator の実装範囲を固定する。

### Phase 2: End-to-End Flow

1. 録音開始・停止・文字起こし・PTY 送信の導線を定義する。

### Phase 3: Verification

1. マイク未検出と録音エラーを含む acceptance を定義する。
