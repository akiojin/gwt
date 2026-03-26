### 技術選定結果まとめ

| 項目 | 選定 | 調査結果 |
|------|------|---------|
| ゲームエンジン | Unity 6000.3.8f1 | URP 17.3.0 による 2D ライティングが要件に合致。Godot は C# 対応が不完全 |
| DI | VContainer | Unity 特化、パフォーマンス最高。Zenject は過機能 |
| 非同期 | UniTask | Unity のコルーチン代替として業界標準。ValueTask ベースで GC フリー |
| PTY | Pty.Net (Microsoft) | クロスプラットフォーム PTY。forkpty (macOS/Linux) + ConPTY (Windows) |
| ターミナルエンジン | 自前 TerminalEmulator | XtermSharp は Unity Mono との統合に課題。自前実装で必要十分な機能を先に実現 |
| データ永続化 | System.Text.Json | TOML 完全廃止。Unity の JsonUtility は制限が多いため System.Text.Json を採用 |
| ローカライゼーション | Unity Localization Package | 公式パッケージ。String Table ベースで EN/JA 対応 |
| セーブ/ロード | Easy Save 3 | Unity Asset Store の実績あるアセット。暗号化・圧縮対応 |
| ピクセルアートアセット | LimeZu エコシステム | 16x16/32x32/48x48 対応。Modern Interiors + Modern Office で統一感 |

### 未解決事項

- Pty.Net の NuGet→Unity 導入方法（OpenUPM or NuGetForUnity）の検証が必要
- PixelMplus フォントの TextMeshPro フォントアトラス最適化パラメータの検証が必要
