> **Historical Status**: この closed SPEC は旧 Unity/C# 実装前提の履歴仕様である。未完了 task は旧 backlog の保存であり、現行の完了条件ではない。現行の terminal emulation は `SPEC-1541` と `SPEC-1776` を参照する。

# PTY 管理基盤

## Background

現行の gwt-core は Rust の `portable-pty` クレートで PTY（疑似端末）管理を実装している。Unity 6 / C# への全面移行に伴い、クロスプラットフォーム PTY 管理を C# で再実装する必要がある。

> **実装方式変更（SPEC 更新 2026-03-10）**: ~~当初 SPEC では **Pty.Net** (forkpty/ConPTY) の採用を計画し、実装では `System.Diagnostics.Process` ベースを採用していたが、~~ **MVP前にPty.Netへの移行を完了必須**とする。ProcessベースからPty.Netへの移行はMVP必須要件に昇格した。
>
> **移行理由**: Processベース実装の `isatty()=false` 制約および `SIGWINCH` 未送信制約は、ターミナルエミュレータとしての品質に直接影響するため、Pty.Net移行で根本解消する。
>
> **Pty.Net移行で解消される制約**:
> - `isatty()=false` → Pty.NetのネイティブPTYにより `isatty()=true` となり、CLI ツールがインタラクティブモードで動作
> - ネイティブ PTY リサイズ不可（SIGWINCH未送信）→ Pty.Netの `forkpty`/`ConPTY` によりネイティブリサイズ対応
>
> **現行実装（Process ベース）の制約事項**（Pty.Net移行までの暫定）:
> - `isatty()=false` となるため、一部 CLI ツールが非インタラクティブモードで動作する
> - ネイティブ PTY リサイズ（SIGWINCH 送信）が不可能。PtySession の Rows/Cols プロパティ更新のみ
> - 環境変数 `TERM=xterm-256color`, `FORCE_COLOR=1` を設定し、カラー出力を強制有効化

現行の実装構成:

| ファイル | 役割 |
|---|---|
| `PtyService.cs` | `IPtyService` 実装。`Process.Start` + StandardOutput/StandardError/StandardInput リダイレクトで PTY セッションを管理 |
| `PtySession.cs` | Process ラッパー。`OutputReceived` / `ProcessExited` イベント、`IObservable` の `OutputStream` を提供 |
| `PlatformShellDetector.cs` | `IPlatformShellDetector` 実装。macOS/Windows/Linux のデフォルトシェルを検出 |

## User Stories

- **US-1 [P0]**: アプリ起動時に OS 上の利用可能なシェル（bash, zsh, PowerShell, fish 等）を検出し、一覧を返せる — 実装済み (`PlatformShellDetector`)
- **US-2 [P0]**: 指定シェルで PTY を生成し、入出力を UniTask ベースで非同期ストリーミングできる — 実装済み (`PtyService.SpawnAsync` + `PtySession.OutputStream`)
- **US-3 [P0]**: ターミナル UI のリサイズ操作が PTY に反映され、シェル側で `SIGWINCH` 相当の処理が行われる — 部分実装（`PtySession` の Rows/Cols プロパティ更新のみ。**Pty.Net移行でネイティブSIGWINCH対応予定**）
- **US-4 [P0]**: PTY 終了時（プロセス終了・明示的 Dispose）にリソースが適切にクリーンアップされる — 実装済み (`PtySession.ProcessExited` イベント + Dispose)
- **US-5 [P1]**: 環境変数キャプチャモード（`login_shell` / `process_env`）を切り替えてシェルを起動できる — 未実装
- **US-6 [P0]**: アプリ終了時に稼働中エージェントがあれば確認ダイアログが表示され、承認後に全 PTY が graceful shutdown される — 未実装

## Functional Requirements

| ID | 要件 | 実装状態 |
|---|---|---|
| FR-001 | **Pty.Net を使用して**クロスプラットフォーム PTY を管理する（**MVP前に移行完了必須**）。現行はProcess ベースで暫定実装 | ⚠️ **SPEC 変更: Pty.Net移行をMVP必須に昇格** |
| FR-002 | macOS / Windows / Linux で利用可能なシェルを自動検出する（bash, zsh, PowerShell, fish, cmd 等） | ✅ 実装済み (`PlatformShellDetector`) |
| FR-003 | PTY のリサイズ（行数・列数の変更）をネイティブにサポートする（**Pty.Netの forkpty/ConPTY によるSIGWINCH対応**） | ⚠️ 部分実装（プロパティ更新のみ。Pty.Net移行で完全対応） |
| FR-004 | PTY の入出力ストリーミングを UniTask ベースで非同期処理する | ✅ 実装済み (`IObservable OutputStream`) |
| FR-005 | 環境変数キャプチャ（`login_shell` / `process_env` モード）をサポートする | 🔲 未実装 |
| FR-006 | VContainer で `IPtyService` として DI 登録し、テスタビリティを確保する | ✅ 実装済み |
| FR-007 | シェルパス検出はプラットフォーム固有ロジックを `IPlatformShellDetector` で抽象化する | ✅ 実装済み |
| FR-008 | PTY プロセスの終了コードを取得できる | ✅ 実装済み (`PtySession.ProcessExited`) |
| FR-009 | アプリ終了時に稼働中エージェントがあれば確認ダイアログを表示し、承認後に graceful PTY shutdown を実行する | 🔲 未実装 |
| FR-010 | 環境変数 `TERM=xterm-256color`, `FORCE_COLOR=1` を設定し、カラー出力を有効化する | ✅ **新規追加（実装済み）** |

### IPtyService インターフェース（実装済み）

```
IPtyService:
  - SpawnAsync(config, ct) → PtySession
  - WriteAsync(sessionId, data, ct)
  - ResizeAsync(sessionId, rows, cols, ct)  // 現行: プロパティ更新のみ → Pty.Net移行後: ネイティブリサイズ
  - KillAsync(sessionId, ct)
  - GetOutputStream(sessionId) → IObservable
  - GetStatus(sessionId) → PtySessionStatus
```

## Non-Functional Requirements

| ID | 要件 | 実装状態 |
|---|---|---|
| NFR-001 | PTY の入出力レイテンシが体感上問題ないこと（目安: 16ms 以内にバッファ読み出し） | ✅ 実装済み（Process の StandardOutput 非同期読み取り） |
| NFR-002 | 複数 PTY の同時管理（エージェント数分）をサポートする設計とする | ✅ 実装済み（PtyService でセッション ID ベース管理） |
| NFR-003 | `IDisposable` / `IAsyncDisposable` を実装し、リソースリークを防止する | ✅ 実装済み |
| NFR-004 | Unity Test Framework（EditMode / PlayMode）でテスト可能な設計とする | ✅ 実装済み（IPtyService インターフェースによる抽象化） |
| NFR-005 | PtyManager は VContainer で Singleton ライフタイムとして DI 登録する（`builder.RegisterEntryPoint(Lifetime.Singleton)`） | ✅ 実装済み |
| NFR-006 | 全ての非同期 PTY 操作は CancellationToken を受け取り、適切にキャンセル可能とする | ✅ 実装済み |

## Success Criteria

| ID | 基準 | 実装状態 |
|---|---|---|
| SC-001 | macOS / Windows / Linux で PTY 生成・入出力が動作する | ✅ 実装済み（Process ベース） |
| SC-002 | シェル検出が各 OS で正しく動作する（最低 2 シェル以上検出） | ✅ 実装済み |
| SC-003 | Unity Test Framework で PTY ライフサイクル（生成→入出力→リサイズ→終了）のテストが通る | ⚠️ リサイズ検証は部分的 |
| SC-004 | VContainer 経由の DI 解決が動作する | ✅ 実装済み |
| SC-005 | アプリ終了時の確認ダイアログ→graceful shutdown フローが動作する | 🔲 未実装 |
| SC-006 | Pty.Net移行後、`isatty()=true` かつネイティブSIGWINCHが動作する | 🔲 未実装（MVP前に検証必須） |

## Known Constraints

| 制約 | 影響 | 対応方針 |
|---|---|---|
| `isatty()=false`（現行Process実装） | 一部 CLI ツール（git, npm 等）が非インタラクティブモードで動作。プログレスバー等が表示されない場合がある | **Pty.Net移行で解消予定（MVP必須）**。移行までは `FORCE_COLOR=1` 等の環境変数で緩和 |
| ネイティブ PTY リサイズ不可（現行Process実装） | シェル側に SIGWINCH が送信されないため、vim 等のフルスクリーンアプリでリサイズが反映されない | **Pty.Net移行で解消予定（MVP必須）**。forkpty/ConPTYによるネイティブリサイズ対応 |

## Interview Notes

**リサイズ戦略:**
- PTYリサイズは200msデバウンス + 即時初回実行
- 連続リサイズイベント（ウィンドウドラッグ中等）のパフォーマンス最適化

**将来のPty.Net導入時:**
- NuGet経由でパッケージ管理（OpenUPM or NuGetForUnity）
- IPtyAdapter薄アダプタ層でProcess版→Pty.Net版の切替を容易にする

**二重クリーンアップ:**
- イベント駆動（ProcessExitedイベント）+ ポーリング（定期的なプロセス生存確認）の二重管理
- イベント漏れによるゾンビプロセスを防止
