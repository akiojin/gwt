> **ℹ️ TUI MIGRATION NOTE**: This SPEC was completed during the gwt-tauri era. The gwt-tauri frontend has been replaced by gwt-tui (SPEC-1776). GUI-specific references are historical.

### 背景

現行の gwt は Tauri WebView 内でターミナルエミュレータを実装している。Unity 6 への移行に伴い、**TextMeshPro** を用いて Unity ネイティブのターミナルエミュレータを再実装する必要がある。

> **実装方式変更（SPEC 更新）**: 当初 SPEC では **XtermSharp**（Miguel de Icaza 作）をターミナルエンジンとして採用する計画だったが、実装では **自前の `TerminalEmulator` + `TerminalBuffer`** を採用した。XtermSharp への将来的な差し替えポイントとして `XtermSharpTerminalAdapter` ラッパーを設けている。
>
> **変更理由**: XtermSharp は .NET Standard 2.0 対応だが、Unity の Mono ランタイムとの統合に課題があり（NuGet パッケージの直接導入が困難、ソース参照時のコンパイル互換性問題）、まず自前の軽量 ANSI パーサーで必要十分な機能を実現し、段階的に XtermSharp への移行を検討する方針とした。
>
> **UIフレームワーク**: **uGUI** (Canvas + Image + TextMeshProUGUI + ScrollRect + VerticalLayoutGroup) を採用。
>
> **入力システム**: **Input System Package (New)** — `Keyboard.current` API を使用。

現行の実装構成:

| ファイル | 役割 |
|---|---|
| `TerminalEmulator.cs` | 自前の ANSI パーサー + `TerminalBuffer` によるターミナルエミュレーション |
| `XtermSharpTerminalAdapter.cs` | `TerminalEmulator` のラッパー。将来 XtermSharp 導入時の差し替えポイント |
| `TerminalRichTextBuilder.cs` | `TerminalBuffer` → TextMeshPro リッチテキスト変換（Catppuccin Mocha テーマ） |
| `TerminalRenderer.cs` (MonoBehaviour) | `TextMeshProUGUI` + `ScrollRect` で描画、30fps スロットル |
| `TerminalInputField.cs` (MonoBehaviour) | `TMP_InputField` ラップ、Enter で `IPtyService.WriteAsync()` 送信 |
| `TerminalTabBar.cs` (MonoBehaviour) | 動的タブボタン生成、`ITerminalPaneManager` イベント駆動 |

### アーキテクチャ

**自前 TerminalEmulator + TextMeshPro レンダラーの2層構成**を採用する。

```
PTY 出力 → TerminalEmulator (ANSI パーサー) → TerminalBuffer → TerminalRichTextBuilder → TextMeshPro → Unity uGUI
キー入力 → TerminalInputField (TMP_InputField) → IPtyService.WriteAsync() → PTY 入力
```

- **エンジン層（TerminalEmulator）**: 自前の ANSI エスケープシーケンス解析、TerminalBuffer によるバッファ管理、カーソル制御。Unity 非依存。
- **アダプタ層（XtermSharpTerminalAdapter）**: TerminalEmulator のラッパー。将来 XtermSharp 導入時にここを差し替えることでエンジン層を交換可能。
- **レンダラー層（TerminalRichTextBuilder + TerminalRenderer）**: TerminalBuffer の状態を TextMeshPro リッチテキストに変換し、uGUI で描画。30fps スロットルで描画パフォーマンスを最適化。**仮想スクロール方式により可視行+上下50行マージンのみ描画。**
- **入力層（TerminalInputField）**: TMP_InputField を使用し、Enter キーで PTY に入力を送信。Input System Package (New) の `Keyboard.current` API を使用。
- **タブ管理層（TerminalTabBar）**: 動的タブボタン生成、ITerminalPaneManager のイベントに反応してタブ切替。

### ユーザーシナリオ

- **US-1 [P0]**: エージェントをクリックするとターミナル UI がオーバーレイパネルとして開き、PTY の出力がリアルタイムで表示される — 実装済み (`TerminalOverlayPanel` + `TerminalRenderer`)
- **US-2 [P0]**: ANSI カラーコード付きのビルドログが正しく色付けされて表示される — 実装済み (`TerminalRichTextBuilder` + Catppuccin Mocha テーマ)
- **US-3 [P0]**: Claude Code の Markdown 風出力（太字・色・罫線）が読みやすく表示される — 実装済み
- **US-4 [P0]**: スクロールバックバッファ内を上下スクロールして過去の出力を参照できる — 実装済み (`TerminalBuffer` 10,000行 + `ScrollRect`)
- **US-5 [P1]**: マウスドラッグでターミナル内のテキストを選択し、選択範囲がハイライト表示される。Cmd+C / Ctrl+C でクリップボードにコピーできる — 未実装
- **US-6 [P1]**: ターミナル内の URL をクリックすると外部ブラウザで開かれる — 未実装
- **US-7 [P2]**: フォントサイズを設定画面から変更でき、即座に反映される — 未実装
- **US-8 [P0]**: ウィンドウリサイズ時にターミナルの行数・列数が自動調整される — 部分実装（**#1540 のPty.Net移行でネイティブリサイズ対応予定**）
- **US-9 [P1]**: vi, top 等の TUI アプリケーションが代替画面バッファで正常動作する — 未実装（Phase 5 以降）
- **US-10 [P2]**: マウスイベント対応 TUI アプリ（mc 等）でマウス操作が動作する — 未実装（Phase 5 以降）
- **US-11 [P2]**: 全ターミナルに同一キー入力を一斉送信（ブロードキャスト）できる — 未実装
- **US-12 [P2]**: ターミナルの ANSI 対応能力を診断・検出できる — 未実装

### 機能要件

| ID | 要件 | 実装状態 |
|---|---|---|
| FR-001 | ~~XtermSharp エンジンを使用し、~~ **自前 `TerminalEmulator` で** ANSI エスケープシーケンス（SGR 色、カーソル移動、画面クリア、スクロール）を対応する | ✅ **SPEC 変更: XtermSharp → 自前 TerminalEmulator**。256色/TrueColor/代替画面バッファ/マウスイベントは未対応 |
| FR-002 | TextMeshPro で等幅フォント（PixelMplus デフォルト）によるターミナルグリッド表示を実現する | ✅ 実装済み (`TerminalRenderer` + `TextMeshProUGUI`) |
| FR-003 | 10,000 行のスクロールバックバッファをサポートする | ✅ 実装済み (`TerminalBuffer`) |
| FR-004 | マウスドラッグによるテキスト選択・ハイライト表示をサポートし、`Cmd+C` / `Ctrl+C` でクリップボードにコピーする。「Copy All」ボタン方式は不採用 | 🔲 未実装 |
| FR-005 | ターミナルのリサイズ時に行数・列数を再計算し、PTY に通知する | ⚠️ 部分実装（**#1540 のPty.Net移行でネイティブリサイズ制約が解消予定**） |
| FR-006 | クリック可能な URL を正規表現で検出し、ハイライト表示する | 🔲 未実装 |
| FR-007 | ダークテーマ（Catppuccin Mocha 相当の 16 色パレット）をデフォルトとする | ✅ 実装済み (`TerminalRichTextBuilder` に Catppuccin Mocha テーマ) |
| FR-008 | フォントサイズを設定可能（8-24px 範囲、デフォルト 14px）とする | 🔲 未実装 |
| FR-009 | ターミナルはスタジオビュー上にフローティングするオーバーレイパネルとして表示する（別ウィンドウではない） | ✅ 実装済み (`TerminalOverlayPanel`) |
| FR-010 | ~~XtermSharp のマウスイベント対応を活用し、~~ TUI アプリのマウス操作をサポートする | 🔲 未実装（Phase 5 以降） |
| FR-011 | 代替画面バッファ（vi, top 等で使用）をサポートする | 🔲 未実装（Phase 5 以降） |
| FR-012 | 全ターミナルへのキーブロードキャスト（同一キー入力の一斉送信）をサポートする | 🔲 未実装 |
| FR-013 | ターミナルの ANSI 対応能力プローブ（診断コマンド送信→応答解析）をサポートする | 🔲 未実装 |
| FR-014 | **uGUI** (Canvas + Image + TextMeshProUGUI + ScrollRect + VerticalLayoutGroup) を UI フレームワークとして使用する | ✅ **新規追加（実装済み）** |
| FR-015 | **Input System Package (New)** の `Keyboard.current` API を入力処理に使用する | ✅ **新規追加（実装済み）** |
| FR-016 | 動的タブ管理で複数ターミナルペインを切り替え可能にする | ✅ **新規追加（実装済み: `TerminalTabBar`）** |
| FR-017 | **仮想スクロール方式（P0）**: 可視行+上下50行マージンのみをTextMeshProで描画する。固定数のTMPテキストオブジェクトをプールし、スクロール位置に応じて内容を差し替える方式で実装する | 🔲 未実装 |

### 非機能要件

| ID | 要件 | 実装状態 |
|---|---|---|
| NFR-001 | 大量出力（ビルドログ等、秒間 1,000 行以上）でもフレームレートが 30fps を下回らない | ✅ 実装済み（30fps スロットル + `TerminalRenderer`） |
| NFR-002 | ~~XtermSharp~~ **TerminalEmulator** エンジンは Unity 非依存であり、EditMode テストで検証可能である | ✅ 実装済み |
| NFR-003 | スクロールバックバッファのメモリ使用量が 10,000 行で 50MB を超えない | ✅ 実装済み (`TerminalBuffer`) |
| NFR-004 | テキスト描画は TextMeshPro のバッチング最適化を活用し、ドローコールを最小化する | ✅ 実装済み |
| NFR-005 | ~~XtermSharp の NuGet パッケージまたはソース参照により、~~ **`XtermSharpTerminalAdapter` ラッパーにより、** 将来の XtermSharp 導入時にエンジン差し替えが容易な構成とする | ✅ **SPEC 変更** |
| NFR-006 | フォントは PixelMplus（日英対応ピクセルフォント）を使用し、TextMeshPro フォントアトラスを最適化する（アトラスサイズ: 2048x2048、サンプリング: Point (No Filter)、パディング: 2px） | ⚠️ フォント導入は未検証 |

### 成功基準

| ID | 基準 | 実装状態 |
|---|---|---|
| SC-001 | ANSI カラーコード（16色）付きテキストが正しく色付け表示される | ✅ 実装済み（256色/TrueColor は未対応） |
| SC-002 | Claude Code の出力（Markdown テーブル、色付きヘッダー、コードブロック）が読みやすく表示される | ✅ 実装済み |
| SC-003 | スクロールバック・マウスドラッグ選択によるコピーが動作する | ⚠️ スクロールバックは実装済み。マウスドラッグ選択・コピーは未実装 |
| SC-004 | `cargo test` 相当の出力（数百行のテスト結果）がスムーズに描画される | ✅ 実装済み（30fps スロットル） |
| SC-005 | ウィンドウリサイズ後もテキスト配置が崩れない | ⚠️ 部分実装 |
| SC-006 | vi, top 等の TUI アプリケーションが代替画面バッファで正常に動作する | 🔲 未実装（Phase 5 以降） |
| SC-007 | マウスイベント対応 TUI アプリ（mc 等）でマウスクリック・スクロールが動作する | 🔲 未実装（Phase 5 以降） |
| SC-008 | 全ターミナルへのキーブロードキャストが動作する | 🔲 未実装 |
| SC-009 | ANSI 対応能力プローブが正しく動作する | 🔲 未実装 |
| SC-010 | 仮想スクロールにより10,000行バッファでも30fps以上を維持する | 🔲 未実装 |

### 既知の制約事項

| 制約 | 影響 | 対応方針 |
|---|---|---|
| 自前 ANSI パーサー | 256色/TrueColor、代替画面バッファ、マウスイベントは未対応 | Phase 5 で XtermSharp 導入を検討 |
| `isatty()=false` (PTY 側制約) | TUI アプリの動作に制限あり | **#1540 でPty.Net移行により解消予定（MVP必須）** |
| ネイティブPTYリサイズ不可 (PTY 側制約) | SIGWINCH未送信によりフルスクリーンTUIアプリでリサイズ不可 | **#1540 でPty.Net移行により解消予定（MVP必須）** |
| Input System Package (New) 使用 | Legacy Input Manager との共存はできない | プロジェクト設定で Input System Package (New) のみに統一済み |

### インタビュー確定事項

**仮想スクロール（Virtual Scroll）— P0確定:**
- 大量行の描画パフォーマンス最適化として仮想スクロールを採用する
- **可視行+上下50行マージン（大きめマージン）方式**
- **固定数のTMPテキストオブジェクトをプールし、スクロール位置に応じて内容を差し替える**
- バッファ全体を一括描画しない
- 優先度をP0に昇格（パフォーマンス上不可欠）

**XtermSharp採用方針:**
- XtermSharp (MIT, pure C#) をパーサーとして採用する方針を確認
- 現状の自前TerminalEmulatorからの段階的移行
- マウスサポート（フルサポート）はXtermSharp導入時に実現

**Pty.Net移行（#1540）に伴う更新:**
- #1540でPty.Net移行がMVP必須に昇格したことに伴い、リサイズ制約（SIGWINCH未送信）およびisatty()制約の解消見通しが確定
- ネイティブPTYリサイズにより、フルスクリーンTUIアプリでのリサイズが可能になる
