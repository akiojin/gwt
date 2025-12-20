# PLAN.md - LSP対応の経緯と今後の対応

## 概要

gwt経由でClaude Codeを起動した際に、TypeScript LSP機能が動作しない問題の調査と修正。

## これまでの経緯

### 1. 問題の発見

- gwt経由でClaude Codeを起動した際、LSPツールが「No LSP server available for file type: .ts」エラーを返す
- 環境変数 `ENABLE_LSP_TOOL=1` が必要（Claude Code v2.0.74で導入）

### 2. 調査結果

#### 2.1 コードパスの特定

gwt には2つの異なるClaude Code起動経路がある：

| 経路 | ファイル | 用途 |
|------|----------|------|
| CLI | `src/claude.ts` → `launchClaudeCode()` | ターミナルからの直接起動 |
| Web UI | `src/services/aiToolResolver.ts` → `resolveClaudeCommand()` | Web UIからのセッション起動 |

#### 2.2 修正済み（Web UI経路）

- `src/services/builtin-tools.ts` の `CLAUDE_CODE_TOOL.env` に `ENABLE_LSP_TOOL: "1"` を追加
- `src/services/aiToolResolver.ts` が `builtin-tools.ts` の設定を使用

#### 2.3 修正済み（CLI経路）

- `src/claude.ts` の `launchClaudeCode()` 関数内で環境変数を追加：

```typescript
const baseEnv: Record<string, string | undefined> = {
  ...process.env,
  ...(options.envOverrides ?? {}),
  ENABLE_LSP_TOOL: "1", // Enable TypeScript LSP support in Claude Code
};
```

### 3. 現在の状態

#### 3.1 確認済み項目

| 項目 | 状態 | 備考 |
|------|------|------|
| `ENABLE_LSP_TOOL=1` 環境変数 | ✅ 設定済み | `echo $ENABLE_LSP_TOOL` で確認 |
| `typescript-language-server` | ✅ インストール済み | v5.1.3 |
| Claude Code バージョン | ✅ 2.0.74 | LSP対応バージョン |
| `typescript-lsp` プラグイン | ✅ 有効化済み | `settings.json` で確認 |
| `cclsp.json` 設定 | ✅ 存在 | 正しい設定内容 |

#### 3.2 未解決の問題

**現在のClaude Codeセッションの問題：**

デバッグログの分析結果：
```
12:22:06 - [LSP MANAGER] LSP notification handlers registered successfully for all 0 server(s)
12:25:43 - Added installed plugin: typescript-lsp@claude-plugins-official
```

- セッション開始時（12:22）にLSPサーバーが0個で初期化された
- `typescript-lsp` プラグインはセッション開始後（12:25）に追加された
- **セッション中のプラグイン追加はLSPマネージャーに反映されない**

## 今後の対応予定

### Phase 1: 動作確認（優先度: 高）

1. **gwt経由で新しいClaude Codeセッションを起動**
   - 現在のセッションを終了
   - `gwt` コマンドで新規セッション開始
   - LSPツールの動作確認

2. **期待される動作**
   - 新セッションでは `ENABLE_LSP_TOOL=1` が起動時から設定される
   - `typescript-lsp` プラグインが初期化時にロードされる
   - LSPサーバーが正しく登録される（1+ servers）

### Phase 2: 検証項目

| テスト | コマンド/操作 | 期待結果 |
|--------|---------------|----------|
| 環境変数確認 | `echo $ENABLE_LSP_TOOL` | `1` |
| LSP hover | TypeScriptファイルでホバー | 型情報表示 |
| LSP定義ジャンプ | `goToDefinition` | 定義位置へ移動 |
| LSP参照検索 | `findReferences` | 参照一覧表示 |

### Phase 3: 追加対応（必要に応じて）

1. **ログ出力の強化**
   - LSPマネージャー初期化時のサーバー数をログ出力
   - プラグインロード状態の可視化

2. **ドキュメント更新**
   - README.md にLSP対応の記載追加
   - トラブルシューティングガイドの作成

## 関連ファイル

- `src/claude.ts` - CLI起動経路（修正済み）
- `src/services/aiToolResolver.ts` - Web UI起動経路（修正済み）
- `src/services/builtin-tools.ts` - ビルトインツール定義
- `~/.claude/settings.json` - Claude Codeプラグイン設定
- `~/.claude/cclsp.json` - LSPサーバー設定
- `~/.claude/plugins/cache/claude-plugins-official/typescript-lsp/` - TypeScript LSPプラグイン

## コミット履歴

| コミット | 内容 |
|----------|------|
| `49fea84` | fix: Web UIからClaude Code起動時にENABLE_LSP_TOOL環境変数を渡す |
| `cf2983b` | feat: Claude CodeのTypeScript LSP対応を追加 |

## Web UI側の確認（2024-12-21 実施）

### コード確認結果

Web UI経由でのLSP環境変数渡しは正しく実装されていることを確認：

1. **`src/config/builtin-tools.ts:28`** - `ENABLE_LSP_TOOL: "1"` を定義
2. **`src/services/aiToolResolver.ts:154-156`** - `CLAUDE_CODE_TOOL.env` を `ResolvedCommand.env` として返す
3. **`src/web/server/pty/manager.ts:108-110`** - `resolved.env` を PTY環境変数にマージ
4. **`src/web/server/pty/manager.ts:117-123`** - PTYプロセス起動時に `env` を渡す

### ビルド確認

- `dist/config/builtin-tools.js` に `ENABLE_LSP_TOOL: "1"` が含まれていることを確認

### E2Eテスト結果

```
✅ 15 passed (13.3s)
```

全てのE2Eテストが通過。

## 次のアクション

1. このセッションを終了
2. `gwt` で新しいClaude Codeセッションを起動
3. LSPツールの動作を確認
4. 動作確認後、このPLAN.mdを更新または削除
