### Core interfaces
- `IPtyService`
  - `SpawnAsync(command, args, workingDir, rows, cols, ct) -> UniTask`
  - `WriteAsync(sessionId, data, ct) -> UniTask`
  - `ResizeAsync(sessionId, rows, cols, ct) -> UniTask`
  - `KillAsync(sessionId, ct) -> UniTask`
  - `GetOutputStream(sessionId) -> IObservable`
  - `GetStatus(sessionId) -> PaneStatus`
- `IPlatformShellDetector`
  - `DetectDefaultShell() -> string`
  - `GetShellArgs(shell) -> string[]`
  - `IsShellAvailable(shell) -> bool`
- **`IPtyAdapter`（新規）**
  - Process版とPty.Net版を切り替えるアダプタインターフェース
  - Pty.Net移行の差し替えポイント

### Runtime models
- `PtySession`
  - `Id: string`
  - `Process: System.Diagnostics.Process`（→ Pty.Net移行後はPty.Netのセッションオブジェクト）
  - `WorkingDir: string`
  - `Rows: int`
  - `Cols: int`
  - `Status: PtySessionStatus`
  - `ExitCode: int?`
  - `OutputStream: IObservable`
  - `OutputReceived: event Action`
  - `ProcessExited: event Action`
- `PtySessionStatus`
  - `Running`
  - `Completed`
  - `Error`
- `PaneStatus`
  - PTY 側状態を UI/他サービスへ公開するための変換先 enum

### Service state
- `PtyService`
  - `_sessions: ConcurrentDictionary`
  - `_disposed: bool`
  - `DefaultTimeout: 30s`
- 起動時環境変数
  - `TERM=xterm-256color`
  - `FORCE_COLOR=1`

### Pending extension points
- `EnvironmentCaptureMode`
  - `login_shell`
  - `process_env`
- graceful shutdown confirmation metadata
  - アプリ終了時に稼働セッション数と対象 sessionId 一覧を提示するための集約モデルが必要
