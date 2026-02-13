# 実装計画: Playwrightベースの実装テスト基盤整備（WebView UI）

## 1. 実装方針

- `gwt-gui` に Playwright を導入し、`e2e/` 配下で Chromium スモークを実行する。
- `page.addInitScript` で `window.__TAURI_INTERNALS__` / `window.__TAURI_EVENT_PLUGIN_INTERNALS__` を注入し、Tauri API 呼び出しを明示的にモックする。
- CI は既存 `.github/workflows/test.yml` に `e2e` ジョブを追加する。

## 2. 変更対象

- `gwt-gui/package.json`
- `gwt-gui/pnpm-lock.yaml`
- `gwt-gui/.gitignore`
- `gwt-gui/playwright.config.ts`（新規）
- `gwt-gui/e2e/open-project-smoke.spec.ts`（新規）
- `gwt-gui/e2e/support/tauri-mock.ts`（新規）
- `.github/workflows/test.yml`
- `README.md`
- `README.ja.md`

## 3. テスト戦略

- Playwright: Open Project スモーク（起動→Recent Project→Agent Mode 送信）
- 既存回帰: `pnpm run test`（Vitest）, `pnpm run build`
- CI 検証: `test.yml` に追加した `e2e` ジョブ実行

## 4. ロールアウト

- 初期導入は Chromium のみ
- 将来 Firefox/WebKit 追加や Tauri ネイティブ統合 E2E は別Issueで拡張
