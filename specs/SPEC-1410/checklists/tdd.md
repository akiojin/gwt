### テスト: "no visibility gap when switching between ready terminal tabs"

初期化済みターミナルタブ間の双方向切り替えで、`rerender` 直後（`$effect` 実行前）の時点で `.terminal-wrapper.active` が常に 1 であることを検証する。

手順:
1. agent-1 を activeTabId として render → waitFor で ready 確認（1 active）
2. term-1 に切り替え → waitFor で ready 確認（1 active）
3. agent-1 に切り替え → 即座に .terminal-wrapper.active === 1（waitFor なし）
4. term-1 に切り替え → 即座に .terminal-wrapper.active === 1（waitFor なし）← 新規検証
