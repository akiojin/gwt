# Lessons Learned

## 2026-04-07 — fix: semantic file search は collection 設計でノイズを消す

### 事象

`gwt-search --files` が skill docs、SPEC、archive、snapshot まで同じ collection に入れていたため、
implementation code を探したい query でも markdown 系 artifacts が先に出やすかった。

### 原因

- 「files」を 1 collection で持ち、planning/docs artifacts と implementation files を分離していなかった。
- embedded skills や local SPECs は別 search surface を持っているのに、file search 側でも再度 index してしまっていた。

### 再発防止策

1. semantic search の品質問題を query tuning だけで片付けず、collection 境界が user intent に合っているか先に確認する。
2. 別の検索 surface を持つ artifacts（embedded skills, SPECs, archive, task logs, snapshots）は implementation-file collection に入れない。
3. implementation search と docs search を両立したい場合は 1 collection に混ぜず、collection を分けて default surface を意図的に選ぶ。

## 2026-04-07 — fix: public skill 名は internal action 名ではなくユーザー責務に合わせる

### 事象

`search-files` / `index-files` を canonical action に直した流れで、standalone skill 名まで
`gwt-file-search` に寄せたが、実際の workflow は「project 内の関連実装箇所を探す」ため、
public naming が体験の責務からずれた。

### 原因

- internal runner API の名詞を、そのまま user-facing skill surface に投影してしまった。
- semantic search の返り値が「files」でも、ユーザーが解きたい課題は project understanding /
  implementation discovery である点を十分に分離できていなかった。

### 再発防止策

1. internal action 名と public skill 名がずれているときは、どちらが「実装都合」でどちらが「ユーザー責務」かを先に言語化する。
2. naming review では「この skill は何を index するか」ではなく「ユーザーは何を達成するために呼ぶか」を基準に canonical 名を決める。
3. internal API 名の正規化を public rename に波及させる前に、SKILL の説明文・出力契約・利用導線が同じ責務語彙を使っているか確認する。

## 2026-04-07 — superseded: skill 名は underlying action / historical owner と揃える

### 事象

standalone file search の実体は `search-files` / `index-files` なのに、skill surface が
`gwt-project-search` に寄っており、`files` 契約と命名が食い違っているように見えた。

### 原因

- internal action 名と public skill 名の責務を分離せず、runtime action の名詞をそのまま public naming に投影した。
- historical owner 名称を、現在の user-facing workflow semantics より優先してしまった。

### 再発防止策

1. runtime action 名と historical owner は確認するが、public skill 名の決定基準にはしない。
2. canonical 名を変える場合は、bundled assets・参照 docs・compatibility alias を同じ修正セットで更新する。
3. asset-only rename でも distribution test で canonical asset と negative case を固定する。

## 2026-04-07 — fix: Python launcher 判定は path heuristic ではなく実行 probe と構造化エラーで固定する

### 事象

project-index runtime の Python bootstrap hardening 後レビューで、Windows Store / launcher Python を
path だけで弾く実装と、clone 後 warning が英語メッセージ部分一致で `index` / `workspace` を
振り分ける実装が見つかった。

### 原因

- `%LOCALAPPDATA%\\Microsoft\\WindowsApps\\python*.exe` を「壊れた alias」と決め打ちし、実行 probe 前に除外していた。
- runtime bootstrap failure の分類を構造化せず、人間向け文言の substring に依存していた。
- その結果、有効な launcher を誤拒否し、失敗理由も `Python not installed` に潰れやすかった。

### 再発防止策

1. 外部 runtime / launcher の可用性判定は path やファイル名の heuristic ではなく、実行 probe と version check を RED テストで固定する。
2. user-facing warning の source 分類は構造化 prefix / enum で運び、英語メッセージ本文の一致判定を禁止する。
3. runtime bootstrap の review では「有効 launcher を通すケース」「壊れた launcher の detail を残すケース」「通知 source が startup と clone で一致するケース」を最低セットで確認する。

## 2026-04-07 — fix: Hooks不具合は単点ではなく「実行チェーン」全体で再発する

### 事象

Claude Code では表示されるのに Codex では Branches のスピナーが出ない、または起動直後に出ない不具合が
同じ機能領域で複数回再発した。

### 原因

- 原因が単一ではなく、以下の複合条件で発生していた。
1. `--enable codex_hooks` 未付与で hooks 自体が未実行。
2. `~/.gwt/sessions/runtime/<pid>` への writable root 未付与で sidecar 書き込み不可。
3. tracked `.codex/hooks.json` の旧式 Node forwarder が残留し、移行されない worktree が存在。
4. interactive Codex の `SessionStart` 遅延で launch 直後に sidecar が未生成。
5. hook assets/settings の materialize が PTY spawn より後ろに回るケース。

### 再発防止策

1. Hooks対応の完了条件を「設定ファイル生成」ではなく「PID-scoped runtime sidecar が実際に書かれ、Branches に表示される」まで拡張する。
2. launch/config/runtime/UI を別タスクで確認せず、同一検証サイクルで以下を必ず確認する。
   - launch args: `--enable codex_hooks` と `--add-dir ~/.gwt/sessions/runtime/<pid>`
   - effective worktree config: `.claude/settings.local.json` / `.codex/hooks.json`
   - runtime output: `~/.gwt/sessions/runtime/<pid>/<session>.json`
   - UI結果: 同一 branch 上で Claude/Codex の複数スピナー表示
3. tracked `.codex/hooks.json` を preserve する仕様変更時は、legacy forwarder を含む tracked fixture の移行テストを必須にする。
4. interactive Codex については `SessionStart` 前提を禁止し、launch bootstrap + hook上書き契約を SPEC に固定する。

## 2026-04-07 — fix: interactive Codex は launch 直後の `SessionStart` hook を前提にできない

### 事象

`feature/branches` の `gwt-tui` から `develop` worktree で Codex を起動すると、
`--enable codex_hooks`、`GWT_SESSION_RUNTIME_PATH`、`--add-dir ~/.gwt/sessions/runtime/<pid>` が
すべて入っていても、Branches の spinner sidecar が空のままだった。

### 原因

- launch wiring や `.codex/hooks.json` の no-Node runtime hook 生成自体は正しかった。
- 最小再現では `codex exec` は `SessionStart` hook で sidecar を書く一方、
  interactive Codex は launch 直後の `SessionStart` hook を発火しなかった。
- そのため「hooks が最初の Running sidecar を作る」前提だと、起動直後の interactive Codex は
  branch spinner に現れない。

### 再発防止策

1. Codex hooks 不具合では `hooks.json` / argv / env の確認だけで終わらせず、`exec` と interactive のイベント差も最小再現で確認する。
2. interactive Codex の startup 可視化は `SessionStart` hook 前提にせず、successful spawn 時の PID-scoped runtime bootstrap を RED テストで固定する。
3. hook contract を spec に書くときは「interactive Codex may delay SessionStart」を明記し、launch bootstrap との責務境界を残す。

## 2026-04-07 — fix: tracked な `.codex/hooks.json` の一律スキップは旧式 runtime hook worktree を永久に直せない

### 事象

`feature/branches` の `gwt-tui` から `develop` worktree で Codex を起動しても、
Branches の spinner sidecar が生成されなかった。

### 原因

- `generate_codex_hooks()` が tracked `.codex/hooks.json` を無条件で skip していた。
- `develop` 側の tracked `.codex/hooks.json` には旧式の
  `node .../.codex/hooks/scripts/gwt-forward-hook.mjs` runtime hook が残っていた。
- そのため `feature/branches` 側で no-Node runtime hook へ移行していても、
  実際に起動された worktree には新しい hook 形状が一切届かなかった。

### 再発防止策

1. tracked な生成設定ファイルでも、「現行ユーザー設定」と「旧式 gwt 管理設定」を区別し、一律 skip しない。
2. launch 不具合では、起動元ブランチの埋め込み資産だけでなく、実際に agent が起動する対象 worktree の config ファイル内容まで確認する。
3. 「tracked file を preserve する」仕様を入れる場合は、旧式 tracked asset を抱えた別 worktree で launch する RED テストも必ず追加する。

## 2026-04-06 — fix: Launch args に依存する runtime path は build 後の env 注入だけでは反映されない

### 事象

`gwt-agent` 側で Codex に `--add-dir` を追加しても、実際に起動した Codex セッションの argv にはその引数が現れず、
Branches の spinner sidecar は依然として生成されなかった。

### 原因

- `LaunchConfig::build()` の時点では session id がまだ未確定だった。
- 実際の `GWT_SESSION_RUNTIME_PATH` は `materialize_pending_launch_with()` で session record を保存した後に env へ注入していた。
- そのため build 時に足した writable-root 補完は、本番 launch で使う runtime path と切り離されていた。

### 再発防止策

1. session id や persisted path に依存する launch args は、`LaunchConfig::build()` ではなく materialization 後に補完する。
2. env と argv が同じ derived path を共有する設計では、「どの時点で path が確定するか」を先に固定する。
3. Launch bug の検証では builder unit test だけで完了扱いにせず、materialization 後の最終 config までテストで固定する。

## 2026-04-06 — fix: Codex の hook runtime sidecar は sandbox writable root を明示しないと `~/.gwt` に書けない

### 事象

`feature/branches` の `gwt-tui` から起動した Codex では `codex_hooks` と `GWT_SESSION_RUNTIME_PATH` が入っていても、
Branches の live spinner 用 sidecar が一切生成されなかった。

### 原因

- runtime sidecar の保存先を `~/.gwt/sessions/runtime/<pid>/<session>.json` にしていた。
- Codex は `workspace-write` sandbox で動くため、追加 writable root を付けない限り `~/.gwt/...` への書き込みが拒否される。
- hook command は fail-open でエラーを握りつぶすため、設定が入っていても無症状で sidecar だけ欠落した。

### 再発防止策

1. Codex hook が workspace 外へ書く設計にした場合は、launch args に対応する `--add-dir` も RED テストで固定する。
2. `GWT_SESSION_RUNTIME_PATH` が入っていることだけで「書ける」と判断しない。sandbox writable roots まで確認する。
3. hook 不具合では `argv/env` だけでなく、実際に sidecar が生成されるかを同じ runtime path で手動再現して切り分ける。

## 2026-04-06 — fix: Codex の hooks は `hooks.json` だけでは起動せず launch feature flag も必要

### 事象

Claude Code では Branches の live spinner が出るのに、Codex では同じブランチ上でも spinner が一切出なかった。

### 原因

- gwt は `.codex/hooks.json` を生成・配布していたが、Codex launch args に `--enable codex_hooks` を入れていなかった。
- OpenAI Codex の現行 hooks は feature flag 前提のため、`hooks.json` が存在しても feature flag が無い session では hook 自体が実行されなかった。

### 再発防止策

1. Codex の hook 依存機能を追加・修正するときは、`hooks.json` の生成だけでなく launch args に `codex_hooks` 有効化が入っているかを RED テストで固定する。
2. Claude と Codex で hook 設定方式が同じだと仮定しない。agent ごとに「設定ファイル」「feature flag」「runtime enablement」を分けて確認する。
3. Hooks 不具合では、まず runtime sidecar の有無と launch config の feature flags を一緒に確認し、`hooks.json` 内容だけで原因判断しない。
## 2026-04-06 — feat: session-title tests must not depend on the real home sessions dir

### 事象

agent tab title を branch-first にする RED テストを最初に `gwt_sessions_dir()` 直書きで組んだところ、
sandbox 環境では home 配下への書き込みが拒否され、期待した振る舞いの失敗ではなく `PermissionDenied`
でテストが落ちた。

### 原因

- session title の branch 解決が `~/.gwt/sessions` 前提だったため、テストも同じ実ディレクトリを書き換えようとした。
- render/title 系のテストで「本当に検証したいのは label/style 契約」であるにもかかわらず、
  ファイル配置の実環境依存を切り離していなかった。

### 再発防止策

1. `~/.gwt/*` のような home 配下の永続ディレクトリに依存する表示ロジックは、テストから注入できる `Path` 引数つき helper を先に用意する。
2. render/title 系の RED テストでは、まず tempdir で再現できる最小 helper を叩き、環境権限エラーを期待失敗に混ぜない。
3. sandbox で書き込めるか不明な path を使うテスト helper は作らず、`tempfile` で閉じた fixture に寄せる。

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

## 2026-04-07 — fix: sibling worktree path は linked worktree 名ではなく main repo 名を基準にする

### 事象

Launch Agent の新規ブランチ導線で、`develop` linked worktree 上から `feature/test` を起動すると
worktree path が `.../develop-feature-test` になり、SPEC-10 の sibling layout と一致しなかった。

### 原因

- `resolve_launch_worktree()` が `model.repo_path` をそのまま `sibling_worktree_path()` に渡していた。
- app を linked worktree (`.../develop`) から起動している場合、`repo_path.file_name()` が main repo 名ではなく
  linked worktree 名の `develop` になるため、path prefix が誤っていた。

### 再発防止策

1. sibling layout を導出する前に、`git rev-parse --git-common-dir` 等で main worktree root を解決する。
2. worktree path のテストは main repo 直下だけでなく、linked worktree を起点にした Launch Agent 経路でも固定する。
3. `git worktree` 系の path 期待値は macOS の `/var` → `/private/var` 正規化を考慮して canonical path で比較する。

## 2026-04-07 — fix: 誤った旧 worktree が残る branch は再作成ではなく再利用する

### 事象

`develop-feature-test` のような誤った旧 worktree が残った状態で同じ branch を Launch Agent すると、
session TOML は保存されるが `git worktree add` が「その branch は別 worktree で checkout 済み」で失敗し、
agent PTY が起動しなかった。

### 原因

- path 生成の不具合を直した後も、`resolve_launch_worktree()` は branch の既存 worktree を見ずに
  新しい sibling path を作ろうとしていた。
- `feature/test` はすでに旧 path で checkout 済みのため、Git が二重 checkout を拒否していた。

### 再発防止策

1. new-branch launch でも、対象 branch が既存 worktree で checkout 済みなら `git worktree add` せずその path を再利用する。
2. linked worktree path 修正の回帰テストだけで終わらせず、「旧 path が残っている再 launch」まで RED/GREEN で固定する。
3. launch failure の切り分けでは `~/.gwt/sessions/*.toml` の増加有無と `git worktree list` を合わせて確認する。

## 2026-04-07 — fix: bare workspace の `gwt.git` を linked worktree 名で代用しない

### 事象

`/Users/.../gwt/gwt.git` を common-dir に持つ legacy bare workspace で Launch Agent から
`feature/test2` を作成すると、worktree path が `gwt-feature-test2` ではなく
`develop-feature-test2` になっていた。

### 原因

- `main_worktree_root()` は `--git-common-dir` が `.git` で終わる normal clone しか想定しておらず、
  bare common-dir の `gwt.git` を見たときに linked worktree 自身 (`develop/`) を返していた。
- さらに sibling path 生成側も repo 名の `.git` suffix を落としていなかったため、
  bare repo path をそのまま渡しても期待どおりの `gwt-*` 名にならない設計だった。

### 再発防止策

1. linked worktree から layout root を引く helper は、normal clone の `.git` と bare repo の `*.git` を分けて扱う。
2. bare workspace を使う実運用が残っている間は、tempdir 上の `gwt.git + develop/` fixture を app 層の RED テストに含める。
3. `git-common-dir` を使う path 変換では、`repo name` と `git control dir` を同一視しない。

## 2026-04-07 — fix: Launch Agent の worktree path は repo 名 flatten ではなく branch 階層を使う

### 事象

Launch Agent で `feature/aaa` を新規作成すると、worktree path が
`.../gwt-feature-aaa` になり、既存 workspace の `feature/...` layout と
一致していなかった。

### 原因

- `sibling_worktree_path()` が branch 名全体を slug 化し、repo 名 prefix
  (`gwt-`) を付けた単一ディレクトリへ flatten していた。
- linked worktree 名の誤用は修正済みでも、layout 契約そのものが既存
  workspace とずれたままだった。

### 再発防止策

1. Launch Agent の worktree path は repo 名由来の prefix ではなく、branch 名の `/` をそのままディレクトリ階層に反映する。
2. worktree path の RED テストは `feature/aaa -> ../feature/aaa` を明示的に固定し、`gwt-*` の flatten path を期待値に残さない。
3. SPEC-10 の workspace layout 例と SPEC-3 の Launch Agent acceptance を同じ path 契約で更新し、実装と文書を分離させない。

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
