# データモデル: Clean up merged PRs 機能

## エンティティ定義

### 1. MenuDisplay
**概要**: メインメニューの表示内容を管理するエンティティ

**フィールド**:
- `message`: string - メニューのプロンプトメッセージ
- `actions`: Array<MenuAction> - 利用可能なアクション一覧
- `choices`: Array<Choice> - 選択可能な項目一覧

**バリデーション**:
- `actions`は最低1つ以上のアクションを含む
- 各アクションのキーは一意である

### 2. MenuAction
**概要**: メニューで利用可能な各アクションを表現

**フィールド**:
- `key`: string - アクションを起動するキー（例: 'n', 'm', 'c', 'q'）
- `label`: string - アクションの表示ラベル
- `handler`: string - アクションハンドラーの識別子
- `enabled`: boolean - アクションが有効かどうか

**バリデーション**:
- `key`は単一文字
- `label`は空文字列不可
- `handler`は既知のハンドラー識別子

### 3. Choice
**概要**: メニューの選択可能な項目

**フィールド**:
- `name`: string - 表示名
- `value`: string - 選択値
- `description`: string | undefined - 説明文
- `disabled`: boolean | undefined - 無効化フラグ

**バリデーション**:
- `name`は空文字列不可
- `value`は一意

## 状態遷移

### MenuDisplay状態
```
初期化
  ↓
メニュー表示
  ↓
キー入力待機 ←─┐
  ↓            │
アクション実行  │
  ↓            │
処理完了 ───────┘
```

### MenuAction状態
```
未選択
  ↓
選択済み
  ↓
実行中
  ↓
完了/エラー
```

## リレーション

```
MenuDisplay
    │
    ├── 1..* MenuAction
    │
    └── 0..* Choice
```

## 制約事項

1. **一意性制約**:
   - MenuAction.keyは同一MenuDisplay内で一意
   - Choice.valueは同一MenuDisplay内で一意

2. **参照整合性**:
   - MenuAction.handlerは実装済みのハンドラーを参照する必要がある

3. **ビジネスルール**:
   - 無効化されたChoiceは選択できない
   - 無効化されたMenuActionは実行できない