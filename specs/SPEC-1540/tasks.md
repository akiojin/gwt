> **Historical Status**: この closed SPEC の未完了 task は旧 backlog の保存であり、現行の完了条件ではない。

- [ ] T001 [S] [US-2] Pty.Net NuGet パッケージの Unity プロジェクトへの導入・動作確認（MVP必須）
- [ ] T002 [F] [US-2] IPtyAdapter アダプタ層の設計・実装（Process版→Pty.Net版の切替ポイント）
- [ ] T003 [F] [US-3] Pty.Net版 IPtyAdapter 実装（forkpty/ConPTY によるネイティブPTY）
- [ ] T004 [U] [US-3] isatty()=true の検証（Pty.Net移行後）
- [ ] T005 [U] [US-3] ネイティブSIGWINCHリサイズの検証（Pty.Net移行後）
- [x] T006 [S] [US-1] `IPtyService` / `IPlatformShellDetector` インターフェース定義
- [x] T007 [F] [US-1] macOS シェル検出実装（`PlatformShellDetector`）
- [x] T008 [F] [US-1] Windows シェル検出実装（`PlatformShellDetector`）
- [x] T009 [F] [US-1] Linux シェル検出実装（`PlatformShellDetector`）
- [x] T010 [F] [US-2] PTY 生成・入出力ストリーミング実装（`PtyService` + `PtySession`）
- [ ] T011 [U] [US-3] PTY リサイズ実装 — Pty.Net移行後にネイティブリサイズ対応
- [ ] T012 [U] [US-5] 環境変数キャプチャモード実装（`login_shell` / `process_env`）
- [x] T013 [S] [US-2] VContainer DI 登録（Singleton ライフタイム）
- [x] T014 [S] [US-4] CancellationToken 対応（全非同期 PTY 操作）
- [ ] T015 [U] [US-6] アプリ終了時の確認ダイアログ + graceful PTY shutdown 実装
- [ ] T016 [FIN] [US-2] Unity Test Framework テスト作成（EditMode）
- [ ] T017 [FIN] [US-2] 各プラットフォームでの統合テスト
