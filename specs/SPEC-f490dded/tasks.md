# タスクリスト: シンプルターミナルタブ

## ストーリー間依存関係

```text
US1（メニュー起動）─┬─→ US2（ショートカット起動）
                    ├─→ US3（タブ表示）─→ US5（cwd追従）─→ US7（復元）
                    ├─→ US4（クローズ）
                    └─→ US6（Windowメニュー）
```

- US1 は全ストーリーの前提
- US2 は US1 の起動メカニズムへのショートカット追加のみ
- US5 は US3 のラベル更新機能に依存
- US7 は US5 の cwd 追跡データに依存

## Phase 1: セットアップ

- [ ] T001 [P] [US1] Tab.type ユニオンに `"terminal"` リテラルを追加し、Tab インターフェースに `cwd?: string` フィールドを追加する `gwt-gui/src/lib/types.ts`
- [ ] T002 [P] [US1] `crates/gwt-core/src/terminal/mod.rs` の `pub mod` 宣言に `pub mod osc;` を追加する（コンパイル用の空ファイル `osc.rs` も同時作成） `crates/gwt-core/src/terminal/mod.rs` `crates/gwt-core/src/terminal/osc.rs`

## Phase 2: 基盤（全ストーリー共通）

- [ ] T003 [US1] PaneManager に `spawn_shell()` メソッドを追加する。`launch_agent()` と同じ pane_id 生成・PaneConfig 構築・TerminalPane::new() フローだが `save_branch_mapping()` を省略する。引数は `config: BuiltinLaunchConfig, rows: u16, cols: u16` `crates/gwt-core/src/terminal/manager.rs`
- [ ] T004 [US1] `spawn_shell()` のユニットテストを追加する。正常系（pane 追加・pane_id 返却）と異常系（不正 working_dir）を検証する `crates/gwt-core/src/terminal/manager.rs`
- [ ] T005 [US1] Tauri コマンド `spawn_shell` を実装する。`working_dir: Option<String>` を受け取り、`$SHELL`（未設定時 `/bin/sh`）を解決し、`PaneManager.spawn_shell()` を呼び出し、`stream_pty_output()` スレッドを起動して pane_id を返す。`working_dir` が None または存在しないパスの場合は `$HOME` にフォールバックする `crates/gwt-tauri/src/commands/terminal.rs`
- [ ] T006 [US1] `spawn_shell` コマンドを Tauri コマンドレジストリ（`generate_handler!` マクロ）に登録する `crates/gwt-tauri/src/commands/mod.rs`

## Phase 3: US1 メニューからターミナル起動 + US2 ショートカット (P0)

- [ ] T007 [US1] `menu.rs` に定数 `MENU_ID_TOOLS_NEW_TERMINAL = "tools-new-terminal"` を追加し、MenuItem を作成する。`Tools` サブメニューの先頭（`Launch Agent...` の前）に配置する `crates/gwt-tauri/src/menu.rs`
- [ ] T008 [US2] T007 の MenuItem に `accelerator: Some("Ctrl+\`")` を設定する。Tauri v2 がバッククォートを受け付けない場合は `None` にして T009 のフォールバックを有効化する `crates/gwt-tauri/src/menu.rs`
- [ ] T009 [US1] `app.rs` のメニューイベントハンドラで `MENU_ID_TOOLS_NEW_TERMINAL` を受信し、フロントエンドに `menu-action` イベント（payload: `"new-terminal"`）を emit する `crates/gwt-tauri/src/app.rs`
- [ ] T010 [US1] `App.svelte` の `menu-action` リスナーに `"new-terminal"` ケースを追加する。起動ディレクトリ決定ロジック（selectedBranch → projectPath → null）を実装し、`invoke("spawn_shell", { workingDir })` を呼び出す `gwt-gui/src/App.svelte`
- [ ] T011 [US1] T010 の成功コールバックで、返却された pane_id から `Tab { id: "terminal-{paneId}", type: "terminal", paneId, cwd, label: basename(cwd) }` を生成し `tabs` 配列に追加、`activeTabId` を設定する `gwt-gui/src/App.svelte`
- [ ] T012 [US3] `MainArea.svelte` のターミナルレイヤーの条件分岐に `tab.type === "terminal"` を追加し、TerminalView コンポーネントをレンダリングする（既存の agent タブと同じ分岐に追加） `gwt-gui/src/lib/components/MainArea.svelte`
- [ ] T013 [US2] T008 でネイティブ accelerator が動作しない場合のフォールバック: `App.svelte` に `keydown` イベントリスナーを追加し、`event.ctrlKey && event.code === "Backquote"` を検出して new-terminal アクションをトリガーする `gwt-gui/src/App.svelte`

## Phase 4: US3 タブバーでの表示と識別 (P0)

- [ ] T014 [US3] `MainArea.svelte` のタブバーレンダリングで、`tab.type === "terminal"` の場合にドットカラーを `var(--text-muted)` に設定する CSS クラス `.tab-dot.terminal` を追加する `gwt-gui/src/lib/components/MainArea.svelte`
- [ ] T015 [US3] `MainArea.svelte` のタブラベル表示で、`tab.type === "terminal"` の場合は `tab.cwd` の basename を表示する（cwd が未設定の場合は "Terminal" をフォールバック表示する）。タブ要素に `title={tab.cwd}` 属性を追加してホバー時にフルパスをツールチップ表示する `gwt-gui/src/lib/components/MainArea.svelte`

## Phase 5: US4 ターミナルタブのクローズ (P0)

- [ ] T016 [US4] `MainArea.svelte` のタブ × ボタンクリックハンドラで、`tab.type === "terminal"` の場合も `close_terminal` を呼び出す分岐を追加する（既存の agent タブと同じフロー） `gwt-gui/src/lib/components/MainArea.svelte`
- [ ] T017 [US4] `App.svelte` の `terminal-closed` イベントリスナーで、`tab.type === "terminal"` のタブも `removeTabLocal()` で除去されるようにする（既存ロジックが paneId ベースであれば追加不要・動作確認のみ） `gwt-gui/src/App.svelte`
- [ ] T018 [US4] `App.svelte` のプロジェクトクローズ処理（`handleProjectClose` 等）に、全 terminal タブの PTY を `invoke("close_terminal", { paneId })` で kill してからタブを除去するロジックを追加する `gwt-gui/src/App.svelte`

## Phase 6: US5 cwd のリアルタイム追従 (P1)

- [ ] T019 [US5] OSC 7 パーサーのユニットテストを作成する（TDD: RED）。テストケース: (1) BEL 終端の正常パース (2) ESC \ 終端の正常パース (3) hostname 省略形（file:///path） (4) URL エンコード文字のデコード（%20→空白） (5) 不正入力（file:// なし）で None 返却 (6) 空バッファで None 返却 `crates/gwt-core/src/terminal/osc.rs`
- [ ] T020 [US5] OSC 7 パーサーを実装する（TDD: GREEN）。`pub fn extract_osc7_cwd(buf: &[u8]) -> Option<String>` 関数: バイトスキャンで ESC ] 7 ; を検出し、file:// プレフィックスをスキップ、hostname 後の / からパスを取得、BEL または ESC \ で終端、URL デコードを適用してパスを返す `crates/gwt-core/src/terminal/osc.rs`
- [ ] T021 [US5] `stream_pty_output()` 関数のシグネチャに `agent_name: String` 引数を追加する。呼び出し元（`spawn_shell` コマンドと既存の launch 処理）を全て更新する `crates/gwt-tauri/src/commands/terminal.rs`
- [ ] T022 [US5] `stream_pty_output()` 内の読み取りループに OSC 7 検出ロジックを追加する。`agent_name == "terminal"` の場合のみ `extract_osc7_cwd(buf)` を呼び出し、前回の cwd と異なる場合に `terminal-cwd-changed` イベント（payload: `{ pane_id, cwd }`）を emit する。前回 cwd を `let mut last_cwd = String::new()` で保持する `crates/gwt-tauri/src/commands/terminal.rs`
- [ ] T023 [US5] `App.svelte` に `terminal-cwd-changed` イベントのリスナーを追加する。受信した `pane_id` に対応する terminal タブの `cwd` フィールドと `label`（basename）を更新する `gwt-gui/src/App.svelte`

## Phase 7: US6 Window メニューでのタブ一覧 (P1)

- [ ] T024 [US6] `App.svelte` の `syncWindowAgentTabs()` 呼び出し箇所で、terminal タブも含めてタブ情報を送信するように変更する（type: "terminal" のタブも WindowAgentTabEntry に含める） `gwt-gui/src/App.svelte`
- [ ] T025 [US6] `crates/gwt-tauri/src/commands/window_tabs.rs` の `WindowAgentTabEntry` に `pub tab_type: Option<String>` フィールドを追加する（デシリアライズ時のデフォルトは None = agent） `crates/gwt-tauri/src/commands/window_tabs.rs`
- [ ] T026 [US6] `crates/gwt-tauri/src/menu.rs` の Window メニュータブ描画で、tab_type の値に関わらず全タブを一覧に含めるようにする（既存ロジックが tab_type を無視していれば追加不要・動作確認のみ） `crates/gwt-tauri/src/menu.rs`

## Phase 8: US7 アプリ再起動後の復元 (P2)

- [ ] T027 [US7] ターミナルタブの永続化・復元のユニットテストを作成する（TDD: RED）。テストケース: (1) terminal タブが localStorage に type:"terminal" と cwd 付きで保存される (2) 復元時に spawn_shell(cwd) が呼ばれる (3) type 未設定のエントリは従来通り agent として復元される `gwt-gui/src/lib/__tests__/agentTabsPersistence.test.ts`
- [ ] T028 [US7] `agentTabsPersistence.ts` の `StoredAgentTab` 型に `type?: "terminal"` と `cwd?: string` フィールドを追加する `gwt-gui/src/lib/agentTabsPersistence.ts`
- [ ] T029 [US7] `agentTabsPersistence.ts` の保存ロジックを修正する。terminal タブの場合は `{ paneId, label, type: "terminal", cwd: tab.cwd }` を保存する `gwt-gui/src/lib/agentTabsPersistence.ts`
- [ ] T030 [US7] `App.svelte` の復元ロジックを修正する。`type === "terminal"` のエントリは `invoke("spawn_shell", { workingDir: stored.cwd })` で新しい PTY を生成し、返却された pane_id で terminal タブを再構築する `gwt-gui/src/App.svelte`

## Phase 9: 仕上げ・横断

- [ ] T031 [P] [共通] `specs/specs.md` の現行仕様テーブルに SPEC-f490dded を登録する `specs/specs.md`
- [ ] T032 [P] [共通] `cargo clippy --all-targets --all-features -- -D warnings` でバックエンド全体の lint を通す
- [ ] T033 [P] [共通] `cargo fmt --check` でフォーマットを検証する
- [ ] T034 [P] [共通] `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json` でフロントエンドの型チェックを通す
- [ ] T035 [P] [共通] `cargo test` で全バックエンドテストを通す
- [ ] T036 [P] [共通] `cd gwt-gui && npx vitest run` で全フロントエンドテストを通す
