# 実装計画: 全画面テキストコピー (Cmd+Shift+C)

**仕様ID**: `SPEC-0e6abbb7`

## 実装アーキテクチャ

### 全体フロー

```text
Cmd+Shift+C → Tauri Menu → "screen-copy" event → Frontend Handler
  → collectScreenText() → navigator.clipboard.writeText()
  → showCopyFlash() + showToast()
```

### バックエンド (Rust)

1. **menu.rs**: Edit メニューに "Copy Screen Text" (CmdOrCtrl+Shift+C) を追加
2. **app.rs**: `menu_action_from_id()` に `"screen-copy"` マッピングを追加

### フロントエンド (Svelte/TypeScript)

3. **screenCapture.ts** (新規): 画面テキスト収集ロジック
   - `collectScreenText()`: 各セクションのテキストを収集・構造化
   - サイドバー: DOM の可視テキスト取得
   - ターミナル: xterm.js buffer API で可視行取得 + ANSI除去
   - 非ターミナルパネル: DOM の可視テキスト取得
   - モーダル: 開いている場合はモーダルDOM のテキスト取得
   - ステータスバー: DOM の可視テキスト取得
   - メタデータ: ブランチ名、アクティブタブ名、ウィンドウサイズ

4. **App.svelte**: メニューアクションハンドラに `"screen-copy"` ケースを追加
   - `collectScreenText()` を呼び出し
   - `navigator.clipboard.writeText()` でコピー
   - フラッシュ + トースト表示

5. **CopyFlash.svelte** (新規 or App.svelte内): コピー時の視覚フィードバック
   - アクセントカラー半透明オーバーレイ (opacity 0 → 0.15 → 0)
   - CSS animation で 200ms

## 実装順序

```text
Phase 1: バックエンド (Rust)
  ├─ menu.rs: メニューアイテム追加
  └─ app.rs: アクション ID マッピング

Phase 2: テキスト収集ロジック (TypeScript)
  └─ screenCapture.ts: collectScreenText()

Phase 3: 統合 + 視覚フィードバック
  ├─ App.svelte: ハンドラ統合
  └─ フラッシュ + トースト
```

## 技術詳細

### xterm.js 可視行テキスト取得

```typescript
// xterm.js buffer API
const buffer = terminal.buffer.active;
const lines: string[] = [];
for (let i = buffer.viewportY; i < buffer.viewportY + terminal.rows; i++) {
  const line = buffer.getLine(i);
  if (line) lines.push(line.translateToString(true)); // trim trailing whitespace
}
```

### ANSI 除去

xterm.js の `translateToString()` はプレーンテキストを返すため、ANSI コードは自動的に除去される。

### 構造化テキスト生成

メタデータ（ブランチ名、タブ名、ウィンドウサイズ）は既存のSvelteステートから取得。

### フラッシュエフェクト

```css
@keyframes copy-flash {
  0% { opacity: 0; }
  30% { opacity: 0.15; }
  100% { opacity: 0; }
}
```

## リスク

- xterm.js の buffer API アクセスがコンポーネント外から可能かの確認が必要
  → TerminalView が terminal インスタンスを公開する仕組みを確認
- DOM からの可視テキスト取得が正確でない可能性
  → innerText は CSS による非表示要素を自動除外するため、大筋で正確
