### Build / report models
- `SystemInfoData`
  - `OS`, `OSVersion`, `DeviceModel`, `ProcessorType`, `ProcessorCount`, `SystemMemoryMB`, `GraphicsDeviceName`, `UnityVersion`, `AppVersion`
- `BugReport`
  - `SystemInfo: SystemInfoData`
  - `Description: string`
  - `LogContent: string`
  - `ScreenshotPath: string`
  - `Timestamp: string`
- `BuildArtifactInfo`
  - `Platform: string`
  - `OutputPath: string`
  - `Version: string`
  - `Signed: bool`
  - `Uploaded: bool`
- `UpdateInfo`
  - `Version: string`
  - `ReleaseNotes: string`
  - `DownloadUrl: string`
  - `Mandatory: bool`
- `CrashReport`
  - `CrashLog: string`
  - `SystemInfo: SystemInfoData`
  - `Timestamp: string`
  - `UserDescription: string`（ユーザー編集可能）

### Service Boundary
- `IBuildService`
  - `GetSystemInfo()`
  - `CaptureScreenshotAsync(outputPath)`
  - `ReadLogFileAsync(logPath)`
  - `CreateBugReportAsync(description)`
- `IUpdateService`（GitHub Release API 自前実装）
  - `CheckForUpdateAsync() -> UpdateInfo?`
  - `DownloadUpdateAsync(updateInfo) -> string`（ダウンロードパス）
  - `ApplyUpdateAndRestart(downloadPath)`
- `ICrashReportService`
  - `DetectPreviousCrash() -> bool`
  - `CreateCrashReport() -> CrashReport`
  - `SubmitCrashReportAsync(report) -> string`（作成されたIssue URL）
  - `IsOptedIn() -> bool`
  - `SetOptIn(value: bool)`

### Persistence / Paths
- logs root: `~/.gwt/logs`
- screenshots root: `~/.gwt/logs/screenshots`
- bug report payload combines system info + logs + screenshot path
- crash reports: `~/.gwt/logs/crashes/`
- opt-in state: `~/.gwt/config/telemetry.json`
