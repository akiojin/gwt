# タスクリスト: macOS リリース署名/公証の必須化

## Phase 1: セットアップ

- [x] T001 [P] [US1] 署名/公証に必要な Secrets 名を整理し workflow に反映する `.github/workflows/release.yml`

## Phase 2: 署名・公証

- [x] T002 [US1] macOS 署名用キーチェーン作成と証明書インポートを追加する `.github/workflows/release.yml`
- [x] T003 [US1] `gwt.app` を codesign し、DMG を notarytool で公証・staple する `.github/workflows/release.yml`

## Phase 3: 検証

- [x] T004 [US1] `spctl --assess` を release job に追加し Gatekeeper 受理を確認する `.github/workflows/release.yml`
- [ ] T005 [US2] Secrets 未設定時に macOS release job が失敗することを確認する `.github/workflows/release.yml`
