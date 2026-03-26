### EditMode tests
- `BuildService.GetSystemInfo()` returns non-empty OS / CPU / Unity version
- `CaptureScreenshotAsync()` creates output file path
- `ReadLogFileAsync()` returns empty on missing file and content on existing file
- `CreateBugReportAsync()` includes timestamp and system info even when screenshot capture fails
- `BugReport` serialization round-trip
- `UpdateChecker.CheckForUpdate()` が GitHub Release API からバージョン情報を取得しローカルと比較
- `UpdateChecker` がセマンティックバージョニングで正しく新旧判定する
- `CrashReportService.DetectPreviousCrash()` がクラッシュログの有無を検出する
- `CrashReportService.CreateCrashReport()` がクラッシュ情報を GitHub Issue ペイロードに変換する

### Integration RED tests
- release artifact metadata generated for macOS / Windows / Linux builds
- update check result shown in HUD notification model
- GitHub issue creation payload includes screenshot path + logs + system info
- GitHub Release API からの更新チェック→ダウンロード→置換フローの統合テスト
- オプトインクラッシュレポートの送信フロー統合テスト
