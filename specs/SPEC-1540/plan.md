### 技術コンテキスト

**影響ファイル:**

| ファイル | 役割 |
|---|---|
| `PtyService.cs` | `IPtyService` 実装（Process→Pty.Net に差し替え対象） |
| `PtySession.cs` | Process ラッパー（Pty.Net セッションに置換対象） |
| `PlatformShellDetector.cs` | シェル検出（変更なし） |
| `IPtyAdapter.cs`（新規） | Process版/Pty.Net版の切替アダプタ |

**影響モジュール:** `Gwt.Core.asmdef`

### 実装アプローチ

**Pty.Net への移行をMVP前に完了する。** 現行のProcess ベース実装から `IPtyAdapter` アダプタ層を経由してPty.Net実装に切り替える。NuGet経由（OpenUPM or NuGetForUnity）でパッケージを導入し、forkpty (macOS/Linux) / ConPTY (Windows) によるネイティブPTY管理を実現する。

**選定理由**: IPtyAdapter アダプタ層を設けることで、Process版→Pty.Net版の移行をインターフェース変更なしで実現でき、既存テストも維持可能。

### フェーズ分割

1. **Phase 1 (Setup)**: Pty.Net NuGet パッケージ導入・動作確認
2. **Phase 2 (Foundation)**: IPtyAdapter アダプタ層設計・実装
3. **Phase 3 (User Story)**: Pty.Net版 IPtyAdapter 実装 + isatty/SIGWINCH 検証
4. **Phase 4 (User Story)**: 環境変数キャプチャモード + graceful shutdown 実装
5. **Phase 5 (Finalization)**: 全プラットフォーム統合テスト

### 設計方針

1. `IPtyService` インターフェースで PTY 操作を抽象化 ✅
2. `IPlatformShellDetector` でシェル検出のプラットフォーム差異を吸収 ✅
3. `IObservable` の `OutputStream` で出力ストリーミング ✅ (**SPEC 変更**: UniTask `IUniTaskAsyncEnumerable` → Rx `IObservable`)
4. `CancellationToken` による協調キャンセル ✅
5. PtyManager は VContainer Singleton ライフタイムで管理 ✅
6. **`IPtyAdapter` 薄アダプタ層でProcess版→Pty.Net版の切替を容易にする（MVP前にPty.Net版に切替完了）**
