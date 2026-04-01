> **🔄 TUI MIGRATION (SPEC-1776)**: This SPEC has been updated for the gwt-tui migration. Assistant Mode now operates as a PTY-based Assistant within gwt-tui Shell tabs.

# Assistant Send Interrupt and Tab Queue

## Background

gwt-tui の Assistant は PTY ベースの Shell タブ内で動作する。推論実行中に入力がブロックされると、ユーザーが次の指示を送れず作業フローが停滞する。この変更により、推論中でも即座に割り込み送信（Enter/Send）および自動フォローアップ配信のためのキュー送信（Tab）が可能になる。

## User Scenarios

### S1: Enter/Send interrupts current inference

- Priority: P0
- Given: Assistant が Shell タブ内で推論中
- When: ユーザーが Enter を押す
- Then: 現在の推論がキャンセルされ、新しいメッセージが即座に処理される

### S2: Tab queues a reply instead of interrupting

- Priority: P0
- Given: Assistant が Shell タブ内で推論中
- When: ユーザーが Tab キーで送信する
- Then: メッセージが FIFO キューに追加され、現在の推論完了後に自動送信される

### S3: Startup analysis does not block a real user send

- Priority: P0
- Given: Assistant のスタートアップ分析が実行中
- When: ユーザーがメッセージを送信する
- Then: スタートアップ分析が中断され、ユーザーメッセージが優先処理される

## Functional Requirements

- FR-001: Enter 送信は割り込み配信を使用し、新しいユーザーメッセージを即座に優先処理する
- FR-002: Tab キー送信はキュー配信を使用し、FIFO 順序を維持する
- FR-003: キューされたメッセージは推論完了後に自動送信される
- FR-004: キャンセルされた推論の結果が最新の状態を上書きしないようにする
- FR-005: スタートアップ分析はユーザー送信で中断可能とする
- FR-006: キューされたメッセージ数を TUI ステータスバーに表示する
- FR-007: 推論中でも入力フィールドを有効に保つ

## Non-Functional Requirements

- NFR-001: 割り込み送信は前の推論完了を待たずに即座に制御を返す
- NFR-002: キュー処理はメッセージの順序を変更しない
- NFR-003: キャンセルされた推論のツール結果やアシスタント応答がコミットされない

## Success Criteria

- SC-001: 推論中の Enter 送信が新しい推論を開始する
- SC-002: 推論中の Tab 送信がキューカウントを増加させ、FIFO で消化される
- SC-003: スタートアップ分析がユーザーメッセージで中断される
- SC-004: gwt-core / gwt-tui のテストがキャンセル/キュー動作をカバーし通過する
