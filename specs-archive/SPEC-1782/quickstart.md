# Quickstart: SPEC-1782 検証手順

## 前提

- `cargo build -p gwt-tui` が成功する
- `~/.gwt/sessions/` にブランチの履歴が存在する（過去にエージェントを起動済み）

## 最小検証フロー

### 1. Quick Start 表示確認

```
1. gwt を起動
2. Branches タブで履歴のあるブランチを選択
3. Enter を押す
4. → Quick Start ステップが表示される（session_id がある場合）
   → "Quick Start — {Agent名} ({Model})" のタイトル
   → Resume / Start New / Choose Different の 3 項目
```

### 2. Resume 確認

```
1. Quick Start で "Resume session" を選択
2. Enter を押す
3. → エージェントが --resume <id> 付きで即座に起動
4. → 全設定（model, version, skip_permissions 等）が復元
```

### 3. Start New 確認

```
1. Quick Start で "Start new session" を選択
2. Enter を押す
3. → エージェントが Normal モードで即座に起動
4. → 全設定が復元、session_id は渡されない
```

### 4. Choose Different 確認

```
1. Quick Start で "Choose different settings" を選択
2. Enter を押す
3. → BranchAction ステップに遷移（フルウィザード）
```

### 5. Quick Start 非表示確認

```
1. 履歴がない（または session_id がない）ブランチを選択
2. Enter を押す
3. → Quick Start をスキップし BranchAction から開始
```

### 6. session_id 検出確認

```
1. 新しいブランチでエージェントを起動
2. エージェントで数ターン会話
3. エージェントを終了
4. 同じブランチで再度 Enter
5. → Quick Start に session_id が表示される
```
