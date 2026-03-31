> **ℹ️ TUI MIGRATION NOTE**: This SPEC was completed during the gwt-tauri era. The gwt-tauri frontend has been replaced by gwt-tui (SPEC-1776). GUI-specific references are historical.

### 背景

- 2026-03-02 に Issue #1265 で Windows Launch Agent の再発が報告された。
- 再発エラー（原文）:
  - `'\"C:\Program Files\nodejs\npx.cmd\"' は、内部コマンドまたは外部コマンド、操作可能なプログラムまたはバッチ ファイルとして認識されていません。`
- 2026-03-03 の方針更新として、事前の「利用可否表示/判定」よりも「実行時に正しいコマンドで起動し、失敗時は実行時エラーを返す」挙動を優先する。

### ユーザーシナリオとテスト

#### ユーザーストーリー 1 - Windows で Launch Agent を安定起動したい (P0)

Windows ユーザーとして、Launch Agent 実行時に `npx.cmd` のクォート混入があっても起動失敗を再発させず、実行時に解決可能なランナー（優先 `bunx`）で起動したい。

独立テスト:
- `terminal::runner` で再発入力 (`'\"...\"'`) を正規化できる。
- `terminal::pty` で正規化後 `.cmd/.bat` 判定が成立する。
- `gwt-tauri` で `bunx` 優先ランナー解決が動作する。

受け入れシナリオ:
1. 前提: Windows で `installed` が選択される、操作: Launch、期待: ローカルコマンドをそのまま実行し、未導入なら実行時エラーを返す。
2. 前提: `latest` / 固定バージョンが選択される、操作: Launch、期待: `bunx` 優先で実行し、必要時のみ `npx --yes` を使用する。
3. 前提: `installed` だが実体コマンドが無い、操作: Launch、期待: 起動前に `latest` へ書き換えない。

#### ユーザーストーリー 2 - UI の表示と実行責務を分離したい (P1)

ユーザーとして、StatusBar の推定状態表示に依存せず、Launch Agent の選択値どおりに実行されることを期待する。

独立テスト:
- `StatusBar` は agent 可用性表示を持たない。
- `AgentLaunchForm` は `installed` を常に選択肢として保持する。

### エッジケース

- 二重エスケープ外側クォート (`\\\"...\\\"`)。
- 外側クォートに trailing args が混在する入力。
- 実行シェル（cmd / powershell / wsl）差異で PATH 解決結果が変わる環境。
- Docker/WSL のような実行ターゲット差異で host 側事前判定が誤る環境。

### 機能要件

- **FR-001**: Windows Launch Agent の command 解決時、実行コマンドを共通正規化で処理しなければならない。
- **FR-002**: `.cmd/.bat` 判定は正規化済みコマンドで評価しなければならない。
- **FR-003**: `PTY` 側でも同一規約で正規化し、経路差異の再発を防がなければならない。
- **FR-004**: Issue #1265 の再発文字列を回帰テストに固定しなければならない。
- **FR-005**: Launch Agent のランナー優先順位は `bunx` を第一優先とし、`npx` 利用時は `--yes` を付与しなければならない。
- **FR-006**: `installed` 選択は UI で常時表示しなければならない。
- **FR-007**: `installed` 選択時にローカルコマンドが見つからなくても、起動前に `latest` へ自動書換してはならない。
- **FR-008**: StatusBar は agent 可用性表示を提供してはならない（誤誘導防止）。

### 非機能要件

- **NFR-001**: 公開 API / Tauri command シグネチャを変更しない。
- **NFR-002**: 既存の `terminal::pty` 回帰テストを維持する。
- **NFR-003**: 仕様変更は Issue-first 文書（本 Issue）に反映する。

### 成功基準

- **SC-001**: Issue #1265 の再発文字列で Windows 経路の正規化が機能する。
- **SC-002**: `gwt-core` と `gwt-tauri` の対象テストが通過する。
- **SC-003**: Launch Agent が `installed` を保持し、実行時エラー責務へ統一される。
