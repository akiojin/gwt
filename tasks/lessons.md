# Lessons Learned

## 2026-04-07 — fix: blank-only overlap を viewport shift と誤判定しない

### 事象

full-screen pane で 2 画面程度の履歴を上までスクロールすると、
最古フレームが空画面になって何も表示されないことがあった。

### 原因

- snapshot history 追加条件が「行配列の shift 一致」だけだった。
- 直前フレームがほぼ空白で次フレームが下端にだけ文字を描くケースで、
  空白部分の一致を viewport shift と誤判定していた。
- その結果、過渡的な空フレームが history に残り、最上端で空表示になった。

### 再発防止策

1. viewport shift 判定は「重なり行が非空白を含む」ことを必須条件にする。
2. blank frame -> bottom-aligned first draw の回帰テストを追加し、`snapshot_count == 1` を固定する。
3. 「最上端スクロールで空表示」は render だけでなく snapshot append 条件の誤検知を疑う。

## 2026-04-07 — fix: full-screen cache history は viewport shift のときだけ伸ばす

### 事象

full-screen pane で同じ行の上書きや clear + redraw が起きると、
scrollback review に「消えたはずの行」まで残って見えてしまった。

### 原因

- pane-local cache が append-only な screen snapshot history として動いていた。
- そのため、同じ visible viewport を描き直しただけの in-place redraw でも、
  以前の visible rows が独立した history 項目として残っていた。
- これは viewport history と redraw mutation を区別していない cache モデルの問題だった。

### 再発防止策

1. full-screen pane の scrollback cache は「全 redraw の履歴」ではなく、「visible viewport が進んだ履歴」として扱う。
2. 同位置 redraw と viewport shift を分ける focused test を追加し、overwrite/clear redraw が latest cache を置き換えることを固定する。
3. scrollback 不具合で stale line が見えるときは、PTY chunk 境界だけでなく snapshot append 条件そのものを点検する。

## 2026-04-07 — fix: snapshot scrollback は PTY reader chunk ではなく drain 単位で切る

### 事象

full-screen pane で PTY 出力が流れている最中に scroll すると、scrollback 上の画面が崩れて見えた。

### 原因

- `spawn_pty_reader()` は PTY 出力を read chunk ごとに channel へ送っていた。
- main loop はその chunk を 1 件ずつ `Message::PtyOutput` に変換していたため、
  snapshot-backed scrollback が「1 フレーム」ではなく「reader chunk 境界」で刻まれていた。
- full-screen UI の再描画が複数 chunk に分かれると、描きかけの中間状態まで snapshot 履歴に残っていた。

### 再発防止策

1. snapshot-backed scrollback の破綻は renderer だけでなく、`drain_pty_output_into_model()` が PTY chunk をどう束ねているかを確認する。
2. 同一 event-loop drain 内の PTY 出力は session 単位で coalesce してから `Message::PtyOutput` に流し、snapshot が draw 境界に近い粒度になるようにする。
3. `main.rs` の event-loop 調整では、「session ごとに byte order を保ったまま merge される」focused test を追加する。

## 2026-04-07 — fix: snapshot scrollbar の thumb 長は viewport 高さ基準で計算する

### 事象

scroll 自体は機能していても、full-screen pane の snapshot-backed scrollback では
scrollbar thumb が極端に短く、1 セルに近い表示になっていた。

### 原因

- snapshot scrollbar の metrics が `content_length = snapshot_count` と
  `viewport_content_length = 1` を返しており、visible pane 高さをまったく使っていなかった。
- そのため「見えているのは 1 画面分」なのに、thumb は「1 フレーム分」だけとして描かれていた。

### 再発防止策

1. snapshot-backed scrollbar は row scrollback と同じく、「追加履歴量 + visible viewport 高さ」で metrics を組み立てる。
2. thumb 長の不具合では、render だけでなく `session_scrollbar_metrics()` の戻り値を直接固定する focused test を追加する。
3. `max_scrollback == 0` 系の調整では、scroll できることだけでなく scrollbar の位置と長さも別テストで検証する。

## 2026-04-07 — fix: trackpad scroll の重さは wheel flood ごとの redraw 回数を先に疑う

### 事象

Terminal.app 上で trackpad scroll 自体は機能していたが、raw mouse report の漏れを止めた後も、
scroll の反応だけがかなり重く感じられた。

### 原因

- 診断ログでは `ScrollUp/ScrollDown` が 1 gesture あたり大量に届いていた。
- main loop は 1 message ごとに `app::update()` のあと即 `terminal.draw()` へ戻る構造だったため、
  wheel flood がそのまま full-frame redraw flood になっていた。
- `Moved` flood を落とすだけでは十分でなく、wheel event 自体も描画前にまとめる必要があった。

### 再発防止策

1. trackpad scroll の「重い」「反応が悪い」は handler の中身だけでなく、`run_app` の redraw cadence を必ず確認する。
2. host terminal が burst で送る wheel event は、最初の非 scroll message を壊さない範囲で bounded batching してから描画する。
3. batching を入れるときは「consecutive scroll をまとめる」と「burst 後の最初の非 scroll を保留する」を focused test で固定する。

## 2026-04-07 — fix: trackpad 修正後の遅さは `Moved` flood と leaked SGR mouse report を分けて見る

### 事象

Terminal.app 上で scroll 自体は動くようになったが、session pane に `[<64;175;43M` のような
mouse report が混ざって表示され、スクロール反応も極端に重かった。

### 原因

- 既存ログでは trackpad 1 gesture あたりに大量の `ScrollDown` と `Moved` が届いており、
  gwt は hover を使わないのに毎回 update/render を回していた。
- さらに、Terminal.app / crossterm の組み合わせでは SGR mouse report がまれに key escape sequence として漏れ、
  terminal focus 時に PTY へ誤転送される余地があった。

### 再発防止策

1. trackpad の体感遅延は、scroll handler だけでなく `MouseEventKind::Moved` の event flood 有無を必ず確認する。
2. terminal focus 中の raw key input には、SGR mouse report (`ESC [ < ... M/m`) の正規化レイヤを挟み、
   leaked sequence が PTY に入らないようにする。
3. outer terminal の mouse leak 対策を入れるときは、通常の `Esc` キーが壊れない timeout 付き test も一緒に追加する。

## 2026-04-07 — fix: `max_scrollback == 0` の pane では transcript ではなく live screen snapshot を先に疑う

### 事象

`Terminal.app` で scroll event 自体は gwt に届いていたが、Codex pane では scrollbar が出ず、
二本指スクロールしても何も遡れなかった。

### 原因

- 診断ログでは `event=scroll` が発火していた一方、対象 pane の `max_scrollback` は常に `0` だった。
- その pane は行ベースの terminal history を積むのではなく、同じ画面を描き直す full-screen UI として動いていた。
- 問題は transcript file の有無ではなく、gwt 側が pane 存命中の recent screen state を一切保持していなかったことだった。

### 再発防止策

1. 「scroll は届くが scrollbar が出ない」不具合では、まず `event=scroll` と `max_scrollback` を同時に確認し、入力経路と履歴保持経路を切り分ける。
2. `max_scrollback == 0` の pane では transcript ingest を先に足すのではなく、まず pane-local な live screen snapshot cache で十分かを検討する。
3. full-screen redraw pane の修正では、前フレーム表示・scrollbar・selection copy・live-follow 維持を focused test で一緒に固定する。

## 2026-04-06 — fix: Terminal.app のトラックパッド scroll は alternate-scroll mode を先に疑う

### 事象

`Terminal.app` 上で session pane の drag selection copy は動いていたが、二本指スクロールだけが
scrollback に入らず、terminal 側で握りつぶされているように見えた。

### 原因

- gwt は alternate screen + mouse capture を有効化していたが、outer terminal startup で
  alternate-scroll mode (`CSI ? 1007 l`) を明示的に無効化していなかった。
- Terminal.app では alternate-scroll mode が有効なままだと、trackpad scroll が mouse wheel ではなく
  cursor-key fallback に変換され、gwt 側の scroll handler に届かないことがある。

### 再発防止策

1. macOS `Terminal.app` で「copy は動くが trackpad scroll だけ死ぬ」報告を受けたら、まず outer terminal の `?1007` 状態を確認する。
2. outer terminal 初期化を触る変更では、ANSI sequence を直接検証する RED/GREEN テストを `src/main.rs` に追加して回帰を固定する。
3. PTY scroll 不具合では app 内の handler だけでなく、host terminal が wheel を別入力へ変換していないかも最初に切り分ける。

## 2026-04-07 — fix: Terminal.app では trackpad scroll が `Drag(Right)` に化けることがある

### 事象

`Allow Mouse Reporting` が有効で、alternate-scroll mode も無効化した状態でも、
`Terminal.app` 上では trackpad scroll が `ScrollUp/ScrollDown` として届かず、gwt の session pane がスクロールしなかった。

### 原因

- 診断ログでは、二本指スクロール中の入力が `Down(Right)` / `Drag(Right)` / `Up(Right)` の列として観測された。
- `gwt-tui` の mouse handler は `ScrollUp/ScrollDown` と left-button selection しか扱っておらず、
  right-button drag を完全に捨てていた。

### 再発防止策

1. Terminal.app の trackpad 不具合は、`ScrollUp/ScrollDown` 前提で考えず、必ず診断ログで実イベント形を確認する。
2. mouse fallback を入れるときは、left-button selection と right-button trackpad fallback を明示的に分離する。
3. host terminal 診断用の小さい event dumper を残して、再発時に `events.log` で比較できるようにする。

## 2026-04-06 — fix: startup 時の agent detection は main thread で同期実行しない

### 事象

`load_initial_data_prefetches_branch_detail_async` が GitHub Actions で約 5 秒ブロックし、
branch detail preload 自体は非同期でも startup 全体が重く見えていた。

### 原因

- `schedule_startup_version_cache_refresh()` が `AgentDetector::detect_all()` を呼び、
  `gh copilot --version` などの version probe を main thread 上で同期実行していた。
- branch detail preload の非同期性とは無関係な agent detection が、同じ startup path に混ざっていた。

### 再発防止策

1. startup で補助的な cache refresh や probe を走らせる場合は、dispatch から background thread に逃がして UI/initial load を塞がない。
2. 非同期 preload の test は、対象 worker だけでなく同じ code path 上の別 I/O が同期で混ざっていないか確認する。
3. global in-flight flag を使う scheduler test は並列実行で干渉するため、test 側で lock を入れて検証を直列化する。

## 2026-04-06 — fix: process-wide fake docker env は並列 app テストの観測値を汚す

### 事象

`load_initial_data_prefetches_docker_once_per_refresh` が CI の full suite でだけ不安定に失敗し、
branch detail preload の docker snapshot 回数が期待より多く観測されることがあった。

### 原因

- app テストは `with_fake_docker()` で process-wide な `GWT_DOCKER_BIN` を差し替えていた。
- 同時に走る別テストが `load_initial_data()` や branch refresh を通じて background preload を起動すると、
  その worker も同じ fake docker を踏み、カウンタや応答を横取りしていた。

### 再発防止策

1. 並列実行される app テストで external command の観測値を固定したい場合は、process-wide env override ではなく model 単位の dependency injection を使う。
2. background worker を含むテストでは、「fake binary の差し替えが同一 process 内の他テストから見えるか」を最初に確認する。
3. call count や遅延を検証するテストは、外部コマンド自体を叩かず closure / function override で deterministic にする。

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

## 2026-04-06 — fix: remote のない repo 起動時は gh 系メタデータ取得をスキップする

### 事象

`load_initial_data_prefetches_branch_detail_async` が GitHub Actions でだけ約 5 秒ブロックし、
startup 時の branch detail preload が同期処理のように見えていた。

### 原因

- `load_initial_data()` は temp repo に remote がなくても `gh pr view` / `gh pr list` 系の取得経路を通していた。
- local 環境では即失敗して目立たなかったが、CI runner では `gh` が repo 解決待ちで数秒ブロックし、
  preload 非同期化の検証時間を食っていた。

### 再発防止策

1. startup 時の GitHub メタデータ取得は、先に `git remote` で remote の有無を確認し、remote なし repo ではスキップする。
2. temp repo / bare repo を使う startup テストでは、fake `gh` を PATH に差し込んで「GitHub CLI を呼ばない」こと自体を RED/GREEN で固定する。
3. startup の非同期性テストでは、目的外の CLI 呼び出し（`gh` など）が計測対象を汚していないかを先に点検する。

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
