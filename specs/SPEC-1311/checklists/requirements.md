# 要件チェックリスト: GUI Session Summary のスクロールバック要約（実行中対応）

**目的**: 仕様の網羅性・明確性・テスト可能性を確認する
**作成日**: 2026-02-12
**仕様**: `specs/SPEC-3a1b7c2d/spec.md`

**注記**: 本チェックリストは `/speckit.checklist` により生成されます。

## ユーザーストーリー

- [x] CHK001 US1 の受け入れシナリオが「Generating...」表示と要約完了まで観測可能
- [x] CHK002 US2 の受け入れシナリオが「最終出力が最新のpane」を明示している
- [x] CHK003 US3 の受け入れシナリオが session_id 確定後の既存フロー維持を明示している

## 機能要件

- [x] CHK004 FR-001/FR-002 が scrollback fallback と最新pane選定の責務を明確化している
- [x] CHK005 FR-003 が UI 表示（`Live (pane summary)`）と session_id 非表示を要求している
- [x] CHK006 FR-005 が読み取り専用であることを明示している

## 非機能/制約

- [x] CHK007 NFR-001 が UI 英語固定であることを規定している
- [x] CHK008 NFR-002 が ANSI 除去済みテキストを要求している
- [x] CHK009 制約に `capture_scrollback_tail` の入力範囲を明示している

## エッジケース

- [x] CHK010 scrollback 取得失敗時の挙動が要件/シナリオで説明されている
- [x] CHK011 pane の増減が激しい場合の対象選定が要件/制約に含まれている

## メモ

- 完了した項目は `[x]` でチェック
- 不足があれば spec.md を更新してから再チェック
