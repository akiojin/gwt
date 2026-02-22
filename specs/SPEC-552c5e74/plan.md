# 実装計画: Launch Agent のデフォルト設定保持（前回成功起動値）

**仕様ID**: `SPEC-552c5e74` | **日付**: 2026-02-14 | **仕様書**: `specs/SPEC-552c5e74/spec.md`

## 目的

- Launch Agent を開いたときの初期表示値を、前回 Launch 成功時の設定にする。
- Launch 未実行/失敗/キャンセル時はデフォルト値を更新しない。
- 壊れた保存データでも UI を安全に開けるようにする。

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（変更なし）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **ストレージ/外部連携**: `window.localStorage`（キー: `gwt.launchAgentDefaults.v1`）
- **テスト**: Vitest + @testing-library/svelte（`gwt-gui/src/lib/components/*.test.ts`）
- **前提**: デフォルト値はグローバル保存で、New Branch 入力は保存しない

## 実装方針

### Phase 1: デフォルト保存基盤

- `gwt-gui/src/lib/agentLaunchDefaults.ts` を新設し、保存スキーマと安全な load/save API を実装する。
- バージョン付きデータ（`version: 1`）を扱い、壊れデータ時は `null` を返す。
- 不正値の最低限サニタイズ（空文字、型不一致）を実装する。

### Phase 2: AgentLaunchForm 連携

- `gwt-gui/src/lib/components/AgentLaunchForm.svelte` で表示時に defaults を読み込み、state へ適用する。
- `detect_agents` 後に保存済み Agent の妥当性を確認し、必要なら利用可能 Agent へフォールバックする。
- `onLaunch` 成功後のみ defaults を保存する（close/fail/cancel では保存しない）。
- 既存の `agentConfig`（`~/.gwt/agents.toml`）保存ロジックとは独立に維持する。

### Phase 3: 互換性とフォールバック

- runtime/docker 選択の復元時に、検出結果と矛盾した値は host へフォールバックする。
- `installed` version が無効な環境では `latest` に補正する。
- 保存対象外フィールド（New Branch 入力）は従来どおり初期化する。

## テスト

### バックエンド

- 変更なし（Tauri/Rust コマンドの挙動は変更しない）。

### フロントエンド

- `gwt-gui/src/lib/agentLaunchDefaults.test.ts`
  - load/save の正常系
  - 壊れ JSON / version 不一致 / 型不一致のフォールバック
- `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
  - Launch 成功時のみ次回デフォルトが更新される
  - Close のみでは更新されない
  - 保存値が無効な場合のフォールバック（agent/runtime/version）
  - 保存対象外（New Branch 入力）が復元されない
