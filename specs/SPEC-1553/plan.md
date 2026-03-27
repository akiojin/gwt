### 技術コンテキスト

- 現行 Tauri ビルドパイプラインから Unity Standalone Build へ移行する
- GitHub Actions + Unity Build Automation でクロスプラットフォームビルドを自動化する
- GitHub Release API による自前更新チェッカーを全プラットフォーム共通で実装する

### 実装アプローチ

- IL2CPP + VContainer 互換性を最優先で検証し、結果に応じて代替パスを決定する
- GitHub Release API 自前実装で全プラットフォーム共通の更新メカニズムを提供する
- オプトインクラッシュレポートで既存の GitHub Issue 基盤を活用する

### フェーズ分割

1. **IL2CPP + VContainer 互換性の即座検証（最優先）**
2. Unity Build Pipeline の設計（GitHub Actions + Unity Build Automation）
3. 各プラットフォーム向けビルド設定の作成（初回リリースから全プラットフォーム対応）
4. **GitHub Release API 自前更新チェッカーの設計・実装**（バージョン比較→ダウンロード→置換→再起動）
5. システムモニタリングの実装（`SystemInfo` クラス + パフォーマンスカウンター）
6. ログ管理の実装（`~/.gwt/logs/` への出力）
7. バグレポート生成の実装（画面キャプチャ、システム情報収集、ログ添付、送信先検出、Issue 送信）
8. **オプトインクラッシュレポート機能の実装**
9. CI/CD パイプライン（GitHub Actions）の構築
10. テストを作成する
