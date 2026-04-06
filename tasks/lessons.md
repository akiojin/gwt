# Lessons Learned

## 2026-04-06 — fix: branch detail worker は Drop で detach せず join する

### 事象

`Test (Rust)` の `load_initial_data_prefetches_docker_once_per_refresh` が CI 全体実行でだけ不安定に失敗し、
docker preload の呼び出し回数が 3 回まで増えることがあった。

### 原因

- `BranchDetailWorker::drop()` が未完了の background thread を `join()` せず detach していた。
- app テストは `with_fake_docker()` で process-wide な `GWT_DOCKER_BIN` を差し替えるため、
  前テストから漏れた worker が後続テストの fake docker を踏み、別テストの counter を増やしていた。

### 再発防止策

1. process-wide env var やグローバル fixture に依存する background worker は、Drop 時に cancel だけで終わらせず thread 終了まで `join()` する。
2. 「単体実行では通るが full suite / CI だけ落ちる」docker 系テストでは、前テストの非同期 worker が detatch されていないかを最初に確認する。
3. fake external binary を使うテストは、worker の終了保証まで含めて fixture の責務として扱う。

## 2026-04-06 — fix: session pane mouse interaction は keyboard focus 前提で捨てない

### 事象

terminal pane の scrollback 実装自体は存在していたが、管理ビューの初期状態から session 上でホイールしても
スクロールせず、最初のマウス操作が無視されていた。

### 原因

- `handle_mouse_input_with_tools()` が `active_focus == FocusPane::Terminal` を満たさない限り
  session 領域上の `ScrollUp` / `ScrollDown` / click / drag をまとめて `Ok(false)` で捨てていた。
- モデルの初期 focus は `TabContent` のため、session 上の最初のマウス操作だけでは terminal focus に遷移できなかった。

### 再発防止策

1. session pane の mouse UX を追加・変更するときは、「keyboard focus が terminal でない状態」からの 1 発目の操作を RED テストで固定する。
2. session 領域上の wheel / click / drag は、必要なら先に terminal focus へ遷移させてから個別処理へ流す。
3. opener 呼び出しの有無だけを見るテストと、イベントが session interaction として handled されるかを見るテストを分けて評価する。

## 2026-04-06 — fix: Branch detail preload は Tick ごとに処理上限を設ける

### 事象

`Branches` の入力パスを async preload 化した後でも、preload 完了イベントを 1 Tick で全件 drain していたため、
ブランチ数が多い環境で 1 フレーム内の同期処理量が増え、一覧操作が重く感じる再発が起きた。

### 原因

- `drain_branch_detail_events()` がキューを空になるまで `loop` で処理していた。
- preload 自体はバックグラウンド化できていても、結果適用が無制限だと UI スレッドを占有しうる設計だった。

### 再発防止策

1. preload/バックグラウンド処理の「結果適用側」でも 1 Tick あたりの上限（frame budget）を明示的に持つ。
2. 「1 Tick で全件 drain しない」ことを固定する RED テストを追加し、回帰で即検知できるようにする。
3. `Branches` 系のレスポンス不具合では、I/O の非同期化だけでなく「メインスレッド適用量」の上限有無まで確認する。

## 2026-04-06 — fix: git exclude では tracked な配布先のブランチ汚染は防げない

### 事象

Agent launch 時の skill distribution が、他ブランチの `.claude/skills/gwt-*` など tracked なソース資産まで上書きし、
作業していないブランチでも差分が発生した。

### 原因

- `distribute_to_worktree()` が `.claude/skills` / `.claude/commands` / `.claude/hooks` / `.codex/skills`
  を無条件で overwrite していた。
- `.git/info/exclude` は untracked file には効くが、すでに Git 管理下のファイルが書き換わること自体は防げない。

### 再発防止策

1. 配布先が Git worktree の場合は、まず `git ls-files` で gwt 管理対象 path の tracked 状態を確認する。
2. tracked な `.claude/*` / `.codex/*` 資産は distribution で上書きせず、untracked な生成物だけ更新する。
3. `.git/info/exclude` の追加だけで「ブランチが汚れない」と判断しない。tracked / untracked を分けて検証する。

## 2026-04-06 — fix: Launch Agent の AI branch suggestion が復活しないことをテストで固定する

### 事象

`origin/develop` をマージした直後、`prepare_wizard_startup()` が `ai_enabled = true` を再導入してしまい、
Branch Name 入力後に AI suggestion step が復活してブランチ作成が阻害された。

### 原因

- `prepare_wizard_startup()` が `WizardState::default()` の `ai_enabled = false` を上書きしていた。
- 標準 new-branch フローで AI を無効にする前提が、テストで固定されていなかった。

### 再発防止策

1. `prepare_wizard_startup()` が `ai_enabled = false` を保持することを RED テストで固定する。
2. `origin/develop` のマージ後は、Launch Agent の新規ブランチ導線で AI step が出ないことを最小テストで検証する。

## 2026-04-04 — fix: Docker 系 broad verification は Cargo を並列実行しない

### 事象

`cargo test -p gwt-core -p gwt-tui` と `snapshot_e2e` を別プロセスで並列に回した際、Docker 系テストが
`PoisonError` や worker timeout で不安定に落ちた。

### 原因

- `gwt-tui` の Docker 系テストは共有 fixture / lock を前提にしており、同じ worktree で Cargo の検証を
  並列実行すると相互干渉する。
- 単体 slice の focused test では再現せず、broad verification を並列化したときだけ壊れるため、
  変更起因の失敗と見分けにくかった。

### 再発防止策

1. Docker 系テストを含む broad verification は、同じ worktree では `cargo test` / `snapshot_e2e` / `clippy`
   を直列で実行する。
2. broad verification が `PoisonError` や Docker worker timeout で落ちた場合は、まず並列実行の有無を確認し、
   修正前に単独で再実行して再現性を切り分ける。
3. 並列化が必要な場合は detached worktree を分けるか、Cargo 実行を 1 本に絞る。

## 2026-04-04 — fix: footer mnemonic 変更は標準幅 snapshot で可視性を確認する

### 事象

Terminal footer の mnemonic を増やした際、220 桁の render test では通っていたが、実際の `80x24`
snapshot では末尾が切れて `Ctrl+G,g` 以降が見えていなかった。

### 原因

- 文字列の存在確認だけを wide render に対して書き、標準幅の footer 可視性を RED で固定していなかった。
- status context / repo path / hints の 1 行合成で、標準幅では hint が切れる前提を考慮していなかった。

### 再発防止策

1. footer / header / pane title など 1 行 chrome を変更するときは、最低でも `80x24` 相当の render test か snapshot を先に RED で追加する。
2. wide-only の `contains(...)` テストで「見えている」を主張しない。
3. status context と hint を同居させる変更では、compact notation か width-aware compaction を最初から比較対象に入れる。

## L001: 実装前ワークフロー（仕様策定 + TDD）の省略

- **事象**: プランの実装指示を受けた際、CLAUDE.md の実装前ワークフロー（GitHub Issue gwt-spec 作成 → TDD RED 確認）を省略し、直接実装に着手した
- **原因**: プランが詳細だったため、そのまま実装に入れると判断した。CLAUDE.md の必須ワークフローを確認しなかった
- **再発防止策**: feat / fix / refactor の実装開始前に必ず以下を確認する
  1. GitHub Issue (`gwt-spec` ラベル) が存在するか → なければ作成
  2. テストを先に書いて RED を確認 → その後に実装コードを書く
  3. `tasks/todo.md` に Plan・ステップを記録してから着手する

## 2026-03-03 — refactor: ChromaDB コレクション名リネーム

### 事象

`refactor:` タイプのコミット（Python スクリプト内の文字列定数リネーム）を実装した後、
ユーザー確認でGitHub Issue 未作成・TDD テスト未追加の漏れが発覚した。

### 原因

- 変更規模が軽微（1ファイル・8行）だったため、CLAUDE.md の「`refactor:` 対象は
  GitHub Issue を作成してから実装」ルールを省略してしまった。
- Python スクリプト内の定数は Rust ユニットテストから直接検証しにくいと思い込み、
  `include_str!` で埋め込まれた文字列として検証できる手段を見落とした。

### 再発防止策

1. **コミットタイプに関わらず** `feat` / `fix` / `refactor` は必ず実装前に
   `gwt-spec` ラベル付き Issue を作成する。適用除外は `docs:` / `chore:` のみ。
2. Python スクリプトが `include_str!` で埋め込まれている場合、
   定数文字列の存在チェックは Rust の `assert!(SCRIPT.contains("..."))` で書ける。
   「Python だからテストしにくい」は誤り。
3. 計画（Plan）実行時に「仕様策定・TDD は済んでいるか？」を必ずチェックリストで確認してから実装に着手する。

## 2026-04-01 — fix: PTY スクロールとコピーの両立

### 事象

Logs タブのスクロール修正後も、ユーザーが求めていたのはメイン PTY の trackpad / mouse wheel scroll だった。さらに、常時 mouse capture を有効にすると terminal-native copy が壊れた。

### 原因

- 「どの画面でスクロールしたいのか」を最初に切り分けず、管理画面の Logs とメイン PTY を同じ問題として扱った。
- crossterm の mouse capture 制約により、フルスクリーン TUI の PTY で「常時ホイールスクロール」と「端末エミュレータの通常選択コピー」は両立しない前提を、先に設計へ反映していなかった。

### 再発防止策

1. スクロール不具合は、まず「管理画面のリスト」か「メイン PTY」かを分離して再現確認する。
2. PTY でマウス UX を追加する場合は、先に `mouse capture` のオン/オフと terminal-native copy の両立可否を確認する。
3. 両立しない場合は、常時 capture を避け、tmux 風の一時 copy mode など明示的な操作モードへ寄せる。

## 2026-04-02 — fix: Agent-first UX では常時仮想ビューを優先する

### 事象

copy mode を追加したあと、通常モードのまま scroll / drag-copy したいという要件が明確になり、modal な操作モデル自体が UX と合わなくなった。

### 原因

- 前回は terminal-native copy との両立を優先しすぎて、Agent 常用 UX より `mouse capture` の制約回避を優先していた。
- 今回の前提では `vim` / `less` など PTY 内アプリのマウス互換は不要だったため、より単純な「常時仮想ビュー」案を最初から選ぶべきだった。

### 再発防止策

1. PTY 操作 UX の設計では、最初に「Agent-first か」「PTY 内アプリ互換を残すか」を確認する。
2. Agent-first で PTY 内アプリのマウス互換が不要なら、copy mode より先に「通常モード常時仮想ビュー」を比較対象へ入れる。
3. transcript-backed viewport を採る場合は、`follow_live`、viewport freeze、drag-copy、`End` での live 復帰を最初からセットで設計する。

## 2026-04-01 — fix: PTY paste は key input と別経路で扱う

### 事象

Main PTY で通常キー入力は動くのに、Terminal.app からのペーストが安定せず、改行を含む payload が期待通りに届かなかった。

### 原因

- `Enter` は `key_event_to_bytes()` で `\r` に正規化していた一方、ターミナルの `Event::Paste(String)` はイベントループで無視していた。
- そのため、paste は key input の延長として扱われず、専用経路が欠落していた。

### 再発防止策

1. PTY 入力の不具合では、まず `Key` と `Paste` が同じ経路か別経路かを確認する。
2. `crossterm` を使う場合は `EnableBracketedPaste` の有無と `Event::Paste` の処理有無を必ず点検する。
3. 改行を含む paste は `/bin/cat` など実 PTY を使ったテストで payload 全体を検証する。

## 2026-04-01 — fix: constitution の正本パスは compile-time / runtime で一致させる

### 事象

運用上の正本は `.gwt/memory/constitution.md` だったが、`gwt-core` の managed asset 埋め込みだけが `memory/constitution.md` を読んでいた。そのため、ローカル検証で legacy ファイルを補わないとビルド前提が崩れる状態になっていた。

### 原因

- runtime の canonical path と compile-time `include_str!` の参照元を別々に管理していた。
- status 判定も legacy root path を許容しており、「移行用互換」と「現在の正本」が混ざっていた。

### 再発防止策

1. managed asset の正本パスを変える場合は、`include_str!`・status 判定・cleanup/migration テストを同時に点検する。
2. migration 用の legacy path は cleanup 専用に留め、登録済み判定の成功条件には使わない。
3. compile-time と runtime の canonical path が一致していることを RED テストで確認してから実装する。

## 2026-04-02 — fix: compile-time asset は canonical path で tracked にする

### 事象

`Clippy & Rustfmt` の CI で `crates/gwt-core/src/config/skill_registration.rs` の
`include_str!(../../.gwt/memory/constitution.md)` が読み込めず、ローカルでは通るのに
GitHub Actions だけが compile error になった。

### 原因

- `.gwt/memory/constitution.md` はローカルには存在していたが、tracked file ではなかった。
- そのため compile-time asset を `include_str!` で参照していても、CI checkout には存在しなかった。
- canonical path を `.gwt/memory/constitution.md` に揃えるだけでは不十分で、repo に含まれる asset である必要があった。

### 再発防止策

1. `include_str!` で読む canonical asset は、必ず tracked file として repo に含める。
2. local exclude / `.gitignore` / worktree-local generated file に依存して compile-time asset を置かない。
3. compile-time asset を canonical path に移す変更では、`git ls-files <path>` でも存在を確認してから push する。

## 2026-04-01 — fix: 終了済みセッションを自動 close すると最終エラーを観測できない

### 事象

Agent / Shell の PTY が短時間で終了すると、`Model::apply_background_updates()` がセッションタブを即 close し、最終出力やエラーメッセージを読む前に Branches へ戻ってしまった。

### 原因

- セッション終了検知を「自動 cleanup」の責務として実装し、失敗調査に必要な transcript 可視性を考慮していなかった。
- completed / error 状態を `SessionTab.status` に反映せず、終了 = close と短絡していた。

### 再発防止策

1. PTY 終了監視を入れるときは、まず「終了後に transcript を読む必要があるか」を確認する。
2. UI から観測したい失敗は、自動 close ではなく status 遷移で扱う。
3. 自動 cleanup を入れる場合でも、completed / error セッションが可視のまま残る RED テストを先に書く。
