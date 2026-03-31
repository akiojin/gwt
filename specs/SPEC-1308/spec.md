> **ℹ️ TUI MIGRATION NOTE**: This SPEC was completed during the gwt-tauri era. The gwt-tauri frontend has been replaced by gwt-tui (SPEC-1776). GUI-specific references are historical.

# 機能仕様: Claude Code Hooks 経由の gwt-tauri hook 実行で GUI を起動しない

**仕様ID**: `SPEC-1b98b6d7`
**作成日**: 2026-02-10
**ステータス**: ドラフト
**カテゴリ**: GUI
**依存仕様**: `SPEC-861d8cdf`（エージェント状態の可視化）
**入力**: ユーザー説明: "gwt を終了させると新しい gwt が立ち上がり、終了のたびに増殖する。CLI では問題なかったが GUI 化後に発生している。Hook が影響している可能性が高い。"
