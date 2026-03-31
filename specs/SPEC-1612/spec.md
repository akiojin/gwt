> **📜 HISTORICAL (SPEC-1776)**: This SPEC was written for the previous GUI stack (Tauri/Svelte/C#). It is retained as a historical reference. The gwt-tui migration (SPEC-1776) supersedes GUI-specific design decisions described here.

### 背景

PR #1617 で Paste / Voice の terminal action は `TerminalView.svelte` のオーバーレイに統合済みであり、現在の実装は Lucide アイコン、クリップボード画像の staging、voice toggle を提供している。

一方で、既存 SPEC には overlay Paste をテキスト貼り付けとして記述した旧前提が残っており、現行実装との差分が生じていた。また、ボタン視認性も 32x32px / アイコン 16px のままで、terminal 上で見つけにくい状態だった。

この issue では、SPEC を現行 overlay 実装に同期しつつ、Paste / Voice ボタンの視認性 contract を 48x48px / 24px icon に更新する。

アーキテクチャ基盤:
- `gwt-gui/src/lib/terminal/TerminalView.svelte` — terminal overlay action UI
- `gwt-gui/src/lib/terminal/agentInputProfile.ts` — runtime-aware image reference formatting
- `gwt-gui/src/lib/terminal/menuPaste.ts` — native text paste helper
- `crates/gwt-tauri/src/commands/terminal.rs` — clipboard image staging backend

### ユーザーシナリオとテスト

**S1 (P0): Paste ボタンでクリップボード画像をアクティブ terminal に挿入**
- Given: ユーザーが画像をクリップボードにコピー済み
- When: Terminal オーバーレイの Paste アイコンをクリックする
- Then: 画像が backend で staging され、アクティブ PTY に runtime-aware な参照/path が書き込まれる

**S2 (P0): Voice ボタンで terminal voice input を開始/停止**
- Given: 音声入力機能が有効かつ利用可能
- When: Terminal オーバーレイの Voice アイコンをクリックする
- Then: `gwt-voice-toggle` が dispatch され、音声入力対象は現在の terminal pane に維持される

**S3 (P1): Voice 利用不可時も状態が視認できる**
- Given: Voice が unavailable / disabled / preparing のいずれかである
- When: ターミナルが表示される
- Then: Voice ボタンはオーバーレイ上に残り、disabled styling と title で状態を説明する

**S4 (P1): Overlay は terminal 操作を妨げない**
- Given: ターミナルが表示されている
- When: ユーザーがボタン以外の terminal 領域をクリック・スクロールする
- Then: overlay container は pointer event を横取りせず、terminal のネイティブ操作を維持する

**S5 (P1): 画像 clipboard が不正な場合は即時にフィードバックする**
- Given: clipboard に画像がない、またはサイズ上限を超えている
- When: Paste アイコンをクリックする
- Then: toast で失敗理由が表示され、terminal focus が復帰する

**S6 (P0): ボタンの視認性**
- Given: ターミナルが表示されている
- When: ユーザーがオーバーレイボタンを探す
- Then: ボタンが 48x48px、アイコンが 24px、gap が 10px、text color / background / border contrast が更新された状態で表示される

### 機能要件

**FR-001: アイコン表示**
- Paste は Lucide `ClipboardPaste` を使用する
- Voice は Lucide `Mic` を使用する
- テキストラベルではなくアイコンのみを表示する

**FR-002: Overlay 配置**
- `TerminalView.svelte` 内の右下オーバーレイとして表示する
- `.terminal-actions` は `pointer-events: none` を維持する
- `.terminal-action-btn` は `pointer-events: auto` を維持する

**FR-003: Paste 動作**
- overlay Paste は clipboard image staging を扱う
- `save_clipboard_image` の結果を `agentInputProfile.ts` 経由で runtime-aware に terminal 入力へ変換する
- plain text paste は keyboard shortcut / native paste event / `menuPaste.ts` 側の経路を維持する

**FR-004: Voice 動作**
- Voice ボタンは `gwt-voice-toggle` を dispatch する
- unavailable / disabled / preparing では押下不可とする
- listening / preparing 状態は既存の active / busy style で表現する

**FR-005: Accessibility**
- Paste / Voice ともに `aria-label` と `title` を持つ
- `button` 要素としてキーボードフォーカス可能である

**FR-006: Visibility contract**
- アイコンサイズ: `24px`
- ボタン最小サイズ: `48px x 48px`
- padding: `11px`
- `.terminal-actions` gap: `10px`
- デフォルト文字色: `var(--text-secondary)`
- 背景混合比: `color-mix(in srgb, var(--bg-secondary) 92%, black 8%)`
- ボーダー混合比: `color-mix(in srgb, var(--border-color) 70%, white 30%)`

### 非機能要件

**NFR-001: Non-blocking overlay**
- ボタン以外の overlay 領域は terminal pointer interaction を阻害しないこと

**NFR-002: Verification cleanliness**
- `TerminalView.test.ts` と `svelte-check` で新規 error を出さないこと

### 成功基準

- **SC-001** Paste ボタンが `ClipboardPaste` 24px アイコンで表示される
- **SC-002** Voice ボタンが `Mic` 24px アイコンで表示される
- **SC-003** overlay button が 48x48px / padding 11px / gap 10px の contract を満たす
- **SC-004** overlay の pointer-event 構成が terminal 操作を妨げない
- **SC-005** `pnpm exec vitest run src/lib/terminal/TerminalView.test.ts` が通過する
- **SC-006** `pnpm exec svelte-check --tsconfig ./tsconfig.json` が 0 error で通過する

---
