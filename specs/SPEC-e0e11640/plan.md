# 実装計画: macOS リリース署名/公証の必須化

**仕様ID**: `SPEC-e0e11640` | **日付**: 2026-02-24 | **仕様書**: `specs/SPEC-e0e11640/spec.md`

## 目的

- macOS リリース成果物が Gatekeeper に拒否される問題を恒久的に防止する。

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **CI/配布**: GitHub Actions release workflow（`.github/workflows/release.yml`）
- **テスト**: `codesign`, `spctl`, `xcrun notarytool`, `xcrun stapler`
- **前提**: Apple Developer ID 証明書と Notary API Key を GitHub Secrets に登録済み

## 実装方針

### Phase 1: 署名・公証の必須化

- macOS release ジョブで必要なシークレットの有無を検証し、欠落時にエラーで停止。
- 署名用の一時キーチェーンを作成し、証明書をインポート。
- `gwt.app` を `Developer ID Application` で codesign（runtime オプション付き）する。
- DMG を notarytool で公証し、staple する。

### Phase 2: 検証

- `spctl --assess --verbose=4` で app/dmg が受理されることを確認。

## テスト

### CI

- macOS リリースジョブ内で署名・公証・staple の成否を検証。
- シークレット欠落時に macOS ジョブが失敗することを確認。
