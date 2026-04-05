### EditMode tests
- `PlatformShellDetector`
  - 利用可能なデフォルトシェルを返す
  - OS ごとの引数を返す
  - 空/null/存在しないパスで `IsShellAvailable` が false
- `PtySession`
  - 初期状態が `Running`
  - `RaiseOutput` が event / observable 両方へ流れる
  - `RaiseExited` が `Completed` + `ExitCode` 設定を行う
  - `Dispose` が token を cancel する
- `PtyService`
  - `SpawnAsync` が sessionId を返しセッション登録する
  - `WriteAsync` が stdin へ書き込む
  - `KillAsync` が終了待機して `Completed` へ遷移する
  - `GetStatus` が `PtySessionStatus -> PaneStatus` を正しく変換する
  - 複数セッション同時管理で互いに独立する

### PlayMode / integration tests
- echo コマンド出力が `OutputStream -> terminal adapter` に到達する
- 複数 PTY が独立バッファへ流れる
- `ResizeAsync` 呼び出し後に session の `Rows/Cols` が更新される
- アプリ終了相当のクリーンアップで全 session が dispose される
- **Pty.Net移行後: `isatty()=true` の検証**
- **Pty.Net移行後: ネイティブSIGWINCHリサイズの検証**

### Pending RED tests
- graceful shutdown confirmation 経由で全 PTY を stop する
- `EnvironmentCaptureMode` に応じて shell 起動引数/環境を切り替える
- リサイズのデバウンス（200ms + 初回即時）
