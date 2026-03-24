### Background

Assistant Mode を開いても、Assistant が自律的に何も始めない。現状の `assistant_start` は engine を作って state に格納するだけで、初回分析を実行しない。そのため、ユーザーが最初のメッセージを送るまで Assistant transcript は空のままで、project open 直後の「参謀」として機能しない。

### User Scenarios

**S1: Assistant tab を開いた直後の初回分析**
- Given: project が開かれていて AI 設定が有効
- When: Assistant session を開始する
- Then: Assistant が project 状態の初回分析を実行し、最初の guidance message を transcript に出す

### Requirements

- **FR-001**: `assistant_start` は Assistant engine 生成だけで終わらず、初回分析を実行すること
- **FR-002**: 初回分析では assistant から最初の guidance message が transcript に追加されること
- **FR-003**: 初回分析の内部プロンプトは transcript に表示しないこと
- **FR-004**: 既存の手動メッセージ送信フローを壊さないこと

### Success Criteria

- **SC-001**: Assistant tab 初回表示後、ユーザー入力なしでも最初の assistant message が表示される
- **SC-002**: 初回分析の内部プロンプトは transcript に露出しない
- **SC-003**: 対象の Rust / frontend tests が追加または更新され通過する
