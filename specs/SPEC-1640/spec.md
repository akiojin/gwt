> **⚠️ DEPRECATED (SPEC-1776)**: This SPEC describes GUI-only functionality (Tauri/Svelte/xterm.js) that has been superseded by the gwt-tui migration. The gwt-tui equivalent is defined in SPEC-1776.

### 背景
GWTアプリ全体のアイコンをLucide Icons (lucide-svelte) に統一する。現在はUnicode絵文字やCSSアイコンが混在しており、ビジュアルの一貫性とメンテナンス性に課題がある。

### ユーザーシナリオとテスト

**S1: アプリ全体でアイコンが統一されている**
- Given: ユーザーがアプリの各画面を閲覧する
- When: サイドバー、タブ、ダイアログ、ツールバー等を確認する
- Then: 全てのアイコンがLucideスタイルで統一されている

**S2: アイコンがレスポンシブに表示される**
- Given: 異なるウィンドウサイズで表示
- When: リサイズする
- Then: アイコンが適切なサイズで表示される

### 機能要件

**FR-01: ライブラリ**
- lucide-svelte（tree-shakeable、Svelte公式パッケージ）を使用

**FR-02: 対象スコープ**
- サイドバーボタン
- タブアイコン
- ダイアログボタン
- Terminal周辺（Paste/Voiceオーバーレイ含む）
- ステータスバー
- その他全UIアイコン

**FR-03: 移行方針**
- Unicode絵文字 → Lucideアイコンに置換
- CSS擬似要素アイコン → Lucideコンポーネントに置換
- SVGインラインアイコン → Lucideコンポーネントに置換

### 成功基準

1. lucide-svelteがプロジェクト依存に追加されている
2. アプリ内の全アイコンがLucideに統一されている
3. ビルド・テスト・型チェックが通過する
4. tree-shakingにより未使用アイコンがバンドルに含まれない

---
