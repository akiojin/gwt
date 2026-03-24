### 技術コンテキスト

**影響ファイル:**

| ファイル | 役割 |
|---|---|
| `TerminalEmulator.cs` | 自前 ANSI パーサー + TerminalBuffer |
| `XtermSharpTerminalAdapter.cs` | TerminalEmulator ラッパー（将来差し替えポイント） |
| `TerminalRichTextBuilder.cs` | TerminalBuffer → TextMeshPro リッチテキスト変換 |
| `TerminalRenderer.cs` | TextMeshProUGUI + ScrollRect 描画 (MonoBehaviour) |
| `TerminalInputField.cs` | TMP_InputField ラップ入力 (MonoBehaviour) |
| `TerminalTabBar.cs` | 動的タブ管理 (MonoBehaviour) |
| `TerminalOverlayPanel.cs` | オーバーレイパネル表示 |

**影響モジュール:** `Gwt.Core.asmdef`（エンジン層）、`Gwt.Studio.asmdef`（UI/レンダラー層）

### 実装アプローチ

~~XtermSharp をターミナルエンジンとして採用し、ANSI/VT100 パーサー・バッファ管理・入力処理を委譲する。~~

**実装済みアプローチ**: 自前の `TerminalEmulator` + `TerminalBuffer` で ANSI パーサー・バッファ管理を実装。`XtermSharpTerminalAdapter` ラッパーにより、将来 XtermSharp 導入時のエンジン差し替えが可能な設計とした。描画層は `TerminalRichTextBuilder` + `TerminalRenderer` (TextMeshPro + uGUI) で実装。入力は `TerminalInputField` (TMP_InputField) + Input System Package (New) で実装。

> **選定理由**: 自前実装を選択した理由は、XtermSharp の Unity Mono ランタイムへの統合に課題があったため。`XtermSharpTerminalAdapter` ラッパーにより、将来的な XtermSharp 導入は低コストで実現可能。

### フェーズ分割

1. **Phase 1 (Setup)**: PixelMplus フォント導入 + TextMeshPro フォントアトラス最適化
2. **Phase 2 (Foundation)**: 仮想スクロール実装（P0）
3. **Phase 3 (User Story)**: テキスト選択・コピー + URL 検出・ハイライト
4. **Phase 4 (User Story)**: リサイズ行列再計算 + フォント設定 + キーブロードキャスト
5. **Phase 5 (User Story)**: 代替画面バッファ + マウスイベント対応 + ANSI 能力プローブ
6. **Phase 6 (Finalization)**: 全テスト・パフォーマンス計測

### パフォーマンス戦略

- 30fps スロットルで描画頻度を制限 ✅
- TerminalBuffer で可視領域管理 ✅
- フォントアトラスは事前生成し、ランタイムのテクスチャ再構築を回避（予定）
- **仮想スクロール（P0）**: 固定数TMPテキストオブジェクトをプールし、可視行+上下50行マージンのみ描画。スクロール位置に応じて内容差し替え
