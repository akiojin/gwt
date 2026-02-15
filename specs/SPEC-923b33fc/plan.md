# 実装計画: 全操作時の断続フリーズ抑止（System Monitor 負荷制御）

**仕様ID**: `SPEC-923b33fc` | **日付**: 2026-02-14 | **仕様書**: `specs/SPEC-923b33fc/spec.md`

## 目的

- `get_system_info` の高頻度・重複呼び出しを抑制して、操作全般の引っかかりを減らす
- ウィンドウ復帰時の不要なウォームアップ再実行を除去する

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **ストレージ/外部連携**: なし（既存 `invoke("get_system_info")` を利用）
- **テスト**: `vitest`（`gwt-gui/src/lib/systemMonitor.svelte.test.ts` を新規追加）
- **前提**: 既存UI APIを変更せず `createSystemMonitor()` の内部挙動のみ改善する

## 実装方針

### Phase 1: TDD RED（挙動固定）

- `systemMonitor` の期待挙動をテストで先に固定する
- 失敗確認する観点:
  - 5秒未満で再ポーリングしない
  - in-flight 中に重複 `invoke` しない
  - visibility 復帰時にウォームアップを再実行しない

### Phase 2: GREEN（ポーリング制御実装）

- `setInterval` ベースを単発 `setTimeout` 連鎖に置換し、重複発火を防ぐ
- in-flight フラグを導入して同時呼び出しを 1 件に制限する
- ウォームアップ実行済みフラグを導入し、初回 start のみウォームアップする
- ポーリング間隔を 5000ms に変更する

### Phase 3: 検証・整備

- 新規テストを GREEN 化し、既存監視挙動の基本回帰（start/stop/destroy）を確認する
- 仕様タスクを完了状態へ更新する

## テスト

### バックエンド

- 変更なし（今回はフロントエンドのポーリング制御のみ）

### フロントエンド

- `gwt-gui/src/lib/systemMonitor.svelte.test.ts`
  - 5秒未満で再ポーリングされないこと
  - in-flight 中に追加 `invoke` が走らないこと
  - hidden→visible 復帰でウォームアップ再実行がないこと
