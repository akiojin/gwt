### 背景
Tauriビルド（.dmg/.msi/.AppImage）、CI/CDパイプライン、GitHub Releases、自動更新チェック機能を包含する。release.yml、cargo tauri build、更新チェック機能は実装済み。Studio時代の #1553（ビルド・配布・システム監視）の概念を現行Tauriスタックで再定義。

### ユーザーシナリオとテスト

**S1: リリースビルド**
- Given: mainブランチにマージされる
- When: release.ymlが実行される
- Then: .dmg/.msi/.AppImageがビルドされGitHub Releaseにアップロードされる

**S2: 自動更新チェック**
- Given: アプリが起動する
- When: 更新チェックが実行される
- Then: 新バージョンがある場合に通知される

**S3: 開発ビルド**
- Given: 開発環境で作業中
- When: cargo tauri devを実行
- Then: ホットリロードで開発ビルドが起動する

### 機能要件

**FR-01: ビルドパイプライン**
- Tauriビルド（macOS/Windows/Linux）
- CI/CD（GitHub Actions）

**FR-02: 配布**
- GitHub Releases
- バイナリアセット管理

**FR-03: 自動更新**
- 更新チェック
- 通知

**FR-04: バージョン管理**
- Conventional Commits連携
- git-cliff CHANGELOG生成

### 成功基準

1. 全プラットフォームのビルドが成功する
2. GitHub Releaseに正しくアセットがアップロードされる
3. 自動更新チェックが動作する

---
