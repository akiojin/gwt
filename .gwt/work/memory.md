# Project Memory

## 2026-05-20 — memory 更新は検索導線だけでなく hook reminder で可視化する

Type: workflow
Context: `tasks/lessons.md` を semantic search できるようにしても、gwt の機能として Agent に memory 更新を促す hook reminder が無ければ、再利用可能な学びがファイルに記録されない。
Learning: 永続 memory は「検索できること」と「更新タイミングで Agent に見えること」の両方が必要。canonical path は `tasks/memory.md` とし、`tasks/lessons.md` は legacy alias / fallback に限定する。
Future Action: memory 関連の修正では `search-memory` / `index-memory` / `gwt-memory-search` を正導線にし、UserPromptSubmit / Stop hook は自動書き込みではなく `tasks/memory.md` 更新 reminder を出す。`lessons` は互換 alias として残す。

## 2026-05-20 — memory 更新は reminder だけでなく writer command を提供する

Type: workflow
Context: SPEC-2805 の memory 化では hook reminder と検索導線を追加したが、active objective は gwt の機能として memory/lessons を更新できることだった。
Learning: Agent に更新を促すだけでは gwt の更新機能とは言えない。永続 memory の運用には検索、reminder、supported writer command の 3 点が必要。
Future Action: memory/lessons 更新導線を変更するときは gwtd memory add と legacy gwtd lessons add の動作、hook reminder、skill guidance を同時に検証する。

## 2026-05-21 — PR 作成前に `User Verification Result: confirmed` を必ず取得する

### 事象

SPEC-2809（Console window）の修正で、`gwt-verify --mode pre-pr` の自動テストが全 PASS した
段階で、ユーザーの視覚確認が click-blocking regression によって実行不能だった。
エージェントは独断で `User Verification Result: skipped(reason: develop 側 picker regression)`
に倒して `gwtd pr create` を実行。PR #2857 がユーザー承認なしに作成された。
ユーザーから「私のチェック前に PR を出した」と指摘された。

### 原因

1. `gwt-verify` skill contract は `User Verification Result ∈ {confirmed, n/a, skipped(<reason>)}`
   を要求するが、`skipped` は **ユーザーが明示的に選択した場合のみ** 許容される。エージェントが
   「自動テスト全 PASS だから skip 妥当」と判断して skip に倒したのは contract 違反。
2. 視覚検証の動線がブロックされた（Open Project picker click 不能 / app_runtime panic）状態を
   エージェントが skipped の reason として記録するだけで、ブロッカーそのものを解消せずに次の
   ステップに進めた。「verification が実行不能」は skip 理由ではなく、解消すべき blocker である。
3. 「進めて」というユーザー指示を、verification 自体の skip 承認と誤解釈した。本来は
   「verification 結果が出ている作業を完了まで進めて」という意味で、verification をスキップ
   する許可ではない。

### 再発防止策

1. AGENTS.md "PR 作成ルール（必須）" セクションを新設し、`gwt-verify` の
   `User Verification Result` が `confirmed` / `n/a` になるまで `gwtd pr create` / `gwtd pr edit`
   を呼ばないことを明文化した。`pending` / エージェント独断の `skipped` は禁止。
2. 視覚検証動線がブロックされた場合は、`skipped` に倒す前にブロッカーの根本原因を特定して
   解消する。今回のケースなら Open Project picker の panic 修正を先にやり、再起動後に
   verification を依頼する順番にすべきだった。
3. 「進めて」を解釈する際は、現在の作業 phase で何が `pending` なのかを明確にしてから動く。
   verification 待ちなら verification を完了させる作業（= ブロッカー解消）に進む。
4. 万一誤って PR を作成してしまった場合の rollback 手順: タイトルへ
   `[DO NOT MERGE — user verification pending]` を即座に付与し、PR comment で
   `gwtd pr comment <n>` を使ってブロックの理由とブロッカー解消予定を告知する。
   verification が `confirmed` になった後にタイトルを戻し、ブロック comment を resolve する。

## 2026-05-21 — startup CPU / power は「起動プロセス数」と「hot payload サイズ」を同時に見る

### 事象

gwt 起動中に CPU 使用率と消費電力が高く、macOS ではファンが異様に回るという報告があった。
`sample` / `ps` / gwt logs を見ると、単一の busy loop ではなく、startup auto-resume、
project index full status refresh、頻繁な `workspace_state` broadcast が同時に負荷を作っていた。

### 原因

- 起動時の auto-resume が recoverable session を無条件に再開し、同一 native
  agent session 由来の重複や古い session まで launch 対象になり得た。
- Settings.Index の full refresh が、startup current-worktree bootstrap と衝突した後の
  retry だけでなく、同時 full refresh 同士でも二重 probe を起こせる in-flight key になっていた。
- hot path の `workspace_state` が workspace work item 履歴を毎回含み、履歴が増えるほど
  serialize / WebSocket / frontend state update のコストが膨らんだ。

### 再発防止策

1. startup の自動再開は、件数上限で復元を落とさない。fresh かつ unique な exact-resumable
   session は全件復元し、native session id dedupe と staleness gate で不要な二重起動だけを抑える。
2. startup current-only probe と Settings.Index full refresh は別 key にしつつ、full refresh 同士は
   single-flight で潰す。retry を許すのは startup bootstrap と衝突した場合だけに限定する。
3. 高頻度 broadcast には履歴 payload を載せない。workspace history は
   `active_work_projection` のような用途特化 payload に寄せ、shell state は構造情報だけに保つ。
4. CPU / power 調査では CPU% だけで結論を出さず、`sample`、子プロセス数、runner 起動ログ、
   WebSocket payload の大きさを同じタイムラインで見る。

## 2026-05-20 — Phase 26 の visibility 述語だけでは silent no-op を防げない: layout box が立つまで `isReady` を flip しない (Issue #2832 / SPEC-2008 Phase 26.A regression)

### 事象

SPEC-2008 Phase 26 で導入した `runTerminalActivationSequence` (refresh→layout flush→fit) と `createTerminalRuntime` の `isReady` / `deferredWrites` handshake は intact だったにも関わらず、Claude Code を gwt agent pane で起動した直後にテキストをペーストすると terminal 表示全体が崩れる症状がユーザー報告 (`work/20260520-1050`) で再発した。resize / window move で復活する。スクリーンショット `.gwt/paste-images/1779274308833-11-image.png` で確認済み。

### 原因

`completeInitialFitHandshake` は `canRefreshTerminalViewport(windowId)` が true を返した時点で `runTerminalActivationSequence` を呼び、結果に関係なく `runtime.isReady = true` を立てて deferredWrites を flush していた。`viewportEligibleForRefresh` は `element.hidden` と `workspaceWindow.minimized` のみ確認するため、**構造的には visible だが parent の flex/grid layout が rAF 時点で propagate していない (clientWidth/clientHeight=0)** 状態を素通りする。この状態で `fitAddon.fit()` を呼ぶと `_renderService.dimensions.css.cell.width` 自体は font metrics 経由で populate されても、`proposeDimensions` が `getComputedStyle(parent).width / cell.width` を計算する段で 0÷n=0 cols になり、fit が silent no-op するか xterm default 80×24 にロックされる。直後の `isReady = true` で deferredWrites が誤グリッドへ flush され、ユーザーには Claude Code splash と paste 直後の表示が崩れて見える。OS resize で fan-out が走るとそこで初めて正しい cols/rows に fit されるため、resize で復活する signature になる。

Phase 26.A の `isReady` handshake は「writes を fit 完了まで buffer する」ことは保証していたが、「fit が valid cols/rows を返した」ことを保証しておらず、layout が settle するまでの race window がそのまま残っていた。

### 再発防止策

1. **handshake は `isReady=true` を flip する前に container の layout box (`clientWidth/clientHeight > 0`) を確認する。** `terminal-viewport-reflow.js::elementHasLayoutBox` で predicate を export し、`completeInitialFitHandshake` から `terminalContainerHasLayoutBox(windowId)` 経由で gate する (Issue #2832 fix)。
2. **layout box が無い場合は rAF retry を上限付き (`HANDSHAKE_RETRY_LIMIT = 60` ≒ 1 秒 @ 60Hz) でループさせる。** 上限超過時は `console.warn` で観測可能にしつつ fall-through して activation を強制する (perma-hidden window が deferred writes を永久に pin しない保険)。
3. **Rust source-string contract で wiring を pin する。** `crates/gwt/src/embedded_web.rs` の `embedded_web_terminal_runtime_buffers_writes_until_initial_fit_handshake` に `terminalContainerHasLayoutBox` / `handshakeAttempts` / `HANDSHAKE_RETRY_LIMIT` regex を追加し、将来 helper を drop する refactor が CI で即座に炎上するようにする。
4. **frontend behaviour test に 0-size container の判定を pin する。** `__tests__/terminal-viewport-reflow.test.mjs` の `elementHasLayoutBox blocks 0-size containers` で clientWidth/clientHeight = 0、getBoundingClientRect fallback、null defensive の各分岐を直接 assert する。

`viewportEligibleForRefresh` 自体を破壊的に書き換えると、host resize fan-out / project tab switch / dock target 等の既存 caller が hidden short-circuit に依存している側面を壊しうるため、predicate を増やす形で blast radius を最小化した (Phase 26 が Phase 24 を上書きせずに helper を増やしたのと同じ方針)。

### 関連 PR / Issue

- Issue #2832 (本件 bug Issue)
- SPEC-2008 Phase 26 / FR-056 / FR-057 / FR-059 (lineage)
- 過去の closed regressions: #2096 / #2091 / #2668 / #2513

## 2026-05-20 — develop merge ≠ 既存 session で動く: hook 経路 binary は `installed gwtd` で固定される (Phase U-9 self-bench で発見)

### 事象

SPEC-2359 Phase U-9 (PR #2819) を develop に merge 後、self-bench として live Claude Code session の `title_summary` を activity descriptor (`PR チェック中`) に書き換え、RemindersState を stale threshold 直前まで prime したが、次の UserPromptSubmit で stale reminder (`# Agent Title Stale`) は context に注入されず、prime した新 field (`last_title_summary_seen` / `unchanged_turn_count` / `phase_changed_in_window`) はすべて wipe された。

### 原因

`.claude/settings.local.json` の hook command は `'/Applications/GWT.app/Contents/MacOS/gwtd' hook event UserPromptSubmit` のように **installed gwtd binary を絶対 path で固定** している。私のセッションが起動した時点の installed binary は `gwtd 9.40.0` 直前の `gwtd 9.38.0` で、Phase U-9 (FR-178 stale detection / 新 RemindersState field) を含まない。

差分検証で確定 (同 primed state + 同 UserPromptSubmit payload):

- 旧 v9.38.0: `additionalContext` 2825 bytes / `Agent Title Stale` 不在 / RemindersState 新 field を null に wipe
- 新 v9.40.0 (`target/debug/gwtd`, PR #2819 を含む): 3268 bytes (+443) / `Agent Title Stale` 注入 / `unchanged_turn_count: 8→9`, `last_title_summary_seen: 'PR チェック中'`, `phase_changed_in_window: true` を保持

Phase U-9 のコード自体は正しく動作するが、live session に reach するのは「次に gwt release を切って installed binary が新版に置き換わるか、`.claude/settings.local.json` を手で `target/debug/gwtd` に向ける」のいずれかが必要。

### 再発防止策

1. **hook 経路の binary は worktree 起動時の `installed gwtd` で固定されると認識する**: `.claude/settings.local.json` / `.codex/hooks.json` は generator が `installed_gwt_candidates()` で resolve した path をハードコードする。一度書き出されたら手で書き換えない限り変化しない。`refresh_existing_managed_gwt_assets_for_worktree` が再生成する時も同じ installed path を使う。development fallback の `target/debug/gwtd` に向くのは installed binary が見つからない時だけ。
2. **release を切らずに動作観測が必要な変更は development binary 経由で test する**: 新 hook 機能の動作観測は `target/debug/gwtd hook event <Event>` を **直接** 投入して output を見るのが正本経路。live session の system-reminder block に injection されるのを待つのは installed binary に依存するため不確実。
3. **「PR merge = 既存 user の挙動が変わる」と誤解しない**: code が `develop` に届いたタイミングと、user の installed binary が新版になるタイミングはずれる。release ワークフロー (`/release` skill 等) を回して app bundle / installer を新版に bump しないと、既存 session の挙動は変わらない。完了報告で「次回 session から効く」と書く時、それが「次回 SessionStart hook の発火」を指すのか「次回 release 配布後の新 session」を指すのかを区別する。Phase U-9 の場合、新規 worktree の materialization は installed binary が呼ぶ generator の話なので、generator が新バージョンを emit するには installed binary 自体の更新が必要。
4. **gwt-verify の Overall PASS は live session 動作の証明ではない**: gwt-verify の test matrix は `target/debug/gwtd` を使って unit / integration test を走らせる。これは「コードが正しい」ことの証明であって「user の live session で動く」ことの証明ではない。視認系 verification (E2E headed / live-session dogfooding) は installed binary deploy 後に追加で必要。

---

## 2026-05-20 — User 観察と code 上の強制機構の方向が逆だった時は salience asymmetry を疑う (title-summary content / Codex vs Claude Code)

### 事象

User が「Codex は agent window の `title_summary` をよく更新するが、Claude Code はあまり更新しない」と報告。しかも更新内容が `PR チェック中` のような activity descriptor (作業内容) になっていて、複数 pane を監視する user が agent の goal を把握できない状態だった。

### 原因

1. **観察と code 上の強制機構が逆向き**だった。Claude Code 側には dual-path 強制 (skill + `board_reminder` empty-trigger reminder) が `.claude/settings.local.json` の 5 hook events 経由で機能していたのに、Codex 側は `.codex/skills/gwt-coordination/SKILL.md` も `.codex/hooks.json` も worktree に materialize されていなかった。code 上の強制機構の配置だけ見ると Claude Code の方が title 更新は強制されるはずだが、実観察は逆。
2. **真因は salience asymmetry**: Claude Code は CLAUDE.md → AGENTS.md import + skills + hook reminders + Board reminder + 永続化 SessionStart context 等の中で title 規則が reminder fatigue で normalized as noise になる。Codex は AGENTS.md を primary system instruction として読むため、競合する context が少なく title 規則が salient に残る。
3. **canonical guidance の Bad 例不足**: `coordination_guidance.rs::SKILL_BODY_EN` の Bad 例は `done` / `Working on X` の 2 つだけで、`PR チェック中` / `verifying tests` / `fixing bug` のような phase/activity descriptor を明示的に Bad と書いていなかったため agent が許容と解釈していた。
4. **empty-trigger reminder が one-shot だった**: `title_summary_required_reminder()` は title が empty の時だけ fire し、初期 set 後 phase が進んでも再注入されない (stale 化)。

### 再発防止策

1. **User 観察と code 上の強制機構が逆向きを示唆する時は code-side enforcement を信じず salience を疑う**: コードに hook + skill + validation が完備されていても、agent がそれを surface しないなら enforcement は無効。reminder fatigue / 競合 context / 長すぎる system prompt は salience を破壊する。仮説検証時は「強い強制が無い側」の方が良く動く可能性を等しく扱う。
2. **content validation は activity descriptor では成立しない**: 「中」/「verifying」/「checking」/「working on」のような activity 系語尾は無限のバリエーションを持ち、string matching ベースの reject / warning は false positive が多すぎて運用に耐えない。代わりに canonical guidance の Bad 例 + auto-derive from owner + per-turn stale reminder の defense-in-depth で対処する (SPEC-2359 Phase U-9 / US-45)。
3. **dual-language SKILL body を扱う時は両方更新を強制する**: `SKILL_BODY_EN` と `SKILL_BODY_JA` がある場合、`render_skill_md` test が EN-only emit を強制するなら、JA-only の Bad 例 (例: `〜中`) は JA-speaking agent に届かない。`required_phrases()` drift guard には EN の representative phrase を入れ、JA body には `skill_body_ja_contains_*` の独立 test で drift を検出する。
4. **一度設定された state の stale 化は empty-trigger reminder では検知できない**: `title_summary` のような「初期 set 後は更新が agent 任せ」になる field は、N turn 連続で同 value + 関連 field (`current_focus` / Board kind 等) の drift を検出する stale-detection reminder を追加する。state は `RemindersState` 等 既存 sidecar に `#[serde(default)]` で field 追加すれば legacy state 後方互換。
5. **`.codex/` 配下 asset の missing は worktree 状態次第で発生する**: managed asset generator (`generate_coordination_guidance_for_codex`, `generate_codex_hooks`) が code にあっても、worktree が generator 追加前に作られていれば materialize されていない。SessionStart hook で missing 検出 → `refresh_existing_managed_gwt_assets_for_worktree` 呼び出しの idempotent re-materialization を追加すると old worktree も自己修復できる (FR-177)。

---

## 2026-05-20 — SPEC の FR で platform scope を限定すると、サイレントな regression を生む（macOS app icon）

### 事象

`v9.38.0` の `/Applications/GWT.app` で Dock / Finder / `⌘Tab` の GWT アイコンが macOS デフォルトに戻っていた。`/Applications/GWT.app/Contents/Info.plist` には `CFBundleIconFile` が無く、`Contents/Resources/` も空だった。`assets/icons/icon.icns` はリポジトリに存在していたが、リリースワークフローが bundle へ wire していなかった（Issue #2799）。

### 原因

旧 Tauri bundler は macOS bundle / icns binding を自動で担当していた。SPEC-1776 で gwt-tauri を削除した時点でこの binding が失われたが、`.github/workflows/release.yml` の手作り `GWT.app` 生成ステップは未追加のまま。後続の SPEC #2041 FR-027 は「**Windows build は**……」と platform を限定して icon assets を再配線したため、Windows 側だけが復活し macOS 側は CI logs にも明示的なエラーが出ないまま落ちたまま継続した。FR の文言から `macOS` の文字が抜けていたことが silent regression を許した直接原因。

### 再発防止策

1. **FR は platform scope を limit するときに必ず明示的に列挙する**: 「Windows build は…」のように一つの platform に限定する FR を書く場合、同じ要件が他の platform でも必要かどうかをレビュー時に必ずチェックする。テンプレートに「OS scope: windows / macos / linux / all」を入れて選択漏れを防ぐ。
2. **bundle resource を扱う変更は test_release_assets.cjs に静的検証を追加する**: release.yml が macOS bundle に `CFBundleIconFile` を書き、`Contents/Resources/icon.icns` を copy しているかは `node scripts/test_release_assets.cjs` で静的にチェックできる。`assert.match(workflow, /<key>CFBundleIconFile<\/key>\s*\n\s*<string>icon<\/string>/)` の様な regex で CI 回帰防止。
3. **routing: バグは plain Issue で扱う**: `gwt-spec` ラベルは仕様用なので、FR scope 漏れによる regression のような bug 修正は plain Issue として登録し `gwt-fix-issue` で TDD 修正する。SPEC を再 open するのは「設計判断を伴う」場合のみに限定する（feedback memory: `feedback-spec-is-for-specifications-not-bugs`）。
4. **app icon は OS ごとに別経路で配線される。1 OS の修正で完結したと判断しない**: macOS は bundle `Info.plist::CFBundleIconFile` + `Contents/Resources/icon.icns`、Windows は `winresource::set_icon`（.exe 埋込）と `wix/main.wxs` の `<Icon>`（installer / Start Menu / ARP）に加えて runtime の `WindowBuilder::with_window_icon` (HWND `WM_SETICON`)、Linux は X11 / Wayland 用の `WindowBuilder::with_window_icon`。冗長に見えても各経路を独立に配線し、各 OS で table を埋めて配線抜けを目視確認する。

---

## 2026-05-20 — Unit / Playwright snapshot tests は E2E ではない

### 事象

SPEC #2780 (Release Notes Window) 実装で、以下のテストが全て GREEN だった:

- gwt-core release_notes::tests 12 件 PASS (parser unit)
- protocol round-trip 5 件 PASS (serde 単体)
- pnpm test:frontend-unit 507 件 PASS (linkedom + node:test の DOM 単体)
- pnpm test:visual 34 件 PASS (Playwright snapshot、20 件は skip)
- cargo clippy / fmt CLEAN
- CI 全 13 check SUCCESS、auto-merge で v9.39.0 として release

それにもかかわらず、production では **release-notes-window.js が 404 で配信されず、機能完全に死んでいた**。ユーザーが GUI を起動して確認するまで誰も気付かなかった。

### 原因

1. **crates/gwt/src/embedded_web.rs に新 JS module の RootJsModuleAsset エントリを追加し忘れた**。embedded server は allow-list ベースで asset を配信する設計だが、新規 web/ ファイルを追加した際に embedded_web.rs への登録が必須であることが workflow / spec / 既存テストのいずれでも強制されていなかった。
2. **既存の Playwright spec で `GWT_PLAYWRIGHT_BASE_URL` 必須の live-mode tests (theme-toggle 等) は CI で env var が未設定のため全て skip されていた**。誰も live mode で動作確認していなかった。`pnpm test:visual` の "34 passed / 20 skipped" の "20 skipped" は全てこのカテゴリ。
3. **Playwright snapshot tests は `installEmbeddedRoutes()` で frontend を route 経由で直接配信するため、embedded_web.rs の allow-list を経由しない**。つまり embedded server の asset routing は test されていない。

### 再発防止策

1. **新規 `crates/gwt/web/*.js` モジュール追加時は、必ず `crates/gwt/src/embedded_web.rs` に `RootJsModuleAsset` エントリと `pub fn <name>_js() -> &'static str` を追加する**。コンパイル時に検出できるよう、app.js の import 一覧と embedded_web.rs の RootJsModuleAsset を突き合わせる integration test を `crates/gwt/tests/embedded_web_routing_test.rs` として追加することを検討する（follow-up Issue）。
2. **live mode E2E spec を最低 1 件は CI で実行する**。現在の `gwt` ブラウザサーバー経路を background で起動して `GWT_PLAYWRIGHT_BASE_URL` を設定する CI step を追加する（follow-up Issue）。
3. **UI 変更を PR にする前に、`cargo run -p gwt --bin gwt` で実際に起動して manual smoke を実施する**。AGENTS.md「For UI or frontend changes, start the dev server and use the feature in a browser before reporting the task as complete」を厳守する。「Playwright tests 通った = 動く」と勘違いしない。
4. **PR body の checklist で「目視確認はレビュアー側で実施」と書いて user に押し付けない**。テストカバレッジに穴がある場合は明示的に "I cannot verify the live UI in this environment" と書き、leaving-it-to-reviewer の責務逃れをしない。
5. **同じ trap が広範に存在する**: `crates/gwt/web/__tests__/playwright-embedded-routes.test.mjs` は app.js の import を `installEmbeddedRoutes` の ROOT_MODULES と突き合わせるが、embedded_web.rs の RootJsModuleAsset との突き合わせは存在しない。すべての route 配信経路を相互検証する skeleton test が要る。

### 検証

- `crates/gwt/playwright/tests/release-notes-live.spec.ts` を新規追加 (live backend、`installEmbeddedRoutes` 不使用)。最初は 8/8 RED で embedded_web.rs の欠落と splash auto-dismiss bug を可視化。embedded_web.rs 修正後、8/8 GREEN (chromium-dark 4 + chromium-light 4、serial)。
- embedded_web.rs 本体の fix は **別 agent の PR #2797** (work/20260520-0409 / cb25afc1) が canonical で develop に merge 済み。並行調査していて重複検出した。PR #2797 は `every_root_js_module_import_in_app_assets_is_registered` 回帰防止 unit test を含む。私の PR #2798 は live E2E spec と本 memory の追記のみに縮小して PR #2797 の補完に位置付けた。

---

## 2026-05-20 — 外部プロセスラッパーは caller buf を raw に保ち、redaction は hub のみに限定する

### 事象

`gwt_core::process_console::spawn_logged` の初版で stdout/stderr 各行を hub にも caller-facing
`SpawnOutput.stdout` にも **redact 後** の文字列で書き込み、`tokio::io::AsyncBufReadExt::lines` で
読んだ後に常に `\n` を append していた。結果、(1) `gh auth token` の戻り値が
`***redacted***` になりトークン取得が壊れ、(2) `print!("job log 91")` (改行なし) を
モックしている既存テストが `"job log 91\n"` を見て assertion 失敗した。

### 原因

- redact を 1 か所に集約する設計判断のとき、「user-visible な log/UI」と「caller が
  そのまま downstream API に渡す raw buffer」の責任分離を忘れた。
- `BufReader::lines()` は line terminator を strip する。「行ごとの forward」と
  「raw な buffer 復元」を同じパスで両立させようとして、便宜的に `\n` を毎回 append
  したため、もともと改行のない出力にも改行が混入した。

### 再発防止策

1. **caller-facing buffer は raw bytes そのまま**: `tokio::io::AsyncReadExt::read_to_end`
   で生のバイト列を取得し、`String::from_utf8_lossy` してそのまま返す。terminator や
   redaction を caller の見える値に施さない。
2. **redaction は hub/log/UI に流す前だけ**: ring buffer push と broadcast send の
   直前で `redact_line` を適用する。caller の戻り値や `tracing` summary には素値を保つ。
3. **行分割は forward 用途に限定**: hub に流す単位として `\n` と `\r` で split する
   が、caller の `buf` には影響させない。CR-only progress line (docker pull / git clone)
   も同じ split で kind ごとの discrete event になる。
4. **既存モックを必ず通す**: 既存テストがコマンド出力を `print!`/`println!` で
   モックしている場合、改行有無まで含めて互換性を維持する。新規 wrapper を導入する
   ときは「`Command::output()` と同じバイト列」を契約に書き、テストを先に通してから
   実装を縮める。
5. **secret redaction の network leak**: gh / git / docker は通常 token を echo
   しないが、verbose / debug ログでは漏れる。FR としては `Authorization` / `token=` /
   `gh_*` / `ghp_*` / `ghs_*` / `ghu_*` を redact pattern に明示し、ring buffer と
   broadcast の両方に適用する。canonical log には summary event だけが書かれるため、
   line-level の漏洩は構造的に発生しない (FR-040 / SC-013)。

---

## 2026-05-20 — `gwtd issue spec create -f <body>` は section マーカーを付けない

### 事象

gwt-discussion → gwt-register-issue で新規 SPEC を登録する際、`gwtd issue spec create --title "..." -f spec.md` を実行して Issue #2780 が作成されたが、`gwtd issue spec 2780 --section spec` で `gwt issue: section 'spec' not found` となり、cache の `body.md` を確認すると `<!-- gwt-spec id=2780 version=1 --> <!-- sections: -->` のヘッダのみで spec section content が空だった。

### 原因

`gwtd issue spec create -f <body>` は body file の生 markdown をそのまま issue body に書き出すだけで、`<!-- artifact:spec BEGIN -->` ... `<!-- artifact:spec END -->` の section マーカーを自動付与しない。section マーカーがないと `gwtd issue spec --section spec` が認識できず、section editor (`--edit`) や cache パイプラインも空扱いになる。

create コマンドが受け付ける body 形式が「section マーカー付き完成形」なのか「section content だけ」なのかが SKILL / AGENTS.md に明文化されていなかったため、後者を渡してしまい空 SPEC が生まれた。

### 再発防止策

1. **SPEC 作成は 2 ステップに分けるのが安全**: `gwtd issue spec create --title "SPEC: <title>" -f <minimal-stub>` で器を作り、直後に `gwtd issue spec <n> --edit spec -f <full-body>` で section content を投入する。`--edit` は section マーカーを自動付与するため確実に section が認識される。
2. SPEC 作成系の手順は `gwt-register-issue` skill 内に正本フローとして書き起こす。可能なら専用 skill `gwt-register-spec` を新設して title 規約 / 7 section 構成 / マーカー処理 / create+edit の一本化をオーナーシップとして持たせる（別 SPEC として議論予定）。
3. SPEC 作成直後に `gwtd issue spec <n> --section spec | head -5` を実行して空でないことを **必ず** 検証する。空なら `--edit spec -f` で再投入してから handoff する。
4. SPEC body には `# SPEC: <title>` の H1 で始まり、`--title` 引数と完全一致させる慣習を保つ（既存 SPEC #2133 / #1932 と同じ）。

---

## 2026-05-18 — Launch Agent Execution Mode の Resume を silent downgrade させない

### 事象

ユーザー報告で「Launch Agent の Execution Mode が機能しない」が判明。Execution Mode で
`Resume` を選んでも実際は `claude --continue` / `codex resume --last` 相当の Continue
動作になり、agent の interactive session picker が開かなかった。

### 原因

1. `crates/gwt/src/launch_wizard.rs:1277` で `mode == "resume"` かつ
   `resume_session_id is None` の場合に **silently `SessionMode::Continue` に
   downgrade** されていた。Quick Start 経由でないと resume_session_id がセット
   されないため、通常フォームの Resume は Continue 扱いになっていた。
2. `crates/gwt-agent/src/launch.rs:638-645` で Codex builder は
   `SessionMode::Resume` + `resume_session_id is None` のとき常に `--last` を
   付与しており、仮に (1) を直しても `codex resume` ではなく `codex resume --last`
   になり picker mode が成立しなかった。
3. `execution_mode_value_from_session_mode` (launch_wizard.rs:3631) が
   `SessionMode::Resume` を `"continue"` に collapse しており、前回 profile から
   Resume を復元できなかった。

### 再発防止策

1. 「ユーザー UI 上の選択肢」と「agent CLI に渡す引数」を 1:1 で対応させる。
   UI が `Resume` と表示している以上、内部で `Continue` に黙って差し替えてはいけない。
   downgrade が必要な状況 (capability 不在 等) は明示的に UI から option を除外する。
2. CLI 引数を組み立てる builder では、`SessionMode::Resume` + `resume_session_id`
   の有無で picker mode と id 指定 mode を **別物として扱う**。デフォルトで
   `--last` のような便宜 flag を勝手に足さない。
3. `SessionMode` <-> 表示文字列の変換 helper は片方向 collapse を避け、
   ラウンドトリップ可能にする (`Resume → "resume"`, `Continue → "continue"`,
   `Normal → "normal"`)。
4. 新しい Execution Mode option を追加するときは、agent 側の capability
   (`AgentId::supports_resume_picker()` 等) を view 構築に渡し、対応していない
   agent では option を除外する設計にする。

## 2026-05-15 — Exact hook trust must include generator resolution and shell format

### 事象

PR #2735 の follow-up review で、Docker trust registration が `GWT_HOOK_BIN`
fallback を独自解決しており、hook 生成時の binary path resolution とずれる
可能性を指摘された。また exact command match を現在 runtime の shell 形式
だけに絞ったため、PowerShell 形式で生成済みの Codex hooks を Linux Docker
内の registration が trust できない互換性リスクも残っていた。

### 原因

「suffix ではなく exact command を trust する」という安全側の修正で、
exact command の入力元を current runtime だけに寄せすぎた。hook trust は
実行環境の shell 形式ではなく、既に生成済みの `.codex/hooks.json` に書かれた
正規 command と一致するかを見る必要がある。

### 再発防止策

1. hook trust の exact match を変更する場合は、binary fallback resolution と
   shell command format の両方を generator とそろえる。
2. Docker / container registration は、host-generated command を検証するための
   fallback path と、container 内で実行する `gwtd` path を別々にテストする。
3. POSIX / PowerShell のように生成 shell が複数ある managed command では、
   「現在 OS の command」だけでなく「生成器が出し得る exact command 群」を
   trust 判定の対象にする。

## 2026-05-15 — Hook trust recognizers must not accept shape-only gwtd paths

### 事象

PR #2733 の Codex review で、Codex hook trust 登録が Docker 内では
`/root/.codex/config.toml` 固定になっており、非 root devcontainer の Codex
設定を更新できないことを指摘された。同時に portable hook command の fallback
判定が `.../gwtd` で終わる任意パスを trust 対象にしていたため、
`/tmp/attacker/gwtd` のようなパスでも review signal を消せる状態だった。

### 原因

Docker registration と hook trust recognizer の責務が混ざり、container 内で
実行する `gwtd` と、host で生成された hook command の fallback path を同一視
していた。さらに recognizer が「gwt が生成した正確な command」ではなく
「gwtd らしい suffix」を見ていたため、攻撃者が制御できる fallback path まで
許容していた。

### 再発防止策

1. trust / approval / allowlist 系の matcher は、suffix や shape ではなく
   生成器が出す exact command か、明示的に渡された exact fallback のみを
   許可する。
2. Docker 内で host-generated hook を登録する場合は、実行バイナリ
   (`/usr/local/bin/gwtd`) と trust 対象 fallback (`GWT_HOOK_BIN`) を分離する。
3. container 内の user config path は `/root` 固定にせず、
   `${CODEX_HOME:-${HOME:-/root}/.codex}/config.toml` のように active user から
   導出する。

## 2026-05-15 — Codex hook trust hashing must mirror event matcher semantics

### 事象

gwt-managed Codex hooks を自動 trust 登録する修正後も、Codex の `/hooks`
では `UserPromptSubmit` と `Stop` が `Modified since last trusted` として
残り、起動時に `hooks need review` warning が表示された。

### 原因

Codex 本体は `UserPromptSubmit` / `Stop` の matcher を dispatch でも trust
identity hash でも無視する。一方、gwt 側の trust hash は全 event で
`.codex/hooks.json` の `matcher="*"` を含めて計算していたため、2 event だけ
Codex の `current_hash` と一致しなかった。

### 再発防止策

1. 外部ツールの trust / fingerprint / signature を再実装する場合は、設定
   JSON の見た目ではなく公式実装の正規化後 identity を読む。
2. hook trust の修正では、`codex --no-alt-screen` の起動 warning だけでなく
   `/hooks` の event 別 `Active` / `Review` 表示で全 managed events を確認する。
3. event ごとに matcher semantics が異なる場合は、全 event 同一処理にせず
   `UserPromptSubmit` / `Stop` のような matcher 非対応 event の fixture test を
   必ず追加する。

## 2026-05-15 — Git config emulation must preserve include insertion order

### 事象

PR #2727 の review thread で、`repo_hash` が `url.*.insteadOf` を top-level
Git config からしか読まず、`[include]` / `[includeIf]` 先の rewrite 設定を
無視する問題を指摘された。追修正の初期実装では include 先を親 config の
末尾へ単純追加していたため、include 先が `remote.origin.url` も定義する
場合に、include 行より後ろのローカル設定が勝つという Git の順序とずれた。

### 原因

Git config の include は「親ファイルを読み終えた後に追加する」処理ではなく、
`path = ...` 行の位置に include 先の内容を挿入する処理である。プロセス起動
を避けるために Git config を自前解決する場合、ファイル単位の `Vec<String>`
へ単純 append すると、同じ key が親子 config の両方にあるケースで上書き順序
が変わる。

### 再発防止策

1. `git config` / `git remote get-url` 相当の処理を自前実装する場合は、
   include / includeIf / user config / local config の順序をテストで固定して
   から実装する。
2. `[include]` は include 行の位置で展開し、loop guard と最大深さを持たせる。
   「親を全部読んでから include を末尾追加する」形にしない。
3. review 指摘への follow-up では、指摘された直接ケースだけでなく、隣接する
   Git config precedence の回帰テストを 1 件追加してから GREEN にする。

## 2026-05-12 — Workspace coordination must not become a global tool lock

### 事象

`Similar active Workspace already exists` が Claude Code hooks で発生し、複数の
gwt-* skill / agent が unrelated work でも実装・編集を継続できなくなった。
特に stale な project-level Workspace title や active Board claim が、現在の
Agent の実作業 intent として扱われ、広範囲の PreToolUse denial に波及した。

### 原因

Workspace / Board は duplicate work を避けるための coordination surface だが、
workflow-policy が active Workspace / Board claim / incomplete work item の
semantic similarity を tool 実行前の hard block として扱っていた。さらに
Unassigned Agent の actionable title/focus を mutation 前に強制 materialize
する設計により、Workspace 所属が任意状態ではなく実行前提になっていた。

### 再発防止策

1. duplicate prevention は `gwtd workspace candidates` / `join` / `create` /
   `ensure` の explicit affiliation boundary で扱い、PreToolUse hook では
   Board/Workspace similarity を理由に mutation を hard block しない。
2. Unassigned は正常な affiliation state として扱う。Board post や通常 mutation
   から暗黙に Workspace を create/join しない。
3. coordination policy を変更する場合は、stale projection title、active Board
   claim、incomplete work item、Unassigned actionable intent の各ケースを
   hook tests で固定してから実装する。

## 2026-05-11 — Actionable Unassigned Agents must materialize before mutation

### 事象

Unassigned Agent が `title-summary` 付きの Board milestone や実装作業を
行っても、Agent window title が更新されず、Workspace Overview では多くの
Agent が Unassigned のまま残った。さらに Workspace Overview の Unassigned
領域と detail pane に明示的な scroll containment がなく、表示内容が見切れた。

### 原因

`title-summary` guard は「Assigned Agent の title 欠落」を防いでいたが、
「作業名や current focus を持つ Unassigned Agent は、まず Workspace に
materialize しなければならない」という前提を workflow-policy / Board post
経路で強制していなかった。Board milestone から Workspace history を更新する
経路も origin Agent の affiliation を確認しておらず、Unassigned 起点の
milestone が所属修復なしに履歴だけを更新できた。

### 再発防止策

1. Agent title / Board milestone / Workspace history のいずれかを変更する
   修正では、Unassigned Agent の materialization 経路 (`workspace ensure` /
   join / create) を必ずテストする。
2. Board post で milestone kind (`claim` / `status` / `blocked` / `handoff` /
   `decision` / `next`) を扱う場合、audience 計算より前に Workspace
   affiliation が確定していることを確認する。意図的な broadcast は例外として
   明示的に opt-out させる。
3. Workspace Overview の固定領域を変更する場合は、`min-height: 0` と
   bounded `overflow` の contract test を追加し、Unassigned / columns /
   detail pane のどれも到達不能にならないことを確認する。

## 2026-05-10 — SPEC section edits must preserve section markers and avoid concurrent writes

### 事象

SPEC-2021 の spec/plan/tasks section を複数 `gwtd issue spec --edit ...`
で並列更新したところ、GitHub Issue body の `sections` metadata と comment
section の内容が競合し、`--section tasks` が一時的に読めなくなった。
その後 SPEC-1784 tasks comment を `gh api PATCH` で直接修正した際も、
`<!-- artifact:tasks BEGIN/END -->` marker を付けずに comment body を
置き換えてしまい、`gwtd issue spec 1784 --section tasks` が
`comment ... does not contain section 'tasks'` で壊れた。

### 原因

SPEC section は GitHub Issue body の section index と、body/comment 内の
artifact marker の組み合わせで成立している。並列 edit は同じ index map の
read/modify/write 競合を起こす。さらに `gh api` で comment を直接 PATCH
する場合、section file 本文だけでは artifact marker が不足し、gwtd parser
が section を識別できない。

### 再発防止策

1. 同一 Issue の `gwtd issue spec --edit <section>` は必ず逐次実行し、
   並列化しない。並列化してよいのは section read / grep などの読み取りだけ。
2. 書き込み後は `gwtd issue spec <n> --section <section>` を必ず読み直し、
   対象 section が parse できることと、変更行が反映されたことを確認する。
3. `gh api` で section comment を直接 PATCH する必要がある場合は、本文を
   `<!-- artifact:<section> BEGIN --> ... <!-- artifact:<section> END -->`
   で包む。marker なしの raw section body を送らない。

## 2026-05-10 — Auto-merge can fire before review-feedback corrections land

### 事象

PR #2602 で CodeRabbit からの指摘 (`PRRT_kwDOPLof2M6A4N7G`: `format_board_help_documents_mention_flag` assertion が `*` 反復マーカーを要求していない) を修正する commit (`2a8853f6`) を push した直後、auto-merge が一つ前の SHA (`c3ec646a`) に対して既に発火しており、PR #2602 は強化前のテストのまま develop に landed した。CodeRabbit suggestion を適用したつもりが、その commit は次の PR (#2603) で別途 land させる必要が生じた。

### 原因

`Auto Merge PR` workflow は「PR の checks が全部 green になった瞬間」に発火する。CodeRabbit が PR にコメントしてから、こちらが修正 commit を push して新しい checks が走り始めるまでの空白期間に、prior commit の checks が完了して auto-merge が prior SHA で merge を実行してしまう競合が発生し得る。auto-merge は SHA-pinned ではないため、新 commit は merge 後に取り残される。

### 再発防止策

1. CodeRabbit / Codex review でアクション可能な提案を受けたら、修正 commit を push する **前** に PR の auto-merge 進行度を確認する (`gwtd pr checks <n>` で `Enable auto-merge` 状態確認)。すでに大半が SUCCESS になっている場合、修正 commit を push しても auto-merge が prior SHA を選ぶ余地が残る。
2. もしどうしても prior commit が先に merge してしまうリスクがあるなら、修正 commit は **次の PR で land させる前提** で書く (本 PR #2603 はその対応)。同時に、その follow-up を必ず開くことを reviewer 通知の reply に明記する。
3. `gwtd pr edit <n> --add-label hold` などで auto-merge を一時的に無効化する手段があれば優先する (現時点で gwtd には専用 flag はないので、修正 commit + immediate watch を運用ルールにする)。

## 2026-05-10 — Verify CLI commands and flags against the parser, not just the help text

### 事象

`docs/spec-1939-phase-12-manual-smoke.md` (PR #2599 で develop merged) に、
実在しないサブコマンド `gwtd start-work develop /tmp/...` を含めてしまい、
follow-up PR #2600 で修正したものの、その PR で「`gwtd board post` には
`--mention` フラグは存在しない」と誤った claim を書いてしまった。実際に
は `crates/gwt/src/cli/board.rs:234` の parser に `--mention <kind:id>` が
landing 済みで、tests (`board_family_parse_post_collects_typed_mentions`)
でも `--mention user:akiojin` 形式が gating されていたため、Codex review
(PRRT_kwDOPLof2M6A4JHA) で指摘され、二度目の follow-up が必要になった。

### 原因

最初の修正で `gwtd --help board post` (実体は `crates/gwt/src/bin/gwtd.rs::
format_board_help()`) の出力だけを根拠にし、parser source (`cli/board.rs`)
や cli.rs の集中 usage 文字列を読まなかった。`bin/gwtd.rs` の subcommand
help は手書きで cli.rs / parser から自動生成されておらず、`--mention` の
ように後から landing したフラグが反映されていない場合がある。help text
を「正本」と仮定してしまったのが事故の構造的原因。

### 再発防止策

1. CLI flag / subcommand の有無を doc / memory に固定する場合、`gwtd
   <subcommand> --help` だけでなく、parser 側 (`crates/gwt/src/cli/*.rs`
   の `--flag` arm) と test (`board_family_parse_post_*` 等) の両方で
   確認する。help 文字列は手書きのため、フラグの抜けが発生する。
2. user 向け手順に CLI snippet を埋め込む際は、commit 前に **実 invocation
   で reproduce** してから snippet を確定する (例: 実際に `gwtd board post
   --mention user:akiojin --body 'test'` を投げて成功するかを board entry
   で確認)。help 文字列は古い場合がある。
3. もし help と parser に乖離があったら、help 側も合わせて修正する
   (`crates/gwt/src/bin/gwtd.rs::format_*_help()`)。ドキュメントを書く
   作業の副産物として help 同期も行うことで、次の reviewer が同じ事故を
   踏まなくなる。
4. 既存 doc / 既存テストの flag 列挙 (parser の `--mention` arm、cli.rs
   の集中 usage 文字列) を参照元として優先し、help の subcommand 出力は
   second-source 扱いとする。

## 2026-05-10 — Read tasks/memory.md before designing tests for window interaction features

### 事象

SPEC-2008 Phase 24 (terminal viewport reflow) を `crates/gwt/web/app.js` に
実装した PR #2588 で、frontend test を全て source-string regex assertion で
書いた。直後に「2026-05-07 — Window interaction features need behavior
tests」の memory と矛盾している指摘を別 Agent から受け、後追いで behavior
test ベースの follow-up PR #2590 を作成する手戻りが発生した。

### 原因

実装着手前に `tasks/memory.md` を確認しなかった。ホット領域の memory は
過去の同種失敗をまとめており、参照すれば即座に適切なテスト設計を選べる。
2026-05-07 memory は「window interaction features は behavior test で操作
可能性を検証する。source-string contract は配線漏れ検出に限定する」と
明示していたが、これを参照せずに既存 helper の延長で source-string
assertion だけを書いてしまった。

### 再発防止策

1. `crates/gwt/web/` の interaction (resize / drag / hidden→visible / click
   dispatch / keyboard / pointer) を変更する作業では、最初に
   `tasks/memory.md` の関連 memory (特に 2026-05-07 window interaction)
   を読んで、test 設計を確定してからコードに着手する。
2. test 設計時は behavior test を default、source-string assertion は
   wiring 漏れ検出限定で 1〜2 件に留める。
3. 同種の問題ドメインで複数 memory が並ぶ場合 (2026-05-07 が 2 件あった
   ように)、AGENTS.md の `Self-Improvement Loop` に従い該当領域の memory
   をすべて並べて読み返す。

## 2026-05-10 — gwt-build-spec must preflight Board active claims

### 事象

ユーザー報告のターミナル安定化 3 症状を SPEC-1919 (TTY) と SPEC-2008
(Window host) に分割して並行実装するため、別セッションで
SPEC-1919 PR #2587 を merge 後、続いて SPEC-2008 Phase 24 を
`gwt-build-spec` で着手し PR #2589 を作成した。同時刻に別 Claude Code
セッション (work/20260509-1639) も SPEC-2008 Phase 24 を Board claim
済みで実装し、PR #2588 として先に CI 完走 → auto-merge した。結果
PR #2589 は `merge: CONFLICTING` の重複 PR になり、SUPERSEDED 扱いで
close 待ちとなった。

### 原因

`gwt-build-spec` の Phase 1 (Context Load) が SPEC tasks セクションは
読むが、対象 SPEC に対して **他 agent が active claim を持っているか
を Board から確認するステップを持たない**。SPEC-1935 FR-014b/c の
`board-reminder` は SessionStart / UserPromptSubmit に最近の Board
posts を注入するが、注入のタイミングと skill 起動のタイミングが一致
せず、別セッションの SessionStart context には PR #2588 の claim
post が含まれていなかった。結果として、先行 claim の存在を検知でき
ないまま並行実装に入り、merge 段階で重複が露呈した。

### 再発防止策

1. `gwt-build-spec` / `gwt-plan-spec` / `gwt-discussion` 起動直後
   (Phase 1 内、対象 SPEC 番号確定後) に `gwtd board show` を読み、
   対象 SPEC owner / Phase に対する `[active]` claim を持つ別 session
   が存在するかをチェックする。存在する場合は当該 session への合流
   提案 (handoff request) または work split の議論に切り替える。
2. 上記チェックを skill 内マニュアル運用ではなく `gwtd build start`
   / `gwtd plan start` / `gwtd discuss start` 等のライフサイクル CLI
   側でも実行し、stderr に warning を出すことを検討する (実装は別
   Issue で議論)。
3. 並行作業を意図的に許可するケース (例: 同 SPEC 内で disjoint なファ
   イル境界が明示されている) は Board claim の `Boundary:` 行で明示
   する運用を継続し、preflight はあくまで "知らずに重複する" を防
   ぐためのものとする。
4. Issue として "[skill-preflight] gwt-build-spec / gwt-plan-spec /
   gwt-discussion must check Board active claims before starting"
   を別途登録し追跡する (本 memory とリンク)。

## 2026-05-07 — Hook fixes must separate diagnostics from shipped behavior

### 事象

Codex hook の `session_id` 欠落有無を確認するために、一時的に
`.codex/hooks.json` へ payload dump hook を追加した。その後の実装修正に
入る前に、ユーザーから「今の hooks.json は戻してください」と明示された。
また、CLI hook tests が実行中 Codex セッションの `GWT_SESSION_*` 環境変数
を拾い、非 managed hook のテストが managed Codex hook として扱われた。

### 原因

診断用 hook は現象確認には有効だが、product behavior の修正とは責務が
異なる。診断設定が残ったまま実装に進むと、実際の修正範囲と観測用の
一時変更が混ざる。さらに hook 系のコードはプロセス環境変数に強く依存
するため、テストが現在の Agent 実行環境を暗黙に継承すると期待と違う
経路に入る。

### 再発防止策

1. Hook payload の観測用変更は、確認が終わった時点で必ず戻してから
   product code に着手する。
2. Hook / CLI tests では `GWT_SESSION_ID`、`GWT_SESSION_RUNTIME_PATH`、
   forward URL/token などの runtime env を明示的に set/unset し、現在の
   Agent セッションに依存しない形で検証する。
3. Codex hook の必須フィールドを扱う修正では、欠落時に既存の persisted
   session metadata を破壊しないことを regression test で固定する。

## 2026-05-07 — Window interaction features need behavior tests, not only source-string contracts

### 事象

SPEC-2008 Window Tabs で backend / protocol / persistence と grouped tab
strip は実装済みだったが、ungrouped window から tab group を作る初回
入口がなく、ユーザーは実際にはタブ化できなかった。既存 frontend test
は配線の存在確認に偏っており、実際の操作可能性を検出できなかった。

### 原因

DOM 生成や event 名の存在を source-string contract で固定しても、
interaction behavior は保証されない。操作の入口、対象判定、負例、既存
動作との境界は、ユーザーが実際に使う流れに近い形で検証する必要がある。

### 再発防止策

1. Pointer / DnD / keyboard interaction を追加するときは、操作可能性と
   負例を behavior test で確認する。
2. Source-string contract は配線漏れ検出に限定し、操作可能性の唯一の
   根拠にしない。
3. 詳細な test plan / implementation note は SPEC / PR に集約する。
   関連: SPEC #2008 / PR #2510。

## 2026-05-04 — Audit-driven a11y coverage finds gaps that audit-by-checklist misses

### 事象

SPEC-2356 polish iteration で modal accessibility を完成させた後、当初は
「全 modal に WAI-ARIA dialog convention 適用」「focus trap 実装」で
work が完了したと判断していた。しかし Ralph Loop で iteration を
続ける中で audit-driven approach (各 surface の form fields / status
indicators / error regions / progress bars 等を 1 surface ずつ網羅
チェックする) を採用したところ、以下のような「checklist では拾えな
い」accessibility gap が次々と検出された:

- 動的に生成される input / textarea / select に `aria-label` がない
  (memo / profile / wizard の form fields。`<label>` で wrap されて
  いない 8 fields)
- 動的に生成される `<progress>` element に `aria-labelledby` がない
  (migration の phase progress)
- preset (Add Window) modal が Esc-close handler 配列から漏れていた
  (他の 5 modal は全部 covered だったが 1 つだけ漏れ)
- live-dot pulse が `forced-colors: active` ブロックには入って
  いたが `prefers-reduced-motion: reduce` には入っていなかった
  (PR #2456 で structural assertion が catch)

これらは「modal a11y チェックリストを埋める」感覚では発見できず、
実際に `grep "createElement\\|createNode" | filter` で全 fields を
列挙して 1 つずつ aria-label の有無を audit する作業で初めて捕捉
できた。

### 原因

Accessibility は HTML / ARIA の組み合わせ問題で、cross-cutting concern
として全 surface に均等にかかるが、開発時は surface 単位で feature を
追加する。1 surface 完了時点では他の surface の同類 element の有無
は意識されず、結果として「ある場所では aria-label が wired、ある場所
では wired していない」という不均一さが生まれる。Code review 時にも
「この surface だけ見ているとそれ単体で OK に見える」ため検出されない。

### 再発防止策

1. **Surface 横断 audit を accessibility 追加時の standard practice
   にする。** 新しい aria-* / role / 役割属性を追加した PR では、
   その属性が他の surface でも必要になる可能性を query で確認:
   - `grep -nE 'createElement\\("input"\\)' src/**/*.js` で input 全箇所
   - `grep -nE 'createNode\\("button"' src/**/*.js` で button 全箇所
   - `grep -n 'role="dialog"' src/**/*.html` で dialog 全箇所
2. **Meta-assertion を chrome-structure tests に置く。** 「全
   `[role="dialog"]` element に accessible name があること」のような
   meta-assertion は新しい dialog が追加されただけで自動的に検証
   範囲に含まれるため、surface 横断の網羅を test layer に固定できる。
   `operator-chrome-structure.test.mjs` の `Every role="dialog" has
   programmatic accessible name` test がこの pattern。
3. **Audit list は test file に書く、document に書かない。** Document
   は drift しやすいが test は CI で実行される。「全 form field の
   aria-label coverage」のような expected 一覧は assertion の expected
   array にまとめておく。

### 適用範囲

- 新しい WAI-ARIA pattern (combobox / tablist / tree 等) を追加する PR
- 新しい role / aria-* attribute を 1 surface に wired した PR
- Memory-driven feedback loop の継続的な audit cycle

## 2026-05-04 — `[\s\S]*?` regex undercapture masks bugs in nested CSS blocks

### 事象

SPEC-2356 polish iteration で `@media (prefers-reduced-motion: reduce)`
ブロックを検証する chrome-structure assertion を書いたとき、最初は
naive な regex `/@media\s*\([^)]*\)\s*\{[\s\S]*?\n\}/g` を使った。これだと
ネストしたルール (`.selector { animation: none; }`) の最初の `}` で
マッチが終了するため、ブロック全体ではなく最初のルールしか拾えない。
その結果、`op-live-pulse` を使っている `.op-status-strip__live-dot`
が reduced-motion で無効化されていない (実際は `forced-colors: active`
にしか入っていなかった) 既存バグを assertion がスルーしてしまった。

depth-tracked extraction (`{` `}` を数えて対応する閉じを探す) に
書き直したところ、即座に live-dot が reduced-motion でカバーされて
いないことが捕捉された。

### 原因

CSS の `@media` ブロックは内部に複数のルールを持ち、それぞれが `{}` を
使う。regex の `[^}]*` や `[\s\S]*?\n\}` で「最初の `}` まで」を
切ると、ブロックの先頭の少数ルールしか captured されない。assertion
は cover されたか否かしか見ないので、cover 漏れが「該当ルールが存在
しない」と誤判定され、本来検出すべきバグを GREEN で見逃してしまう。

### 再発防止策

1. **CSS の `@media` / 入れ子ブロックを regex で切るときは brace-depth
   tracking を使う。** ヘルパ関数 `extractMediaBlocks(css, condition)`
   を新設し、`{` と `}` を数えて対応する閉じを探す。テスト時は単純な
   regex に逃げず、ヘルパを再利用する。
2. **新しい coverage assertion を書いたら、その assertion が確実に
   gap を捕捉できる「false case」を意識的に作って verify する。**
   今回の assertion は最初から GREEN で通ってしまっていたが、もし
   live-dot のような実バグが既に存在していれば即座に RED になるべき
   だった。assertion 設計時は「バグがあれば落ちる」を必ず確認する。

### 適用範囲

- すべての CSS 構造解析 assertion (`@media` / `@supports` / `@keyframes` /
  `@layer` 等のネストブロック)。
- 同種のパターンを抱える HTML / source 解析 assertion も同じ落とし穴
  を避けるため、構文認識の必要があるものは regex に逃げず depth /
  parser を使う。

## 2026-05-04 — Chrome-structure assertions alone don't catch contrast regressions

### 事象

SPEC-2356 polish iteration の PR #2439 で、Status Strip の ACTIVE / IDLE
セルに state-color tinting を追加した。chrome-structure assertions で
「`color: var(--color-state-*)` が定義されていること」を機械的に検証して
GREEN になったため merge した。しかし light-theme で実際に rendering を
確認すると、IDLE = 3.49:1 / ACTIVE = 3.15:1 / BLOCKED = 2.61:1 と
すべて WCAG AA (4.5:1) を下回っていた。BLOCKED の 2.61:1 は PR #2439 で
新規導入した訳ではなく、それ以前から存在した既存バグだったが、PR #2439
が assertion で「現状維持」をロックインしてしまった。

### 原因

Status Strip の bg は dark/light 両テーマで dark (`#050709` / `#1a1d24`)
である一方、`--color-state-*` トークンは theme 別に「その theme の bg
に合う色」として tuned されていた。light-theme の state-* は light bg
向けの暗い saturated 色で、dark な strip bg に重ねると contrast が
急落する。chrome-structure tests は「セレクタとプロパティが存在するか」
しか検証していなかったため、実際の color 値と bg の組み合わせは見逃し
た。

### 再発防止策

1. **テキストを表示する chrome surface には必ず contrast assertion を
   追加する** ―― selector 存在チェック (chrome-structure tests) だけでは
   AA 違反を検出できない。`contrast.test.mjs` に theme × state ×
   surface_bg の組み合わせを必ず assert する (PR #2441 で active / idle /
   blocked × dark / light の 6 件 + structure 1 件を追加)。
2. **theme-agnostic な surface (両テーマで bg が同じ系統の chrome) は、
   token を local に scope-override して固定値にする** ―― theme tokens
   をそのまま使うと、片方のテーマで AA を満たしても他方で破綻する。
   `.op-status-strip { --color-state-*: #...; }` のように surface 単位で
   scoped custom property override を入れて、「両テーマで同じ on-bg
   palette」を強制する。

### 適用範囲

- `--color-state-*` を直接 chrome surface に流しているすべての箇所
  (現在は Status Strip のみ; 将来 Header chrome / Floating overlay 等に
  同パターンが現れたら必ず scoped override + contrast test を併設)。
- chrome-structure assertions を新設するときは、検証対象が「テキスト
  色」を含むなら **必ず併せて contrast assertion を追加** する。

## 2026-05-04 — Cache restore failures must not block the release pipeline

### 事象

v9.16.0 の release workflow (run 25321263396) で `Build MSI installer (Windows)`
ジョブが失敗し、後続の `Upload to GitHub Release` と `Publish to npm` が
skip されてリリース全体がブロックした。失敗ステップは
`Swatinem/rust-cache@v2` の Restore Cache。Restoring cache → Post job
cleanup までわずか 1.5 秒で終了しており、他のキャッシュ復元成功
ジョブと同条件のため、cache provider 側の一過性失敗 (flaky) と判断。

### 原因

`Swatinem/rust-cache@v2` は GitHub Actions cache provider の応答失敗時
にステップ自体を fail させる。すべての rust-cache 使用箇所に
`continue-on-error` を付けていなかったため、cache infra の一過性
障害がそのままジョブ失敗→release 全体失敗へ伝播した。Cache は
ビルドの最適化であり correctness 要件ではないので、cache restore
失敗時はキャッシュなしで build を続行すべきだった。

### 再発防止策

すべての workflow (`release.yml` / `build.yml` / `test.yml` / `lint.yml`
/ `coverage.yml`) の `Swatinem/rust-cache@v2` ステップに
`continue-on-error: true` を付与。cache provider の一過性失敗で job
が落ちず、cache miss としてビルドが続行する。

### 適用範囲

- `.github/workflows/release.yml` (build-gwt, build-msi, build-dmg)
- `.github/workflows/build.yml` (build, check-windows)
- `.github/workflows/test.yml` (test, test-index-e2e, test-windows-rust)
- `.github/workflows/lint.yml` (lint)
- `.github/workflows/coverage.yml` (coverage)

### 補足: 同種パターンへの一般化

外部 GitHub Action のうち「ビルド/テスト correctness の前提ではない
最適化系の action (cache, telemetry, artifact mirror など)」は、
`continue-on-error: true` を default にして infra 由来の一過性失敗で
リリース pipeline が止まらないようにする。Correctness 要件のステップ
(toolchain install / build / test / sign / publish) は今まで通り
default fail-fast を維持する。

## 2026-05-04 — Board projection guards must distinguish append order from chronology

### 事象

PR #2390 の Codex review で、 `projection_needs_rebuild` が
`manifest.last_entry_id` を Board projection の `newest_entry_id` と
比較している点を指摘された。 late legacy `events.jsonl` import では、
古い `created_at` の legacy entry が新しい segmented entry の後に
append されうる。その場合、 projection は時系列順で正しく
`newest_entry_id = newer segmented entry` を指すが、 manifest の
last append は older legacy entry になる。

### 原因

Board の hot projection は chronological order の view だが、 segment
manifest の `last_entry_id` は append order の metadata。 両者を同じ
"newest" として扱ったため、 backdated legacy import 後に整合済み
projection を毎回不整合扱いし、 load hot path で不要な rebuild を
繰り返す可能性があった。

### 再発防止策

storage metadata を guard に使う前に、 その metadata が **append
order / chronological order / update order** のどれを表すかを明示する。
異なる order 軸を比較しない。 hot projection の整合性を見る場合は、
最後に append された entry が projection の時系列 window に入るとき
だけ「projection 内に存在するか」を検査し、 chronological newest ID
とは比較しない。

## 2026-05-04 — multi-round correction loops on the same docs are a smell, not a feature

### 事象

architecture.md / README の daemon 説明を refresh しようとして、 同一の
docs を 3 ラウンド連続 codex P2 で訂正することになった
(PR #2335 → #2336 → #2337):

1. **PR #2335**: `gwtd daemon` が GUI 起動時に "auto-bootstrap" すると
   記載 → 実装は endpoint メタデータを書くだけで daemon サーバーを
   起動しないため事実誤認。 加えて endpoint ファイル名を
   `endpoint.json` と書いたが、 実際は
   `RuntimeScope::endpoint_path` で `<worktree-hash>.json`。
2. **PR #2336**: 上記 2 件を訂正したが、 「最初の `gwt` GUI 起動時に
   endpoint メタデータが記録され、 後続プロセスが live な daemon を
   見つけられる」と書き直した → これも誤り。 `is_alive` predicate が
   `|pid| pid == self.pid` という狭い条件のため、 `gwtd daemon
   start` の endpoint を「dead」扱いで削除し sentinel で上書きする。
   GUI 起動が live daemon の endpoint を **clobber する** という
   逆方向の事実だった。
3. **PR #2337**: 残りの誤った claim をすべて削り、 verifiable な記述
   だけに切り戻した。 副産物として、 codex の指摘から実装の
   front-door clobber bug が浮上したため Issue #2338 として登録。

### 原因

各ラウンドで「修正したつもり」でも、 自分が新しく書いた wording が
別の事実誤認を含むまま push し、 codex が次の round で別の角度から
反証してきた。 共通する根本原因は **ストーリー先行で書いた wording
を、 実際の code path で逐語検証していなかった** こと。 PR #2313 で
書いた `tasks/memory.md` rule 4 そのものに自分自身が複数回失敗した
形になる。

具体的に逃した検証ステップ:

- "auto-bootstrap" と書く前に `serve_blocking` の caller を grep
  すれば、 GUI front door からは呼ばれていないことがすぐ判った
- "endpoint.json" と書く前に `endpoint_path` の実装を読めば、
  filename が `<worktree-hash>.json` であることが判った
- "subsequent processes can find a live daemon" と書く前に
  `prepare_daemon_front_door_for_path` の `is_alive` closure
  を読めば、 自分の PID 以外を全部 dead 扱いする狭さに気づけた

### 再発防止策

1. **同一 PR が 2 ラウンド以上 codex P2 を受けたら、 反射的に追加
   修正で押し返さず、 一度全文を verifiable claim だけに削ぎ落とす**。
   "自分の story を docs で再構築する" モードに入っている合図。
2. **daemon / IPC / runtime のような並行系挙動を docs で説明する
   ときは、 各文を書く前に「この文を反証できる code path はないか」
   を 1 文ごとに 30 秒だけ自問する**。 grep 1 回で済む確認が
   review round 1 周分のコストを節約する。
3. **副産物として bug を発見したら、 docs PR の場で fix まで踏み込
   まず Issue 化する**。 PR #2337 → Issue #2338 が良い分離例。
   front-door clobber は SPEC-2077 owner の design 判断が要るため
   autonomous fix の対象ではない。
4. **memory.md の rule 4 自体を、 失敗するたびに具体例として
   追記する**。 抽象ルールだけだと自分の脳内で「適用済」と錯覚
   しやすい。 具体 instance が増えるほど、 同じ罠の sniff test が
   早く効くようになる。

PR #2305 / #2310 / #2311 / #2315 (codex P2 round 1) と本ラウンド
(round 2) の両方が「verify-before-write」の同じ rule を破った
事例なので、 Phase H2-H4 docs を書くときは特に注意する。

## 2026-05-04 — review feedback: claims must be verified against the actual code path

### 事象

このセッションで codex review が同じパターンの P2 finding を 3 回連続で
出した。 PR #2305 (regression test の `tokio::time::sleep(300ms)` で
stale-token attempt が起きる前に rotation が走る race)、 PR #2310
(README が daemon auto-bootstrap を全プラットフォーム共通として記述)、
PR #2311 (Windows note が `gwtd daemon status` も unavailable と記述)。
すべて「自分が書いた claim が actual code path で本当に成立するか」
を verify せずに ship したのが共通原因。

### 原因

3 件の root cause:

1. **PR #2305 (test race)**: subscriber thread が CI 上で遅れて起動した
   場合、 sleep 後の resolver flip より前に thread が動かないので、
   `stale token → rejected → reconnect with live token` の regression
   path が exercise されない。 「sleep 300ms すれば必ず stale 試行が
   先に走る」という暗黙の前提が壊れる。
2. **PR #2310 (README scope)**: daemon の auto-bootstrap / fan-out は
   `#[cfg(unix)]` only。 Windows では `gwtd daemon start` は "not
   yet implemented" を返すだけ。 README で無条件に "auto-bootstraps
   a per-project runtime daemon" と書くと Windows ユーザーが期待
   外動作に当たる。
3. **PR #2311 (Windows note overscope)**: `gwtd daemon status` 自体は
   全プラットフォームでコンパイルされる (`crates/gwt/src/cli/daemon/mod.rs::run`
   の Status arm に cfg gate が無い)。 Windows でも実行できて
   `stopped scope=...` を返すが、 README で "unavailable" と書くと
   ユーザーが diagnostic command を skip してしまう。

3 件とも「claim を書く時に grep / read で対応箇所を確認」していれば
review 前に気づけた:

- (1) 「sleep が必ず先に観測される」は TIMING の claim → resolver の
  call counter で観測可能な signal にできる
- (2) 「全プラットフォームで auto-bootstraps」は PLATFORM の claim →
  `#[cfg(unix)]` を grep すれば限定範囲が分かる
- (3) 「Windows で daemon status は unavailable」は AVAILABILITY の
  claim → `report_status` の cfg を確認すれば全プラットフォーム可と
  分かる

### 再発防止策

test や docs を書く前に「自分の claim を 1 つに絞り、 actual code
path で verify する」step を必須にする:

1. **Test の wait condition は wall-clock sleep ではなく observable
   signal**: 「N ms 待てば X が起きているはず」は CI の負荷で容易に
   崩れる。 代わりに `AtomicUsize` の counter / `Notify` /
   `RwLock<Vec<Event>>` などで「観測したい遷移そのもの」を polling
   する loop を書く。 wait の上限は CI slow path 用に generous に。
2. **Platform / cfg の claim は cfg を grep して verify**: docs や
   user-facing comment に「X is available on Y」と書く前に、
   `grep -n "cfg.*<platform>" path/to/source.rs` で対応する `#[cfg(...)]`
   を確認する。 unsupported / partially-supported を分けて書く。
3. **"Available" vs "Useful" を区別**: コマンドが compile されるか
   と、 そのコマンドが意味のある結果を返すかは別。 Windows daemon
   status は compile される (available) が、 daemon が動かないので
   結果は常に `stopped` (limited usefulness)。 docs では両方を分けて
   表現する。
4. **Review 前 self-check**: 自分が書いた claim を 1 つずつ抜き出し、
   「これを否定する code path はないか」を確認する。 sleep が race
   を隠していないか、 cfg gate が範囲を狭めていないか、 「unavailable」
   claim が実は compile される command を含めていないか。

これらは Phase H2-H4 や future README 更新でも同じ落とし穴が再現する
ため、 docs を書く前 / regression test を書く前にチェックリスト化
する。

## 2026-05-04 — fix(daemon): broadcast forwarder treated RecvError::Lagged as fatal

### 事象

SPEC-2077 Phase H1 の per-channel broadcast forwarder
(`crates/gwt/src/cli/daemon/server.rs`) が `broadcast::Receiver::recv()`
の `Err(_)` をすべて fatal 扱いして loop break していたため、 slow
subscriber が DEFAULT_CHANNEL_CAPACITY (64) を超えて lag した瞬間に
購読が無音で終了する。 client 側は出口 EOF だけを観測し、 reconnect
からのフル resubscribe が必要だった。

### 原因

`tokio::sync::broadcast::error::RecvError` には 2 variant あり、
意味が完全に異なる:

- `Lagged(u64)`: 「N frames を捨てたよ」という回復可能な warning
  signal。 channel そのものは健全で、 次の `recv()` は新しい frame
  を返す。
- `Closed`: 全 Sender が drop された terminal 状態。 これ以降 frame
  は来ない。

両者を `Err(_)` でまとめて break すると、 lag burst で副次的に
subscription 全体が落ちる。 capacity 64 は通常運用では十分だが、
forwarder task が runtime 上の他タスクで一瞬 starve した間に publish
バーストが来ると現実的に発生する。

同じ落とし穴は他の async primitive にも潜む:

- `mpsc::error::TryRecvError::Empty` (一時的に空) vs `Disconnected`
  (terminal) を `Err(_)` でまとめると spurious wakeup を terminate
  扱いしてしまう
- `std::io::ErrorKind::WouldBlock` / `Interrupted` を `Err(_)` で
  まとめると nonblocking IO で正常な再試行を諦める
- `tokio::time::error::Elapsed` (timeout、 retry 可能) と underlying
  IO error (terminal) を区別せず break する

### 再発防止策

並列 / 非同期 primitive の `Result::Err` を match するときは、
**variant ごとに recoverable / terminal を明示分類する**:

1. `RecvError`, `TryRecvError`, `io::ErrorKind`, `Elapsed` などの
   error 型を `Err(_)` でまとめて catch しない。 必ず variant を
   pattern match して各 variant の意味を確認する。
2. recoverable variant (Lagged / WouldBlock / Interrupted /
   Empty / Elapsed) は **continue + 観測のための warn log**、
   terminal variant (Closed / Disconnected / 真の IO error) は
   **break / 上位エラー化** に分岐する。
3. forwarder のような producer-consumer pattern では、
   recoverable error 経路を必ず unit test で pin する。 underlying
   primitive (broadcast::Receiver, mpsc::Receiver) に対してオーバー
   フロー / drain 試験を書き、 「subscription は alive のまま、
   次の recv() で newer frame が返る」契約を assertion で固定する。
4. defensive code は code comment で **どの variant が来うるか** /
   **どう振り分けたか** を明記する。 単に `match err` だけだと
   reviewer は「全 Err が同じ意味」と読みがち。

これは Phase H2-H4 で `handle_runtime_output` / `handle_runtime_hook_event`
を daemon 経由化するときも同じパターンが再出現する。 broadcast
channel に乗せる別 channel を増やすたびに、 forwarder の error
handling を見直すこと。

## 2026-05-03 — fix(daemon): SPEC-2077 Phase H1 GREEN review-driven hardening

### 事象

SPEC-2077 Phase H1 の daemon IPC 経路 (PR #2278〜#2300) を ship した
ところ、 codex / coderabbit reviewer から P1 / P2 合計 12 件の race /
leak / blocking 指摘を受け、 #2295〜#2300 の 6 PR で逐次修正することに
なった。 主要なものは:

- Subscribe ack 前の Event を `_ack` で吸収して取りこぼす race
- Publish Ack と broadcast Event の send 順序が forwarder task の
  spawn 順に依存して非決定的 (FSM 崩壊)
- 接続終了時に forwarder task が `out_tx` clone を握り続けて writer
  /connection task / ConnectionGuard が leak、 connections counter が
  inflate
- `BroadcastHub::publish` が channels mutex を hold しながら
  `Sender::send` (内部で payload clone) を呼び、 cross-channel HOL
  block
- 非-Unix の `is_process_alive_pid` が常に true で stale endpoint が
  永続的に「running」報告
- protocol version 据え置きで旧 daemon と新 client が handshake 後に
  schema mismatch
- CLI 短命プロセスで detached publish thread が process exit と共に
  kill され broadcast 喪失
- daemon 起動時 readiness lines が `&mut String` buffer 経由で
  shutdown 後にしか visible でない

### 原因

**根本原因の共通テーマは「async / 並列 primitive の lifetime と
synchronization の組合せを設計時に詰めていない」ことだった。**

具体的に言語化すると:

1. **`Notify::notify_waiters` は fire-and-forget**: 待機者がいない
   タイミングで notify すると永久に消える。 これに気づかず stop
   signal として単独で使うと race window が残る。
2. **broadcast `Sender::send` は payload clone**: ロックを跨いだ
   呼び出しは fan-out 規模に応じて lock hold 時間が線形に伸びる。
3. **mpsc の Sender clone は senders count を増やす**: 全 clone が
   drop されないと receiver は close されない。 spawned task に
   clone を渡す場合、 task 終了経路を必ず設計する。
4. **Spawn 時 capture の値は immutable な snapshot**: daemon
   restart のような外部状態変化を待ちたい場合、 closure が新値を
   resolve できる resolver pattern にする。
5. **detached thread は process lifetime に縛られる**: 短命 CLI
   process では sync で完結させ、 長命 GUI process でのみ thread
   spawn に逃がす。
6. **wire schema 変更 は protocol version bump とセット**: handshake
   は通って後段で frame parse 失敗、 という「中間状態互換」の罠。

### 再発防止策

新しい daemon 機能 / 並列 primitive を追加するときは、 review に
出す前に以下を順に確認する:

1. **Async cancellation pattern**: `tokio::sync::Notify` を stop
   signal に使うときは必ず `Arc<AtomicBool>` と pair にし、 select
   loop の先頭で flag を再 load する。 race window を closure 化
   しない。
2. **mpsc Sender lifetime**: spawned task に `out_tx.clone()` を
   渡したら、 reader 終了時に「全 forwarder を起こす経路」
   (select! の cancel arm + flag) を必ず実装する。 drop だけでは
   broadcast 受信中の forwarder は永久に park する。
3. **Mutex hold range**: 共有 mutex 配下で重い操作 (clone, send,
   network I/O) を呼ばない。 必要なものを Arc clone で snapshot
   して guard を drop してから動かす。
4. **CLI / GUI thread spawn の分離**: 短命 CLI command path では
   detached thread を使わず sync 完結。 Long-running GUI / daemon
   path でのみ thread spawn を許す。 各 caller のプロセス
   lifetime を意識する。
5. **External state を closure で再 resolve**: long-lived consumer
   (subscriber 等) が外部 endpoint / config に依存する場合、
   constructor で endpoint を受け取るのではなく `Fn() -> Result<T>`
   resolver を受け取り、 必要な瞬間に都度 resolve する。
6. **Wire schema 変更 = protocol version bump**: post-handshake の
   frame schema を typed enum に変える、 新 variant を追加する、
   serde tag を変えるなどはすべて protocol version bump とセット。
   bump で endpoint reuse を拒否させ、 強制 respawn で互換性を切る。
7. **CLI buffered output**: long-running command (`gwtd daemon
   start` のような) では readiness 出力を `&mut String` に貯めない。
   `&mut dyn io::Write` を受け取って即時 flush することで supervising
   script から observable にする。
8. **Subscribe / RPC ack は frame 型を validate**: ack 待ちで最初の
   1 フレームを `_ack: T` で discard すると、 race で先に来た
   broadcast event を吸収する。 `read_frame::<DaemonFrame>()` を
   loop して `match` で `Ack` を観測するまで他 frame は callback /
   stdout に流す。

これらは Phase H2-H4 (handle_runtime_output / hook_event /
launch_complete) の daemon 移行でも同じ pattern が再出現するため、
最初からテンプレに組み込んでから書く。

## 2026-04-30 — fix(skills): managed skills must not assume gwtd is on PATH

### 事象

`$gwt-discussion` / `$gwt-manage-pr` などの managed skill 実行時に、
Codex セッションの `PATH` へ `gwtd` が存在しないため、CLI 呼び出しが失敗する余地が残っていた。
同時に npm package の `bin` は `gwt` だけを公開しており、release bundle に `gwtd` が
含まれていても `gwtd` command shim としては利用できなかった。

### 原因

`GWT_BIN_PATH` 注入は実装済みだったが、managed command asset は
`GWT_BIN="${GWT_BIN_PATH:-gwtd}"` のままで、`GWT_BIN_PATH` がない環境では bare `gwtd`
lookup に戻っていた。さらに macOS の DMG は GUI-first 配布であり、CLI を PATH に置く
install script が実体として存在しなかった。

### 再発防止策

1. managed skill / command asset は `GWT_BIN_PATH`、PATH 上の `gwtd`、repo-local
   `target/debug/gwtd` の順で解決し、見つからない場合は actionable error を出す。
2. release / npm / install script の配布契約は `gwt` と `gwtd` を必ず同時に検証する。
3. PATH 問題の修正では、実行ログの症状、package shim、install guidance、managed asset の
   実テキストを同時に確認する。

## 2026-04-30 — fix(ci): ローカル lint は CI と同じ package selection でも確認する

### 事象

PR #2230 の `Clippy & Rustfmt` で、Linux CI の
`cargo clippy -p gwt-core -p gwt --all-targets --all-features -- -D warnings` が
`crates/gwt-agent/src/environment.rs` の `push_path_value` を dead code として失敗した。
手元では先に workspace 全体の clippy を通していたが、CI と同一の package selection では
確認していなかった。

### 原因

`push_path_value` は macOS の `path_helper` 経路だけで使われる helper だったが、
`#[cfg(not(windows))]` で Linux にも compile されていた。workspace 全体の all-targets では
test target 側の利用条件に隠れ、PR CI の lib dependency build でだけ dead code が表面化した。

### 再発防止策

1. PR lint を確認するときは、広い workspace clippy に加えて CI と同一の
   `cargo clippy -p gwt-core -p gwt --all-targets --all-features -- -D warnings` も実行する。
2. OS 固有 helper は「使われる呼び出し元」と同じ `cfg` 条件まで狭め、Linux CI に不要な symbol を残さない。
3. CI で失敗したら、ローカル成功結果より GitHub Actions の exact command と package selection を優先して再現する。

## 2026-04-30 — fix(gui): 修正済み判断の前にインストール済みアプリの最新ログで再検証する

### 事象

v9.11.9 適用後も `$gwt-discussion` / Codex 起動で
`PTY creation failed: Unable to spawn npx because: No viable candidates found in PATH "/usr/bin:/bin:/usr/sbin:/sbin"`
が再発していた。`~/.gwt/projects/8a5edab282632443/logs/gwt.log.2026-04-30`
には 2026-04-30T11:36:32+09:00 の失敗が記録されており、アプリも
`/Applications/GWT.app` v9.11.9 だった。

### 原因

前回修正は PTY spawn 時に `config.env["PATH"]` から bare command を解決するだけで、
GUI / LaunchServices 由来の最小 `PATH` そのものを Host launch env で復元していなかった。
また、project log はエラーを記録していたが、PTY に渡した `PATH` でコマンドが解決可能かの
診断がなく、原因の切り分けが遅れた。

### 再発防止策

1. 「修正済み」と判断する前に、インストール済みアプリの version、起動中プロセス、
   project-scoped log の最新失敗時刻を確認する。
2. GUI Host 起動の `PATH` 問題では、runner 選択だけでなく Host base env 生成元を検査する。
3. PTY spawn failure では、エラー本文に加えて `env_path`、path entry 数、
   command の env PATH 解決可否を structured log に残す。

## 2026-04-30 — fix(gui): PTY creation failure は runner 名ではなく起動環境から切り分ける

### 事象

`$gwt-discussion` 起動時に `PTY creation failed: Unable to spawn npx because:
No viable candidates found in PATH "/usr/bin:/bin:/usr/sbin:/sbin"` が出た際、最初に
`npx` / `bunx` の runner 選択へ寄せて考えすぎた。ユーザーから「本質はそこではない」と
指摘され、GUI から起動される PTY の effective environment が active profile と
OS 環境を正しく反映していないことが根本原因だと整理し直した。

### 原因

エラーメッセージ上の executable 名に引きずられ、`PATH` が最小値
(`/usr/bin:/bin:/usr/sbin:/sbin`) になっている事実を起点に、
GUI process env、active profile、disabled env、PTY spawn inheritance の境界を
先に確認しなかった。

### 再発防止策

1. `No viable candidates found in PATH` 系の PTY 起動失敗では、runner fallback より先に
   GUI process env と session effective env を確認する。
2. profile env の修正では、`OS env - disabled_env + env_vars` が preview だけでなく
   shell / agent / package runner probe / PTY spawn まで同一に届くことをテストする。
3. `disabled_env` は env map からの欠落だけでなく、inherited env の明示削除として扱う。
4. PTY spawn の確認では、env を子プロセスへ渡すだけでなく、bare command の
   executable lookup が effective `PATH` を使うことを regression test で固定する。

## 2026-04-29 — fix(launch): Launch Wizard は runtime 検出と実行可否検証を混ぜない

### 事象

Launch Agent の初期表示が Docker status probe に引きずられて遅延し、Codex などの
agent 選択でも Host 側の検出結果により `Agent option is unavailable` が出た。
ユーザーから、Docker での起動が存在するのに Host PATH / Docker status を
Wizard の選択可否へ使っている点を再度指摘された。

### 原因

Wizard hydration が Docker file/config detection と `docker compose ps` による
runtime status detection を混同していた。さらに agent option の `available` を
Host 上の agent 検出結果として扱い、その値で built-in / custom agent の選択と
launch config build をブロックしていた。

### 再発防止策

1. Wizard は候補提示と設定収集だけを行い、Host/Docker/将来 runtime の実行可否は
   Launch preparation で選択 runtime に対して検証する。
2. Docker/DevContainer 検出は Wizard 初期表示ではファイル存在確認と設定ファイル
   読み取りに限定し、Docker CLI / daemon / `docker compose ps` は呼ばない。
3. built-in agent と configured custom agent は Host 検出結果で無効化しない。
   command/package runner 不在は session/tab 作成前の preparation error として扱う。
4. 同種の修正では、Docker CLI を呼ばない regression test と、Host 未検出 agent が
   選択できる regression test を先に RED にする。

## 2026-04-29 — fix(gui): Agent 起動失敗は UI エラーだけでなく構造化ログに残す

### 事象

Launch Wizard で `Agent option is unavailable` が表示されても、`~/.gwt/logs/`
から同じ失敗を追跡できず、E2E/手動確認時の原因切り分けが困難になった。

### 原因

`handle_launch_wizard_action` と async launch completion の失敗経路が UI state
更新だけで完結しており、wizard id / tab id / selected agent / window id などの
調査に必要な文脈をログへ出していなかった。

### 再発防止策

1. Agent / Shell / Launch Wizard の起動失敗は、ユーザー向けエラー表示と同時に
   structured error log を出す。
2. ログ追加時は env/API key/hook token/raw command args を含めず、再現に必要な
   id と stage のみを出す。
3. 起動失敗の修正では、ログ出力を捕捉する regression test を RED にしてから
   実装する。

## 2026-04-29 — test(gui): Launch Wizard 修正は action flow と E2E まで同時に固定する

### 事象

Launch Wizard の agent 選択不具合で、原因調査と unit regression だけを先行し、
ユーザーから「テストやE2Eテストを実行していないのですか？」、
「調査と同時にテストしてください」と指摘された。

### 原因

状態モデル上の `agent_id` と UI の `selected` index がずれる不具合は、
`LaunchWizardState` 単体だけでなく `handle_launch_wizard_action` 経由の
submit path まで確認しないと、実際の GUI launch path の再発防止にならない。

### 再発防止策

1. Launch Wizard の選択・submit 系修正では、state unit test と runtime action flow test を同時に追加する。
2. ユーザー操作で起きる GUI 不具合は、調査と並行して再現テストを RED にしてから実装する。
3. 完了前に unit / integration / frontend smoke / ignored E2E の実行結果を揃えて報告する。

## 2026-04-29 — fix(shell): GUI shell terminal は login shell 初期化を読む必要がある

### 事象

`gh` を `~/.local/bin` に配置しても、gwt の Shell terminal 側では OS / user shell
環境が反映されず、PATH 上のコマンドとして見えない可能性が残っていた。ユーザーから
「エージェント起動時には割り当てられているが、シェルターミナル起動時には
OS 環境変数が割り当てられていない」と指摘された。

### 原因

- agent 起動経路と shell terminal 起動経路を混同していた。
- Shell terminal は `detect_shell_program()` で `/bin/zsh` などを引数なし起動しており、
  macOS GUI / launchd 由来の最小環境から、ユーザーの login shell profile を読めていなかった。

### 再発防止策

1. GUI から通常 shell を起動する場合は、Terminal.app 相当の login shell 起動かを確認する。
2. PATH 問題では、binary の配置だけでなく、起動元 process env と shell 初期化経路を分けて調査する。
3. agent env 注入で直る問題と、plain shell terminal の OS / user env 継承問題を別経路として扱う。

## 2026-04-29 — fix(hooks): timeout 原因は実測なしに単一要因へ断定しない

### 事象

Hook timeout 調査で、古い複数 hook 設定と `gwtd hook` の再 spawn が見つかった段階で、
それを `Hook timed out` の直接原因として強く扱いすぎた。ユーザーから
「本当にそれであっていますか？」と指摘され、追加実測の結果、素の handler は短時間で
戻るため、直接原因は session 環境や state 条件を含めて未確定だと分かった。

### 原因

- 「確実な悪化要因」と「timeout の根本原因」を分けて説明できていなかった。
- handler 別 duration や実際の hook command path を観測する仕組みがないまま、
  既存コード構造から原因を推定した。

### 再発防止策

1. 性能問題では、実測済みの事実、強い仮説、未確定事項を分けて報告する。
2. hot path の改善は進めても、timeout の根本原因は handler 別 timing などの
   診断で確定してから説明する。
3. stale config や余分な process spawn は悪化要因として直すが、それだけで
   symptom が完全解消すると断定しない。

## 2026-04-27 — fix(branches): HTML class refactor must extend contract guard to JS-side selectors

### 事象

SPEC-2008 FR-035 (`de81fa24`) で modal shell class を `.modal` → `.modal-shell` に
統一した際、`crates/gwt/web/app.js:41` の `branchCleanupModal.querySelector(".modal")`
だけ移行が漏れた。結果として v9.11.0 で Branches → Cleanup を開くと
`branchCleanupDialog` が `null` になり、`renderBranchCleanupModal` が最初の
DOM mutation で `TypeError` を投げて終了。modal-backdrop と空の `.modal-shell`
チャラだけが残り「タイトルだけ表示されて中身がない」状態になった。

### 原因

SPEC-2008 のコントラクトテスト
`embedded_web_existing_modals_compose_with_modal_shell_primitive`
(`crates/gwt/src/embedded_web.rs`) は HTML 側の class 移行のみを assert していた。
JS 側に廃止クラスのセレクタが残っていないかは検査されておらず、
`grep` で見つかる単一の取りこぼしを CI が検出できなかった。

### 再発防止策

1. HTML の primitive 命名 (class/id) を変更するリファクタでは、JS 側の
   `querySelector` / `getElementById` セレクタを同じコントラクトテストで
   guard する。SPEC-2008 系では `embedded_web_app_js_uses_modal_shell_selector`
   を追加し、`querySelector(".modal")` の残存を assert で検出するようにした。
2. modal や surface など共有 primitive を経由するロジックは、依存注入可能な
   形に切り出して DOM smoke test (Node + linkedom) で本文描画まで検証する。
   今回は `crates/gwt/web/branch-cleanup-modal.js` を抽出し
   `crates/gwt/web/__tests__/branch-cleanup.smoke.test.mjs` で stage 別に
   描画内容を assert している。
3. リリース直後の hotfix ブランチでは、grep で対象クラスの残存を
   全リポジトリ走査することを必ず行う。

## 2026-04-27 — fix(docker): format! の `\<改行>` 継続は次行の先頭空白も削除する

### 事象

Launch Agent で Docker を選択すると `Docker error: services must be a mapping`
が返り、エージェントが起動できなかった。`docker-compose.gwt.override.yml` を
`format!()` で生成しているが、`services:` 配下のインデントがすべて消えており
`services` キーが null（mapping ではない）になっていた。

### 原因

Rust の文字列リテラルは `\` の直後の改行 **と次行の先頭空白すべて** を削除する。
`format!("services:\n  {svc}:\n    volumes:\n")` のような複数行を
`\<改行>` で継続して書くと、2 スペース・4 スペースの YAML インデントが全部消えて
完全フラットな YAML が生成される。同じバグが
`crates/gwt/src/docker_launch.rs`、`crates/gwt-agent/src/prepare.rs`、
`crates/gwt/src/docker_setup.rs` の 3 箇所にコピペされていた。
既存テストは `content.contains(...)` だけで内容文字列を確認しており、
YAML を parse して構造検証していなかったため検出されなかった。

### 再発防止策

1. 複数行リテラルを組み立てるときは `\<改行>` 継続を使わず、
   `concat!("line1\n", "line2\n", ...)` で連結するか、
   1 行 `\n` 埋め込みに統一する。インデントが必須の format（YAML/Python等）では
   `\<改行>` 継続を使わない。
2. 生成 YAML / JSON のテストは `content.contains(...)` で文字列検証するだけでなく、
   必ず `serde_yaml::from_str` / `serde_json::from_str` で parse してから
   構造を assert する。
3. 同一ロジックの複数コピーは構造的負債。新規バグ修正時に発見したら
   修正範囲を 3 箇所同時にし、follow-up Issue で共通化を検討する。

## 2026-04-27 — fix(process): timeout では process tree を閉じてから reader を join する

### 事象

timeout 付き subprocess helper で stdout/stderr を drain thread に移したあと、
direct child だけを kill して reader thread を `join()` すると、子孫 process が
pipe handle を継承して生存している場合に EOF が届かず、timeout 後の return が待たされた。
一方で reader thread を detach するだけでは、子孫が出力を続けたときに thread と buffer が
background に残り得る。

### 原因

timeout/error path で direct child、子孫 process、pipe reader の lifecycle を一体で扱わず、
process tree を閉じる前に reader の終了保証だけを求めていた。

### 再発防止策

1. timeout/error path では direct child だけでなく process tree / process group を終了させる。
2. process tree を閉じて EOF を発生させたあとに reader thread を join し、background leak を避ける。
3. 子孫 process が pipe handle を保持する command で、timeout 後に短時間で戻ることをテストする。

## 2026-04-27 — fix(process): timeout 付き process は pipe を実行中に drain する

### 事象

Branch Cleanup の remote delete timeout 対応で `git push --delete` の stdout/stderr を
pipe したが、child 終了後にだけ `wait_with_output()` 相当の回収を行っていたため、
remote hook などの出力が多い場合に pipe buffer が詰まり、child が終了前に block して
false timeout になる可能性があった。

### 原因

`Command::output()` は実行中に pipe を drain するが、timeout polling のために
`spawn()` + `try_wait()` へ置き換えた際、その drain 責務を移植していなかった。

### 再発防止策

1. timeout 付き subprocess helper で stdout/stderr を pipe する場合、child 実行中に別 thread
   または async task で drain する。
2. `Command::output()` から `spawn()` polling へ置き換える変更では、exit status だけでなく
   stdout/stderr capture の同等性を回帰テストで固定する。
3. verbose child のテストを追加し、pipe buffer 詰まりによる false timeout を検知する。

## 2026-04-27 — fix(cleanup): frontend timer で backend 実行結果を推測しない

### 事象

Branch Cleanup で remote branch delete を有効にすると、backend の cleanup thread と
`git push --delete` がまだ実行中でも、frontend の固定 30 秒 timer が先に
`Branch cleanup timed out` を表示できる状態だった。

### 原因

結果確定の source of truth が backend の `branch_cleanup_result` ではなく、
frontend の推測 timer にも分散していた。remote delete はネットワーク・認証・複数 branch
処理で 30 秒を超え得るため、UI だけが failure に進む race があった。

### 再発防止策

1. 長時間 backend operation の UI は frontend timer で failure 確定せず、backend result
   event または connection loss を結果確定 signal にする。
2. remote / network を含む Git 操作の timeout は backend 側で明示的に扱い、per-branch
   result として success / partial / failed に落とす。
3. frontend contract test では user-facing timeout copy だけでなく、固定 timeout 定数が
   残っていないことも確認する。

## 2026-04-27 — fix(board): GUI watcher は hot path で同期せず lifecycle owner を持つ

### 事象

Board projection watcher を `UserEvent::Frontend` ごとに同期していたため、
terminal input など高頻度イベントのたびに project root の canonicalize と watcher
lookup が走る設計になっていた。さらに watcher thread は stop signal / join handle を
持たず、tab を閉じたあとも watch registration が残り得た。

### 原因

初回登録の重複防止を `HashSet<PathBuf>` だけで表現し、watcher の lifecycle owner を
持たせなかった。tab set が変わるイベントと通常 frontend event を分離せず、
「念のため同期」を hot path に置いてしまった。

### 再発防止策

1. GUI の filesystem watcher / background thread は registry 型で所有し、stop signal と
   join handle を持たせる。
2. watcher 同期は startup / open project / close tab など tab set が変わる境界だけで行い、
   terminal input や board post など高頻度 event から filesystem work を外す。
3. review comment が auto-merge 後に出た場合も、valid な lifecycle / hot path 指摘は
   follow-up PR で解消する。

## 2026-04-27 — test(async): background thread の完了確認に固定 sleep を使わない

### 事象

`cargo test -p gwt-core -p gwt` の full suite で、
`app_runtime_background_knowledge_refresh_silent_paths_do_not_dispatch` が断続的に
`expected fake gh to be invoked for stale cache` で失敗した。単体実行では通るため、
full suite の負荷で background thread が 250ms 以内に fake gh marker を作れない
タイミング依存だった。

### 原因

テストが background refresh の完了を `thread::sleep(Duration::from_millis(250))`
で推測していた。実際に確認したい状態は「fake gh が呼ばれたこと」なので、
固定時間ではなく marker file という positive signal を待つべきだった。

### 再発防止策

1. background thread / async dispatch のテストでは、固定 sleep だけで完了扱いにせず、
   event log、marker file、channel など観測可能な positive signal を待つ。
2. full suite でだけ落ちるテストは、production logic より先に test wait condition を疑い、
   単体実行と full 実行の差を確認する。
3. no-op / silent path のテストでも、可能な限り「処理がその分岐まで到達した」ことを
   別 signal で固定してから副作用なしを検証する。

## 2026-04-25 — fix(board): canvas wheel routing の allowlist に新しい scroll surface を必ず登録する

### 事象

GUI Board を chat timeline に寄せても、Board 上の trackpad / wheel scroll が
canvas pan に奪われると、ユーザーからは「Board がスクロールできない」ように見える。

### 原因

canvas は capture-phase の wheel handler で terminal / repo browser など一部 surface
だけを native scroll として早期 return していた。Board の scroll container は
allowlist に入っておらず、plain wheel が canvas pan として処理されていた。

### 再発防止策

1. 新しい window 内 scroll surface を追加したら、DOM/CSS だけでなく
   `findNativeWheelScrollSurface` の allowlist を同時に更新する。
2. window 内 scroll は、scroll 端でも canvas pan へフォールバックしない方針を
   surface ごとに明示し、embedded frontend contract test で固定する。
3. 「UI が分かりにくい」と「操作不能」が同時に出た場合、まず wheel ownership と
   primary layout model を分けて原因を確認する。

## 2026-04-25 — fix(board): async post 成功判定は対象 response に相関させる

### 事象

GUI Board の composer は投稿後に draft を消す必要があるが、`board_entries` は
投稿成功 response だけでなく通常 refresh / load response でも使われる。submit 中に
古い load response が先に届くと、実際の投稿成功前にユーザー入力を消せる状態だった。

### 原因

- frontend state は `submitting=true` だけを成功判定に使っていた。
- `post_board_entry` response と `load_board` response が同じ `board_entries` event を使うため、
  response の由来を state だけで区別できなかった。
- composer の textarea 自体も native scroll allowlist に入れておらず、container だけの
  wheel routing test では入力欄上の scroll 取りこぼしを検知できなかった。

### 再発防止策

1. 非同期 request の副作用で入力を破棄する場合は、`submitting` だけでなく request marker
   と response 内容を照合してから clear する。
2. 既存 event を複数 request 種別で共有する場合、frontend contract test で「無関係な
   response では draft を消さない」ことを固定する。
3. scrollable container を増やしたときは、container だけでなく textarea/list など実際に
   wheel target になる child element も allowlist 対象か確認する。

## 2026-04-25 — fix(update): installer URL は platform kind 判定後にのみ installer 扱いする

### 事象

`cargo test -p gwt-core -p gwt` で update tests が macOS 上だけ失敗した。
cache に Windows MSI のような current platform 非対応 installer URL が残っていると、
portable asset があるのに `choose_apply_plan` が installer path で `None` に落ちた。

### 原因

`installer_kind_for_url(platform, url)?` を使っていたため、platform 非対応 installer URL が
「installer なし」ではなく「apply plan 全体なし」として扱われた。さらに macOS CLI install
test は `/usr/local/bin` の実 filesystem writable 状態に依存していた。

### 再発防止策

1. installer URL は `installer_kind_for_url` が `Some` を返したときだけ installer plan にする。
   platform 非対応 URL は portable fallback を妨げない。
2. writable / non-writable 分岐をテストする場合、実環境の `/usr/local/bin` などに依存せず
   temp dir または explicit writable helper を使う。
3. update cache の fallback asset test では、portable と installer の両方があるケースで
   current platform 非対応 installer が portable を潰さないことを固定する。

## 2026-04-24 — fix(index): chunked index の health check は下限不変条件を残す

### 事象

Project index hardening の PR で、SPEC は 1 Issue から複数 Chroma record を生成できるため
`document_count != manifest_count` を一律に許容した。その結果、manifest には複数 SPEC が
あるのに Chroma collection が一部 record しか持たない部分破損でも `healthy=true` と
報告される余地ができた。

### 原因

- chunking による「manifest より多い record」は正常だが、「manifest より少ない record」は
  破損という片側条件を明文化していなかった。
- Review 後に auto-merge が先に完了し、未解決 review thread の確認が完了報告前の
  gate として固定されていなかった。

### 再発防止策

1. chunked collection の health check は `document_count >= manifest_count` のような
   one-sided invariant として書き、等価比較を丸ごと無効化しない。
2. `gwt pr checks` が全通過しても、完了報告前に `gwt pr review-threads <n>` を再確認し、
   auto-merge 後に出た valid comment は follow-up PR で解消する。
3. health status と audit log の両方を追加する変更では、UI 表示だけでなく structured
   log の top-level status が同じ truth を示す回帰テストを追加する。

## 2026-04-24 — fix(logging): structured runtime log は project-scoped canonical file 以外へ出さない

### 事象

ログ機能の仕様追加を重ねる中で、#1924 の古い `~/.gwt/logs/` 契約、#2021 の
`~/.gwt/projects/<repo-hash>/logs/` 契約、実装上の `gwt_logs_dir()` 使用が混在し、
新しい診断機能が別ログファイルや legacy path へ出力される余地が残っていた。

### 原因

- `gwt_core::logging::init` 自体は単一入口だったが、呼び出し側が任意の
  `log_dir` を渡せるため、entrypoint ごとに path 解決が割れた。
- README と #2021 は project-scoped path を示していた一方、#1924 の Phase 5
  説明と一部コードコメントに legacy path が残っていた。
- 新規診断機能追加時に「Logs window と同じ canonical file に出ること」を
  テストや acceptance で固定していなかった。

### 再発防止策

1. structured runtime log の保存先は
   `~/.gwt/projects/<repo-hash>/logs/gwt.log.YYYY-MM-DD` のみとする。Git origin
   がない場合は `project_scope_hash(path)` の path hash fallback を使う。
2. logging 初期化では `gwt_logs_dir()` を直接使わず、必ず project-scoped
   canonical resolver を通す。
3. 新しい診断機能は独自の `.log` / `.jsonl` writer を追加せず、`tracing`
   target / fields / spans として canonical structured log に流す。
4. logging 変更では writer / watcher / housekeeping / SPEC / README / docs を同時に確認し、
   legacy path が残っていないか `rg "~/.gwt/logs|gwt_logs_dir\\(\\)"` で確認する。
5. regression test で `LoggingConfig::new(gwt_logs_dir())` や `crates/gwt` の
   logging setup における direct legacy path 使用を失敗させる。

## 2026-04-23 — fix: Python の file I/O は `encoding="utf-8"` を必ず明示する

### 事象

`bunx @akiojin/gwt@latest` を日本語Windows (`C:\Users\AkioJinsenji秦泉寺章夫`)
で実行すると、indexing フェーズで `chroma_index_runner.py` が
`'cp932' codec can't decode byte 0x94 in position 172: illegal multibyte sequence`
で `RUNTIME_ERROR` を返し、`SessionStart` / `UserPromptSubmit` フックも同じ
エラーで落ちていた。2026-04-22 の memory にも同症状の記録あり。

### 原因

`chroma_index_runner.py` の production 側 `Path.read_text()` /
`Path.write_text()` / `open()` が `encoding` 未指定だった。Python 3 は
`locale.getpreferredencoding(False)` をデフォルトに使うため、日本語Windows
では `cp932` として UTF-8 の `~/.gwt/cache/issues/<n>/meta.json` 等を読もう
として失敗していた。`_load_cached_issue_documents` の except で
`UnicodeDecodeError` (実体は `ValueError` subclass) を飲み込んでいたため、
静かに空リストを返していた経路もあった。

### 再発防止策

1. Python で `Path.read_text()` / `Path.write_text()` / `open()` を **テキスト
   モード**で使うときは、必ず `encoding="utf-8"` を明示する。ロケール依存の
   暗黙デフォルトは許容しない。
2. Lock file や binary-safe なファイルを開く場合は `open(path, "a+b")` など
   binary モードを使い、暗黙デコードを発生させない。
3. JSON / YAML の読み込みには `(UnicodeDecodeError, ValueError,
   json.JSONDecodeError, OSError)` を except で受け、旧キャッシュの混入
   エンコードでサイレントに進まないようにする。意図を明確化するために
   `UnicodeDecodeError` を明示する。
4. 回帰テストでは `locale.getpreferredencoding` に頼らず、`io.open` を
   monkey-patch して `encoding is None` 時に cp932 を注入するパターンで
   Windows 日本語ロケールを再現する (`tests/test_cp932_safety.py` 参照)。

## 2026-04-23 — fix(gui): agent-color の「色が出ない」は ANSI 色ではなく frontend surface contract を先に疑う

### 事象

`feature/agent-color` で「Window一覧でも terminal でもエージェント毎の色が出ない」
という報告に対し、初手で ANSI/terminal color 側の切り分けに寄ってしまった。
実際には対象は `SPEC #2133` の agent surface color で、`origin/develop`
取り込み後の frontend bundle (`index.html` / `app.js`) から
`data-agent-color` / `agent-dot` 契約が落ちていたのが原因だった。

### 原因

- 「terminal でも色が出ない」という表現を ANSI color 問題として解釈し、
  現在の owner SPEC と UI contract を先に突き合わせなかった。
- `feature/agent-color` が `origin/develop` に大きく behind している事実を
 先に使わず、古い branch 上の実装前提で調査を始めた。

### 再発防止策

1. `agent-color` / `エージェント毎の色` 系の報告では、まず ANSI 色ではなく
   `workspace window` / `window list` / `launch wizard` / `board` の
   surface contract を確認する。
2. feature branch が `origin/develop` に大きく behind のときは、個別修正前に
   先に develop を取り込んでから退行点を再確認する。
3. frontend bundle 分離後の回帰では、`embedded_web.rs` に CSS/DOM bind
   契約テストを追加してから修正する。

## 2026-04-23 — refactor: Windows spawn splitでも interactive cmd wrapper 契約を落とさない

### 事象

Windows の Codex pane で通常対話中にもキー入力が断続的に欠落した。`terminal_input`
fast-path は残っていたが、Codex のような shim 起動エージェントでだけ再発していた。

### 原因

`b330d7e8` で Windows spawn 解決を `crates/gwt-terminal/src/pty/windows_spawn.rs`
へ分離した際、`#1604/#1608` で確立していた interactive batch wrapper 契約
(`cmd.exe /D /K <expression> & exit`) が脱落し、`.cmd` / `.bat` shim が再び
`/C` で包まれていた。Codex は `codex.cmd` / `npx.cmd` などの shim 経由で起動
しやすく、ConPTY の入力転送が最初に壊れた。

### 再発防止策

1. Windows の `.cmd` / `.bat` shim 解決を触るときは、path 解決だけでなく
   interactive wrapper 契約 (`/D /K <expression> & exit`) まで引き継ぐ。
2. 回帰テストには、spaced shim path、metachar を含む引数、`/S` omission、
   Node.js 配布の `npx.cmd` shim を必ず含める。
3. Codex の入力欠落を調査するときは、frontend や WebSocket より先に
   Windows launch wrapper の `/K` / `/C` 契約を確認する。

## 2026-04-23 — release: macOS `.app` 内の CFBundleExecutable を他バイナリより先に codesign しない

### 事象

`release.yml` の `Build DMG installer (macOS)` で `Sign app bundle` ステップが
`dist/GWT.app/Contents/MacOS/gwt: code object is not signed at all`
`In subcomponent: .../MacOS/gwtd` と出して exit 1。結果として v9.8.0 の DMG ビルドが
失敗し、`upload-release` / `publish-npm` まで連鎖失敗してドラフトリリースが公開されなかった。

### 原因

`GWT.app/Contents/MacOS/` に CFBundleExecutable (`gwt`) と helper (`gwtd`) が
並んで置かれているとき、CFBundleExecutable を先に `codesign --sign` すると
codesign はバンドルの他の subcomponent (この場合 `gwtd`) が既に署名済みで
あることを要求し、未署名だと "code object is not signed at all" で失敗する。

### 再発防止策

1. `.app` バンドル内を個別に署名する場合、helper バイナリ → CFBundleExecutable → バンドル
   の順で `codesign --sign` を呼ぶ。順序を逆にしない。
2. もしくは `codesign --force --deep --sign` を `.app` 1 回だけ呼ぶ。
3. 新しい helper バイナリを `.app/Contents/MacOS/` に追加するときは、`release.yml` の
   署名順を必ず見直す。

## 2026-04-22 — verify: Windows GUI smoke は `browser URL` 出力だけで成功扱いしない

### 事象

`target/debug/gwt.exe` を起動すると `gwt browser URL: http://127.0.0.1:<port>/`
までは出たが、その直後に index runner が
`'cp932' codec can't decode byte 0x94 ... illegal multibyte sequence`
で `RUNTIME_ERROR` を返し、native GUI の手動確認を最後まで進められなかった。

### 原因

- `browser URL` の出力は front-door server 起動成功しか保証せず、indexing/runtime の
  後続失敗は別で起こりうる。
- この環境では Windows ユーザープロファイル配下の Unicode path と index runner の
  文字コード処理が衝突し、GUI 操作前に runtime error になった。

### 再発防止策

1. Windows GUI smoke では `browser URL` 出力後に `~/.gwt/logs/index/*.log` か
   stderr を必ず確認し、index/runtime error がないことまで見て成功判定する。
2. front-door HTML が `127.0.0.1` で取れても、native GUI 操作まで届かなければ
   manual E2E 完了扱いにしない。
3. Unicode path 由来の index runner failure が出た場合は、当該手動確認タスクを
   block として記録し、別 issue/scope に切り出して扱う。

## 2026-04-22 — test: `HOME` / `USERPROFILE` を触る gwt-core test は crate-wide lock を共有する

### 事象

`crates/gwt-core/src/notes.rs` の repo-scoped notes test を追加した後、
`cargo test -p gwt-core -p gwt` で既存の
`coordination::tests::git_repo_without_origin_uses_project_scoped_coordination_dir`
が intermittent に `指定されたパスが見つかりません` で落ちた。

### 原因

- `notes` test と `coordination` test の両方が `HOME` / `USERPROFILE` を
  temp dir に差し替えていた。
- それぞれが module-local な mutex を持っていたため、crate 全体では排他になっておらず、
  並列 test 実行時に環境変数が競合した。
- path helper は global env を直接読むので、別 module の test でも影響を受けた。

### 再発防止策

1. `HOME` / `USERPROFILE` / `RUST_LOG` など process-global env を触る test helper は
   crate-wide shared module (`test_support`) に置いて共通 lock を使う。
2. 新しい env-mutation test を追加するときは、既存 module に local lock がないかを
   先に確認し、別 lock を増やさない。
3. `cargo test -p gwt-core -p gwt` の full run を、env を触る test 追加直後の
   fail-fast check に含める。

## 2026-04-21 — spec: `gwt issue spec --edit` を短時間に連続実行したら readback を即確認する

### 事象

`#1784` の `data-model` / `quickstart` を `gwt issue spec 1784 --edit ...` で追加した直後、
comment 自体は作成されていたが Issue body の `<!-- sections: -->` index から抜け落ち、
`gwt issue spec 1784 --section data-model` と `--section quickstart` が
`section not found` になった。

### 原因

- 連続した section write の途中で stale cache を起点に後続 write が走り、
  body index を再計算した際に直前に追加した section を落として上書きした。
- `comment があるか` だけを見て完了扱いし、`--section <name>` の readback を
  各 write 後に即確認しなかった。

### 再発防止策

1. `gwt issue spec --edit` で新しい section を追加したら、毎回すぐ
   `gwt issue spec <n> --section <name>` で readback を確認する。
2. 複数 section を短時間に連続更新するときは、途中で `gwt issue view <n> --refresh`
   で cache を強制更新してから次の write に進む。
3. `comment は存在するが section not found` になったら、Issue body の
   `<!-- sections: -->` index 欠落を疑って GitHub 側の body と comment ids を直接確認する。

## 2026-04-21 — fix(ci): WiX Component に複数 File を入れるときは未バージョン化 keypath で auto GUID を破綻させない

### 事象

Release workflow `24696970550` の `Build MSI installer (Windows)` が
`wix/main.wxs(26) : error WIX0367` で失敗し、`v9.7.0` の draft release が
asset 0 件のまま止まった。直前に `feat: gwtとgwtdのbundle配布を追加`
(299193c4) で同じ `<Component>` に `gwtd.exe` を追加し、File が 2 つに
なっていた。

### 原因

- WiX v4 は `<Component>` の `Guid="*"` 省略時に GUID を自動生成するが、
  複数 File を持つ Component は「keypath の File がバージョン情報を持つ」
  ことを要求する。
- Cargo の `cargo build --release` はデフォルトで Windows PE の
  VERSIONINFO を埋め込まないため、`gwt.exe` は未バージョン化扱いとなり、
  keypath が条件を満たさず WIX0367 が発生した。
- PR マージ前に Windows MSI ビルドをローカル検証せず、生成 MSI を
  確認しないまま main にマージしたため、release workflow で初めて
  表面化した。

### 再発防止策

1. `wix/main.wxs` では 1 File = 1 Component を原則とし、複数 File を
   1 Component に入れる場合のみ「keypath File が versioned か」を
   明示的にレビュー項目に入れる。
2. installer 構成に触る PR では、`wix build` をローカルで走らせるか、
   少なくとも別の小さな PR で Windows MSI job を pre-flight 実行して
   release workflow での失敗を前倒しで拾う。
3. release workflow を一度壊したら、`gh release view v<N>` で
   asset 数と draft 状態を確認し、tag だけ push されて asset 0 件の
   draft が残っていないかを毎回セルフチェックする。

## 2026-04-20 — fix(ci): test import と期待値に OS 固定前提を埋め込まない

### 事象

PR #2095 の CI で Linux `cargo clippy` と `cargo test` が失敗した。`crates/gwt/src/issue_cache.rs`
の test module では Windows 専用テストでしか使わない `env` を無条件 import しており、
Linux では `unused import` で落ちた。`crates/gwt/src/cli/update.rs` の
`run_with_covers_helper_copy_and_spawn_paths` は `make_helper_copy()` が常に 1 回呼ばれる前提で
assert していたが、実装は Windows のみ helper copy、非 Windows は current exe 再利用だった。

### 原因

- test module の import と assertion が `cfg!(windows)` / `#[cfg(target_os = "windows")]`
  で分岐する実装契約と揃っていなかった。
- Windows ローカルでの検証結果をそのまま CI 全 OS に一般化していた。
- `cargo llvm-cov --ignore-filename-regex` による `main.rs` 除外へ依存していたため、
  Windows ローカルでは path 表現差分で 90% gate が誤判定する余地があった。

### 再発防止策

1. OS 分岐を持つ実装のテストでは、import・fixture・assertion も同じ条件分岐で拘束する。
2. `cargo clippy --all-targets --all-features` の失敗が test module import に由来しないかを
   まず確認する。
3. helper process / installer / path まわりのテストでは、`Windows only` と
   `cross-platform fallback` を別々に明示して期待値を固定する。
4. カバレッジ gate の除外判定は `cargo llvm-cov` の regex 文字列だけに寄せず、
   summary JSON を読んで path 正規化込みで自前判定する。

## 2026-04-20 — fix: preset 経由の agent launch で wizard 側の default arg が抜ける

### 事象

preset ボタンから起動した Codex は、wizard 経由の launch と違い
`--no-alt-screen` が抜けた状態で spawn されていた。結果 alternate screen に入り、
xterm.js の scrollback ring に行が蓄積されず、Plan mode 入力待ちに入ると
マウスホイールが完全 no-op になった (Issue #2091)。

### 原因

- agent-specific な launch 既定値 (`--no-alt-screen` 等) は
  `gwt-agent/src/launch.rs` の `build_codex_args` にしか書かれておらず、
  preset resolver (`crates/gwt/src/preset.rs`) は bare command で spawn していた。
- `normalize_launch_args` はセッション永続化のロード経路でのみ呼ばれていて、
  fresh preset spawn には適用されていなかった。
- agent 起動経路が「wizard」「preset」「persisted session load」で分岐しており、
  既定引数の one source of truth が存在しなかった。

### 再発防止策

1. agent 固有の起動既定値を変更する際は、wizard / preset / session load の
   3 経路すべてに適用されていることを確認する。
2. preset 経由の agent spawn では、対応する agent の既定引数 (`--no-alt-screen`
   等) を必ず組み込む。regression テストを preset unit test に追加する。
3. 「wizard では動くが preset では動かない」系の挙動差分は、経路ごとの
   `LaunchSpec` / `LaunchConfig` を突き合わせて探す。

### 追記 (2026-04-20 Phase 53 完了)

SPEC-1921 Phase 53 で `gwt_agent::canonical_launch_args(&AgentId) -> Vec<String>` を
single source of truth として追加し、preset / wizard / session-load が同じ API を
通るよう refactor 済み (#2091 後続作業として同 PR #2092 に収録)。新しい agent 既定
引数を追加するときは canonical API 1 箇所の編集で全経路に反映される。session 永続
化は `schema_version` を導入し、`Session::load` は verbatim 化、migration は
`Session::migrate_legacy_launch_args` に明示的に切り出した。再発防止策 1-3 の運用は
継続するが、3 経路の同期漏れは canonical API の強制ルートでアーキテクチャ的に
不可能になった。

## 2026-04-20 — fix(hooks): PreToolUse で `stopReason` は表示されない。`hookSpecificOutput.permissionDecisionReason` を使う

### 事象

`gwt hook block-bash-policy` が `gh issue/pr/run` をブロックした際、ユーザーには
`Direct GitHub workflow CLI commands are not allowed` という短文しか表示されず、
詳細な代替コマンド一覧 (`gwt issue view` / `gwt pr view` / `gwt actions logs` /
`gwt-search`) と `Blocked command: ...` 原文が届かなかった。他の block 系フック
(cd / file-ops / branch-ops / git-dir) も同じ欠陥を抱えていたが、短文自体が自己
説明的だったため見落とされていた。

### 原因

- `BlockDecision` が stdout へ `{"decision":"block","reason":"<short>","stopReason":"<long>"}` を
  emit していた。
- Claude Code の PreToolUse フックでは `stopReason` はパースされない
  (`stopReason` は Stop/SubagentStop 専用)。長文はどこにも届いていなかった。
- Codex も同様に Claude Code 仕様を踏襲しており、`stopReason` は PreToolUse で
  無視される。

### 再発防止策

1. PreToolUse フックの block 出力は `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"<all text>"}}` の
   正式形を使い、visible 情報は必ず `permissionDecisionReason` に集約する。
   レガシーの top-level `decision` / `reason` / `stopReason` は emit しない。
2. `BlockDecision::new(short, long)` の call API を維持し、構造体内部で
   `short + "\n\n" + long` を `permissionDecisionReason` に畳み込む。
3. ブロック理由をユーザーが本当に見ているかを、個別フックに頼らず
   `hook_types_test` 層で wire format 契約として固定する。
4. Stop / SubagentStop 用の `stopReason` / `continue:false` と、PreToolUse 用の
   `permissionDecision` / `permissionDecisionReason` は別物であることを前提として
   フックを設計する。

## 2026-04-20 — fix: discussion の深さは「継続質問する」と書くだけでは維持できない

### 事象

`gwt-discussion` に「次の高インパクト質問を続ける」と書いていても、実際の会話は
数問で終わりやすく、深掘りが必要な論点が残ったまま exit していた。

### 原因

- 深さの停止条件が弱く、どの coverage を埋めるまで続けるかが曖昧だった。
- clarification に固定 question cap が残っており、discussion 本体の継続方針と
  矛盾していた。
- Plan Mode で始めて Plan Mode を抜ける handoff までを契約化しておらず、
  discussion の終わり方が曖昧だった。
- asset contract test が depth gate / coverage / exit blocker を拘束していなかった。

### 再発防止策

1. discussion 系 asset では「継続する」と書くだけでなく、Coverage Checks と
   Exit Blockers を明示して exit 条件を固定する。
2. clarification / deepening / intake など周辺 reference の停止条件も同じ変更セットで
   揃え、固定上限や浅い exit を残さない。
3. Plan Mode で開始し、最終 handoff で leave Plan Mode できるところまでを
   command / skill / test で同時に拘束する。

## 2026-04-20 — fix(gui): GUI unit test で EventLoop/GTK 初期化を持ち込まない

### 事象

PR #2074 の Linux CI で `tests::app_state_view_includes_current_app_version` が失敗した。
最初は `tao` の EventLoop を main thread 外で初期化したことにより落ち、その場しのぎで
`with_any_thread(true)` を入れると次は GTK 初期化失敗に変わった。

### 原因

- version 表示の回帰テストが、本来確認したい `AppStateView` の組み立てだけでなく、
  GUI runtime (`AppRuntime`) の生成まで引き込んでいた。
- `AppRuntime` のテスト補助が `tao::EventLoop` / platform backend 初期化を前提にしており、
  headless Linux CI では GTK backend が使えず失敗した。

### 再発防止策

1. GUI state の unit test は EventLoop や WebView を生成せず、pure な state builder/helper に分離して検証する。
2. `tao` / `wry` / GTK backend を触るテストは、thread 制約だけでなく headless backend 制約も前提に置く。
3. 「表示用 state を確認したいだけ」のテストでは runtime 全体を組み立てず、必要な parts を直接渡す helper を先に用意する。

## 2026-04-20 — ci(release): cross-platform archive step は shell と runner の同梱コマンド差分を前提に分ける

### 事象

`main` へ release PR #2072 を merge した後、Release workflow `24650370386` の
`Build gwt (windows-x86_64)` が `Prepare artifact` で失敗し、`v9.6.0` の
Windows zip asset だけ GitHub Release に載らなかった。

### 原因

- `.github/workflows/release.yml` の artifact 作成が全 OS 共通で `shell: bash` になっていた。
- Windows 分岐では `zip` を呼んでいたが、GitHub の Windows runner + Git Bash 環境には
  `zip` binary が常にある前提を置けなかった。
- build 自体は成功しており、最後の packaging だけ shell 依存で壊れていた。

### 再発防止策

1. cross-platform workflow の packaging / file operation は「同じ shell で書けるか」ではなく、
   各 runner に標準であるコマンドを基準に step を分ける。
2. Windows artifact 作成では `zip` のような Unix 由来コマンドを前提にせず、
   `Compress-Archive` など OS 標準機能を優先する。
3. release workflow を触ったら、job ごとの最後の packaging/upload step までログを確認し、
   「ビルド成功で安心しない」を checklist に入れる。

## 2026-04-20 — fix: canvas window ID を「同 preset 件数 + 1」で採番すると欠番で live window に衝突する

### 事象

Launch Wizard の `Start new` から別ブランチの Agent window を開いたとき、
既存の Agent window が新しいブランチ内容で上書きされた。実際には
`agent-1` を閉じて `agent-2` だけ残っている状態で次の新規 Agent も
`agent-2` として生成され、window/runtime/session の紐付けが衝突していた。

### 原因

- `crates/gwt/src/workspace.rs` の window ID 採番が
  「同じ preset の live window 件数 + 1」だった。
- 同じ preset に欠番があると、件数ベース採番が既存 live window の suffix と一致し、
  frontend の `windowMap`、backend の `window_lookup`、active session が
  同じ ID で上書きされた。

### 再発防止策

1. floating window の ID は件数ではなく既存 live window の ID 集合から決める。
   少なくとも同 preset の最大 suffix + 1 など、live window と衝突しない方式を使う。
2. 「閉じた window がある状態で同 preset を新規作成する」回帰テストを
   `WorkspaceState` に必ず追加する。
3. Quick Start の `Start new` と既存ウィンドウ再利用導線は意味を分離し、
   live window がある場合は `Focus` と明示する。

## 2026-04-20 — fix: auto-close 判定では「終了したか」だけでなく exit 成否を分ける

### 事象

agent window の auto-close 修正後、review で non-zero exit まで `Exited` 扱いになり、
失敗した agent terminal も自動で閉じて最後のエラー文脈を失う指摘が出た。

### 原因

- `PaneStatus::Completed(0)` と `PaneStatus::Completed(non-zero)` を
  両方とも `WindowProcessStatus::Exited` に潰していた。
- close 条件は active agent ownership で絞れていても、
  成功終了と失敗終了の意味分離までは入っていなかった。

### 再発防止策

1. auto-close のトリガは ownership だけでなく successful completion まで条件に含める。
2. process status 変換では `Completed(0)` と `Completed(non-zero)` を別経路にし、
   失敗終了は `Error` surface を残す。
3. auto-close 回帰テストには「active agent success で close」と
   「active agent failure では残る」を対で入れる。

## 2026-04-20 — fix: path 文字列ヒューリスティクスは host OS の `Path` semantics に依存させない

### 事象

`managed_assets` の hook binary 選択で、Windows 形式 path を使う unit test が
Linux CI の `Test (Rust)` だけ失敗した。`gwt.exe` と `bunx-*` を見分ける
helper が、host OS の path separator 解釈に引っ張られていた。

### 原因

- `Path::file_stem()` と `Path::components()` は host OS の separator 規則で動くため、
  Linux 上で `C:\\...\\gwt.exe` を渡すと Windows path として分解されない。
- 「他 OS 形式の path 文字列をどう扱うか」をテストは固定していたが、
  実装側はその前提を吸収していなかった。

### 再発防止策

1. 実行ファイル名や temp wrapper 判定のような path ヒューリスティクスでは、
   必要なら `\\` / `/` を明示的に正規化してから文字列ベースで判定する。
2. cross-platform 回帰テストでは、「Windows path を Linux で評価する」ような
   foreign-path case をそのまま残し、host OS 依存退行を早めに拾う。
3. `Path` API を使う helper を追加したら、「その判定は path 操作か、文字列契約か」を
   先に切り分ける。

## 2026-04-20 — fix: project-scoped asset は「repo を見つけたか」と「origin があるか」を分けて扱う

### 事象

coordination の保存先は `~/.gwt/projects/<repo-hash>/coordination/` が正なのに、
origin がない git repo / worktree では `.gwt/coordination/` へ落ちていた。あわせて
managed hook 再生成が `current_exe()` の一時 bunx 実体をそのまま焼き込み、
古い `gwt` バイナリを hooks が呼び続ける経路が残っていた。

### 原因

- project-scoped path への切り替え条件を「git repo か」ではなく
  「origin から repo hash を引けるか」にしていた。
- 永続化する hook command の実体選択で、`current_exe()` が
  bunx temp / helper binary のときの扱いを分けていなかった。

### 再発防止策

1. project-scoped data directory は、repo 検出と hash 取得を分離する。
   repo root が取れるなら project scope を使い、hash は origin か path hash で決める。
2. 設定ファイルへ永続化する実行パスは、transient wrapper (`bunx-*` など) を検知し、
   安定した `gwt` 実体か明示 override を優先する。
3. path migration の回帰テストは「origin なし git repo」と
   「一時実体からの hook 再生成」の両方を必ず固定する。

## 2026-04-20 — fix: Windows shim 解析は「実行ファイルがある」だけで確定せず、runtime と script の組み合わせを見る

### 事象

PR #2063 の merge 後 review で、`crates/gwt-terminal/src/pty.rs` の
`build_windows_shim_target` が `.exe/.com` を見つけた時点で即 return しており、
`node.exe + cli.js` のような npm shim で script 引数を落とす指摘が出た。
あわせて `windows_env_value` が `remove_env` を先に見ていたため、
`env` で明示指定した値まで無視する非対称も見つかった。

### 原因

- shim 解析を「最初に見つかった実行ファイル」中心で書き、runtime と script が
  セットで現れる wrapper を考慮していなかった。
- 環境変数解決で `portable_pty::CommandBuilder` の適用順
  (`remove_env` の後に `env`) と同じ優先順位を維持していなかった。

### 再発防止策

1. Windows の shim 解析では、`.exe` 単体だけでなく `runtime + script` の
   wrapper パターンを先に洗い出してから target 決定ロジックを書く。
2. wrapper で runtime が見つかっても、`.js/.cjs` が同居する場合は
   「runtime 単体」ではなく「runtime + script arg」を回帰テストで固定する。
3. spawn 前の env 正規化 helper は、実際に適用する `CommandBuilder` と
   同じ優先順位になるよう unit test を先に足す。

## 2026-04-20 — fix: OS 固有実装を足したら import も同じ cfg 境界に置く

### 事象

Windows PTY shim 修正後、PR #2063 の `Clippy & Rustfmt` が Linux CI で失敗した。
原因は `crates/gwt-terminal/src/pty.rs` の `Path` import がトップレベルにあり、
Windows 専用関数でしか使わないのに Linux では unused import になっていたことだった。

### 原因

- 実装本体は `#[cfg(windows)]` で囲っていたが、use 宣言の cfg 境界を揃えていなかった。
- 手元確認が Windows 実行中心で、Linux compile/lint 時の未使用 import を見落とした。

### 再発防止策

1. OS 固有 helper を追加するときは、type import / helper function / test を同じ cfg 境界で揃える。
2. `cfg(windows)` 専用コードを触った後でも、CI と同じ package 指定の `cargo clippy` を必ず回す。
3. クロスプラットフォーム crate のトップレベル import 追加では、「他 OS で unused にならないか」を差分確認に含める。

## 2026-04-20 — fix: Windows PTY で PATH 解決をそのまま信じると npm shim に吸われる

### 事象

Windows で installed Claude agent を PTY 起動すると、`CreateProcessW` が
`%APPDATA%\\npm\\claude` を直接実行しようとして `os error 193`
(`%1 は有効な Win32 アプリケーションではありません`) で失敗した。

### 原因

- `portable-pty` の Windows `search_path()` は PATHEXT 候補より先に「拡張子なしの実在ファイル」を返す。
- npm global install は `claude` / `codex` の拡張子なし shim を配置するため、
  Win32 実行可能ファイルではない shell shim が最優先になっていた。
- `cmd.exe` ラップに逃がすだけでは ConPTY 上で挙動が不安定なケースがあり、
  実体の `.exe` / `node + .js` まで解決した方が安定する。

### 再発防止策

1. Windows で PATH 検索結果を PTY に渡す前に、npm shim を実体コマンドへ正規化する。
2. `.exe` / `.com` は直起動し、shell shim が `node_modules/.../*.js` を指す場合は
   `node` と script path に分解して起動する。
3. Windows の PTY 起動修正では、少なくとも「shim 解決」「spawn 成功」の両方を
   回帰テストで固定する。

## 2026-04-20 — fix: repo browser の wheel ownership を generic な edge fallback へ一般化しない

### 事象

Branches ウィンドウで一覧を最上端/最下端までスクロールしたあと、さらに同じ方向へ wheel/trackpad scroll すると、
内部リストは止まる一方で canvas pan が始まり、repo browser surface 上の操作が window 内で完結しなかった。

### 原因

- 4/17 の修正で「surface が delta を実際に消費できるときだけ native scroll を優先する」という一般則を入れた。
- この一般則を repo browser surface にもそのまま適用した結果、scroll edge では capture-phase handler が
  canvas pan 経路へフォールバックしてしまった。
- Branches / File Tree の UX では、surface 上の plain wheel は canvas に流さず no-op に留める契約だった。

### 再発防止策

1. wheel ownership は surface ごとに決める。repo browser list は「scroll 可能時は native scroll、edge では no-op」、canvas 背景だけが pan owner。
2. `can surface consume delta? -> else canvas` のような generic fallback を、window 内 scroll surface へ無条件に再利用しない。
3. repo browser の回帰テストには「内部スクロール可能」「edge で no-op」「canvas 背景で pan」の 3 観点をセットで入れる。

## 2026-04-17 — fix: scrollable pane の wheel 奪取は「surface が実際に消費できる delta」だけに限定する

### 事象

Branches / File Tree の wheel 修正後、repo browser pane 上では内部スクロールを優先できるようになった一方、
pane に overflow がない場合や scroll 端に達している場合でも canvas pan へ fall back せず、
gesture が no-op になる回帰を reviewer に指摘された。

### 原因

- `wheel` handler が「scrollable pane 配下であること」だけで native scroll へ早期 return していた。
- event target の面が scroll container であっても、その delta を実際に消費できるか
  （overflow の有無、top/bottom/left/right の境界）を見ていなかった。

### 再発防止策

1. canvas から `wheel` を奪う条件は「pane 配下」ではなく「pane がその delta を実際に scroll できる」ことにする。
2. trackpad / mouse wheel の routing では、vertical だけでなく horizontal delta と scroll 境界も確認する。
3. repo pane の interaction 変更では、「scroll できる時は pane」「scroll できない時は canvas pan」の両方を回帰観点に入れる。

## 2026-04-17 — fix: CI lint 再現は workflow と同じ package / feature 範囲で実行する

### 事象

PR #2052 の `Clippy & Rustfmt` が CI で失敗したが、手元では直前に `cargo clippy` を
通したつもりだった。実際の失敗箇所は `crates/gwt-github/src/client/fake.rs` の
`unnecessary_sort_by` で、`gwt` の feature 経由で lint 対象に入っていた。

### 原因

- 手元の確認で、workflow に書かれている package 指定と同じコマンドを厳密に再現していなかった。
- 「workspace 全体を見ているはず」という前提で済ませ、CI job 定義をその場で確認しなかった。
- transitive dependency / feature 経由で lint 対象になる crate を、変更ファイルだけ見て外していた。

### 再発防止策

1. CI 失敗の再現では、先に `.github/workflows/*.yml` の実コマンドを確認し、そのまま手元で実行する。
2. `-p` 指定の lint/test でも、feature 経由で別 crate が対象に入る前提でログを確認する。
3. 「ローカルで通った」は抽象化せず、最終報告では実行した正確なコマンド列を残す。

## 2026-04-17 — fix: embedded WebView JS の回帰確認は整形文字列ではなく契約と対称性を見る

### 事象

Web terminal copy 修正の PR で、CodeRabbit から 3 件の follow-up 指摘が出た。
`include_str!` ベースの HTML 回帰テストが単一行の exact string に依存していて
整形変更に弱かったこと、`createTerminalRuntime()` の新規作成 path だけ返り値に
`cleanup` を含めていなかったこと、copy 用の `mouseup` listener が capture/bubble の
二重登録になっていたことが原因だった。

### 原因

- 埋め込み HTML の契約テストで、挙動ではなくフォーマット済み文字列そのものを固定していた。
- JS helper の reuse path と create path の返り値形状を並べて確認していなかった。
- event listener の追加/削除を対で見ず、window capture listener と terminalRoot listener の
  役割重複を残していた。

### 再発防止策

1. `include_str!` で埋め込む HTML/JS の回帰テストは exact snippet ではなく、必要な token や契約を構造的に確認する。
2. factory/helper 関数を変更するときは、既存再利用 path と新規作成 path の返り値 shape を揃えて確認する。
3. DOM event handler 変更では、登録と cleanup を対で確認し、capture/bubble の重複 listener が本当に必要かを見直す。

## 2026-04-16 — fix: read-only CLI は eager GitHub auth を起動時に解決しない

### 事象

`gwt pr current` が無応答に見えた。実際には `pr current` 本体ではなく、
`DefaultCliEnv::new()` が command dispatch 前に
`HttpIssueClient::from_gh_auth()` を通して `gh auth token` を同期実行していたことと、
`cargo run -q` の無音ビルド待ちが重なって原因切り分けを難しくしていた。

### 原因

- read-only CLI と write-capable GitHub Issue client の初期化が分離されていなかった。
- `gwt pr current` は `gh pr view` だけで成立するのに、起動時に Issue client auth を先に解決していた。
- 外部 `gh` 呼び出しとビルド待ちに progress/timeout の観測点がなく、体感上は「固まった」ように見えた。

### 再発防止策

1. read-only command path では GitHub Issue client を lazy init し、`env.client()` を触るまで auth を発火させない。
2. `PrCurrent` のような read-only command で Issue client を使っていないことをテストで固定する。
3. CLI 無応答調査では、cargo build 待ちと外部 command 待ちを別々に計測してから原因を特定する。

## 2026-04-15 — fix: Quick Start の resume は struct やテストではなく実 session TOML への保存実態で確認する

### 事象

Launch Agent の Quick Start で `Continue` を押すと、既に起動済みの agent window を
再利用せず、新しい window を重ねて起動していた。
あわせて `~/.gwt/sessions/*.toml` を実確認すると、`Session.agent_session_id` field は
コード上に存在するのに、実ファイルには `agent_session_id` が1件も保存されていなかった。

### 原因

- Quick Start 側が `--continue` 的な新規起動経路へ寄っており、live window focus と
  saved session resume を分けて扱っていなかった。
- hook runtime が受け取る Codex/Agent 側の `session_id` を、gwt session TOML へ
  書き戻す production path が存在しなかった。
- struct 定義と unit test があることで、「保存されているはず」という前提を
  実ファイル確認なしに置きやすい状態だった。

### 再発防止策

1. resume/continue 系の不具合では、まず `~/.gwt/sessions/*.toml` を直接確認し、
   必要な key が実際に保存されているかを事実ベースで確認する。
2. Quick Start の reuse は「live window があるなら focus」「保存済み session id が
   あるなら resume」「どちらもなければボタン非表示」を明示的に分けて実装・テストする。
3. session metadata を UI が参照する変更では、struct や fixture だけでなく
   production hook/runtime から persistence まで通る回帰テストを必ず追加する。

## 2026-04-15 — fix: build.rs の skill frontmatter 検証で repo 管理外 skill を読まない

### 事象

Windows 環境で `cargo build -p gwt` を実行すると、`.claude/skills/` 配下に残っていた
repo 管理外の旧 skill `gwt-spec-plan/SKILL.md` の壊れた YAML frontmatter を
`crates/gwt-core/build.rs` が読んで panic し、ビルド全体が失敗した。

### 原因

- build script が `.claude/skills/*/SKILL.md` をディレクトリ走査で無差別に検証していた。
- retired / untracked / ローカル作業用 skill まで build-time validation の対象に入っていた。

### 再発防止策

1. build-time validation は repo 管理下の `SKILL.md` のみに限定する。
2. `git ls-files` が使える環境では tracked files を起点に検証し、ローカル残骸で build を壊さない。
3. skill 配布名の改名・retire を行った変更では、旧 skill ディレクトリがローカルに残っていても build できることを一度確認する。

## 2026-04-15 — fix: GUI binary から hook/CLI 実体を `current_exe()` で逆算しない

### 事象

GUI PoC から managed hooks を再生成すると `.claude/settings.local.json` /
`.codex/hooks.json` が `poc-terminal hook ...` を埋め込み、Claude/Codex hook
発火時に追加の GUI window が起動した。

### 原因

- hook generator の binary 解決が `current_exe()` 前提で、GUI 起動時は GUI binary
  自身を canonical CLI と誤認した。
- GUI binary 側にも CLI verb の early dispatch がなく、`hook` / `issue` でも
  WebView 起動経路へ入っていた。

### 再発防止策

1. GUI から managed hooks を生成するときは、canonical CLI binary を明示的に解決して
   `GWT_HOOK_BIN` へ注入する。
2. GUI binary は起動直後に CLI verb を判定し、window を作る前に delegate する。
3. hook / launch path の修正では、「生成された hook command が GUI binary を指さない」
   と「GUI binary に CLI verb を渡しても window を開かない」の両方を回帰テストまたは実行確認で固定する。

## 2026-04-15 — fix: GUI workspace restore では process window を自動再 spawn しない

### 事象

GUI PoC の起動時に、前回保存していた Shell / Claude / Codex / Agent window を
そのまま再 spawn してしまい、複数 project / window を開いていた環境では
大量のプロセスが同時に立ち上がった。

### 原因

- restore を「layout の復元」と「process の再起動」を分けずに扱っていた。
- `workspace.json` に残っている process window を `bootstrap()` が無条件に起動していた。

### 再発防止策

1. GUI restore ではまず layout/state を復元し、process window は paused/exited 扱いで読む。
2. startup bootstrap は persisted window 全件ではなく、「自動再開が明示された window」だけを起動する。
3. restore semantics を変える変更では、既存 state fixture を使って「起動時に追加プロセスが増えない」回帰テストを入れる。

## 2026-04-14 — feat: WebView-only GUI PoC でも browser access の可能性があるなら transport を最初から外出しする

### 事象

native window の内部だけで `window.ipc` を使う WebView PoC を進めたあと、
実行時に local server を起動して同じ UI を browser からも開ける要件が追加された。

### 原因

- frontend transport を WebView 専用 API に寄せたため、browser reuse の余地がなかった。
- 「native shell の中の WebView」という見え方に引っ張られ、UI host と frontend transport を
  別問題として切り分けていなかった。

### 再発防止策

1. WebView を使う GUI PoC では、後から browser 共有が起こりうるかを最初に確認する。
2. browser reuse の可能性が少しでもある場合、frontend transport は `window.ipc` ではなく
   local HTTP/WebSocket など host 非依存の経路にする。
3. native app と browser が同じ frontend を読む構成では、初期 state sync と live update を
   最初から分けて設計する。

## 2026-04-14 — fix: ローカル永続 state の schema 追加では既存ファイルを `serde(default)` で先に受ける

### 事象

`workspace.json` に `viewport` を追加したあと、既存の PoC state を読もうとして
`missing field \`viewport\`` で GUI 起動時に panic した。

### 原因

- 既に保存済みの local state が存在する前提で migration / backward compatibility を見ていなかった。
- 新 field を必須扱いで deserialize し、旧 schema を default 埋めせずに読み始めた。

### 再発防止策

1. ローカル永続 JSON/TOML の schema を拡張するときは、新 field に `serde(default)` を付けてから
   読み込み側の変更を入れる。
2. persistence 変更では、旧 schema を手書き fixture で読み込む回帰テストを追加する。
3. GUI / TUI の startup path に関わる state 追加では、`cargo run` で既存 state がある環境を
   実際に一度起動確認する。

## 2026-04-14 — planning: terminal UI の依頼では TUI と GUI を明示的に切り分けて確認する

### 事象

「Rust で terminal を作成し、Claude や Codex を二画面表示したい」という依頼に対して、
repo 文脈と既存資産に引っ張られ、最初に TUI 前提で設計を進めてしまった。

### 原因

- 「terminal を作る」という依頼を、端末エミュレータ全般ではなく ratatui/crossterm の
  TUI と短絡的に解釈した。
- 既存 repo に `gwt-terminal` / `gwt-tui` があることを根拠に、windowed GUI の可能性を
  十分に切り分ける前に設計を固めた。

### 再発防止策

1. `terminal` / `pane` / `split view` の依頼では、最初に TUI / desktop GUI / browser GUI の
   どれを指すかを明示的に確認する。
2. 既存実装資産が強くても、UI modality の前提は repo 都合で固定せず、ユーザーが求める
   interaction surface を先に固める。
3. 「Figma のような」「window が浮く」など spatial metaphor が出たら、layout 比較を
   早めに出して認識合わせを行う。

## 2026-04-14 — fix: migration 互換パスでは legacy event tag を先に受理してから merge/delete する

### 事象

project-scoped coordination storage へ移行済みの前提なのに、legacy worktree-local
`events.jsonl` に旧 tag `board_post` が残っている repo で `SessionStart` hook が
`unknown variant \`board_post\`` で code 1 失敗した。

### 原因

- migration と snapshot rebuild が現行 `CoordinationEvent` へ直接 deserialize しており、
  legacy tag を読む前に落ちていた。
- 「storage path を shared に寄せた」ことと「旧 event schema を読める」ことを
  別問題として扱い、互換経路の受理条件を最後まで確認していなかった。

### 再発防止策

1. migration / compatibility path では、legacy enum tag や field 名を先に受理してから
   merge / delete / marker 書き込みへ進む。
2. schema rename を伴うイベントログ変更では、「旧ログを読める」「書き戻しで現行形式へ正規化される」を
   別々の回帰テストで固定する。
3. project-scoped storage への移行では、shared dir の marker 有無だけでなく
   legacy worktree-local 実データを使った初回アクセスも検証する。

## 2026-04-14 — fix: `.codex/hooks.json` の所有権を Git の tracked/untracked 状態に委ねない

### 事象

`.codex/hooks.json` の生成契約を `tracked/untracked` で分岐させたまま
`info/exclude` にも追加していたため、repo 利用者が version 管理したい設定まで
gwt がローカル専用ファイルとして扱ってしまった。

### 原因

- `.claude/settings.local.json` の「ローカル生成物」という性質を
  `.codex/hooks.json` にもそのまま当てはめた。
- hook 設定の所有権を「ユーザー/リポジトリが決める契約」ではなく、
  Git の tracked/untracked 状態で近道判定していた。

### 再発防止策

1. `.codex/hooks.json` のように利用者が version 管理可否を選べるファイルは、
   `info/exclude` や `.gitignore` へ自動追加しない。
2. 既存設定ファイルの生成契約は tracked/untracked ではなく、
   「存在しなければ作成、存在すれば managed 部分だけマージ」で統一する。
3. ローカル専用ファイルとユーザー所有ファイルの境界を変えるときは、
   README と owner SPEC を同じ変更セットで更新する。

## 2026-04-14 — chore: プロダクト既定の所有権ポリシーと repo ローカル運用を混同しない

### 事象

`.codex/hooks.json` は gwt の既定契約としては repo 利用者に tracking 判断を委ねる
べきだったが、`akiojin/gwt` repo 自体では version 管理しない運用にしてよい、
という repo ローカル方針の切り分けを一度で整理できなかった。

### 原因

- 「生成ロジックの既定動作」と「この repo だけの ignore 方針」を同じ層で考えた。
- ツールのデフォルト契約を変える変更と、repo の運用設定を変える変更を分離していなかった。

### 再発防止策

1. ツールの既定契約は利用者一般向けに設計し、repo 固有の運用は `.gitignore` や
   `info/exclude` のような repo ローカル設定で切り分ける。
2. 「この repo では ignore してよい」という判断は、生成コードではなく
   repo 側の ignore 設定で実装する。
3. 所有権ポリシーの議論では、「全 repo 共通の挙動」と「この repo 限定の運用」を
   明示的に分けて確認する。

## 2026-04-14 — fix: shared coordination storage のスコープ変更では SPEC と保存キーを先に揃える

### 事象

Board を複数 worktree 間で共有したいのに、実装と SPEC の一部が `repo-local` /
worktree 配下前提のまま残っており、`~/.gwt/` 配下の project-scoped storage へ
寄せる判断と文面が食い違っていた。

### 原因

- 「同じ repo で共有したい」という要件に対して、保存キーを worktree path と
  project identity のどちらに置くかを最初に固定していなかった。
- path helper だけ直せばよいと見て、関連 SPEC と README の user-facing path を
  同じ変更セットで更新する前提が弱かった。

### 再発防止策

1. shared Board / logs / cache の保存スコープを変えるときは、まず project key
   (`RepoHash` など) を明示してから path helper を実装する。
2. worktree-local から project-scoped へ移す変更では、legacy merge/delete 方針を
   SPEC と data model に先に書き戻す。
3. user-facing path が変わる変更では、README と Issue SPEC の更新をコード変更と
   同じ変更セットに含める。

## 2026-04-14 — fix: terminal copy UX は host shortcut 前提より既存 selection 契約を優先する

### 事象

terminal copy を `Cmd+C` 中心に寄せた結果、host terminal / crossterm の実イベント差で
実機では copy できず、さらに途中のクリック処理変更で Claude の範囲選択まで壊した。

### 原因

- 「platform shortcut に統一したい」という意図を優先し、既に成立していた
  `drag selection -> copy` の user-facing 契約を軽く扱った。
- shortcut 自体の実機観測が不十分な段階で、selection と click routing まで同じ変更で動かした。

### 再発防止策

1. host terminal が奪う可能性のある shortcut 変更では、まず既存の selection / copy 契約を
   温存したまま追加実装で検証する。
2. click focus、drag selection、copy trigger は別々に変更し、1 回の修正で束ねない。
3. terminal UX の変更は unit test だけでなく、実機で「クリック」「ドラッグ」「コピー」を
   連続確認してから完了扱いにする。

## 2026-04-14 — fix: terminal selection copy は visible screen の実サイズに clamp してから vt100 へ渡す

### 事象

snapshot history を見ている状態で terminal viewport を広げたあと、広がった幅のまま複数行選択すると
`vt100::Screen::contents_between()` が `attempt to subtract with overflow` で panic した。

### 原因

- selection の row/col は現在の TUI viewport 基準で保持していたが、コピー時に参照する visible screen は
  過去 snapshot のままで、幅・高さが現在 viewport より小さい場合があった。
- `selected_text()` が selection 座標を clamp せず、そのまま `contents_between()` に渡していたため、
  `start_col > screen.cols()` となり `cols - start_col` が underflow した。

### 再発防止策

1. terminal selection を外部 crate (`vt100`) の row/col API に渡す前に、必ず visible screen の実サイズへ clamp する。
2. snapshot/history と resize が絡む選択コピーでは、「現在 viewport より狭い snapshot に対する複数行選択」を
   回帰テストで固定する。
3. mouse 座標や selection state を保持する変更では、render surface と copy surface が同一サイズとは限らない前提で
   境界条件を確認する。

## 2026-04-13 — tui: 常設入力欄では plain 文字ショートカットを残さない

### 事象

Board tab に常設入力欄を出したあとも `k` / `r` を plain key shortcut のまま残していたため、
`i` で入力モードに入らない限り本文入力と衝突した。結果として、入力欄が見えていても
直打ちできず、最初の文字が `k` / `r` の投稿もできなかった。

### 原因

- hidden composer 前提の keybind を引きずり、常設入力欄では「plain printable key は本文入力」
  が最優先になることを操作契約として固定していなかった。
- Board を閲覧 UI と入力 UI の中間状態で扱い、shortcut と text entry の責務を分離していなかった。

### 再発防止策

1. 常設入力欄のある TUI では、plain printable key をショートカットに使わない。
2. printable key と衝突する操作は `Ctrl+<key>` など modifier 付きに移す。
3. key routing test には「plain key が本文入力になること」と「modifier shortcut が別操作になること」を両方固定する。

## 2026-04-13 — tui: 端末ホストが奪うショートカットを送信キー前提にしない

### 事象

Board tab の composer 送信を `Ctrl+Enter` にしていたが、Terminal.app では別ウィンドウ系の
ホストショートカットに吸われ、実際には投稿操作として成立しなかった。加えて、
入力欄自体が hidden composer で、ユーザーが投稿先を見つけにくかった。

### 原因

- TUI 内のキー衝突だけを見て、Terminal.app / iTerm2 などホスト端末が先に処理する
  ショートカットを確認していなかった。
- 「入力欄は `i` で開けばよい」という実装都合を優先し、投稿導線が視覚的に存在するかを
  user-facing UX として見ていなかった。

### 再発防止策

1. TUI の送信・決定系ショートカットは、まずホスト端末で奪われにくいキー
   (`Enter`, `Ctrl+J`, `Esc` など) を候補にする。
2. hidden composer やモーダル前提で投稿導線を隠す変更では、常設入力欄が必要かを
   先に検討する。
3. 端末依存のショートカット変更では、render test だけでなく key routing test に
   実際の送信・改行契約を固定する。

## 2026-04-13 — docs/skills: 公開 surface を切り替えたら alias を残す前提を置かない

### 事象

`gwt-discussion` / `gwt-plan-spec` / `gwt-build-spec` / `gwt-manage-pr` を公開入口に
切り替えたあとも、`gwt-issue` / `gwt-pr` / `gwt-spec-*` を compatibility alias として
残す前提で skill asset / AGENTS / SPEC を更新してしまい、ユーザーから
「重複して見えるので不要」と指摘を受けた。

### 原因

- 「後方互換を残すほうが安全」という一般論を優先し、この repo の公開 UX では
  重複 surface 自体がコストになることを軽視した。
- skill 本体だけでなく、distribution test、lint script、SPEC 正本まで含めて
  alias 削除の影響範囲を最初に洗い出していなかった。

### 再発防止策

1. 公開 task entry point を新設したら、まず「alias を残す必要が本当にあるか」を
   user-facing UX で判断する。
2. alias を削除する場合は、skill/command asset、distribution test、
   lint script、AGENTS、SPEC を同じ変更セットで更新する。
3. `git ls-files` を使う検証スクリプトは、削除済み tracked file を拾って
   途中状態で壊れないか確認する。

## 2026-04-13 — spec: shared board を intake 専用に狭めず、通信 + タスク管理 + agent presence を分けて設計する

### 事象

shared board の初期整理を「未分類要求の intake board」中心で進めた結果、
ユーザーが必要としていた「各エージェントが今何をしていて、次に何をすべきで、
他へ何を伝えるべきか」という coordination 面を十分に表現できていなかった。

### 原因

- board entry を request intake と会話ログの延長で捉え、agent 自身の current state
  を first-class な domain object として扱っていなかった。
- `gwt-discussion` / `workflow-policy` / session runtime の既存責務分離を踏まえず、
  coordination 面の正本を単一の board stream に寄せすぎた。

### 再発防止策

1. shared board / collaboration 系の設計では、最初に「request stream」と
   「agent presence / card」を分離して考える。
2. board を intake 専用と決め打ちせず、communication、next action、blocked、
   handoff、decision をどこまで正本に持つかを明示する。
3. 直接 message API を考える前に、shared board と projected latest state で
   十分な coordination が成立するかを検討する。

## 2026-04-13 — test: 通知経路が再入する helper では pending PTY queue の観測点を誤らない

### 事象

`handle_discussion_resume_message_with_resume_queues_prompt_input` のテストで、
resume prompt を `pending_pty_inputs` に積んだ直後に通知を出した結果、
通知側の `update()` 再入で queue が即時 drain され、`queued prompt` assertion
が失敗した。

### 原因

- `apply_notification()` は内部で `update()` を呼び、`update()` の末尾で
  `drain_pending_pty_inputs()` が走るため、helper 単体テストの観測点として
  `pending_pty_inputs` が不安定だった。
- `crates/gwt-tui/src/app.rs` の helper は `apply_notification()` のあとに
  `push_input_to_session()` を呼んでおり、notification 側の nested `update()` が
  queue 観測に割り込んでいた。

### 再発防止策

1. PTY 入力投入と通知発火を同じ helper で行う場合は、`update()` 再入による
   queue drain の有無を先に確認してからテストを書く。
2. helper 単体で pending queue を観測したい場合は、通知より後に入力を積むか、
   queue ではなく配信先を観測する。
3. TUI テストで「積まれたはずの入力」が見えないときは、実装バグより先に
   `apply_notification()` などの nested update を疑う。

## 2026-04-13 — fix: startup self-heal の生成物を bundle materialize の既存状態と混同しない

### 事象

`info/exclude` と hook 設定だけを pristine repo/worktree に self-heal したいのに、
次回 startup で `.claude/settings.local.json` / `.codex/hooks.json` の存在を見て
埋め込み skill bundle まで materialize してしまう経路が入り得た。

### 原因

- startup refresh の gate が「managed bundle の既存状態」と「self-heal で常に生成する
  hook 設定ファイル」を同じ root list で判定していた。
- `git rev-parse --git-path info/exclude` は relative path を返す場合があり、
  テスト側も実装側も Git が解決した path をそのまま正本として扱う必要があった。

### 再発防止策

1. startup repair の gate は「bundle materialize を許可する signal」と
   「常に補完すべき self-heal 生成物」を分離する。
2. linked worktree や bare/alternate Git layout を触るテストでは、
   `git rev-parse --git-path info/exclude` の戻り値が relative / absolute の両方を
   取り得る前提で path を解決する。
3. self-heal 追加時は「1回目の起動」だけでなく「2回目の起動でも余計な materialize が
   起きない」ことまでテストで固定する。

## 2026-04-13 — fix: tick 駆動の更新は redraw gate だけでなく deadline の寿命も固定する

### 事象

Cleanup progress モーダルが実際には進んでいても、`Ctrl+G` を押すまで進捗件数や
`Cleanup Complete` への切り替わりが画面に出なかった。

### 原因

- cleanup worker のイベント drain は `Message::Tick` に依存していた。
- main loop 側は outer loop を 1 周するたびに `Instant::now() + 100ms` で
  tick deadline を作り直しており、PTY output や他イベントでループが回り続けると
  Tick 自体が飢餓した。
- `Ctrl+G` prefix は keybind 上 `Message::Tick` を直接発行するため、手動で
  cleanup queue をポンプしたときだけ画面が更新されていた。

### 再発防止策

1. tick 駆動の overlay / modal を追加するときは、redraw gate だけでなく
   「tick deadline を non-tick イベントで延長しない」ことまでテストで固定する。
2. event loop の定期処理が `Tick` 依存なら、deadline は Tick を実際に消費するまで
   保持し、外側ループの都度 `now + interval` で再計算しない。
3. `Ctrl+G` のような prefix 消費でだけ UI が進む報告は、「手動で Tick を注入すると
   直る」シグナルとして扱い、queue drain と scheduler の両方を確認する。

## 2026-04-13 — fix: Issue / SPEC cache と index は repo 単位の exact cache を唯一の入力にする

### 事象

Branches 一覧や Launch Agent の Issue 選択、Specs 一覧が global cache
(`~/.gwt/cache/issues/`) を横断しており、開いている repo と無関係な Issue / SPEC が混在した。
さらに issue index は cache ではなく `gh issue list` から直接作っていたため、
`GitHub/gh -> cache -> index -> UI` の一方向フローを外していた。

### 原因

- `~/.gwt/cache/issues/` を gwt 専用または global な一覧 source と誤認し、
  repo hash ごとの境界を consumer 側で維持していなかった。
- index builder が cache consumer ではなく direct fetcher になっており、
  UI と index が別の truth source を見ていた。

### 再発防止策

1. `~/.gwt/cache/issues/` を触る変更では、最初に `repo-hash` 配下が current repo の
   exact cache root であることを確認する。
2. Issue / SPEC / Launch Agent / index の consumer は全て同じ repo-scoped cache root を
   共有し、remote fetch は cache 更新レイヤーに閉じ込める。
3. cache path や startup refresh を変えるときは、consumer 側の Rust test と
   index runner 側の Python test を同じ変更で固定する。

## 2026-04-12 — fix: 端末喪失時の crossterm 内部スピンによる CPU 100%

### 事象

ターミナルタブを閉じた後、gwt-tui プロセスが CPU 100% で数百分回り続けていた。
`ps aux` で 5 プロセスが 100% 消費（うち 3 つは fd revoked、2 つは TTY hung-up）。

### 原因

crossterm 0.29.0 の `try_read()` 内部リードループ（`mio.rs:95-120`）に 3 つの欠陥:

1. `read()` が `Ok(0)` (EOF) を返しても `break` しない
2. `read()` が `Err(EIO/EBADF)` を返しても error ハンドリングが空でループ継続
3. 外側のタイムアウトチェック（line 149）に内部ループから到達不能

結果として `crossterm::event::poll()` 自体がリターンせず、gwt-tui 側のエラー
ハンドリング (`unwrap_or(false)`) は無関係だった。

### 再発防止策

1. `libc::tcgetattr` で `poll()` 呼び出し前に端末の健全性を検査（revoked fd と
   hung-up terminal の両方を検知）
2. `signal-hook` で SIGHUP を捕捉し、メインループでグレースフルに終了
3. `crossterm::event::poll()` の `Err` を `unwrap_or` で握りつぶさず `match` で
   明示的にハンドリング
4. テスト環境（stdin が非端末）では `OnceLock` で初期状態を記録し、検査をスキップ

### 教訓

- `sample <pid> 1` でスレッド別のホットスポットを特定すること。コード分析だけでは
  スピンが crossterm 内部かアプリ側かの切り分けができない
- サードパーティの `poll()`/`read()` がリターンしない可能性を想定し、呼び出し前に
  前提条件を検証する防御的プログラミングを行う
- `unwrap_or(false)` はエラーを握りつぶすアンチパターン。`match` でエラーパスを明示する

## 2026-04-12 — test: Profiles の環境一覧クリックは固定キー順を前提にしない

### 事象

PR #1966 の CI で
`profiles_mouse_click_selects_profile_row_and_focuses_env_pane` が
`left: Some("ACCEPT_EULA") / right: Some("API_URL")` で失敗し、後続の
`with_temp_home` 系テストが `HOME_ENV_TEST_LOCK` の poison で連鎖失敗した。

### 原因

- Profiles の環境一覧は `std::env::vars()` と profile override をまとめて
  キー昇順で描画するが、テストは「先頭の可視行をクリックする」実装のまま
  `API_URL` が選ばれる前提を置いていた。
- GitHub Actions runner では `ACCEPT_EULA` など `API_URL` より前に並ぶ
  OS 環境変数が存在し、ローカルより先頭行が変わっていた。

### 再発防止策

1. `std::env::vars()` を含む UI テストでは固定キー名の表示順を仮定せず、
   クリックする可視行から期待値を計算する。
2. process-global env lock を使うテスト群で 1 件の panic が連鎖失敗を生む場合、
   先頭の順序依存 assertion を優先して安定化する。

## 2026-04-10 — fix: wide glyph の trailing clear は「出すか」だけでなく「順序」まで固定する

### 事象

wide glyph redraw で stale trailing text を防ぐために trailing clear を追加したが、
crossterm backend では `wide glyph -> trailing blank` の順で `Print` され、
一時的な余分な空白が表示された。マウス選択で glyph が再描画されると見た目だけ
正常化した。

### 原因

- `ratatui-core::Buffer::diff` が wide glyph の visible cell を先に、trailing clear を
  後に emit していた。
- `ratatui-crossterm` backend は diff をそのまま cursor move + `Print` に変換するため、
  blank 後描きがそのまま視覚アーティファクトになった。

### 再発防止策

1. wide glyph の redraw 回帰では「trailing clear があること」だけでなく、「clear が visible glyph より前に出ること」を diff レベルでテストに固定する。
2. backend 依存の表示不具合では buffer equality だけで完了扱いせず、diff の update 順と backend の print 順まで追う。
3. 「選択や hover で正常化する」症状は redraw 順序不正のシグナルとして扱い、overlay 側の見た目修正で済ませない。

## 2026-04-10 — fix: wide glyph の見切れは renderer だけでなく backend diff の trailing clear まで確認する

### 事象

Codex agent terminal で、日本語の wide glyph が full-screen redraw のたびに
右端や行中でまだ見切れた。`cell.contents()` を保持する修正後も、
前フレームの trailing cell の文字が右半分に残った。

### 原因

- `renderer.rs` 側では grapheme cluster と visible-right-edge crop を直していたが、
  `ratatui-core::Buffer::diff` が CJK wide glyph では trailing clear を明示出力せず、
  stale trailing cell が残る端末条件を見落としていた。
- 併せて、selection overlay が wide continuation cell 側だけを選択したケースで、
  可視 glyph 側へ style を集約できていなかった。

### 再発防止策

1. wide glyph の不具合では `vt100 cell -> Buffer` だけで完了扱いせず、`Buffer::diff -> backend Print` まで追って trailing clear の有無を確認する。
2. 回帰テストには「CJK wide glyph redraw で trailing clear が diff に出ること」を必ず含める。
3. wide continuation cell は hidden cell として扱い、selection / URL overlay は glyph 幅全体に集約する。

## 2026-04-10 — spec: skill 文面だけで満足せず、質問UI契約は asset contract test まで固定する

### 事象

`gwt-spec-brainstorm` の SPEC と SKILL.md には「選択UIで継続質問する」と
書かれていたが、repo 側の回帰テストは `gwt issue spec list` 参照や command
存在確認しか見ておらず、1 回回答で止まる退行を検出できなかった。

### 原因

- 継続質問を「prompt 上の方針」としてしか扱わず、repo 内で拘束される
  asset contract として扱っていなかった。
- command 文面も selection UI first / next highest-impact question /
  single-answer exit 禁止まで明示しておらず、実質的に LLM 任せだった。

### 再発防止策

1. 質問UIや handoff 条件のような prompt contract を変更したら、SKILL.md だけでなく command と asset contract test を同じ変更セットで更新する。
2. 「継続する」と書くだけで満足せず、「回答後に何を再計算し、何が残っていたら exit してはいけないか」まで文面で固定する。
3. GitHub-backed SPEC を更新したら、対応する repo-side test がその契約を拾えているかを確認する。

## 2026-04-10 — fix: managed asset の startup sweep は prune だけで終わらせず、missing tracked file も restore する

### 事象

`GWT_MANAGED_HOOK=runtime-state` や split bash blocker が一部 worktree の
`.codex/hooks.json` / `.claude/settings.local.json` に残り続け、さらに
tracked な `.claude/commands/gwt-spec-brainstorm.md` が一度消えると
最新 binary を入れても自然復旧しなかった。

### 原因

- startup 時の repo/worktree sweep が `prune_stale_gwt_assets` だけで、
  distribute / hook regeneration / exclude update を回していなかった。
- `distribute_to_worktree` は tracked path を常に skip しており、
  既に削除された tracked managed asset まで restore できなかった。

### 再発防止策

1. managed asset の migration を commit したら、「新規 launch 時」だけでなく
   「既存 repo/worktree の startup self-heal」で反映されるかを確認する。
2. tracked asset の preserve ロジックでは「既存 file を上書きしない」と
   「missing file を復旧しない」を混同しない。
3. startup repair を追加するときは、empty repo を不用意に dirty にしない gate
   まで同時にテストで固定する。

## 2026-04-10 — fix: vt100 renderer は `chars().next()` で cell を潰さず、visible area の右端 crop も見る

### 事象

Codex agent terminal で日本語や emoji を含む行が右端で見切れた。特に
emoji presentation sequence は先頭 codepoint だけに潰れ、wide glyph は
visible pane が vt100 幅より狭いときに右端で半端表示され得た。

### 原因

- `crates/gwt-tui/src/renderer.rs` が `vt100::Cell::contents()` 全体ではなく
  `chars().next()` だけを使っており、cell 内の grapheme cluster を失っていた。
- renderer が「vt100 screen 幅には収まるが、現在の visible area には trailing cell が
  入らない」ケースを見ておらず、wide glyph をそのまま描こうとしていた。

### 再発防止策

1. vt100 renderer を触るときは、`cell.contents()` を「表示用の完全な terminal grapheme」として扱い、先頭 codepoint のみへ落とさない。
2. wide glyph の描画では terminal 全体の列数だけでなく、現在の visible area 幅で trailing cell が収まるかを必ず判定する。
3. renderer 回帰テストには「emoji grapheme 保持」と「cropped right edge での wide glyph 抑止」をセットで追加する。

## 2026-04-10 — feat: `~/.gwt/cache/issues/` を SPEC 専用と決め打ちしない

### 事象

`gwt issue view/comments/create/comment` を実装した際、`gwt-github::Cache::write_snapshot`
が `SpecBody::parse` を必須としていたため、plain GitHub Issue の body を cache できず
`Parse(MissingHeader)` で失敗した。

### 原因

- cache root が `~/.gwt/cache/issues/` で共通なのに、実装が `gwt-spec` header を
  持つ entry しか来ない前提になっていた。
- `gwt issue spec ...` の契約をそのまま plain issue read/write に流用し、
  cache entry の consumers が増えたことを data model に反映できていなかった。

### 再発防止策

1. `~/.gwt/cache/issues/` を触る変更では、「SPEC artifact cache だけか」「plain issue snapshot も入るか」を先に分けて考える。
2. body marker を持たない entry が入る可能性がある場合、cache layer は snapshot-only fallback を持たせ、spec-only parse failure で全体を落とさない。
3. cache schema を広げるときは `gwt-github` の unit tests と、consumer 側 (`gwt-tui` CLI/TUI) の integration tests を同時に追加する。

## 2026-04-10 — feat: Branch Cleanup の selectable/blocked/risky 契約は glyph・toast・confirm を同時に更新する

### 事象

Branch Cleanup の仕様議論で「`·` は warning 付きで選択可能、`–` だけ blocked」に寄せたあとも、
実装の一部が旧契約のままで、未マージブランチや remote-tracking 行の選択可否が
表示と一致しない状態になった。

### 原因

- cleanup 可否判定、gutter glyph、blocked toast、confirm warning、SPEC artifacts が
  別々に更新されていた。
- 特に `safe-selectable` / `unsafe-selectable` / `blocked` の3段階モデルが
  1つの型や表で管理されておらず、読み手が `✔ だけ selectable` と誤解しやすかった。

### 再発防止策

1. Branch Cleanup の選択契約を変更するときは、`BranchesState` の判定API、gutter glyph、`Space` の toast、confirm modal の warning、footer hint、SPEC/quickstart を同じ変更セットで更新する。
2. `origin/foo` のような remote-tracking 行は「表示名」と「実行対象 local branch」がズレるため、UI 表示用データと execution metadata を分けてテストで固定する。
3. 「押しても何も起きない」報告が来たら、通知 state や描画だけでなく、対象行が safe/risky/blocked のどこに分類されているかを最初に確認する。

## 2026-04-10 — fix: tick で更新される overlay は terminal focus でも redraw gate に含める

### 事象

Branch Cleanup の進捗モーダルが実際には完了していても、`Cleanup Complete` 表示への切り替わりが
次のキー入力まで見えず、`Ctrl+G` などを押したときにだけ更新された。

### 原因

- cleanup worker の完了イベントは `Tick` で drain していたが、dirty-driven redraw の gate
  (`tick_redraw_required`) が terminal focus 時に `cleanup_progress` を periodic redraw
  対象として扱っていなかった。
- そのため state は更新済みでも、非 tick 入力が来るまで再描画されない経路が残っていた。

### 再発防止策

1. worker queue を `Tick` で drain する overlay / modal を追加したら、対応する redraw gate にも同じ surface を追加する。
2. 「state は変わるが次のキー入力まで見えない」不具合では、event queue drain と redraw 判定をセットで確認する。
3. idle redraw 抑制の回帰テストには、terminal focus 中でも更新が必要な overlay を最低 1 つ含める。

## 2026-04-10 — spec: 旧 GitHub-backed SPEC を更新する前に `gwt issue spec --section` / `repair` の健全性を先に確認する

### 事象

SPEC 更新作業で `#1930` を `gwt issue spec 1930 --section spec` と
`gwt issue spec repair 1930` で扱おうとしたところ、どちらも
`body parse error: malformed marker: nested BEGIN for 'spec' inside 'tasks'`
で失敗した。

### 原因

- 旧 GitHub-backed SPEC の artifact 配置が current parser の前提とずれており、
  section 読み取り前提の CLI 経路自体が壊れていた。
- 「既に GitHub Issue 化されている SPEC なら `gwt issue spec` で必ず
  section 単位に読める」と思い込んで更新フローを組み立てた。

### 再発防止策

1. 旧 SPEC の更新前に `gwt issue spec <n> --section spec` と
   `gwt issue spec repair <n>` の smoke check を先に実行し、section parser
   が生きていることを確認する。
2. parser が壊れている legacy SPEC は、無理に section CLI に載せず、
   raw issue body / comment を直接更新する暫定経路へ切り替える。
3. 暫定経路を使った場合は、後続で artifact format 自体を repair する
   follow-up を SPEC / tasks に明記し、同じ壊れ方を放置しない。

## 2026-04-10 — fix: terminal-focus redraw gate は raw summary 全体ではなく visible surface と tick 前後差分で判定する

### 事象

Branches の live session indicator で、filter/search で見えていない branch や
summary strip が表示できない狭い row に `Running` session があるだけで
terminal focus 中の idle redraw が復活した。
さらに visible な spinner が `Running -> WaitingInput` に変わる tick で、
最後の 1 回の repaint が落ちて古い spinner が残り得た。

### 原因

- redraw gate が `live_session_summaries` 全体を見ており、
  実際に描画中の branch row / summary width / viewport を反映していなかった。
- tick 後の `needs_render` 判定が post-update state だけを見ていたため、
  `Running` が消えた tick 自体は redraw 不要と誤判定した。

### 再発防止策

1. redraw suppression / animation gate は backing store 全体ではなく、実際に render される visible surface から計算する。
2. tick-driven animation が static state や hidden state に切り替わる UI は、post-tick の要否だけでなく pre/post の visible snapshot 差分で final repaint を保証する。
3. render と gate で幅計算や visibility 判定を二重実装しない。少なくとも同じ helper を通して narrow-row / filtered-row の挙動を揃える。

## 2026-04-10 — fix: `SessionStart` は「起動した」だけで `Running` とみなさない

### 事象

Branches の live session indicator で、agent を起動した直後まだ入力待ちのはずなのに
spinner が回り続けて見えた。

### 原因

- launch bootstrap が runtime sidecar を `Running` で初期化していた。
- hook state mapping でも `SessionStart` を `Running` にしており、
  実行中イベントと待機イベントの境界を誤っていた。
- そのため、ユーザー入力も tool 実行も始まっていない session でも
  `WaitingInput` ではなく `Running` として Branches に表示されていた。

### 再発防止策

1. hook event を state に写像するときは、`session started` と `work started` を同一視しない。
2. launch bootstrap が必要でも、初期状態は「見えてほしい state」にするのであって、「最初に困らない animation state」に寄せない。
3. live indicator の仕様変更では、launch 直後の bootstrap、hook の `SessionStart`、実行中の `Pre/PostToolUse` を別々に RED テストで固定する。

## 2026-04-09 — fix: `tracing_appender::rolling::daily` の日付境界は思い込みで local 扱いしない

### 事象

`gwt-core/tests/logging_init.rs` が深夜帯に恒常失敗し、`init()` 後に
`tracing::info!` を流しても `current_log_file()` が見に行くファイルが空のままだった。

### 原因

- `crates/gwt-core/src/logging/writer.rs` と `crates/gwt-tui/src/logs_watcher/watch.rs` が
  `gwt.log.YYYY-MM-DD` の日付を local 前提で扱っていた。
- しかし実際に使っている `tracing-appender 0.2.4` の `rolling::daily` は UTC 日付で
  ファイル名とローテーション境界を決めていた。
- そのため JST 深夜のような UTC 日付との差分がある時間帯では、
  テスト・watcher・housekeeping が別の日のファイルを見ていた。

### 再発防止策

1. 外部 crate の時間境界や file naming はコメントや記憶ではなく、使っている version の source / docs で確認する。
2. log rotation のような date-sensitive contract は writer だけでなく watcher / housekeeping / spec を同時に揃える。
3. 深夜帯だけ落ちる integration test を見たら、まず local time と UTC のどちらを境界にしているかを疑う。

## 2026-04-09 — fix: IME 切り分けで `raw` だけ正常なら「入力解釈」ではなく idle redraw を先に疑う

### 事象

`gwt` では日本語 IME 候補選択が壊れる一方、同じ terminal で raw `crossterm`
probe は正常だった。さらに layered probe では `raw` は正常で `redraw` /
`ratatui` が同様に壊れた。

### 原因

- 問題の本体は `crossterm` の raw event 解釈ではなく、周期 redraw が
  composition 中の IME UI を中断していたことだった。
- `gwt` の main loop は outer loop ごとに無条件で `terminal.draw(...)`
  しており、tick ベースの idle repaint が terminal focus 中でも止まらなかった。

### 再発防止策

1. terminal/IME の不具合で raw probe だけ正常な場合、まず「event routing」より
   先に「idle redraw / cursor reset / clear」を疑う。
2. 再現 probe は `raw` / `redraw` / `ratatui` のように 1 変数ずつ増やし、
   redraw そのものと rendering stack を分離して検証する。
3. main loop の描画は無条件 repaint ではなく dirty-driven にし、
   terminal focus 中の idle tick redraw は明示的な必要条件があるときだけ許可する。

## 2026-04-09 — fix: dirty-driven redraw にしたら PTY drain の全経路で redraw dirty を立てる

### 事象

idle tick redraw を止めたあと、IME 確定文字だけでなく ASCII 入力も 1 文字遅れで
表示され、次のキー入力でまとめて見えるようになった。

### 原因

- outer loop 冒頭で PTY output を drain した場合は redraw dirty を立てていたが、
  poll loop 中に到着した PTY output を drain した場合は dirty flag を更新していなかった。
- そのため model 自体は更新されていても、次の入力イベントまで描画が走らない経路が残った。

### 再発防止策

1. dirty-driven redraw への変更では、「どの経路が redraw を要求するか」を helper に集約し、複数ループで手書きしない。
2. PTY output のような非入力イベントは「state が変わった」だけでなく「見た目を更新する必要がある」ことを RED テストで固定する。
3. idle redraw suppression の修正後は、IME だけでなく plain ASCII / shell output でも 1 文字遅延がないかを必ず確認する。

## 2026-04-08 — fix: `REPORT_EVENT_TYPES` を有効化したら `Repeat` を `Press` と別経路で扱う

### 事象

IME 候補の最初の一覧表示は維持されたが、次ページへ送るタイミングで入力制御を
失う症状が残った。

### 原因

- 起動時に `PushKeyboardEnhancementFlags(DISAMBIGUATE_ESCAPE_CODES | REPORT_EVENT_TYPES)`
  を有効化したことで、互換端末では `KeyEventKind::Repeat` が届くようになった。
- しかし `event.rs` の翻訳層は `KeyEventKind::Press` だけを `Message::KeyInput` に
  流し、`Repeat` を無条件で捨てていた。
- そのため、ページ送りのような繰り返しキーに依存する入力経路が途中で途切れた。

### 再発防止策

1. `crossterm` の keyboard enhancement flag を増やしたら、対応する
   `KeyEventKind` の downstream 契約まで必ず確認する。
2. event loop の key filter 変更では `Press` だけでなく `Repeat` / `Release` の
   扱いを RED テストで固定する。
3. IME や端末依存の入力不具合では、flag 有効化そのものだけでなく、その後段の
   イベント翻訳層が新しい event kind を落としていないかを先に疑う。

## 2026-04-08 — fix: 重い Cargo テストを並列実行すると flaky なタイムアウトを誘発しやすい

### 事象

`cargo test -p gwt-tui` と `cargo test -p gwt-core -p gwt-tui` を同時に走らせた際、
`update_branches_docker_stop_executes_and_refreshes_detail` が一度だけ timeout で失敗した。
同じテストを単独で再実行すると成功した。

### 原因

- package cache / build directory の lock 待ちと並列負荷が重なり、
  Docker worker の完了待ちが一時的に遅延した。
- 重い Cargo テストの並列化は、CI と異なるローカル負荷条件で flaky を誘発しやすい。

### 再発防止策

1. `cargo test -p gwt-tui` と `cargo test -p gwt-core -p gwt-tui` のような重複する重い検証は並列で回さず、順次実行する。
2. flaky が出た場合は直ちに単独再実行し、再現性を確認してから結果を採用する。
3. 並列化は読み取り系コマンド中心に限定し、負荷の高い Rust ビルド/テストには適用しない。

## 2026-04-09 — fix: 狭い幅の status bar 通知は「出ているか」ではなく full render で「読めるか」を確認する

### 事象

`Branch Cleanup` で選択不可の branch に `Space` を押すと `Info` notification 自体は
発火していたが、通常の 80 桁前後の画面では footer に載った文言が切れて、
ユーザーには「何も表示されない」ように見えた。

### 原因

- 通知生成の実装と state 更新は正常だったが、status bar の narrow layout が
  notification ありのとき compact 化されず、prefix (`INFO cleanup:`) が幅を消費していた。
- 既存テストは notification object の存在や unit-level render しか見ておらず、
  実際の full render でメッセージ本体が読めるかを固定していなかった。

### 再発防止策

1. 狭い terminal 幅でも必要な本文が残るよう、status bar の compact/verbose 表示契約を明示してテストで固定する。
2. notification 系の修正では state だけでなく `render_model_text(..., 80, ...)` 相当の full render を確認し、「表示された」ではなく「読める」を検証する。
3. 長い UX 文言を追加する場合は source/severity prefix を含んだ実表示幅で確認し、必要なら短い文言を選ぶ。

## 2026-04-09 — fix: Branches の action は list だけでなく detail focus でも契約を揃える

### 事象

`Branch Cleanup` の選択不可理由 toast を実装したあとも、実利用では
selected branch の detail pane に focus がある状態で `Space` を押すと
何も起きなかった。

### 原因

- `Space` の cleanup toggle は `route_key_to_management()` の Branches list 経路にだけあり、
  `route_key_to_branch_detail()` には同じ action が存在しなかった。
- 修正時の検証が list focus の unit test に偏っており、detail focus の入力経路を見ていなかった。

### 再発防止策

1. Branches のように list/detail が同じ selected entity を共有する画面では、共通 action を helper に寄せて両 focus から使う。
2. keybind 追加や修正では `TabContent` だけでなく `BranchDetail` / `Terminal` を含む focus matrix を最低限確認する。
3. 「何も起きない」系の報告では state/render だけでなく、まず key routing が現在 focus に結び付いているかを調べる。

## 2026-04-09 — fix: cleanup の active-session guard は stopped agent tab を block しない

### 事象

`Space` で cleanup 選択できない branch があり、ユーザーから見ると
「セッション起動中ではないのにトグルしない」状態になっていた。

### 原因

- `refresh_active_session_branches_with()` は agent tab が存在するだけで branch を
  active 扱いし、persisted/runtime status が `Stopped` でも block していた。
- 一方で branch list の live session summary は runtime-aware に
  `Running` / `WaitingInput` だけを表示していたため、見た目と block 判定が食い違っていた。

### 再発防止策

1. 同じ「active session」概念を複数箇所で使う場合は、status 判定を helper で共有し、UI 表示と block guard を別実装にしない。
2. cleanup guard の変更では `Running` だけでなく `Stopped` の負例を必ずテストに含める。
3. 「セッション起動中ではないのに操作できない」報告では、tab の存在と runtime status を分けて確認する。

## 2026-04-07 — fix: IME 問題に常用トグルを足す前に実アプリの入力証跡を取る

### 事象

日本語 IME 候補選択の途中送信に対して、`Ctrl+G,y` で切り替える
terminal IME mode を先に実装したが、ユーザーから「入力モードを切り替えるのは
非常に使いにくい」とフィードバックを受け、常用 UX として成立しなかった。

### 原因

- `crossterm` / 端末 / IME の境界でどのキーが実際に届いているかを確定する前に、
  明示トグルという回避策を product surface に乗せた。
- 「安全側の暫定回避」を、そのまま常用 UX として残してしまった。

### 再発防止策

1. 端末入力まわりの不具合では、まず実アプリ上の入力経路（raw event / keybind / PTY forward）の証跡を取る。
2. 根本原因が未確定の段階では、常用を前提にした入力モード切替や persistent UI affordance を追加しない。
3. 調査用機能は env-gated logging など fail-open な計測を優先し、通常 UX への影響を 0 に近づける。
## 2026-04-08 — fix: coalesced `home + repaint` payloads need frame segmentation before Codex row-history derivation

### 事象

Codex の inline mode でも、1 回の `PtyOutput` payload に複数の `home + repaint`
full-screen redraw が畳まれるケースでは、scroll が 다시 page-sized snapshot fallback に落ちた。

### 原因

- `split_agent_snapshot_segments()` は `\x1b[2J\x1b[H` だけを redraw 境界としていた。
- そのため `\x1b[H` で始まる full repaint が 1 payload に複数入っても最後の frame しか見えず、
  `detect_vertical_redraw_shift()` が必要とする隣接 frame 間の shift が消えていた。
- 結果として `max_scrollback == 0` のままになり、Codex pane が line-granular row history へ昇格できなかった。

### 再発防止策

1. agent redraw segmentation は `clear+home` だけでなく、「top rows を連続再描画する qualified home repaint」も frame 境界として扱う。
2. `1 payload 内に 2 回の home repaint がある Codex 相当ケース` を model/app の RED テストで固定する。
3. Codex inline mode 導入後も、大きい PTY payload は `multiple redraw frames per payload` を疑ってログとコードを照合する。

## 2026-04-08 — fix: runtime distribution must prune stale managed asset paths, not just root entries

### 事象

`distribute_to_worktree()` は current bundle を materialize できていたが、
managed roots に残っている古い `gwt-*` skill / command / hook が残存した。
さらに root entry だけを消す実装では、現行 bundle に残っている skill directory の内側に
ぶら下がった stale file / subdirectory までは消えず、launch を重ねても過去の遺産が worktree に居座った。

### 原因

- distribution は「今ある managed asset を書く」ことしかしておらず、
  「今の bundle に存在しない managed path を消す」責務を持っていなかった。
- 最初の cleanup は managed roots の root-level `gwt-*` entry にしか効かず、
  retain される `gwt-*` directory の内部までは source tree と照合していなかった。
- tracked-file 保護は current bundle の write skip には有効だったが、
  stale residue の pruning まで止める設計ではなかった。

### 再発防止策

1. managed roots は write 前に embedded bundle source tree と照合し、current bundle に存在しない path を prune する。
2. stale path の pruning は tracked / untracked を問わず、root entry だけでなく retained managed directory 配下の nested file / subdirectory まで適用する。
3. recursive cleanup は current bundle が管理する tree に限定し、非 `gwt-*` root entry や hook config (`settings.local.json`, `hooks.json`) には触れない。

## 2026-04-08 — fix: launch-only cleanup leaves previously active worktrees dirty until relaunch

### 事象

agent launch 時の cleanup を実装した後も、すでに起動済みだった worktree には
old `gwt-*` skill / command residue が残り続け、Claude/Codex から旧 surface が見えた。

### 原因

- cleanup の契約を launch materialization にだけ置いていたため、
  「過去に launch 済みで、まだ relaunch していない active worktree」は sweep 対象外だった。
- その結果、現在の binary が clean でも、既存 worktree 上の legacy residue は次の launch まで残留した。

### 再発防止策

1. launch 時の full distribution とは別に、startup / repo-worktree discovery で prune-only sweep を持つ。
2. startup sweep は repo root と active worktree だけを対象にし、unrelated directory には触れない。
3. startup sweep は stale path の削除だけを行い、missing bundle asset の materialize は launch 時に限定する。

## 2026-04-08 — chore: managed gwt asset cleanup must distinguish source-of-truth from generated residue

### 事象

`gwt-*` の削除整理で、「埋め込み/managed かどうか」を search embedding と混同し、
さらに `repo-tracked` の有無だけで削除可否を判断しかけた。
その結果、source-of-truth の managed asset、broken symlink の tracked residue、
ignored な generated residue が同じバケツに入っていた。

### 原因

- 判定基準を `crates/gwt-skills` の bundle/distribution 契約ではなく、名前や追跡状態に寄せてしまった。
- `.codex/skills/*` のように「tracked でも source-of-truth ではないもの」と、
  `.claude/commands/gwt-file-search.md` のように「untracked だが local residue として削除すべきもの」を分けていなかった。

### 再発防止策

1. `gwt-*` の削除前に、`assets.rs` / `distribute.rs` / `SPEC-9` / `AGENTS.md` を照合して managed contract を確定する。
2. 削除対象は `tracked stale residue` と `untracked unmanaged residue` に分け、source-of-truth と generated copy を混同しない。
3. local residue を削除する場合は、対応する tracked command/skill docs の参照先も同時に正規 surface へ揃える。

## 2026-04-08 — fix: PTY output must signal dirty state, not permission to redraw immediately

### 事象

snapshot churn、visible row O(rows²)、live render cache miss、Python index worker を潰した後も、
`gwt-tui` 本体がなお `20-30%` 台の CPU を消費し続け、`sample` では
`drain_pty_output_into_model()` と `render_session_surface()` が高頻度で積み上がっていた。

### 原因

- event loop は PTY 出力を 1 回でも drain すると、入力がなくても直ちに inner loop を抜けて `terminal.draw()` していた。
- そのため active agent pane が継続的に PTY 出力を流す状況では、描画の必要量ではなく reader wakeup 回数に引きずられて redraw していた。
- 既存の `1ms` grace slice は「入力を取り逃さない」には有効だったが、draw rate の上限にはなっておらず、実際には `~80-180fps` 級の redraw が発生していた。

### 再発防止策

1. PTY 出力は「dirty になった」合図として扱い、次の redraw は最後の draw からの最小フレーム間隔で pace する。
2. redraw 待ち時間は sleep ではなく input poll に使い、ユーザー入力で即座に抜けられるようにする。
3. `sample` で render hot path が残るときは、1 フレーム当たりのコストだけでなく、1 秒当たり何回 redraw しているかも必ず確認する。

## 2026-04-08 — fix: watcher-driven incremental indexing must preserve scope specificity

### 事象

`gwt-tui` の親プロセス自体の CPU を下げたあとも、`ps` では `gwt-tui` の子として
`chroma_index_runner.py --action index-specs --mode incremental` が code edit のたびに高 CPU で走っていた。

### 原因

- watcher batch は changed paths を持っていたが、`schedule_incremental_index()` はそれを見ずに
  `files` / `files-docs` / `specs` の 3 scope を毎回順番に起動していた。
- coalescing state も `dirty: bool` だけだったため、build 中に追加イベントが来ても
  「次に何の scope を再実行すべきか」を保持できず、狭い変更でも広い Python work に戻りやすかった。

### 再発防止策

1. watcher から runner を起動する前に changed paths を `files` / `files-docs` / `specs` へ分類し、必要 scope だけを queue する。
2. coalescing state は bool ではなく scope union を保持し、follow-up pass でも narrow rebuild を維持する。
3. `index.log` に watcher batch ごとの scope を出し、実運用で Python runner が何を起動しているかを直接確認できるようにする。

## 2026-04-08 — fix: live render paths must not rebuild the visible parser and URL scan every frame

### 事象

snapshot churn と visible row O(rows²) を止めたあとも、再起動した `gwt-tui` では
`render_session_surface()` -> `visible_screen_parser()` -> `collect_url_regions()` が hot path に残り、
CPU がなお高止まりした。

### 原因

- live render / selection copy / Ctrl+click hit testing が、それぞれ visible screen 用 parser を再構築していた。
- URL 領域も draw ごとに毎回フル画面再計算しており、画面内容が変わっていなくても
  `state_formatted` / text assembly / URL regex 走査を繰り返していた。

### 再発防止策

1. live surface を読むだけの経路では parser clone を増やさず、借用ベースの helper で同じ `vt100::Screen` を使い回す。
2. URL region のような純粋導出データは、surface が不変な間は cache し、invalidate 条件を `process` / `resize` / scrollback mode change に集中させる。
3. 1 つの hot path を消した後は `sample` を撮り直し、次の支配項が render / parsing / sidecar のどこへ移ったかを確認してから次の修正に進む。

## 2026-04-08 — fix: per-row visible line scans must not go through `contents_between()`

### 事象

snapshot churn を止めた後も、更新済み `gwt-tui` を再起動すると依然として高 CPU が残り、
`sample` では `VtState::process()` -> `ScreenSnapshot::from_screen()` ->
`screen_visible_lines()` -> `vt100::Screen::contents_between()` が hot path になっていた。

### 原因

- `screen_visible_lines()` が各 row ごとに `contents_between(row, 0, row, cols)` を呼んでいた。
- vt100 側の `contents_between()` は単一行ケースでも内部で `rows(start, width).nth(row)` を辿るため、
  visible row 全体を読むだけで O(rows²) になっていた。
- redraw-shift 判定は agent repaint ごとに走るため、この隠れた二乗コストが CPU 張り付きとして表面化した。

### 再発防止策

1. 画面全行を読む用途では selection/clipboard API を流用せず、`rows()` のような単一走査 API を使う。
2. per-frame hot path に入る helper は、呼び出し先ライブラリの計算量まで確認してから採用する。
3. プロファイルで hot path が移ったら「前のボトルネックを消せた」だけで満足せず、次の支配項まで潰す。

## 2026-04-08 — fix: live redraw comparison state must not double as user-visible snapshot history

### 事象

`AgentMemoryBacked` scrollback が row history へ正規化できているのに、
`gwt-tui` が依然として高 CPU のまま張り付き、`sample` では
`VtState::capture_snapshot()` -> `ScreenSnapshot::from_screen()` が hot path になっていた。

### 原因

- redraw shift を検出するための「最新フレーム比較用 snapshot」と、
  ユーザーが scrollback で辿る「snapshot history」を同じ `snapshots` deque で兼用していた。
- そのため `uses_snapshot_scrollback() == false` に切り替わった後も、agent pane は
  各 PTY chunk ごとに full-surface snapshot を append / dedupe し続けていた。
- `process()` 末尾の `capture_snapshot()` は、同じ更新サイクル内で一度作った `current_snapshot`
  を再利用せず、`ScreenSnapshot::from_screen()` をもう一度実行していた。

### 再発防止策

1. full-screen redraw の比較用 state と user-visible history を分離し、row history が有効な間は snapshot storage を最新 1 枚の baseline に潰す。
2. `process()` の更新サイクルで既に構築した current frame snapshot を、最終 capture に再利用して二重構築を禁止する。
3. 「row history mode では snapshot_count が 1 のまま増えない」RED テストを model に固定する。

## 2026-04-08 — fix: abandoning SGR parsing must replay buffered input in original order

### 事象

`Esc` で始まる入力が一度 SGR mouse report 候補として取り込まれたあと、
実際には mouse report ではなかったケースで、後続キーが `Esc` より先に app へ渡っていた。

### 原因

- `flush_all()` が `Esc` だけを pending queue に積み、現在キーは即 return していた。
- そのため `Esc` + `j` や `Esc` + `[` のような sequence で、downstream には元順ではなく
  `j`, `Esc` のような並びで届く経路が残っていた。
- pending queue を app へ流す経路も、polled input と同じ keybind dispatch を共有していなかった。

### 再発防止策

1. SGR parsing を abandon する分岐では `Esc` だけでなく取り込んだ prefix 全体を pending queue へ元順で replay する。
2. `input_normalizer.pop_pending()` の出力も、polled message と同じ post-normalization dispatch を必ず通す。
3. `Esc`-prefixed non-SGR fallback の RED テストで、返却順と keybind 経路の両方を固定する。

## 2026-04-08 — fix: Terminal.app right-drag fallback state should clear only when terminal ownership is lost

### 事象

Terminal.app の right-drag fallback について、drag anchor の stale state を消す修正を入れた後、
正常な `TabContent -> Terminal` focus 移動まで anchor を消してしまい、既存の right-drag scroll が壊れた。

### 原因

- cleanup 条件を「focus が変わったら消す」と広く取りすぎていた。
- right-drag 開始時は session hit により `TabContent` から `Terminal` へ focus が移るのが正常挙動であり、
  そこでは anchor を維持する必要があった。
- 本当に消すべき条件は「outside mouse-up」「active session 変更」「terminal から離れる focus change」だった。

### 再発防止策

1. input-state cleanup は「状態を持つ権利を失った時」に限定し、状態を持ち始める正常遷移では消さない。
2. right-drag fallback では inside `Down(Right)`、outside `Up(Right)`、session switch、focus switch を別々に RED テストで固定する。
3. pointer fallback 状態は pane ownership の lifecycle と一緒に考え、単純な focus diff だけで処理しない。

## 2026-04-07 — fix: prefer Codex's official inline mode over terminal-side redraw reconstruction

### 事象

Codex pane だけが実機で `max_scrollback == 0` のまま `snapshot` mode に落ち続け、
テストで redraw-shift 正規化を積み増しても、実際の PTY では page-sized scroll が残った。

### 原因

- 問題の中心は gwt の scroll math ではなく、Codex が alternate screen の full-screen redraw を出していたことだった。
- その状態で gwt が redraw 差分から line history を再構成し続けると、agent 実装に依存した推測ロジックが増え、
  実ログとテストの乖離が大きくなる。
- Codex CLI 自体が `--no-alt-screen` を提供しており、scrollback preservation は agent 側 capability として既に解かれていた。

### 再発防止策

1. agent が公式に scrollback-preserving / inline mode を持つ場合は、まずそれを使い、gwt 側の heuristic 再構築を増やさない。
2. 実ログで `max_scrollback == 0` と `mode=snapshot` が続くときは、gwt の viewport ロジックより先に agent launch mode を疑う。
3. launch config のような責務境界の解決策がある場合は、renderer / scrollback reconstruction の修正より優先する。

## 2026-04-07 — fix: Codex redraw shift detection must survive sparse overlap churn

### 事象

ループ感は消えても、Codex pane が 다시 page-sized snapshot scroll に落ち、
行単位ではなく画面単位のようなスクロールに戻るケースが残った。

### 原因

- `detect_vertical_redraw_shift()` が contiguous exact overlap を主条件にしていた。
- Codex の redraw では vertical shift 自体は残っていても、その間に progress や spinner の更新が挟まり、
  一致行が same-offset に疎に散るだけで contiguous overlap が壊れる。
- その結果、実際には line scroll 相当の shift があるのに `max_scrollback == 0` のままとなり、
  snapshot fallback の 1 step = 1 frame へ戻っていた。

### 再発防止策

1. redraw-shift 検出は contiguous overlap を優先しつつ、取れない場合は sparse same-offset match を fallback として使う。
2. scrollback に積む対象は「先頭から消えた row」であり、progress/spinner churn で overlap が分断されても page scroll へ戻さない。
3. 「sparse same-offset match しか残らない Codex redraw」RED テストを model で固定する。

## 2026-04-07 — fix: snapshot-backed history source must stay locked while the user is reviewing it

### 事象

PTY 出力が続いている最中に local snapshot history を見ていると、
scroll 中の viewport が row history へ切り替わり、同じ付近を往復するような
loop/jitter に見える状態が発生した。

### 原因

- `AgentMemoryBacked` は `max_scrollback == 0` の間だけ snapshot mode を使っていた。
- そのため、ユーザーが snapshot history を見ている最中に新しい redraw から
  row history が 1 行でも導出されると、`process()` が `snapshot_cursor` を捨てて
  表示ソースを snapshot から row history へ動的に切り替えていた。
- 問題は scroll input ではなく、「scroll 中の history source が PTY 更新で勝手に変わる」
  という state machine の破綻だった。

### 再発防止策

1. snapshot-backed history は `max_scrollback == 0` の副作用ではなく、`snapshot_cursor` が示す明示的な閲覧状態として扱う。
2. PTY 更新で新しい row history が生えても、ユーザーが live-follow に戻るまで visible source を切り替えない。
3. 「snapshot 閲覧中に row history が後から発生しても表示ソースが変わらない」RED テストを model で固定する。

## 2026-04-07 — fix: scrollbar affordance must not diverge by agent type

### 事象

Claude Code pane では scrollbar が出ず、Codex pane では出る状態が残り、
同じ terminal surface なのに agent ごとに UI affordance が食い違っていた。

### 原因

- scrollbar 表示可否が viewport history の実装詳細に結び付いており、
  PTY-owned / local scrollback / snapshot fallback の経路差がそのまま UI 差分になっていた。
- そのため scroll semantics の修正を繰り返すたびに、scrollbar overlay の有無まで agent 別に揺れていた。

### 再発防止策

1. terminal pane の visual chrome は scrollback 実装詳細から切り離し、必要なら一括で on/off できるようにする。
2. Claude Code と Codex の両方で、overflow 時でも terminal width が変わらない回帰テストを持つ。
3. scroll behavior を直す変更では「入力経路」と「視覚 affordance」を分けて検証する。

## 2026-04-07 — fix: full-viewport vt100 scrollback cannot represent partial redraw scroll regions

### 事象

Codex pane で fixed header / status を含む redraw を line scroll にしようとしても、
synthetic row を `scrollback_parser` へ流し込む実装では header 行ばかりが scrollback に押し出され、
本当に消えた本文行が戻ってこなかった。

### 原因

- `scrollback_parser` は terminal 全体の scrollback を表現するもので、部分スクロール領域を持つ redraw をそのまま再現できない。
- その状態で synthetic row を最下段へ流すと、parser が押し出すのは viewport 最上段の row であり、
  fixed header がある pane では消えた本文行ではなく header 行が履歴化される。
- つまり問題は shift 検出だけではなく、`full-viewport scrollback parser` を
  partial scroll region の history source に流用していた設計そのものだった。

### 再発防止策

1. agent pane の local line history は `vt100 scrollback` に無理やり注入せず、pane-local row cache として保持する。
2. fixed header / status churn を含む vertical shift は「どの row が本当に画面外へ出たか」を RED テストで固定する。
3. scrollback source と visible viewport source を一致させ、partial redraw の history だけ別 parser semantics に依存させない。

## 2026-04-07 — fix: redraw-shift row-history promotion must not depend on `clear+home`

### 事象

Codex pane の scroll が依然として snapshot mode のままで、
wheel / trackpad の 1 step が 1 frame になり、ページ単位のように見えた。

### 原因

- `detect_vertical_redraw_shift()` 自体は存在したが、`synthetic_scrollback_rows()` が
  `\x1b[2J\x1b[H` を含む segment に限定していた。
- そのため `home + repaint` や差分 redraw のように `clear+home` を出さない full-screen repaint では、
  snapshot 間に明確な縦シフトがあっても local row history に昇格しなかった。
- 結果として `max_scrollback == 0` が続き、Codex pane は snapshot fallback のまま page-like scroll になっていた。

### 再発防止策

1. agent pane の redraw-shift 正規化は特定の control sequence に縛らず、連続 snapshot の visible/formatted surface 差分そのものを基準にする。
2. `clear+home` ありケースだけでなく、`home + repaint` ケースの RED 回帰テストを model で固定する。
3. live debug 前には「見ている log が現行プロセスのものか」を PID と env で確認し、stale log を根拠にしない。

## 2026-04-07 — fix: snapshot-backed agent scroll を PTY keyboard input に変換してはいけない

### 事象

Codex pane で trackpad scroll すると、scroll ではなく `Up/Down` キー入力として解釈され、
行移動やカーソル移動になってしまった。

### 原因

- `snapshot-backed agent + no SGR mouse reporting` を `PTY-owned keyboard scroll` に振ったのが誤りだった。
- scroll capability を持たない pane に対して `\x1b[A` / `\x1b[B` を送っても、それは terminal scroll ではなく通常のキー入力になる。
- 本来必要だったのは PTY ownership の拡大ではなく、gwt-local memory scrollback を line-granular にすることだった。

### 再発防止策

1. PTY-owned scroll は explicit な SGR mouse reporting がある pane に限定する。
2. mouse reporting がない agent pane では、wheel / right-drag を local viewport scroll として扱い、PTTYへ cursor-key を注入しない。
3. Codex-style full-screen redraw pane では、隣接 frame の vertical shift から scrolled-off rows を導出して local row history に昇格させる。

## 2026-04-07 — fix: alternate-screen agent の local snapshot fallback は line scroll semantics を満たさない

### 事象

Claude Code は行単位で自然にスクロールするのに、
Codex は gwt の local snapshot history を 1 frame ずつ辿り、
画面単位のような不自然な scroll になった。

### 原因

- scroll ownership を `SGR mouse reporting あり -> PTY / なし -> local` の二択で見ていた。
- そのため alternate-screen 上で動く agent が mouse reporting を出さない場合、
  gwt-local snapshot fallback に落ちていた。
- snapshot scrollback の 1 step は 1 visible frame であって、embedded agent が持つ
  line-granular scroll semantics の代用品にはならない。

### 再発防止策

1. agent pane の scroll ownership は `PTY mouse / PTY keyboard / local` の 3 分類で考える。
2. alternate-screen agent が mouse reporting を出さない場合は、wheel / right-drag を repeated cursor up/down として PTY へ返す。
3. local scrollback fallback は non-alternate-screen pane に限定し、alternate-screen agent の line scroll 代替として使わない。

## 2026-04-07 — fix: alternate-screen は PTY keyboard scroll の十分条件ではあるが必要条件ではない

### 事象

alternate-screen agent 向けに PTY keyboard scroll を実装しても、
ユーザー環境の Codex は依然として local snapshot scroll に入り続け、挙動が変わらなかった。

### 原因

- 実ログでは Codex pane が `max_scrollback=0` の snapshot mode に入っていたが、
  `alternate_screen()` 前提で分岐していたため PTY keyboard path に到達しなかった。
- Codex の full-screen redraw は main-screen 上の clear+home 更新でも発生しうるため、
  `snapshot-backed かどうか` が本来の判定軸だった。

### 再発防止策

1. agent scroll routing の判定は `alternate_screen()` ではなく、まず `uses_snapshot_scrollback()` を見る。
2. 「実ログで routing がどの経路に入ったか」を残す `event=scroll_route` を先に入れて、仮説を次回すぐ検証できるようにする。
3. full-screen redraw 型 agent の再現テストは alternate-screen と non-alternate-screen の両方で固定する。

## 2026-04-07 — fix: coalesced PTY payloads can hide intermediate agent redraw frames

### 事象

Codex pane は local snapshot scrollback 経路に入っているのに、
実際には少数 frame しか遡れず「ほとんどスクロールできない」状態になった。

### 原因

- event loop は同一 session の PTY 出力 chunk を 1 payload に coalesce していた。
- `VtState::process()` はその payload 全体を処理した後に 1 回だけ snapshot を取っていたため、
  1 payload 内に複数の `clear + home` full-screen redraw があると中間 frame が履歴に残らなかった。
- Claude Code は PTY-owned scroll で回避されていたが、Codex は local snapshot path を使うため、
  この frame collapse がそのまま scrollback 欠落として見えていた。

### 再発防止策

1. agent memory-backed snapshot capture は coalesced payload 内の redraw 境界ごとに distinct frame を保持する。
2. 「1 payload 内に複数 full-screen frame がある Codex 相当ケース」の model/app 回帰テストを固定する。
3. PTY coalescing の有無だけで scrollback 深さが変わらないことを前提に設計する。

## 2026-04-07 — fix: agent scroll should defer to PTY mouse reporting when available

### 事象

agent pane の scroll を gwt 側の local scrollback で抱え続けた結果、
full-screen redraw と local snapshot/history が競合し、修正を重ねても
「途中しか遡れない」「起動画面が混ざる」「更新中に破綻する」が再発した。

### 原因

- source code 上、agent pane は「PTY 出力は agent 側が責務」という前提なのに、
  gwt が wheel / trackpad scroll まで local history として奪っていた。
- とくに SGR mouse reporting を有効化している agent では、
  本来 PTY に返すべき scroll input を gwt が消費してしまい、
  agent 自身の redraw / viewport 制御と二重管理になっていた。

### 再発防止策

1. agent pane が SGR mouse reporting を有効化している場合は、wheel / trackpad scroll を PTY へ返す。
2. gwt local scrollback は mouse-reporting 未対応 pane の fallback に限定する。
3. wheel と Terminal.app の right-drag fallback の両方で、PTY forwarding の回帰テストを固定する。

## 2026-04-07 — fix: agent type を scroll ownership の根拠にしない

### 事象

Claude Code では改善したのに、Codex を同じ remote-scroll agent とみなした瞬間、
Codex だけまったくスクロールしなくなった。

### 原因

- `Codex` という agent type だけを根拠に PTY-owned scroll 扱いへ昇格したのが誤りだった。
- scroll ownership は agent の名前ではなく、そのセッションが実際に出した
  mouse capability (`?1000h`, `?1002h`, `?1003h`, `?1006h`) に従うべきだった。
- capability を出していない pane まで remote-scroll へ振ると、wheel input が消えるだけになる。

### 再発防止策

1. scroll ownership は capability-driven に保ち、agent type 固有の特例を入れない。
2. PTY-owned scroll のときだけ local scrollbar overlay を抑止し、local-scroll pane では従来どおり local history を使う。
3. explicit mouse reporting がない pane でも alternate-screen agent なら PTY keyboard scroll を検討し、local fallback 可否を alternate-screen 状態まで含めて判断する。

## 2026-04-07 — fix: PTY-bound key input must exit history mode before forwarding

### 事象

scrollback を見ている最中にキー入力しても、
viewport が古い履歴位置に残ったままで、入力だけが PTY に送られていた。

### 原因

- scrollback の live/history 遷移はマウススクロール経路だけで管理しており、
  キー入力経路では live 復帰を行っていなかった。
- そのため row scrollback / snapshot history のどちらでも、
  「表示は過去、入力は現在の PTY」という不整合が起きえた。

### 再発防止策

1. PTY に転送するキー入力の直前で、active session が history 表示中なら `follow_live=true` に戻す。
2. row scrollback と snapshot history の両方で、キー入力後に live screen へ戻る回帰テストを固定する。

## 2026-04-07 — fix: agent memory-only scrollback still needs frame fallback when row history stays zero

### 事象

Agent pane を `memory-only normalized row scrollback` に寄せた後、
full-screen redraw 型の agent で `max_scrollback=0` のままになり、
スクロール自体ができなくなった。

### 原因

- `session log` / transcript を外したこと自体は正しかったが、
  その代わりを `row history only` に狭めすぎた。
- Claude/Codex のような full-screen redraw 型 pane では、VT 的には
  visible frame が更新されても row scrollback が増えないケースがある。
- その結果、memory-only 設計でも in-memory frame cache を併用しないと
  「PTY は正しく更新されているのに遡れない」回帰が起きた。

### 再発防止策

1. agent pane の runtime scrollback source は引き続き PTY-derived memory only に限定する。
2. ただし agent scrollback は `row-first` とし、`max_scrollback == 0` の full-screen redraw では同じ memory cache 内の snapshot/frame history を fallback に使う。
3. 「agent pane full-screen redraw + row history zero でも scroll できる」回帰テストを model/app の両方で固定する。

## 2026-04-07 — fix: agent scrollback source must stay PTY-derived and memory-only

### 事象

Agent pane の scrollback で色・装飾が消えたり、
session log 由来の plain text / 別 session / dead zone が混ざったりした。

### 原因

- gwt 側が agent runtime scrollback の正本を PTY ではなく session `jsonl` にも求めていた。
- その結果、terminal 状態ではなく transcript 再構成文字列が viewport source に混入し、
  ANSI 属性・overwrite・clear・launch redraw の terminal semantics を壊していた。
- 「復元」は agent 側の責務なのに、gwt 側が履歴補完まで担って責務境界を越えていた。

### 再発防止策

1. Agent pane の runtime scrollback source は `PTY -> vt100 -> in-memory cache` だけに限定する。
2. gwt は pane が生きている間の一時 cache と viewport 制御だけを担当し、session log を scrollback source にしない。
3. session log / transcript は必要でも別責務に隔離し、terminal viewport の描画・scrollbar・copy 経路へ混ぜない。

## 2026-04-07 — fix: snapshot frame history is not terminal scrollback for agent panes

### 事象

scroll 中に agent launch 画面や blank 画面が割り込み、
scrollbar は動いていても viewport が「別フレーム履歴」を辿っていた。

### 原因

- agent pane の recent cache を distinct full-screen snapshot の履歴として扱っていた。
- そのため clear / overwrite / launch redraw のような
  「本来は current screen を置き換えるだけの画面」が scrollback entry として残った。
- さらに transcript source も worktree 単位の最新 file を選んでおり、
  同じ worktree 上の別 session が fallback 候補に混ざる余地があった。

### 再発防止策

1. agent pane の recent scrollback は alt-screen toggle を除いた正規化 row buffer から作る。
2. full-screen snapshot は terminal-like history の代用品にしない。
3. transcript source は modified time だけでなく session start metadata で選ぶ。

## 2026-04-07 — fix: transcript fallback must collapse the recent snapshot overlap tail

### 事象

scrollbar は進むのに viewport がしばらく古い履歴へ進まず、
「スクロール開始まで遊びがある」ように見えるケースがあった。

### 原因

- local snapshot cache と transcript history を単純連結していたため、
  recent snapshot と同じ visible surface が transcript 側 tail にも残っていた。
- その結果、scrollbar position と viewport routing が duplicated recent history を
  追加で踏み、older unique history に入るまで dead zone が発生した。

### 再発防止策

1. local snapshot cache と transcript tail の overlap は visible surface 比較で検出する。
2. overlap は scrollbar metrics と transcript entry/exit cursor の両方で同じ値を使う。
3. 「snapshot overlap を飛ばして older unique transcript に入る」回帰テストを固定する。

## 2026-04-07 — fix: transcript hydration must preserve raw tool-output blocks

### 事象

Agent pane の transcript fallback に入ると、session `jsonl` 側に ANSI 付き出力が残っていても
色や装飾が消え、場合によっては scrollback 自体が空になるケースがあった。

### 原因

- transcript reader が会話メッセージ本文だけを抽出し、Codex `function_call_output.output` と
  Claude `tool_result.content` を捨てていた。
- その結果、tool 実行結果由来の scrollback 行が生成されず、ANSI 属性を持つ生データも
  viewport parser へ届かなかった。

### 再発防止策

1. Claude/Codex transcript hydration では、会話メッセージと tool-output event を別経路で抽出する。
2. ANSI を含む tool-output は role prefix 付きの plain text へ潰さず、raw line のまま保持する。
3. Codex `function_call_output` と Claude `tool_result` の styled hydration 回帰テストを固定する。

## 2026-04-07 — fix: snapshot だけでは agent 会話ログ全量の scrollback を満たせない

### 事象

スクロール入力自体は動作していても、`max_scrollback=0` + `snapshot_count` が少数固定の pane では
「全ログを遡れない」状態が継続した。

### 原因

- full-screen redraw 主体の agent pane では、distinct frame を積んでも history が数件に留まるケースがある。
- gwt 側 scrollback が PTY 可視フレーム由来の cache に限定され、agent 本体の session `jsonl` 履歴を参照していなかった。

### 再発防止策

1. `snapshot_count` と `max_scrollback` を debug log で先に確認し、「入力経路不具合」と「保持母数不足」を分離する。
2. Claude/Codex pane では session `jsonl` を in-memory transcript scrollback に同期し、viewport API（描画/scrollbar/scroll）を同一経路で扱う。
3. 「snapshot が少数でも transcript で遡れる」回帰テストを固定する。

## 2026-04-07 — fix: 画面描画とスクロール指標は同一の可視サーフェスを参照させる

### 事象

スクロールバーは動くのに、画面表示が追従しない不整合が発生した。

### 原因

- app 層に live/snapshot の分岐があり、scrollbar・描画・URL/選択コピーで
  参照サーフェスが分岐ごとにズレる余地があった。

### 再発防止策

1. viewport 操作は `VtState` に集約し、`visible_screen_parser` / `scroll_viewport_lines` / `scrollbar_metrics` を単一入口にする。
2. app 層は visible surface の取得ロジックを持たず、`VtState` API の結果だけを使う。
3. scrollbar 追従時に描画も変わることを focused test とフルテストで固定する。

## 2026-04-07 — fix: alt-screen では row scrollback ではなく snapshot を優先する

### 事象

スクロールバーは動くのに、実際の terminal 画面が切り替わらないケースがあった。

### 原因

- scroll path / scrollbar metrics が `max_scrollback > 0` を優先していた。
- main-screen の row scrollback が残っている状態で alt-screen に入ると、
  実際に見ているのは alt-screen なのに row scrollback 経路へ入り、表示と指標が乖離した。

### 再発防止策

1. `alternate_screen()` 中は snapshot-backed scrollback を優先する。
2. scroll handler と scrollbar metrics で同じ判定（snapshot優先）を共有する。
3. 「main output 後に alt-screen へ遷移しても snapshot scroll が効く」回帰テストを固定する。

## 2026-04-07 — fix: snapshot cache は viewport-shift 推定ではなく VT確定フレーム履歴で扱う

### 事象

full-screen pane で上書き・clear 後の再描画が続くと、
scrollback 復元時に「消えたはずの行」や「取りこぼしたフレーム」が発生した。

### 原因

- cache 追加条件を viewport-shift の overlap 推定に依存していた。
- そのため、TTY 的には有効な redraw でも heuristic の判定次第で
  履歴に残ったり残らなかったりして、復元一貫性が崩れた。

### 再発防止策

1. snapshot cache は「VT 解釈後の最終 visible frame」を単位にし、distinct frame は履歴へ追加する。
2. 連続同一フレームだけを dedupe し、overlap ベースの shift 推定を履歴条件に使わない。
3. `model.rs` と `app.rs` の focused test で「distinct frame 追加」「blank prefix 剪定」「live↔history遷移」を固定する。

## 2026-04-07 — fix: snapshot viewport shift 判定は「全行一致」だと実運用で取りこぼす

### 事象

full-screen pane で明らかに画面が流れているのに snapshot history が増えず、
スクロールバックがほぼ効かないケースがあった。

### 原因

- viewport-shift 判定が overlap 全行一致を必須にしていた。
- 実際の TUI はヘッダー/ステータス等が毎フレーム微妙に変わるため、
  実質的に shift していても 1 行でも差分があると history 追加が失敗していた。

### 再発防止策

1. 判定を majority contiguous-overlap 方式に変更し、部分的な行変動を許容する。
2. 「部分的行変動ありでも shift 扱い」「低類似リライトは非shift」の
   2系統テストを固定する。
3. フルスクリーン系不具合ではまず shift 検出の厳しさ（false negative）を疑う。

## 2026-04-07 — fix: SGR leak 正規化は terminal focus に依存させない

### 事象

Terminal pane 非フォーカス時に発生した SGR wheel leak が
正規化されず、スクロール開始時に literal escape 断片が混入した。

### 原因

- `InputNormalizer::normalize()` が `terminal_focused == false` のとき
  早期 return していた。
- そのため、focus handoff 前に届いたリークシーケンスが
  MouseInput 化されず通常キー入力として処理されていた。

### 再発防止策

1. SGR leak 正規化はフォーカス状態に関係なく常時適用する。
2. 非フォーカス時でも `ESC [ < ... M` が MouseInput へ変換される
   回帰テストを固定する。
3. フォーカス遷移と入力正規化の責務を分離し、正規化層は
   「入力の意味復元」に専念させる。

## 2026-04-07 — fix: SGR mouse leak のタイムアウト基準は「開始時刻」ではなく「無入力時間」

### 事象

トラックパッドスクロール時に `"[<64;...M"` のような文字列が pane に混入し、
スクロール反応も不安定になった。

### 原因

- SGR 正規化のタイムアウトを「ESC を受け取った時刻からの経過時間」で判定していた。
- 文字列全体の到着時間が閾値を超えると途中で正規化が崩れ、後続断片が通常キー入力として漏れていた。

### 再発防止策

1. タイムアウト判定を「最後の文字受信からの無入力時間」に変更する。
2. 文字間隔が許容内で総受信時間が長いケースの回帰テストを追加する。
3. `"[<...M"` 漏れを再現したら、まず timeout 基準（start vs idle）を確認する。

## 2026-04-07 — fix: live から最初の snapshot scroll で frame を飛ばさない

### 事象

snapshot-backed pane で live 状態から最初に上スクロールすると、
1段ではなく2段ぶん古いフレームへ飛ぶ挙動が発生していた。

### 原因

- `scroll_snapshot_up()` が `snapshot_cursor == None`（live）時に
  `last_past_snapshot` を基点にしてから `rows` を減算していた。
- このため `rows=1` でも `latest-2` へ移動し、オフバイワンで1フレーム飛ばしていた。

### 再発防止策

1. live 時の基点は `latest`（末尾）を使い、そこから `rows` 分だけ戻す。
2. 最初の上スクロールが `latest-1` に着地する focused test を固定する。
3. scroll position ログで `previous_position` と `next_position` の差分が `rows` に一致するかを確認する。

## 2026-04-07 — fix: snapshot 先頭の blank prefix は自動で間引く

### 事象

blank-only overlap 判定を締めても、履歴先頭に残った古い blank frame に
スクロールが当たると、最上端で空表示になるケースが残った。

### 原因

- snapshot 履歴追加条件は改善済みでも、過去に取り込まれた blank frame の先頭残留は別問題だった。
- top scroll は `snapshot_cursor == 0` に到達するため、先頭が blank のままだと空画面を描いてしまう。

### 再発防止策

1. non-blank frame が存在する場合、leading blank snapshot prefix を自動で削除する。
2. prefix 削除時は `snapshot_cursor` を `saturating_sub` で同期してカーソル破綻を防ぐ。
3. snapshot 系不具合は「append 条件」と「履歴正規化（prefix pruning）」を分けて検証する。

## 2026-04-07 — fix: viewport shift 判定を厳しくしすぎると scroll が消える

### 事象

blank-only overlap を除外するために viewport shift 判定を厳しくしたところ、
full-screen pane の snapshot history が伸びず、実質的にスクロール不能になるケースが出た。

### 原因

- viewport shift 判定を「非空白 overlap 必須」にしたことで、
  sparse な full-screen redraw（ほぼ空白 + 一部更新）の多くが history 追加対象から外れた。
- 結果として `snapshot_count` が増えず、ユーザー視点では scrollback が効かなくなった。

### 再発防止策

1. overlap-based の viewport shift 判定は維持し、history 進行を止めない。
2. 空表示対策は判定厳格化ではなく、leading blank prefix pruning で分離して解く。
3. 「scroll 不能」と「最上端空表示」を別の故障モードとして切り分けてテストする。

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

## 2026-04-07 — fix: Claude auto-mode系エラーは skip 復元抑止ではなく gwt 固有launch差分を先に疑う

### 事象

`Auto mode is unavailable for your plan` の報告に対し、Quick Start の
`skip_permissions` 復元そのものを無効化する修正を先行してしまい、実際の
症状（gwt経由のみ再現）と原因候補の切り分けがずれた。

### 原因

- 「直接 `bunx @anthropic-ai/claude-code` では再現しない」という強い事実より
  前に、保存済みフラグ復元を主因と仮定した。
- gwt固有の launch 差分（追加 env / 追加 args）の比較を先に固定しなかった。

### 再発防止策

1. CLIラッパー経由でのみ再現する不具合は、まず「直接CLIとの差分（env/args/runner）」を最優先で比較する。
2. ユーザーが「フラグ自体ではない」と明示した場合、フラグ復元ロジックを触る前に launch 組み立て層の証跡（実際の引数・環境）を確定する。
3. 既存UXを弱める回避策（復元OFFなど）は、根本原因を確定するまで入れない。

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

## 2026-04-07 — fix: PTY高頻度更新中の入力スターブでスクロール不能になる

### 事象

PTY 出力が継続している間だけ、トラックパッド/マウススクロールが効かない、または極端に遅延した。
出力が止まると同じ操作で正常にスクロールできた。

### 原因

- `run_app` のイベントループで、`drain_pty_output_into_model()` が true の場合に入力取得より先に再描画ループへ戻っていた。
- その結果、連続出力時は入力キューの処理機会が不足し、スクロールイベントが実質的にスターブしていた。

### 再発防止策

1. 出力優先ループを設計する場合でも、pending input の先頭消費を必ず先に行う。
2. PTY 出力があったフレームでは、即 break せず「短い入力猶予スライス」で入力を一度だけ取りに行く。
3. イベントループの公平性（output/input）をユニットテストで固定し、順序退行を防ぐ。

## 2026-04-07 — fix: transcript fallback が recent VT cache より先に出ると style が失われる

### 事象

Claude/Codex の scrollback で transcript が有効な状態だと、少し上にスクロールしただけで recent cache ではなく transcript 表示に入り、ANSI 色や文字装飾が消えた。

### 原因

- `VtState::scroll_viewport_lines()` が transcript 利用可能かどうかだけで transcript 経路を優先していた。
- そのため local VT/snapshot cache が十分残っていても、visible surface が plain-text transcript parser に切り替わっていた。

### 再発防止策

1. transcript は「available」と「active」を分けて扱い、スクロール経路は active 状態だけで切り替える。
2. cache-backed history と transcript fallback が共存する場合は、境界遷移の上り/下りを別ルールとしてテスト固定する。
3. style を持つ viewport source と plain-text fallback source を混在させる実装では、色属性を直接検証するテストを必ず追加する。

## 2026-04-10 — fix: 長時間の Docker 起動を quick timeout や同期 UI 経路に載せない

### 事象

Launch Agent Wizard から Docker runtime を選ぶと、image build を伴う
`docker compose up -d <service>` が 5000ms 前後で timeout し、進捗も見えないため
画面が固まったように見えた。さらに失敗後は `Docker Status: Failed` overlay が残り続けた。

### 原因

- `gwt-docker` の quick probe 用 timeout を `compose up` にも流用していた。
- Launch Wizard の Docker 前処理を UI thread 上で同期実行していたため、
  progress state を更新しても描画ループが回らなかった。
- 失敗時に Docker progress overlay と Error overlay を同時に残し、
  primary error surface が分裂していた。

### 再発防止策

1. `docker info` / `compose ps` のような probe と、`compose up` のような build/start 操作は timeout を分離する。
2. 進捗を出したい長時間処理は UI thread で同期実行せず、background worker と event queue に載せる。
3. progress overlay と error overlay が共存する flow では、失敗時にどちらを primary surface にするかを先に決め、RED テストで閉じ忘れを防ぐ。

## 2026-04-10 — fix: 長時間の外部コマンドは stage 表示だけでなく stdout/stderr の観測面を必ず用意する

### 事象

Docker launch の `BuildingImage` stage は表示されていたが、`docker compose up -d`
の stdout/stderr がどこにも出ておらず、実際に build 中なのか、失敗済みなのか、
あるいは無出力待機なのかをユーザーが判断できなかった。

### 原因

- Docker progress overlay は stage と要約メッセージしか持っておらず、
  外部コマンドの実出力を扱う経路がなかった。
- `compose up` の成功時出力は破棄され、失敗時だけ一部 stderr が detail に残る設計だった。
- そのため「overlay は生きているが実体が分からない」観測不能状態が生まれた。

### 再発防止策

1. build/start のような長時間コマンドを導入したら、overlay とは別に stdout/stderr を Logs タブとログファイルへ流す経路を最初から設計する。
2. RED テストでは「進捗 stage が見える」だけでなく「実出力が log surface に現れる」ことまで固定する。
3. 成功時出力を捨てず、少なくとも line-oriented な structured log として残す。

## 2026-04-10 — fix: bundled command の source-of-truth を stale managed asset と誤認した

### 事象

`.claude/commands/gwt-spec-brainstorm.md` が worktree から消えていたが、managed asset cleanup の残骸かもしれない前提で扱いそうになった。

### 原因

- `.claude/commands/*` は distribute 対象である一方、repo に tracked された bundled command は source-of-truth でもある。
- 「managed namespace の file」と「bundle に含まれる canonical tracked asset」を区別せずに見ていた。

### 再発防止策

1. `.claude/commands/gwt-*.md` の削除を見たら、まず `assets.rs` の埋め込み対象かを確認する。
2. stale prune の判断前に `git ls-files` と distribute/prune tests で tracked / bundled / generated の境界を確認する。
3. bundled command を守る regression test を追加してから cleanup ロジックを触る。

## 2026-04-09 — fix: Branch Cleanup の「候補判定」と「選択可否判定」を同じ関数に潰さない

### 事象

Branch Cleanup の非選択理由トーストを追加したあと、snapshot で `Computing` / `NotMerged`
行の gutter が spinner / dot ではなく `–` に退行した。

### 原因

- `is_cleanable_candidate()` を `cleanup_selection_blocked_reason()` に寄せたことで、
  「cleanup 候補として描画するか」と「今この瞬間に Space で選択可能か」の意味が混ざった。
- その結果、本来は候補のまま merge 状態を別 glyph で見せるべき `Computing` /
  `NotMerged` が、protected branch と同じ blocked 表現に潰れた。

### 再発防止策

1. UI の候補表示と destructive action の selectable 判定は別 helper に分け、意味を共有しない。
2. spinner / dot / dash のように state ごとに glyph が違う画面では、snapshot test を必ず残す。
3. 「理由を返す API」を追加したら、その API を既存 render 判定へ流用して意味が変わっていないか確認する。

## 2026-04-14 — fix: Launch Agent wizard のキー入力で Docker 状態確認を毎回走らせない

### 事象

Launch Agent wizard で `↑↓` を押すたびに反応が鈍く、Docker 対応 repo では体感遅延が大きかった。

### 原因

- `Message::Wizard` のたびに `sync_wizard_docker_status()` を無条件に呼んでいた。
- その結果、`BranchAction` や `AgentSelect` の移動のように Docker と無関係な操作でも
  `docker compose ps` 相当の確認が毎キーで走っていた。

### 再発防止策

1. wizard の post-update hook で外部 I/O を呼ぶときは、前後 state を比較して
   本当に依存 state が変わった時だけ実行する。
2. Launch Agent の入力性能を触る変更では、fake docker を使って
   「無関係な `MoveUp/MoveDown` で Docker CLI を呼ばない」回帰テストを残す。
3. opt-in trace (`GWT_INPUT_TRACE_PATH`) に wizard dispatch の所要時間と
   Docker 同期の有無を残し、入力遅延の再発時に観測だけで切り分けられるようにする。

## 2026-04-15 — SPEC フォーマットはスキルではなく CLI に持たせるべき

### 事象

gwt-spec 全体のフォーマット統一作業で、registration.md にテンプレートを定義してサブエージェントに Markdown を生成させた。後から「フォーマットは CLI 側で制御し、LLM は JSON で渡すべき」と判明。大量の手作業リフォーマットが CLI 実装後に再度必要になる。

### 原因

- フォーマット定義の責務をスキル参照ドキュメント（LLM 向けガイド）に置いた
- CLI はダムパイプ（Markdown をそのまま通す）として設計されていた
- フォーマットの一貫性を保証する仕組みがなかった

### 再発防止策

- フォーマット・バリデーション・構造化の責務は CLI（コード）に持たせる
- スキルファイルはワークフロー手順のみ記載し、出力フォーマットは CLI の `--help` に委譲する
- 大量のコンテンツ作業を始める前に、インフラ（CLI の入出力設計）を先に議論する

## 2026-04-15 — gwt issue spec create はマーカー付きファイルを期待する

### 事象

`gwt issue spec create -f <file>` で新規 SPEC を 9件作成したが、全件の spec セクションが空だった。ファイルに `<!-- artifact:spec BEGIN/END -->` マーカーがなかったため、`extract_sections()` が空を返した。

### 原因

- `spec create` は `extract_sections()` でファイルをパースし、マーカーで囲まれたセクションを抽出する
- マーカーなしのプレーン Markdown を渡すとセクションが 0 件になる
- `--edit spec` はマーカー不要（直接セクション内容を書き込む）だが、`create` は必要

### 再発防止策

- 新規 SPEC 作成は `spec create` でタイトル+ラベルだけ作成し、内容は `--edit spec` で投入する
- または `spec create` のファイル形式を把握してからマーカー付きファイルを渡す

## 2026-04-16 — `/release` コマンド実行時に Step 9.2 がスキップされた

### 事象

v9.2.0 リリース実行中に `/release` コマンドの Step 9.2（`scripts/release_issue_refs.py` を使った Closing Issue 分類）がスキップされた。その結果、Release PR の Closing Issues セクションに PR 番号が記載された。

### 原因

- release.md の Step 9.2 は正確に記載されており、scripts/release_issue_refs.py も正しく実装されている
- 根本原因は AI 実行エラー：LLM が release.md に記載された必須ステップ（`**必ず** ... 実行してください`）を読み飛ばした
- v9.2.0 では実際に影響なし：コミットに PR/Issue 参照がなかったため、Step 9.2 スキップでも自動クローズすべき Issue がなかった
- release.md は `/release` コマンドの実体であり、単なるドキュメントではないことが誤解の背景

### 再発防止策

1. `/release` コマンドの Step 9.2 は必須実行ステップであり、スキップは許されない。AI 実行中も手順を逐一確認し、省略可能なステップと必須ステップを明確に区別する
2. Step 9.2 実行結果（`ISSUE_NUMBERS` / `REFERENCE_ONLY_ISSUES` / `ISSUE_WARNINGS`）を UI で表示し、Step 5（ユーザー確認）と Step 10（PR 本文生成）での検証チェックポイントを追加する
3. Release PR の Closing Issues セクション生成時に「自動クローズ対象があるか」をユーザーに明示し、空の場合は明確に `None` と記載する
4. release.md の Step 9 冒頭に「GitHub 自動クローズは Issue のみが対象であり、PR 番号は無視される」と注釈を追加し、PR/Issue 分類の重要性を強調する
5. `tasks/memory.md` にこの教訓を記録し、同種の長時間手順コマンド設計時の参考にする

## 2026-04-20 — fix: agent auto-close は共有 status ではなく active agent ownership で絞る

### 事象

agent の正常終了で window を自動 close したい変更を入れる際、
`WindowProcessStatus::Exited` だけを条件にすると shell など他の process window まで
一緒に閉じる危険があった。あわせて、起動直後に window を閉じたとき
reader detach timeout の best-effort cleanup が stderr に
`did not exit within 500ms; detaching` を出し、失敗のように見えていた。

### 原因

- `Exited` は agent 専用ではなく、一般 terminal window も共有する process status だった。
- close 契約を window の owner ではなく共通 status enum だけで分岐すると、
  別 surface の lifecycle まで巻き込む。
- `stop_window_runtime()` は timeout 後 detach を許容する設計なのに、
  その通常回復経路を stderr に出していた。

### 再発防止策

1. process status を契機に UI surface を消す変更では、status だけでなく
   domain ownership（例: active agent session）を必ず条件に含める。
2. auto-close 系の回帰テストでは、対象の正例だけでなく
   `Error` と non-agent window の負例を必ず固定する。
3. best-effort cleanup が timeout 後 detach を許容する設計なら、
   irrecoverable failure でない限り user-visible な stderr を出さない。

## 2026-04-20 — fix: Issue 完了報告の前に branch HEAD が base branch に含まれるか確認する

### 事象

Issue #2045 で PR #2049 が merge された直後に「実装はマージ済み」「完了確認」と
コメントしたが、同じ `bugfix/issue-2045` ブランチ上の follow-up commit
`a5579c6e` は `develop` に入っていなかった。

### 原因

- PR の merge 状態と branch HEAD の反映状態を同一視した。
- `git cherry -v origin/develop HEAD` や
  `git merge-base --is-ancestor <head> origin/develop` で、
  base branch 側に未反映 commit がないか確認していなかった。
- SPEC / Issue artifact の完了更新時に、どの commit までを対象にした完了報告なのか
  固定していなかった。

### 再発防止策

1. Issue 完了コメントや「マージ済み」報告の前に、
   `git cherry -v origin/<base> HEAD` か同等確認で branch HEAD の未反映 commit を確認する。
2. 「PR が merge 済み」と「branch HEAD が base branch に含まれる」を分けて記録する。
3. 完了報告では commit hash か revision range を明示し、follow-up commit が残っていないことを確認してから
   SPEC / Issue artifact を完了扱いにする。

## 2026-04-17 — fix: clipboard fallback は focus を奪ったら必ず terminal へ戻す

### 事象

Web terminal の copy 実装で `navigator.clipboard.writeText()` が使えない環境では、
hidden `textarea` + `document.execCommand("copy")` fallback を使っていたが、copy 後に
terminal input focus が戻らず、次のキー入力が shell / agent に届かなくなった。

### 原因

- fallback 実装が clipboard 書き込み成功だけを見ており、focus ownership の回復を考慮していなかった。
- async clipboard API が使える通常経路だけを前提にして、permission-restricted WebView の
  fallback 実機セマンティクスをテストで固定していなかった。

### 再発防止策

1. hidden input / textarea を使う clipboard fallback では、cleanup 時に元の interactive surface
   へ focus を戻す処理を必須で入れる。
2. WebView の permission 差分がありうる API は、正常経路だけでなく fallback 後の focus /
   input routing 契約も埋め込みテストで固定する。
3. terminal copy UX の変更では、copy success だけでなく「直後の次キー入力が terminal へ届くか」
   を review 観点に含める。

## 2026-04-20 — research: board-reminder hook の Claude Code / Codex 両対応

### 事象

SPEC-1974 Phase 8 (US-6 / US-7) の `board-reminder` hook は、Agent の推論可視化チャネルとして Board を使う前提で、`hookSpecificOutput.additionalContext` を stdout に出力して Agent の context に inject する契約を採用した。Claude Code と Codex の hook 契約が同一であるかは事前に確定させきれず、実装時点で両対応の実測検証は E2E (T-086) に委ねている。

### 原因

- Claude Code の `additionalContext` 挙動は既存 managed hooks (`runtime-state`, `coordination-event`) と同じ宛先に書き込むため、stdout JSON 形式であれば取り込まれることが確認されている。
- Codex 側の `hooks.json` 契約は SPEC-1935 が owner として配信経路を管理しているが、`additionalContext` を Agent の context に inject する厳密な形式は runtime バージョン差で変わり得るため、実測で見るまで断定できない。
- `board-reminder` の実装では、Claude Code が確実にサポートする `{"hookSpecificOutput": {"hookEventName": "...", "additionalContext": "..."}}` を単一出力形式として採用した。Codex で異なる取り込み方が必要になった場合は、SPEC-1935 の配信層で output を re-wrap する層を足す判断になる。

### 再発防止策

1. Hook の stdout JSON は、Claude Code の `hookSpecificOutput.additionalContext` をデフォルト契約として採用する。これは Claude Code / Codex の両方で「unknown JSON は stdout dump として扱う」フェイルオープン動作に助けられ、最悪でも context に生 JSON が露出するのみで hook の suppression は起きない。
2. Codex 側で異なる形式が必要な場合は、hook 本体を触らず、SPEC-1935 の配信層で runtime 別の adapter を挟む。Codex runtime の実動作は E2E (T-086) で実測するまで確定扱いにしない。
3. 実機で Codex が `additionalContext` を Agent context に取り込まないと判明した場合、本 memory を更新し、adapter 層の責務を `runtime-state` / `coordination-event` と同じ層で取り扱うことを SPEC-1935 に戻す。

## 2026-04-21 — fix: Python/Rust の gwt home 解決は同じ環境変数優先順にする

### 事象

Project index の manual quickstart で、Python runner は `HOME` 注入先の `.gwt/index` を見て self-heal した一方、Rust bootstrap は別の home を見て legacy `worktrees/<wt>/specs` を cleanup できなかった。

### 原因

- Python runner は `HOME`、`USERPROFILE`、`Path.home()` の順で index root を解決していた。
- Rust 側の `gwt_home()` は `dirs::home_dir()` 固定で、テストや manual verification の `HOME` 注入と一致していなかった。
- unit test では index root を直接注入していたため、production path の home 解決差異が隠れていた。

### 再発防止策

1. Python と Rust の両方が同じ on-disk layout を読む場合、`HOME` / `USERPROFILE` / fallback の優先順を一致させる。
2. runtime helper を manual verification する場合、fresh `HOME` では managed venv 作成に入るため、既存 runtime を使う通常 HOME と temp index subtree の cleanup も確認する。
3. index root を注入する unit test だけでなく、public path resolver の環境変数 contract も regression test で固定する。

## 2026-04-24 — fix: SPEC section 更新の PowerShell join は必ず全文 readback で検証する

### 事象

Project index hardening の SPEC #1939 更新時、PowerShell で既存 section に追記するつもりの式が
`($tasks -join "\n" + $append)` になり、演算子優先順位により各行の間へ追記文が挿入された。直後に
`gwt issue spec <n> --section tasks` を readback して検知し、plan/tasks を全文置換して復旧した。

### 原因

- PowerShell の `-join` と `+` を同じ式で使う際の結合範囲を固定していなかった。
- section 更新後の readback を実施したため検知できたが、更新式自体は構造化されていなかった。
- SPEC body は markdown section parser に依存するため、1 行の結合ミスでも別 section への parse error に波及する。

### 再発防止策

1. `gwt issue spec ... --edit` に渡す markdown は、差分追記よりも一時ファイルの全文生成を優先する。
2. PowerShell で配列結合と文字列連結を混ぜる場合は、`(($lines -join "`n") + $append)` のように結合結果を明示的に括る。
3. SPEC section を更新した直後は、必ず対象 section を readback し、見出し構造と末尾の expected lines を確認してから次へ進む。

## 2026-04-24 — fix: detached CLI の監査ログは非同期 tracing guard に依存しない

### 事象

Project index の手動 `gwt index status` / `gwt index rebuild` ログを正規の
`gwt.log.YYYY-MM-DD` へ統合する際、GUI 起動後の tracing subscriber だけを前提にすると
detached CLI 経路ではログが残らない設計になることを確認した。

### 原因

- `runtime_support::run_cli()` は CLI dispatch 後に `std::process::exit` する。
- `std::process::exit` は通常の drop を走らせないため、`tracing_appender` の
  non-blocking `WorkerGuard` flush に依存するとイベントが失われる。
- `gwt index` は GUI 起動前に処理される detached CLI なので、GUI 用 logging init の対象外だった。

### 再発防止策

1. `std::process::exit` を通る短命 CLI の必須監査ログは、同期 write + flush の JSONL append にする。
2. 既存ログ基盤へ統合する場合も、ファイル名は `current_log_file()` に揃え、別名ログファイルを増やさない。
3. CLI ログ追加のテストでは、出力先が `gwt.log.YYYY-MM-DD` であることと、旧 `index.log` を作らないことを同時に固定する。

## 2026-04-24 — process: 実装前に関連 SPEC を必ず gwt-search で確認する

### 事象

Codex 対応モデル一覧の更新で、関連 SPEC が存在するにもかかわらず初動で SPEC
確認を省略し、ユーザーから「関連SPECがないですか？あるはずです。」と指摘された。

### 原因

- 変更が小さく見えたため、`feat` 相当の実装前ワークフローを軽視した。
- `gwt-search` による SPEC / Issue / project file の横断検索を、実装前の必須ゲートとして扱わなかった。
- SPEC owner が見つかる前に実装計画を進めようとしたため、受け入れ条件と実装対象の同期が遅れた。

### 再発防止策

1. `feat` / `fix` / `refactor` の実装前は、変更規模に関わらず `gwt-search` で関連 SPEC / Issue を先に確認する。
2. ユーザーが「SPEC があるはず」と示唆した場合は、実装を止めて SPEC owner を特定し、SPEC / plan / tasks を更新してから再開する。
3. 完了報告前に、対象 SPEC が今回の変更と受け入れ条件を反映していることをセルフチェックする。

## 2026-04-25 — fix(agent): runtime hook の session 相関は gwt_session_id を優先する

### 事象

エージェント終了時に、以前の修正で auto-close するはずの Agent window が閉じずに残った。
PTY の `Completed(0)` 経路では閉じるが、runtime hook の `status=Stopped/Exited` 経路では
`RuntimeHookEvent` の broadcast だけで workspace から window が削除されなかった。

### 原因

- `handle_runtime_hook_event` は hook state を `window_hook_states` に反映して status badge を更新するだけで、
  `should_auto_close_agent_window` / `close_window_from_workspace` を通していなかった。
- `active_window_for_runtime_event` が `agent_session_id` を gwt 管理 session ID と比較していた。
  実際の hook payload では `gwt_session_id` が gwt session、`agent_session_id` が上流 agent session なので、
  active window との相関に失敗しうる状態だった。

### 再発防止策

1. Runtime hook event を active agent window に結び付けるときは `gwt_session_id` を優先し、
   互換 fallback としてのみ `agent_session_id` を使う。
2. Agent の正常終了を扱う経路は、PTY status と runtime hook status の両方で
   `should_auto_close_agent_window` と workspace close helper を通す。
3. hook event の regression test では、`gwt_session_id != agent_session_id` の実 payload 形状を使い、
   status badge だけでなく workspace から window が消えることまで固定する。

## 2026-04-26 — fix(ui): GUI/TUI イベントループで `gh` / Python runner を同期実行しない

### 事象

SPEC / Issue ウィンドウを開く、または検索文字を入力すると GUI が固まる UX になっていた。

### 原因

- Knowledge Bridge の初期ロードが stale cache refresh 経由で `gh issue list/view` を同期実行していた。
- セマンティック検索が Python runner / ChromaDB をフロントエンドイベント処理中に同期実行していた。
- 既存の検索結果を保持したまま「検索中」を表示し、追加入力を最新クエリへ畳み込む契約テストが不足していた。

### 再発防止策

1. GUI/TUI のユーザー入力経路では、外部プロセス・ネットワーク・重い runner を必ず background dispatch に逃がす。
2. ウィンドウ open / scope switch / detail select はローカルキャッシュ読み取りだけで即時応答し、refresh は別応答で反映する。
3. セマンティック検索は in-flight 1 件に制限し、追加入力は最新クエリへ coalesce する契約テストを維持する。

## 2026-04-27 — test: `HOME` / `USERPROFILE` 差し替えテストは gwt home 利用テストも同じロックで守る

### 事象

`cargo test -p gwt-core -p gwt` の既定並列実行で、`app_runtime` の Knowledge Bridge / Memo / Board 系テストが
一時的に別テストの `HOME` / `USERPROFILE` を参照し、cache / notes / coordination の保存先がずれて失敗した。

### 原因

- 一部テストは `HOME` / `USERPROFILE` を `ScopedEnvVar` で差し替えていたが、同じ `~/.gwt` 系 path を読むだけのテストは
  `env_test_lock` を取得していなかった。
- `gwt_core::paths::gwt_cache_dir()` / `gwt_notes_dir()` / project coordination path は process-global env に依存するため、
  読み取り側も env mutation と同じ排他範囲に入れる必要があった。

### 再発防止策

1. `~/.gwt` 派生 path を使うテストは、env を変更しない場合でも `env_test_lock` を取得する。
2. lock を取得した後に `HOME` / `USERPROFILE` をテスト専用 temp dir へ固定し、guard は lock より先に drop される順序で宣言する。
3. `cargo test -p gwt-core -p gwt` は既定並列実行でも確認し、`--test-threads=1` だけを成功条件にしない。

## 2026-05-04 — Phase 0 setup pattern for design-system SPEC

### 事象

SPEC-2356 (Operator Design System) の Phase 0 Setup を実行する際、 フォント取得と
Playwright 導入を一気にやろうとして、 GitHub の repo 構造が想定と違って何度か
リダイレクトを踏んだ:

- `github/mona-sans` の Variable WOFF2 は `fonts/webfonts/variable/MonaSansVF[wdth,wght].woff2` (URL エンコード必要)
- `github/hubot-sans` には Variable WOFF2 が存在しない (TTF のみ) → Bold + Condensed-Bold + Regular の static WOFF2 で代替
- `JetBrains/JetBrainsMono` の Variable WOFF2 は `fonts/webfonts/JetBrainsMono[wght].woff2`

### 再発防止策

- フォント追加が要件にある SPEC では、 「実際にダウンロードできる URL を確認してから tasks に書く」を Phase 0 の最初に行う。 推測 URL のまま tasks を確定させない。
- Variable font を要件にした場合、 該当 family が WOFF2 で variable を配布しているかを最初に確認。 配布していない場合、 static weight の組合せで代替する案を early に提案。
- フォントは OFL ライセンス本文も同階層に置く (リポジトリ自身の LICENSE / OFL.txt をそのままコピー)。
- WOFF2 → TTF 変換ツール (`woff2_compress`) は CI / dev 環境に存在する保証がない。 コンバージョンを工程に組み込まない。

## 2026-05-04 — フロントテストランナー: Node native + Playwright の二段構え

### 事象

SPEC-2356 で frontend の自動検証を増やす際、 既存は `node --test`
(smoke) のみだった。 設計上、 contrast / theme manager / hotkey は
ロジック単体で `node --test` に載るが、 visual regression は
Playwright が必須。

### 再発防止策

- 新しいフロントテストは「ロジック単体は `node --test`、 描画 / インタラクションは Playwright」に二分する。 ブラウザ起動が必要かをロジックレベルで判定する。
- `package.json` には `test:frontend-unit` (Node native) と `test:visual` (Playwright) を分離して定義する。 CI も別ジョブで実行し、 失敗箇所が一目で分かるようにする。

## 2026-05-06 — 楽観的 UI の rollback は pre-mutation snapshot で即時に行う

### 事象

SPEC-2017 Phase 2 で Kanban D&D を実装した際、最初の design では
drop 時にサーバー応答を待ってから DOM 更新する pessimistic UI を
検討していたが、レビューで「drop した瞬間にカードがビジュアルで動か
ないと操作が成立したか不明」と指摘されて楽観的 UI に転換した。
最初の楽観的 UI 実装では「失敗時は再度 load_knowledge_bridge を呼ぶ」
という rollback 戦略を取ったが、これだと再取得の非同期応答が来るまで
楽観的に動かしたカードが target column に留まり続け、矛盾した
状態が続いてしまう。

### 原因

楽観的 UI における rollback は、サーバー応答待ちの非同期性ではなく、
クライアント側の「pre-mutation snapshot」で即時に行うべきだった。
最初の設計は「rollback = サーバーから再ダウンロード」という
pessimistic fallback に引きずられていた。

### 再発防止策

- 楽観的 UI を実装するときは、drop / submit 時に必ず pre-mutation
  snapshot を取り、エラー応答時はそのスナップショットから即時
  rollback する。サーバー再取得は最後の手段。
- `dndSnapshot` のような state スロットを設計初期に確保し、UI 操作と
  非同期通信の境界を明確にする。
- WebSocket protocol の error response には、UI rollback に必要な情報
  （request_id / target / error message）を必ず含め、フロントが
  サーバー再取得に依存しなくて済むようにする。

## 2026-05-06 — Drawer 化のときは右ペインを残置せず Drawer に詳細を寄せる

### 事象

SPEC-2017 Phase 1 では Knowledge Bridge を Kanban に置換しつつ、
既存の右ペイン (`.knowledge-detail-pane`) は残したまま着地させた。
Phase 3 で Drawer を導入する際、最初は右ペインも Drawer も両方
表示する案を検討したが、レイアウト的に Kanban Board の横幅を著しく
圧迫し、6 カラムが画面に収まらなくなる問題が出た。

### 原因

「既存 UI を残置すれば破壊的変更を最小化できる」という保守的な
考えに引きずられたが、Kanban のような横幅を要求する surface では
右ペインの占有を完全に取り除いた方が UX 的にも実装的にも素直
だった。SPEC-2356 で確立済の Drawer pattern (`.op-drawer`) は
右からスライドする一時的な surface なので、Kanban の横幅を奪わない。

### 再発防止策

- 横幅制約のある surface (Kanban / Timeline / Wide Table 等) で詳細
  ペインを併存させる場合、必ず Drawer / Slide-over パターンを採用
  する。永続ペインは縦長 surface (List / Tree / Form) でのみ正当化
  される。
- 既存 UI を残置するか Drawer に寄せるかの判断は、機能の差分よりも
  「surface の横幅要件」を起点に行う。

## 2026-05-07 — fix(ui): Active Work は保存 projection だけで actionable にしない

### 事象

Agent が残っていない状態でも Sidebar の Active Work が残り、Focus が有効に見える状態になった。
また、Board が履歴ログである一方、現在の Workspace 状態と LLM 要約をどこに保存するかが仕様と実装で十分に固定されていなかった。

### 原因

- Active Work rendering が保存済み Workspace projection を信用しすぎ、live Agent window / session の存在確認と lifecycle cleanup を十分に挟んでいなかった。
- 「Board は共有ログ」「Workspace は current projection + summary journal」という責務境界を受け入れ条件として固定していなかった。
- no-Agent 状態で Active Work セクションを残して Quick Start を出す旧仕様が、ユーザーの期待する「Active Work 自体を消す」挙動と食い違っていた。

### 再発防止策

1. Active Work の Focus / Agent card は、保存 projection ではなく live window/session と照合してから表示する。
2. Agent 停止・window close 経路では保存済み Workspace projection から該当 Agent を除去し、0 件なら idle/no active work に戻す。
3. Board は履歴・共有・報告に使い、現在状態と LLM 要約は Workspace current projection と summary journal に保存する契約を SPEC とテストに残す。
4. no-Agent 状態では Active Work セクションを非表示にし、Quick Start は既存の Quick section / Start Work entrypoint に任せる。

## 2026-05-07 — 新規 frontend root module は bundle syntax check に必ず追加する

### 事象

Board UI の helper を `board-surface.js` に分離した際、初回実装では
embedded serving と unit test は追加したが、`package.json` の
`test:frontend-bundle` syntax check 対象に新規 root module を含めて
いなかった。

### 原因

既存の embedded import contract test は root module の配信漏れを検出
できる一方、`node --check` 対象は `package.json` の明示リストであり、
新規ファイル追加時に自動で追随しない。配信検証と syntax check の
責務が別であることを差分レビュー時に最初から確認していなかった。

### 再発防止策

1. `app.js` から import する root-level frontend module を追加したら、
   `embedded_web` の配信テストと `package.json` の `test:frontend-bundle`
   の両方に追加する。
2. frontend module 分割の差分レビューでは、`rg "from \"/.*\\.js\""`
   で root import を確認し、配信・syntax check・unit test の3点が
   揃っているかを見る。

## 2026-05-10 — Agent カウントは `windowMap` 走査ではなく preset で判定する

### 事象

Sidebar Layers の Agents 行が、実 Agent pane 数 (2) ではなく
全 workspace window 数 (Agent 2 + Board / Workspace 等 2 = 4) を
表示していた。`recomputeOperatorTelemetry()` が `windowMap.values()`
を走査し、`data-agent-state` を持つ window をすべて `counts.agents`
に加算していた。

### 原因

`data-agent-state` は CSS overlay / animation 用に **全 window** へ
打たれる DOM marker であり、agent 種別を表すものではない。Agent 判定の
真実源は `presetSupportsWaitingStatus(preset)` (`agent | claude | codex`)
だが、`recomputeOperatorTelemetry` ではこの述語を呼ばずに DOM の
data 属性だけで判定していた。同じく `activeWorkProjection` 由来の
`Math.max(counts.agents, activeAgents + blockedAgents)` も二重計上の
温床になり得る。

### 再発防止策

1. `windowMap.values()` / `.entries()` を走査して "agent らしさ" を
   集計する箇所では、必ず `presetSupportsWaitingStatus(preset)` を経由
   する。`workspaceWindowById(windowId)` で `windowData.preset` を解決
   してから判定する。
2. `data-agent-state` は CSS / overlay 用途であり、agent 判定の
   primary signal として使わない。差分レビューでは
   `rg "dataset.agentState" crates/gwt/web` で利用箇所を確認する。
3. Sidebar / Status Strip / Mission Briefing が共有する集計関数は、
   regression を `operator-chrome-structure` 等の source-level
   assertion で固定し、preset filter の脱落を CI で拾う。

## 2026-05-10 — Git read-only 判定は subcommand 名だけで許可しない

### 事象

title-summary 未設定時の read-only exploration allowlist が `git config` と
`git remote` を subcommand 名だけで許可していたため、`git config user.name ...`
や `git remote add ...` のような変更系 command が title-summary gate を通過した。

### 原因

Git の subcommand は同じ名前でも読み取りと変更の両方を持つものがあるが、
`is_read_only_git_subcommand` が引数を見ずに `config` / `remote` / `branch`
全体を read-only として扱っていた。allowlist の単位が粗く、guard の目的
である「作業開始前の変更を止める」契約と一致していなかった。

### 再発防止策

1. Hook guard の read-only 判定では command 名だけでなく引数まで見る。
2. `git config` / `git remote` / `git branch` のように読み書きが混在する
   subcommand は、明示的な読み取り形式だけを allowlist する。ただし
   `git branch --contains <commit>` や `git branch --list <pattern>` のように
   読み取り flag が positional value を取る形を「裸の引数」と誤判定しない。
   short flag でも `-l` / `-i` のような read-only alias を漏らさない。
   ただし `--no-list` のように read-only mode を解除する flag は、単純な
   valueless read flag として扱わず、`-l` / `--list` で立った list mode を
   明示的に解除する state transition として検証する。解除後の positional
   value は作成対象へ戻るため block する。Git option は後方に置かれても
   先行 positional operand の解釈を変えるため、positional value は見つけた
   時点ではなく最終的な list mode で判定する。
3. allowlist を広げる場合は、読み取り positive test と変更 blocking test
   を必ず対で追加する。

## 2026-05-10 — GitHub コメント本文を shell 引数に直書きしない

### 事象

`gh issue close --comment "..."` の本文に Markdown backtick を含めたため、
zsh が command substitution として解釈し、本文中の `target/debug/gwt` や
`cargo test ...` などを余計に実行した。Issue は閉じられたが、初回コメント
本文にコマンド出力が混入し、後から GitHub API で修正する必要が出た。

### 原因

Markdown のコード表記を shell の double-quoted argument に直接入れた。
GitHub コメント本文は shell 評価を通すべきではないデータだが、本文作成と
CLI 実行を同じ quoting context に混ぜてしまった。

### 再発防止策

1. GitHub Issue / PR コメント本文に backtick、`$`、改行、ANSI 出力などを
   含む場合は、`--body-file` または `gh api --input -` を使う。
2. 一時本文ファイルを作る場合は repo 外 (`/tmp`) か既存の local task file
   を使い、Markdown 本文を shell double quote に直接埋め込まない。
3. `gh issue close --comment` を使う場合でも、本文は単一引用符で安全に
   表現できる短文だけに限定する。複数行の検証ログは別途ファイル入力にする。

## 2026-05-10 — 自動更新クリック後の silent failure を 5 回連続で見逃した

### 事象

`自動更新のクリックの対応` を 5 回連続で fix したが、ユーザーから「全く対応できていません」と
強い不満を受けた。修正対象はすべて click 前 (cache / DOM / 表示) で、click 後の silent failure
path は untouched だった。

- 76be413f: 5分ポーリング + 永続更新ボタン (UI 表示のみ)
- 293a1627 / 04d721cc / ca2b8221: toast → CTA 統一 (UI 表示のみ)
- ffe40f46 / c5348421: asset wiring / ラベル文言 (UI 表示のみ)
- 3a2e0628: dismiss button (UI 表示のみ)
- 665dea8e: WebView cache 無効化 (click 検知のみ)

### 原因

1. `apply_update_state_and_exit` (`crates/gwt/src/update_front_door.rs:277`) が
   `std::process::exit(0)` で親プロセスを click 直後に殺す設計のため、
   helper subprocess の失敗が UI に surface されない silent failure path だった。
2. テストが pass していたため done 宣言を繰り返したが、実環境では helper の失敗が見えないため
   UX としては破綻していた。CI green = done 宣言が untested path を覆い隠した。
3. 「click 検知が動かない」という assumption に基づいて修正対象を選んだが、根本的には
   「click は反応しているが post-click が silent」という別問題だった。
4. ユーザー意図 UX (modal で進捗 → 完了確認 → restart) を確認せず、SPEC は click 前領域の
   みを規定していた。post-click UX が SPEC に書かれていない feature を「直す」ことができていなかった。

### 再発防止策

1. **CI green = done 禁止**: バグ修正・新機能の完了宣言には Gate 3 manual smoke
   (実機で実 user flow を 1 往復) を必須化する。SPEC-2041 Phase 14 (FR-066) で正式化済み。
2. **silent failure path の禁止**: 親プロセスが即時 exit する設計は許容しない。
   download / install / replace / spawn の各 stage を必ず frontend (UI) に surface する。
   `*_and_exit` という関数名が出てきた時点で silent failure を疑う。
3. **修正前の前提検証**: 「動かない」報告を受けたら、表面的な仮説 (cache / DOM) で fix を
   始める前に、ユーザーが実際に何を見ているか・どこで止まっているかを最低 1 度確認する。
   「クリック反応するが画面出ない」と「クリック自体反応しない」は別問題。
4. **再発カウントによる切り替え**: 同じ feature を 2 回以上 fix している場合、表面的修正
   でなく architecture / silent failure path の review に切り替える。fix を重ねる前に
   「過去の fix が効かなかった理由は何か」を root cause として specifically 特定する。
5. **post-action UX の SPEC 義務化**: button / link / action を SPEC に書く際は post-action
   UX (進捗・成功・失敗・確認) を必ず受け入れシナリオに含める。click 前後を分離して片方しか
   書かない SPEC を禁止する。

## 2026-05-11 — Agent pane heading が `workspace update` 単独では更新されない (SPEC-2359 US-26 / Phase U-1..U-3)

### 事象

ユーザーから「実行中のエージェントが何をしているのか、一目でタイトルで分かるべき」という UX 観点で
指摘を受け、本 session で `gwtd workspace update --agent-session <id> --title-summary "X"` を
発行しても、Web UI の Agent pane heading は `agent_id` フォールバック ("CLAUDE CODE") のままで
あることを再現確認した。projection JSON (`~/.gwt/projects/<hash>/workspace/current.json`) には
`agents[<i>].title_summary = "X"` が確かに書かれていた。Codex も別 session で同 root cause を
Board request `2b256bfc-...` で報告していた。

### 原因

1. **同じ UI 真実が 2 つの broadcast channel に分かれていた**: 
   - Active work card / Workspace Kanban: `BackendEvent::ActiveWorkProjection` で受信。
     `projection.agents[<i>].title_summary` を直接参照する。
   - Agent pane heading (`windowDisplayTitle`): `BackendEvent::WorkspaceState` で受信。
     `windowData.dynamic_title` を参照する。`dynamic_title` が空だと `agent_id` まで
     フォールバック→ CSS uppercase で "CLAUDE CODE" 表示。
2. **caller によって到達する broadcast surface が異なっていた**: 
   - `gwtd workspace update --title-summary` 経路 (`handle_workspace_projection_changed_events`)
     は `sync_agent_window_titles_from_workspace_projection` で in-memory `dynamic_title` を
     更新するが、`ActiveWorkProjection` しか broadcast しなかった。`WorkspaceState` 不在の
     ため、frontend の `windowData.dynamic_title` は次の hook event / window 構造変更まで
     古いまま。Pane heading は fallback に張り付く。
   - Board flow は `update_agent_window_dynamic_title_for_board_entry` で別経路に in-memory
     書き込みする。やはり `WorkspaceState` 不在。
3. **`active_agent_sessions` 未登録 session が静かに無視されていた**: launch flow が
   track していない session (GUI 再起動後 / 外部から `claude` 直接起動など) は projection に
   は載っていても、`sync_agent_window_titles_from_workspace_projection` の filter_map が
   None を返し、pane heading 更新を silently drop していた。

### 再発防止策

1. **複数 surface に reach する write は canonical orchestration API に集約する**: 
   SPEC-2359 US-26 で `apply_workspace_projection_title_sync` を追加し、5 surface
   (projection JSON / journal / in-memory `dynamic_title` / `ActiveWorkProjection` /
   `WorkspaceState`) を 1 関数で同期する契約にした。新規 caller は必ずこの API を経由する。
2. **partial-write を構造的に不可能にする**: orchestration API が 5 surface を 1 batch で
   触るので、caller は 4 + 1 / 3 + 2 の組合せを書けない。「うっかり broadcast を一つ
   忘れる」が type-system + test level でブロックされる。
3. **fallback resolution を許す**: `active_agent_sessions` 未登録でも `projection.agents[<i>]`
   が `window_id` / `worktree_path` を持っているなら、orchestration API はそちらをフォール
   バックして in-memory `dynamic_title` を更新する。Launch lifecycle (US-24) には触らず、
   title sync の resolution だけ補強する。
4. **CI green = done を疑え (再掲)**: 既存テストは「`ActiveWorkProjection` が出る」しか
   assert していなかったため、`WorkspaceState` 欠落は test 上見えなかった。新規テスト
   `apply_workspace_projection_title_sync_emits_workspace_state_when_dynamic_title_changed` と
   `handle_workspace_projection_changed_events_broadcasts_workspace_state_for_pane_heading` で
   構造的に固定。「frontend がこの broadcast を読んでいる」surface を素 grep で確認してから
   テスト assertion を組む。
5. **UI surface インベントリを書く**: 同じ data field を読む UI 面が複数あるなら、どの
   broadcast event で reach するかを表形式で残す (SPEC 7.2 の `Inventory: write path vs
   surface` 表)。Inventory を見ながら write path を設計すれば「すべての surface を
   touch しているか」を caller 側で必ず確認できる。

## 2026-05-11 — Test-only HOME mutation は crate-wide lock に統一する

### 事象

`cargo test -p gwt-core -p gwt` の並列実行で Board/Workspace 系テストが intermittent に失敗した。
単独実行では通り、失敗時は `GWT_SESSION_ID` から保存済み session を読めず、current workspace
audience が欠落していた。

### 原因

`app_runtime` test module と一部 helper test が `HOME` / `USERPROFILE` を module-local lock
または lock なしで変更していた。CLI Board/Workspace tests は crate-wide `env_test_lock` を
使っていたため、同じ process 内で別 test が HOME を差し替え、session path resolution が
別 tempdir を参照した。

### 再発防止策

1. `HOME` / `USERPROFILE` / `GWT_SESSION_ID` など process-global env を触る test は、
   module-local lock を作らず crate-wide `env_test_lock` に寄せる。
2. 単独 test green で満足せず、env mutation を伴う修正後は default parallelism の
   `cargo test -p gwt-core -p gwt` を必ず 1 回通す。

## 2026-05-11 — Built-in AgentId を consumer 側の個別表で持たない

### 事象

develop 取り込み後、Board origin focus の `agent_option_matches_session` が `AgentId::OpenClaw`
と `AgentId::Hermes` を網羅しておらず、`cargo test -p gwt-core -p gwt` が compile error で
停止した。

### 原因

`gwt-agent` は built-in agent descriptor (`command`, `display_name`, `color` など) を持っているが、
Board 側で `AgentId` variant ごとの `option.id` 対応表を別途 match していた。新しい built-in
agent が追加されたとき、descriptor は更新されても Board 側の表が同期されなかった。

### 再発防止策

1. built-in `AgentId` の表示名・command・色・cache key は `AgentId` / descriptor API を使い、
   consumer 側に variant 別の重複表を作らない。
2. custom agent だけは `custom_agent.id` の互換判定が必要なので、built-in と custom の差分だけを
   consumer 側に残す。
3. 新規 AgentId 追加後は、Agent 起動だけでなく Board / Workspace / UI projection の resume 経路も
   focused test で通す。

## 2026-05-13 — xterm fitAddon.fit() は cell.width=0 で silent no-op になる (SPEC-2008 Phase 26)

### 事象

SPEC-2008 Phase 24 (canRefreshTerminalViewport の `.hidden` skip + tab activation での
scheduleTerminalFocusActivation) を入れた後も、ユーザーから「Claude Code 起動直後の表示崩れ」「tab
切替後の wheel scroll dead-zone (resize で復活)」「Windows 不安定」が継続報告された。3 並列で
Explore 調査をかけたところ、3 症状すべてが「xterm の cell metrics と PTY cols/rows と viewport
state を、起動・visibility 遷移・resize の各 race window で確定する単一権威経路が存在しない」
構造欠陥に帰結することが判明した。

最も性質の悪い path は xterm.js `addon-fit.mjs` の `proposeDimensions()` 内の以下分岐:

```js
if (e.css.cell.width === 0 || e.css.cell.height === 0) return; // undefined
```

`_renderService.dimensions.css.cell.width` は xterm が一度も描画していない間は 0 のまま。
display:none → display:block 直後や terminal.open() 直後はまさにこの状態で、fitAddon.fit()
が **無音で no-op** する。Phase 24 で fit を呼ぶ経路を増やしてもこの no-op は埋まらないため、
viewport が pre-hidden の cols/rows のまま固定され続け、OS host resize で fan-out が走った
ときだけ復活する症状になる。

### 原因

xterm.js は描画される前は cell metrics を提供しない。fit はその metrics を必要とする。よって
「render してから fit する」順序を強制しない実装はすべて silent no-op を生む。Phase 24 は
visibility 遷移時に `fitTerminal → scheduleTerminalViewportRefresh` の順で呼んでいたが、
refresh は rAF で 1 frame 後に走るため、fit の時点ではまだ render が走っていない。

加えて `createTerminalRuntime` の初期化順序が `terminal.open() → rAF(fit) → 同期 replay`
だったため、pending snapshot / pending output は xterm default 80×24 grid に layout-locked
されたまま書き込まれ、後続の fit で grid 次元が変わっても scrollback は再 parse されない
race も存在した。

### 再発防止策

1. **xterm の fit を呼ぶ前に必ず `terminal.refresh(0, terminal.rows - 1)` で強制 render
   する**。中央集権化された helper `runTerminalActivationSequence` (crates/gwt/web/
   terminal-viewport-reflow.js) に refresh → flush layout → fit → sendGeometry → focus の
   順序を閉じ込めた (SPEC-2008 Phase 26.B / FR-056)。activation 経路と snapshot replay 経路の
   両方をこの helper 経由にする。
2. **`createTerminalRuntime` は initial-fit handshake (`isReady` / `deferredWrites`) を持ち、
   activation が完了するまで write を buffer する** (SPEC-2008 Phase 26.A / FR-057)。これに
   より起動直後の Claude Code バイトが default 80×24 grid に固定される race を排除する。
3. **terminal lifecycle に手を入れるときは「resize で復活する症状」のシグナルを refactor
   が壊していないか、Rust source-string contract test + Jest behaviour test の両方で
   pin する**。Phase 24 は contract test を「fit が呼ばれる」までしか pin していなかった
   ため、silent no-op の regression を許してしまった。`runTerminalActivationSequence` の
   refresh-before-fit ordering を behaviour test に含めることで、ordering 変更が即座に CI
   で炎上するようにした。

### 関連 PR / Issue

- PR #2693 (Phase 26.B / 26.A / 26.C-1 一括)
- SPEC-2008 (Phase 26 spec/plan/tasks に FR-056〜FR-059、T-199〜T-220、SC-035〜SC-038)
- closed regressions: #2096 / #2091 / #2513 / #2668 / SPEC-2014 Phase C1

## GUI hot path の同期プロセス起動は CPU% だけでは見落とす

### 事象

macOS/Windows GUI が全体的に重く、キー入力・ウィンドウ操作・Launch Agent 起動が遅い。
CPU 使用率は高く見えない一方、macOS `sample` では GUI main thread が `poll` / `posix_spawnp`
配下で時間を使っていた。

### 原因

UI の Active Work / Workspace 投影 hot path で、repo hash 解決のたびに `git remote get-url origin`
を同期起動し、さらに Workspace cleanup candidate の remote delete 可否判定で branch inventory
hydration を同期実行していた。Board/Workspace daemon update や frontend ready で繰り返されるため、
CPU% ではなく main-thread blocking として体感遅延になった。

### 再発防止策

1. GUI hot path では `git` / `gh` / `docker` などの外部プロセスを同期起動しない。必要なら
   config/file cache を読むか、blocking task に逃がす。
2. 性能回帰テストは fake executable を `PATH` 先頭に置いて呼び出しログを検査し、対象処理が
   外部プロセスを起動していないことを pin する。
3. CPU% が低い重さは `sample` / profiler で main-thread blocking と process spawn を確認する。

### 関連 PR / Issue

- Issue #2725

## Frontend gating を追加するときは変数初期化順序を実行時に pin する

### 事象

Launch Wizard の Runtime confirmation 分離で `showSetupForms` を追加した後、Start Work 直後の画面で
Agent select が操作できなくなった。静的 contract test は通っていたが、実 UI では wizard body
rendering が壊れていた。

### 原因

`renderLaunchWizard()` 内で `showSetupForms = showManualSetup && !isRuntimeConfirmation` を
`showManualSetup` の `const` 宣言より前に評価していた。JavaScript の temporal dead zone により
render 時に例外が発生し、フォームが正常に描画されなかった。既存テストは「文字列が存在すること」
を主に見ており、依存する local const の初期化順序を pin していなかった。

### 再発防止策

1. frontend render helper に新しい gating 変数を追加するときは、その変数が依存する local const の
   宣言後に評価されることを contract test に含める。
2. 可能なら static string assertion だけでなく、`node --check` / frontend unit / live smoke の
   どれかで runtime parse/execution に近い確認も併用する。
3. Launch Wizard の Settings/Runtime 分離では、Start Work / Launch Agent / Quick Start の入口ごとに
   初期 phase と Runtime confirmation phase の両方を確認する。

### 関連 PR / Issue

- PR #2731
- SPEC-2014

## Project Index は起動時 visibility と repair scheduling を分離する

### 事象

GUI 起動直後に Project Index が全 active worktree の status probe と auto-repair を走らせ、
worktree が多いリポジトリでは 15 秒以上 Python/Chroma プロセスが連続起動して UI がカクついた。

### 原因

Project tab の dot 表示、Settings.Index の全 worktree health 表示、startup auto-repair の
3 つが同じ aggregated status 経路を共有していた。起動時に必要なのは現在の worktree と
repo-shared scopes だけなのに、Settings.Index 向けの全 worktree 可視性まで毎回読んでいた。

### 再発防止策

1. 起動時 status probe は current worktree 限定にする。全 worktree の health table は
   Settings.Index を開いた時だけオンデマンドで取得する。
2. Startup auto-repair は repo-shared scopes と current worktree の file scopes だけを対象にし、
   inactive worktree の repair は Settings.Index の明示操作に任せる。
3. 性能修正では `sample` と子プロセス監視で、起動 smoke 中に `index-files` や全 worktree
   `--action status` が勝手に並ばないことを確認する。
4. 起動時 probe とオンデマンド full refresh を分離する場合、in-flight coalescing は
   「要求を捨てる」のではなく「必要な重い可視性を後続で流す」契約にする。Settings.Index
   のようにユーザーが明示的に開いた full table は、startup current-only probe と衝突しても
   後続 retry で最後に full status を配信する。この retry は固定短時間 timeout で捨てず、
   bootstrap が長引く初回 runtime 準備や大規模 repo でも要求を保持する。

## Agent launch は親環境の色抑制フラグをそのまま継承しない

### 事象

Codex セッションから Windows GUI を起動して手動確認したところ、ホイールスクロールは動いたが
Agent window 内の TTY 表示が白一色になった。

### 原因

親 Codex セッションの環境に `NO_COLOR=1` と `TERM=dumb` があり、gwt は `TERM` と `COLORTERM`
を `xterm-256color` / `truecolor` に補正していた一方で、`NO_COLOR` は子 Agent に引き継いでいた。
初回修正では launch env map から `NO_COLOR` を削除しただけで、PTY spawn が親 process env を継承する
経路の `env_remove("NO_COLOR")` 相当を設定していなかったため、子 Agent process には `NO_COLOR=1` が
残り続けた。そのため agent CLI 側が ANSI 色を出さず、xterm.js / CSS ではなく launch env が色を抑制していた。

### 再発防止策

1. GUI / Agent launch 環境を作るときは、terminal capability を補正するだけでなく、親の
   color suppressor env (`NO_COLOR` など) が意図せず残らないことを確認する。`env_vars` から
   消すだけでは不十分で、PTY spawn の `remove_env` にも入れて inherited env を明示削除する。
2. `NO_COLOR` を profile env で明示した場合は尊重し、親 process 由来の値だけを剥がす。
3. WebView の色回帰を調査するときは、xterm.css の読み込み、WebSocket payload の SGR、
   frontend DOM の computed color、launch env の順に切り分ける。

## Runtime path を versioned 化したら test fake も同じ公開 API を使う

### 事象

Project Index runtime を content-addressed runner / venv に変更した後、Knowledge Bridge の
async semantic search テストが timeout した。旧 `~/.gwt/runtime/chroma-venv` にだけ fake python
を置くテストヘルパーが残り、実コードは新しい versioned python path を見ていた。

### 原因

本番コードの runner / python path を `gwt_core::runtime::project_index_*_path()` に寄せた一方、
テストの fake runtime が legacy path を直書きしていた。さらに fake `gh` テストは専用 lock だけを
使い、`PATH` / `GWT_FAKE_GH_MODE` を触る他テストとの並列実行で mode が混線した。

### 再発防止策

1. Runtime path を変える場合、テスト fake は legacy path 直書きではなく同じ公開 path API に置く。
2. `PATH` や env var を変更する共有 fake harness は、fake 専用 lock だけでなく global env test lock
   も取る。
3. `cargo test -p gwt-core -p gwt --all-features` は単独 targeted pass だけで代替せず、デフォルト並列で
   最後に通して env 競合を拾う。

## URL をユーザー操作ログに出すときは authority までに正規化する

### 事象

`gwt_ui_action` の backend connection test ログで `api_key` は出力していなかったが、`base_url` を
そのまま `ui_target` に入れていたため、userinfo や signed query を含む URL がログに残る可能性があった。

### 原因

既存の `sanitize_ui_action_field()` は改行や制御文字をログ向けに整形するだけで、URL の
credential / path / query / fragment を機密情報として扱っていなかった。レビューでは
`https://user:pass@example.com/v1?token=...` のような入力がそのまま残る点を指摘された。

### 再発防止策

1. ユーザー操作ログに外部 URL を入れるときは、既定で `scheme://authority` までに正規化し、
   userinfo / path / query / fragment を落とす。
2. `api_key` の非出力だけで安全と判断せず、URL 自体に credential や token が含まれるケースを
   regression test に入れる。
3. ログ sanitization の helper 名は「format sanitization」と「secret redaction」を区別し、
   期待する保護範囲をテスト名で明示する。

## 起動時 auto resume は保存済みUI状態と履歴セッションを混同しない

### 事象

Agent session auto resume の実装後、実HOMEで `gwt` ブラウザサーバーを起動すると、前回開いていた
プロジェクトだけでなく過去の session TOML に残る多数の worktree が復元候補になった。
結果としてブラウザURLが出る前に大量の agent/version process が起動し、UIには見覚えのない
プロジェクトが並ぶ状態になった。

### 原因

起動時の即時復元が `session.json` の保存済み project tabs ではなく、`~/.gwt/sessions/*.toml`
全体を source of truth として扱っていた。さらに migration で `Interrupted` にした旧セッションや、
失敗した自動復元で生成された `Resume` セッションには、ユーザーが直前にUIで開いていた根拠がないのに
exact auto resume 候補として扱える余地があった。

### 再発防止策

1. 起動時に新しい project tab を履歴 session TOML から作らない。即時 auto resume は
   `session.json` など保存済みUI状態で既に復元された tab に一致する session だけに限定する。
2. lifecycle hook の根拠がない legacy / migrated session は、manual resume 候補として残しても
   exact auto resume 候補にはしない。
3. 復元系の修正では、実HOMEに近い「多数の古い session TOML がある」ケースをテストまたは
   headed browser smoke で確認し、URL 出力前に大量の agent/version process が起動しないことを見る。

## Hook runtime state の env mutation test は crate-wide lock を共有する

### 事象

Release PR の `cargo test -p gwt-core -p gwt --all-features` で、
`handle_user_prompt_submit_uses_hook_cwd_for_board_scope` が `HookOutput::Silent` になり、
後続の workspace tests が `PoisonError` で連鎖失敗した。

### 原因

`runtime_state` tests が独自の `ENV_LOCK` を持っており、crate-wide の `env_test_lock()` で保護された
`board_reminder` / workspace tests と同時に `GWT_SESSION_ID` などの process-wide env を変更できた。
そのため並列 test 実行時に hook session env が消え、最初の panic が shared lock を poison した。

### 再発防止策

1. `HOME` / `USERPROFILE` / `GWT_SESSION_ID` / `GWT_SESSION_RUNTIME_PATH` などの process-wide env を触る
   gwt tests は、ローカル mutex を作らず必ず `crate::env_test_lock()` を共有する。
2. lock 共有の regression test では、保持中に取得できないことだけを短い timeout で確認し、
   lock 解除後の取得完了は固定 timeout ではなく thread join で待つ。全体並列実行では他の env tests が
   先に lock を取る可能性がある。
3. env lock を使う巻き添え側の tests は、`lock().unwrap()` ではなく poisoned lock を回復する helper を使い、
   先行 panic の調査を `PoisonError` のノイズで隠さない。

## Hook tests は親エージェントの Codex env を暗黙に使わない

### 事象

PR #2861 の Linux CI で `event_dispatcher_keeps_blocked_stop_runtime_state_running` が失敗した。
ローカルでは同じ test が通っていたが、これは実行中の Codex agent 由来の `CODEX_THREAD_ID` が
process env に残っていたためだった。

### 原因

hook integration tests が `CODEX_THREAD_ID` / `GWT_SESSION_ID` /
`GWT_SESSION_RUNTIME_PATH` などの親プロセス環境を明示的に固定していなかった。
さらに Codex の placeholder session id (`agent-session`) と実 session id の違いを test payload が
曖昧にしていたため、CI の clean env でだけ missing `session_id` と判定された。

### 再発防止策

1. Codex hook identity を扱う tests は、`CODEX_THREAD_ID` を明示的に unset または set して期待条件を固定する。
2. hook diagnostics など runtime state が主目的ではない tests は、`GWT_SESSION_ID` /
   `GWT_SESSION_RUNTIME_PATH` を unset して現在の agent session を汚染しない。
3. Codex payload の `session_id` を使う tests では placeholder の `agent-session` を避け、
   実 resume 可能 ID を表す別値を使う。

## 2026-05-22 — SPEC section edit stdin must be validated before piping to gwtd

Type: failure-pattern
Context: While updating SPEC-2014 tasks, a malformed perl append command produced no stdout, but the downstream `gwtd issue spec 2014 --edit tasks -f -` still wrote an empty tasks section.
Learning: Pipelines into section-edit commands can erase canonical SPEC content when the producer fails; pipefail alone is not enough if the consumer accepts empty stdin.
Future Action: For `gwtd issue spec --edit <section> -f -`, first materialize or generate the new section content with a size/header check, then edit; after editing, immediately reread the section and verify nonzero size plus expected tail.

## 2026-05-22 — headless-browser-check はユーザーの production gwt / GWT.app を絶対に停止させない

Type: lesson
Context: Issue #2867 の Recent Projects pollution 修正で headless 確認を行う際、私が独断で「production GWT.app (PID 9569) を終了してください」と要求した。ユーザーから skill にそのような手順は無いと指摘され、誤って kill されると困るとの再発防止依頼を受けた。
Learning: headless-browser-check skill は現在 checkout の `target/debug/gwt` を別プロセスとして起動する前提で、ユーザーの常用 gwt / GWT.app を停止する手順は含まない。session.json や app-instance.lock の競合懸念があっても、エージェントが自動で停止判断をしてはならない。
Future Action: headless 起動時に共有 state (session.json / app-instance.lock / port) の懸念がある場合は、懸念点を明示してユーザーに判断を委ねる。production gwt を停止する選択肢は提示してよいが、エージェント側で前提化したり、依頼したりしない。skill の guardrail にも明文化済み (.claude/.codex の SKILL.md 両方)。

## 2026-05-23 — Index search must resolve workspace-home project roots

Type: lesson
Context: Index window search can run with the active tab project_root set to the gwt workspace home rather than a concrete git worktree. The workspace home itself has no git origin, while its child bare repo and launched worktree do.
Learning: Project-index callers must separate repository identity from the searchable/default worktree. Resolve repo hash through the workspace-home child bare repo and prefer the running process cwd worktree for repo-scoped and file searches.
Future Action: When changing index search or status paths, test both direct worktree roots and workspace-home roots created by gwt-managed layouts before relying on git rev-parse or origin detection.

## 2026-05-23 — Do not assume Workspace update is the current coordination path after Project State migration

Type: lesson
Context: After implementing durable discussions and Project State storage paths, I still followed stale generated gwt-coordination guidance and ran gwtd workspace update. The user pointed out from Board that Workspace is gone for the current discussion/coordination framing and told me to merge origin/develop to see the current state.
Learning: Generated local skill text can lag the branch's product terminology. For this work, current-state reporting should be checked against Board and the latest develop/SPEC context; do not treat gwtd workspace update as the default user-facing path when Project State / Work / Discussion / Branch separation is being discussed.
Future Action: Before posting coordination updates in SPEC-2359 terminology work, merge or inspect origin/develop, read recent Board entries, and prefer Board-only reporting unless the active instructions explicitly require the compatibility gwtd workspace path.

## 2026-05-22 — Launch Runtime confirmation must stay side-effect free and cached

Type: lesson
Context: User reported that Launch Agent Runtime felt slow and asked whether Runtime startup was creating a worktree. During SPEC-2014 follow-up, Runtime confirmation called resolve_launch_worktree and also scanned thousands of stale session records through legacy git root matching.
Learning: Runtime confirmation should only inspect an existing target worktree or project root for Docker/runtime context; final Launch owns materialization. Session repo matching must cache current repo identity, treat mismatched repo_hash as authoritative, and avoid git root probes for missing session paths.
Future Action: When changing Launch Wizard Runtime hydration or previous-profile/Quick Start matching, add tests that missing target worktrees are not created and that stale session history does not fan out per-session git probes.

## 2026-05-23 — Launch Wizard click bugs require state inspection before input fallbacks

Type: failure-pattern
Context: Start Work の Create and Launch が押せない報告で、最初は pointer/click fallback と pending feedback を修正したが、headed Chrome E2E で最終ボタンが disabled=true になっている別原因を確認した。WebSocket state では Start with last settings 後に selected_launch_path=quick_start かつ quick_start_entries=[] で primary_action_enabled=false だった。
Learning: UI action が効かない不具合は input event loss だけとは限らない。実ブラウザで disabled/aria/state を直接観測し、バックエンド state と照合してから fallback を増やすべき。
Future Action: Launch Wizard / Start Work のボタン不具合では、headed E2E で実座標クリックを行う前に DOM disabled state と launch_wizard_state payload を保存し、disabled なら状態遷移テストを先に追加する。

## 2026-05-25 — include_str web assets require touch embedded_web.rs to rebuild

Type: lesson
Context: SPEC-2359 Work Unification で web ファイル (index.html, app.js, workspace-kanban-surface.js) を変更したが cargo build がキャッシュを使い、旧バイナリが serve された
Learning: include_str! で埋め込まれた web assets は Rust ソースファイルの変更として検出されない。web ファイルを変更した後は touch crates/gwt/src/embedded_web.rs を実行してから cargo build する必要がある
Future Action: web ファイル変更後は必ず touch crates/gwt/src/embedded_web.rs してからビルドする

## 2026-05-24 — Codex hook visual verification must use fresh launch binary

Type: workflow
Context: SPEC-2077 T-266 initially failed in SS.mov because the recorded Codex window used hooks generated by an older /Applications/GWT.app gwtd fallback and lacked enabled trust entries in ~/.codex/config.toml; the dev browser-server URL had also rotated.
Learning: A browser URL being reachable is not enough for Codex status verification: confirm the running gwt binary is rebuilt, the served URL is current, and the tested worktree has enabled=true trust entries for its .codex/hooks.json before asking for visual confirmation.
Future Action: Before manual Codex hook/status verification, rebuild target/debug/gwt and target/debug/gwtd, restart only the dev gwt process launched from the current checkout, share the new URL, and inspect ~/.codex/config.toml for the tested worktree or relaunch the Codex agent from the fresh server.

## 2026-05-24 — Codex linked worktree hooks require root checkout materialization

Type: failure-pattern
Context: Codex 0.133 /hooks showed Installed 0 / Active 0 in gwt-managed linked worktrees even though each worktree had .codex/hooks.json.
Learning: Codex resolves project hook declarations for linked Git worktrees from the matching root checkout .codex folder, not from hook files stored only in the linked worktree.
Future Action: When changing Codex managed hook generation or trust registration, test linked worktrees and make generated hooks plus trust keys use the same root-checkout path Codex discovers.

## 2026-05-24 — Codex hook discovery version boundary

Type: lesson
Context: SPEC-1935 investigation and implementation found that Codex hook discovery path differs by CLI version in gwt-managed worktrees.
Learning: Codex versions older than 0.131.0-alpha.21 discover <worktree>/.codex/hooks.json; 0.131.0-alpha.21 and newer discover the workspace-home/CODEX_HOME hooks path. Unknown installed versions should use a both-path fallback.
Future Action: When changing Codex managed hook materialization or trust registration, route path selection through CodexHookDiscoveryMode and verify old, new, and unknown-version cases.

## 2026-05-25 — headless-browser-check must launch fresh gwt from the current checkout

Type: lesson
Context: User clarified that headless-browser-check must not stop production gwt, must not reuse an already-running gwt address, and must server-launch the gwt binary from the launch/current checkout.
Learning: Manual browser verification can be falsely performed against a stale or production server if an agent reuses a reachable URL or an installed gwt binary. The skill must start a fresh `gwt` process from the current checkout's target/debug/gwt and accept only that process's URL handoff/log.
Future Action: Before sharing a headless-browser-check URL, capture the launch directory, resolve that same checkout's repository root and target/debug/gwt, start a new `gwt` process with a fresh GWT_BROWSER_URL_FILE, verify that fresh URL, and stop only the process launched by the skill when the user is done.

## 2026-05-25 — headless-browser-check must use repo-root absolute gwt paths

Type: lesson
Context: Codex review on PR #2884 pointed out that headless-browser-check resolved the repository root but still showed a relative target/debug/gwt launch command, which fails when the skill is triggered from a subdirectory such as crates/gwt.
Learning: A skill can say 'current checkout' and still be ambiguous if command examples remain relative to the caller's cwd. For browser-server handoff checks, the launch command must either cd to the resolved repository root or use an absolute <repo-root>/target/debug/gwt path.
Future Action: When updating headless-browser-check or similar launch skills, make every executable path in the workflow match the resolved root/checkout contract; test the instructions mentally from a subdirectory as well as from repo root.

## 2026-05-25 — Check review threads after auto-merge

Type: lesson
Context: During PR #2887, CI passed and auto-merge completed while automated review threads were posted shortly before merge.
Learning: A PR can be merged before late automated review comments are inspected; merged state does not mean review feedback is clear.
Future Action: After any auto-merge, run gwtd pr review-threads for the merged PR and address actionable feedback via a follow-up PR when the original PR can no longer be updated.

## 2026-05-25 — Normalize Windows child-process paths at launch boundaries

Type: lesson
Context: Start Work / Launch Agent on Windows can leak PowerShell provider-qualified or verbatim paths like Microsoft.PowerShell.Core\FileSystem::\\?\E:\gwt\work\... into cwd and GWT_PROJECT_ROOT.
Learning: Normalize child-process cwd/env at worktree discovery, launch config, GUI/shell launch, Docker host path planning, and PTY spawn boundaries; preserve resolve_launch_worktree_request direct-call no-op semantics unless a SPEC changes that contract.
Future Action: Before changing Windows launch cwd/env handling, add RED tests for provider-qualified and verbatim paths, then verify both focused launch tests and the full Rust matrix.

## 2026-05-25 — Recurring Windows Start Work launch failure needs regression memory

Type: lesson
Context: Windows Start Work / Launch Agent failed to start again after a previous similar launch failure. This occurrence involved PowerShell provider-qualified or verbatim cwd values such as Microsoft.PowerShell.Core\FileSystem::\\?\E:\gwt\work\... causing Claude Code to error and Codex to fall back to C:\Windows.
Learning: Treat Windows agent launch failures as recurring regression candidates, not one-off environment issues. The cwd, GWT_PROJECT_ROOT, worktree discovery path, and PTY spawn cwd must all be inspected for Windows-specific provider/verbatim path forms before declaring the launch path healthy.
Future Action: When a Windows Start Work / Launch Agent failure is reported, first search memory for this recurrence, then add focused RED tests for provider-qualified/verbatim cwd normalization across launch config, env, worktree parsing, and PTY spawn before fixing.

## 2026-05-25 — Preserve non-UTF8 paths when normalizing Windows launch cwd

Type: lesson
Context: PR #2894 review found that normalize_windows_child_process_path used Path::to_string_lossy(), which can replace valid Unix non-UTF8 path bytes with the UTF-8 replacement sequence when the shared helper is called broadly at launch/worktree boundaries.
Learning: Windows-specific path normalization helpers must not lossy-convert Path values. Only rewrite when Path::to_str() succeeds and a known Windows provider/verbatim prefix is present; otherwise return the original PathBuf to preserve platform path bytes.
Future Action: Before broadening any Path normalization helper, add Unix non-UTF8 byte-preservation tests alongside the Windows prefix tests, then verify the helper returns unchanged PathBuf for non-UTF8 and no-op inputs.

## 2026-05-26 — Separate Launch Wizard runtime detection root from target worktree materialization

Type: lesson
Context: SPEC-2014 managed workspace parent bug: Start Work opened at /Users/akiojin/Workbench/gwt while Docker files lived in nested develop checkout, so Runtime target selection was hidden.
Learning: Runtime confirmation must first prefer an existing target worktree, but when the target worktree does not exist it should use an existing checkout such as develop/main for Docker context detection without resolving or creating the target worktree.
Future Action: When touching Launch Wizard runtime context, add tests for workspace parent roots and assert the target worktree remains absent until the final launch/materialization step.

## 2026-05-26 — session.json project_root が非 git ディレクトリを指すと workspace.json ハッシュ不一致で空 workspace が読み込まれる

Type: lesson
Context: ブラウザサーバー検証時、session.json の gwt タブが /Users/akiojin/Workbench/gwt（bare repo 親ディレクトリ、git リポジトリではない）を指していたため、detect_repo_hash が失敗し compute_path_hash（パスベース b19aac38305901f5）にフォールバック。実際の workspace.json は remote URL ベースハッシュ 99a8660247f5bc49 に保存されており、空の workspace が読み込まれてウィンドウが 0 件になった。
Learning: project_scope_hash は (1) origin URL ベースと (2) パスベースの 2 種類のハッシュを返す。session.json の project_root が git リポジトリでない場合、パスベースにフォールバックし、production app が書いた workspace.json と別ファイルを読む。
Future Action: ウィンドウ復元テスト時は session.json の project_root が実際の git ワーキングツリーを指しているか確認する。非 git ディレクトリ → パスハッシュフォールバック問題は別 Issue として登録する。

## 2026-05-25 — 既存ブランチの実装確認は git log --all で先に行う

Type: lesson
Context: SPEC-2359 Phase W (Workspace → Work) を develop で実装し始めたが、work/20260524-0442 ブランチで既に完全に実装済み (PR #2885) だった
Learning: 新しい作業を始める前に git log --all --oneline --grep でキーワード検索し、他ブランチで既に実装されていないか確認する。SPEC の tasks が unchecked でも、別の worktree/agent が実装済みの可能性がある
Future Action: 実装着手前に git log --all --grep と gh pr list で既存実装・オープン PR を確認する preflight を必ず行う

## 2026-05-25 — visual regression workflow 削除後の required check 残骸

Type: lesson
Context: v9.46.0 リリースで visual-regression.yml を削除したが、main branch protection の required status checks に Operator Design System (Playwright) が残っていたため PR が BLOCKED になった
Learning: CI workflow を削除する際は、対応する branch protection required status checks も同時に削除する必要がある
Future Action: workflow ファイルを削除する commit を含むリリースでは、branch protection required checks の棚卸しを行う

## 2026-05-25 — git status の M (tracked modified) を untracked と誤認しない

Type: lesson
Context: リリース作業中に tasks/memory.md の stash pop コンフリクトを解消する際、git status で M (modified/tracked) と表示されていたファイルを「バージョン管理対象外のローカルファイル」と誤認し、--theirs で一方的に上書きした
Learning: git status の M は tracked file の変更を示す。?? が untracked。ファイルがバージョン管理対象かどうかは git status の表記で判断でき、思い込みで判断してはならない
Future Action: stash pop コンフリクト解消時は、必ず両側の差分を確認し、tracked file であれば両方の変更を保持するマージを行う。一方的に --theirs / --ours で上書きしない

## 2026-05-26 — Work preset missing in Phase F wizard refactor

Type: lesson
Context: Work画面のGit BranchesタブからLaunch/Resumeすると Window is not a Work surface エラー。wizard.rsのopen_launch_wizardとresume_branch_latest_agent_eventsがWindowPreset::Branchesのみ許可していた。レガシーのlaunch_wizard_runtime.rsはBranches+Work両方許可。
Learning: app_runtime Phase F リファクタで旧コード(launch_wizard_runtime.rs)からの移植時にプリセット条件を狭くしたリグレッション。mod.rs:5420のload_branches_eventsは正しく両方許可していたので、同一ファイル内にパターンの不整合があった。
Future Action: wizard.rsのプリセットチェック変更時はmod.rsの同種チェック(load_branches_events等)と一貫性を確認する。Work surface embedded branches パターンでは WindowPreset::Work も許可が必要。

## 2026-05-27 — collect_resumable_agents excluded running agents causing empty Resume Picker

Type: lesson
Context: Resume Picker showed 'No resumable agents' even when Codex was Running. Root cause: collect_resumable_agents in wizard.rs filtered out live_session_ids entirely. Also, branch cleanup preset check at mod.rs:6323 only allowed WindowPreset::Branches, missing Work.
Learning: When adding a new preset check (like the Work surface unification), grep all sites with the same error message string to ensure none are missed. Four sites had 'Window is not a Work surface' but one (branch cleanup) was not updated. Also, filtering running agents from the Resume Picker was a UX anti-pattern — the user sees a Running agent in the workspace card but Resume says none exist.
Future Action: After any WindowPreset guard change, search all occurrences of the error message to confirm all sites are consistent. For picker/list UIs, prefer including all states with appropriate badges over silently filtering items the user can see elsewhere.

## 2026-05-27 — Do not treat manual visual confirmation as E2E

Type: lesson
Context: During Claude Code Fast mode startup support, I initially reported completion after Rust tests and user visual confirmation, then admitted no automated E2E had been run when the user asked.
Learning: Manual visual confirmation is not a substitute for an automated live E2E when the changed behavior crosses Launch Wizard frontend, WebSocket/backend state, and runtime context resolution.
Future Action: For Launch Wizard or Start Work UI changes, add or run a live Playwright E2E that drives the actual user path before claiming E2E coverage; report manual checks separately from automated E2E.

## 2026-05-27 — Claude Code startup alone is not a billing reason to skip E2E

Type: lesson
Context: After adding a live E2E for Claude Code Fast mode, I incorrectly justified not launching real Claude Code by saying it could incur billing. The user corrected that starting Claude Code alone does not charge.
Learning: Do not cite billing as a reason to avoid a Claude Code launch smoke. The valid concerns are environment/auth availability, external process stability, and cleanup; if those are acceptable, launch smoke should be performed.
Future Action: When explaining why an E2E stops before an external AI tool, separate real constraints from assumptions. For Claude Code startup, prefer an env-gated real-launch smoke with explicit cleanup instead of claiming startup cost risk.

## 2026-05-27 — Resume Picker must filter by workspace_id — headed E2E caught what unit tests missed

Type: lesson
Context: Resume Picker returned agents from ALL branches instead of the selected Work item. Unit tests passed because they only tested presence/absence of agents, not cross-branch leakage. The bug was only caught when the user asked for a headed browser check.
Learning: Unit tests for list/picker UIs must include a negative case: create agents for TWO different workspace_ids and assert that filtering by one excludes the other. Headed E2E is essential for verifying picker scope — automated tests alone cannot catch cross-scope leakage when only one scope is populated in the test fixture.
Future Action: For any picker/list that accepts a scope filter (workspace_id, branch, etc.), always write a multi-scope unit test that asserts exclusion. Run headed E2E before declaring Resume/Launch picker changes complete.

## 2026-05-28 — Windows Claude Code npx cache can leave claude.exe renamed to .old

Type: failure-pattern
Context: During SPEC-2809 F4 manual verification on Windows, Claude Code launch through `npx` failed with `...@anthropic-ai\\claude-code\\bin\\claude.exe is not recognized`. The npm `_npx` install directory contained `claude.exe.old.<timestamp>` hardlinks in both `@anthropic-ai/claude-code/bin` and `@anthropic-ai/claude-code-win32-x64`, while `.bin/claude.cmd` still referenced `bin/claude.exe`.
Learning: The error can be a corrupted npm `_npx` ephemeral install, not a gwt quoting/runtime failure. `npm cache verify` does not necessarily repair `_npx` directories. Removing the specific resolved `_npx` package directory and letting `npx` reinstall restored `bin/claude.exe`; `npx --yes @anthropic-ai/claude-code@latest --version` then returned 2.1.153 (Claude Code).
Future Action: When Windows Claude Code reports `claude.exe` not recognized from `npm-cache\\_npx`, inspect the referenced `_npx` `node_modules` first. If only `claude.exe.old.<timestamp>` exists, verify the target path is under `npm-cache\\_npx`, remove that specific `_npx` directory or reinstall the package, then verify with `npx --yes @anthropic-ai/claude-code@latest --version` before changing gwt code.

## 2026-05-28 — Panel roots must be anchored before pane overflow can work

Type: failure-pattern
Context: Profile window scroll bug (#2916): .profile-root had flex/min-height styles but was not included in the shared .window-body root group that applies position:absolute and inset:0.
Learning: Child panes with overflow:auto only scroll when their root receives the window-body height constraint. A flex/grid pane can look correct in CSS but still expand past the window when the root is not anchored.
Future Action: When adding or fixing panel surfaces, verify the surface root participates in the shared root containment rule and add an embedded-web contract test for scroll boundaries.

## 2026-05-28 — Profile grid autosave rows need stable editable keys

Type: lesson
Context: SPEC-2015 Profile Environment Variables grid implemented row-level autosave and re-rendered rows from normalized profile payload.
Learning: Editable rows that are keyed by the value being edited can lose subsequent key changes after the first autosave/re-render unless the row-local key mirror and backing draft entry are updated together. Pending added rows should also update visible Result cells immediately while debounce save is pending.
Future Action: When adding autosaved table rows, test multi-character key edits, pending row value edits, backend roundtrip re-render, and duplicate-key collapse before declaring UI behavior complete.

## 2026-05-28 — Work surface rerender must honor legacy preset aliases

Type: failure-pattern
Context: Visual E2E failures after Work unification: Quiet Work rows stayed empty for windows with preset=workspace because workspace-kanban-surface.renderWindows only refreshed preset=work. Branch Cleanup E2E also still targeted the old standalone branches surface while branches now lives under the Work surface tab.
Learning: When a surface is renamed or consolidated, async rerender paths and E2E selectors must use the same preset alias set as the mount path. It is easy to update initial mount logic while leaving event-driven refresh paths on the new canonical preset only.
Future Action: For future surface renames, add unit tests for renderWindows/event refresh using legacy preset aliases, then update Playwright fixtures/selectors to navigate through the canonical visible UI rather than stale standalone surface classes.

## 2026-05-28 — Profile visual checks require screenshot inspection

Type: lesson
Context: Profile env grid UI was changed and headed E2E passed, but the user reported Profile Metadata was not visible. Screenshot inspection showed the metadata section was collapsed to 0px because .profile-editor-pane used a 2-row grid while renderProfile appends actions, metadata, and env sections.
Learning: Headed browser execution alone is not visual verification. For layout changes, inspect screenshots or pixel/DOM geometry that proves the intended visible regions are actually visible and non-overlapping.
Future Action: Before reporting UI layout work as visually verified, capture and open screenshots for the relevant state and add assertions for visible geometry/non-overlap, not only DOM presence.

## 2026-05-28 — Verify CSS tokens exist before using them in new components

Type: lesson
Context: SPEC-2780 v2 で Release Notes update button を実装した際、最初 --color-accent / --color-warning / --color-on-accent を fallback hex 付きで使ったが、これらは gwt のデザイン token として未定義だった。ユーザー目視で「CSS は合っていますか?」と指摘されて気付き、--color-state-active / --color-state-blocked / --color-text-disabled / --color-scrim 等の実 token に置換した。
Learning: 新規 CSS を書くときは fallback hex 付きで未定義 token を仮置きしてはいけない。fallback だけが効くので見た目はテーマと不整合になるが、ビルド・テストは通って気付かない。
Future Action: 新規 component の CSS を書く前に grep -E '^\s*--color-' crates/gwt/web/styles/tokens.css で実在 token を確認する。precedent component (update-modal__btn--primary / update-cta.is-error 等) を必ず参照して同じ token を使う。

## 2026-05-28 — gwt-register-spec: rebind owner_spec by re-issuing `register start --spec <real-id>` after Issue creation

Type: workflow
Context: gwt-register-spec skill workflow doc says to call `gwtd register start --spec 0` initially (placeholder) and then `gwtd register phase --spec <real-id> --label create` after Issue creation to bind the real id. However, skill_state_runtime.rs rejects the phase update when owner_spec does not match (`phase refused: state owns SPEC-Some(0)`). Hit this while registering SPEC #2920.
Learning: skill_state_runtime::run for SkillStateAction::Start is idempotent: a second `register start --spec <real-id>` overwrites the existing state (new SkillState constructed with owner_spec = Some(real-id)). Use this to rebind from the placeholder before continuing with phase updates.
Future Action: When invoking gwt-register-spec: 1) register start --spec 0, 2) issue spec create --title <t> -f <stub> -> capture real id, 3) register start --spec <real-id> (rebind), 4) register phase --label create, 5) issue spec <n> --edit spec, 6) register phase --label edit, 7) verify section roundtrip, 8) register phase --label roundtrip, 9) register complete. Future SPEC-2784 edit should clarify the doc to say start (not phase) is the rebind point.

## 2026-05-28 — PR auto-merge can outrun review reply commits

Type: lesson
Context: SPEC-2780 v2 PR #2917 was set to auto-merge once CI passed. Codex+CodeRabbit posted P1/Major review threads after the initial push, but by the time the review reply commit was prepared and pushed, the PR had already auto-merged with the original buggy code on develop. The follow-up fix had to ship as a separate PR (#2918).
Learning: When a PR can auto-merge, the agent must (1) check for unresolved review threads BEFORE pushing any commit that would trigger another CI cycle, AND (2) gate auto-merge on review resolution rather than just CI pass — otherwise review fixes land in a follow-up PR while the original P1 issues sit in develop unfixed.
Future Action: Before declaring a PR ready: run gwtd pr review-threads <n> to inspect unresolved threads. If any P0/P1/Major comments exist, prefer addressing them BEFORE the auto-merge gate clears. If a PR has already auto-merged with unaddressed reviewer concerns, immediately raise a follow-up PR and Board-handoff to the user; do not stop at 'thread resolved' on the merged PR alone.

## 2026-05-28 — gwt-managed .gwt exclude covers attachment drop files

Type: lesson
Context: Planning SPEC-2012 attachment redesign: D&D/paste files will be copied under the worktree-local .gwt/drop-files directory and later removed with the worktree.
Learning: gwt-skills already writes a broad .gwt/ entry to the gwt-managed block in .git/info/exclude for managed worktrees. Because Git worktree remove deletes ignored untracked files, .gwt/drop-files does not need a dedicated cleanup path when the managed exclude is present.
Future Action: Before adding cleanup code for new project-local .gwt subdirectories, first verify the managed .git/info/exclude contract and prefer regression tests over redundant deletion logic.

## 2026-05-28 — Headed E2E visual verification cannot be replaced by metric measurement when the user explicitly asks for visual check

Type: lesson
Context: While fixing Claude Code launch regressions (#2923, #2924), I reported PR-ready status after measuring cell-grid columns and CSS values via Playwright but had NOT actually launched a headed browser to observe the rendered terminal. The user (akiojin) pointed this out twice: 'あなた自身がHeadedブラウザでE2Eテストで目視してチェックしてください' and 'あなたはHeadedブラウザによるE2Eテストで目視チェックしていないですよね？画面崩れが発生していますよ？'. Only after I actually navigated Playwright to the served URL, resized the viewport, scrolled the canvas to bring the active Claude Code agent into the visible 720x420 frame, and took a real PNG screenshot did the user accept the verification. The Ready PR Gate User Verification Result hinges on this distinction; metric-based confidence is not visual confirmation.
Learning: When AGENTS.md or the user demands a Headed visual E2E check, do not substitute it with DOM metric reads, calculation, or computed-style assertions. The Ready PR Gate requires that the agent actually opened a headed browser, navigated to the served URL, positioned the relevant window into the viewport, and captured / observed a real screenshot of the rendered state. Pixel-level corruption (font fragmentation, wrap of a single character, stray byte echoes) only surfaces in the real rendering pipeline; calculations are necessary but not sufficient.
Future Action: Before declaring User Verification done, run a Headed Playwright session and (1) navigate to the URL, (2) resize the browser to a viewport larger than the agent window, (3) scrollIntoView / reset zoom so the target window is fully visible inside the viewport, (4) take a screenshot, (5) Read the screenshot file in the conversation and inspect it visually. Use textual symbol-string assertions ONLY as supporting evidence, never as a replacement. If a metric measurement disagrees with a visual rendering, trust the visual.

## 2026-05-29 — GUI 視覚検証は稼働中 tray インスタンスが旧アセットを serve するため新ビルドが見えない

Type: lesson
Context: SPEC-2014 Launch Agent コントロール刷新 (frontend app.js/launch-controls.js/app.css) を実装後、headed 視覚検証を試みた。稼働中の GWT.app(v9.49.0) が per-user tray singleton lock (gwt::cli::tray::lock, gwt_home 由来) を保持し、target/debug の新ビルドを --no-tray --no-open でも並行起動できず、稼働インスタンスの URL(旧アセット)が返るだけだった (新 module /launch-controls.js が稼働 URL で 404 で確定)。GWT_FORCE_NEW_INSTANCE は GUI lock 用で tray-resident guard には効かない。HOME を分離すると tray lock は回避できるが、プロジェクト文脈が無く Launch Agent 動線に到達できず first-run runtime セットアップで stall した。
Learning: SPEC-2920 以降 gwt は per-user tray singleton。新ビルドの GUI 視覚検証には稼働インスタンスの停止が前提で、エージェント session が gwt-managed pane の場合は停止で session も落ちる。embedded asset は include_str! でビルド時固定のため、稼働中インスタンスを再起動しない限り新 frontend は反映されない。HTTP 層の配線確認 (curl /launch-controls.js, /styles/app.css の新クラス) は serve できれば可能だが、wizard の UI 操作確認には実プロジェクトを開いた新ビルドインスタンスが必須。
Future Action: GUI frontend 変更の視覚検証は、(1) 自動検証 (linkedom unit + cargo + clippy/fmt) を先に green にし commit/push、(2) 視覚検証は『稼働 GWT.app 停止 → cd worktree && ./target/debug/gwt 起動 → browser URL → プロジェクト/ブランチ → 対象 UI』の 導線で user に依頼、(3) session-kill リスクがある場合は user が別ターミナルで実施し結果報告を待つ、という handoff にする。PR は視覚検証 confirmed 後に作成 (PR gate)。分離 HOME 単独起動は project 文脈不足で UI 動線検証には不向き。

## 2026-05-30 — GUI frontend 変更後は target/debug/gwt をリビルドしてから視覚検証する (embedded asset はコンパイル時固定)

Type: lesson
Context: SPEC-2014 Launch Agent コントロール刷新の headed 検証中、隔離 HOME で起動した dev バイナリ (11:01 ビルド) が、その後 11:08 に編集した launch-controls.js の makeSwitch リファクタを含んでおらず、旧版 (sr-only 1px の checkbox) を配信していた。Playwright で toggle が 1x1・クリック intercept され、curl /launch-controls.js で makeSwitch 0 hit を確認して stale と判明。cargo build -p gwt --bin gwt でリビルド後、toggle が 34x18・overlay input が最上位ヒットになりクリック可能に修正された。
Learning: `crates/gwt/web/` 配下の frontend asset (app.js, launch-controls.js, styles/app.css 等) は embedded_web.rs の include_str! でコンパイル時にバイナリへ焼き込まれる。.js/.css を編集しても target/debug/gwt をリビルドしない限り配信される frontend は変わらない。cargo test/clippy はテストバイナリを作るが gwt bin の frontend は更新しない。linkedom unit test はファイル直読みなので GREEN でも binary は古いことがある。駆動検証 (Playwright で実際に操作) しないと、ビルド済み binary の stale frontend や CSS の崩れ (inline span への width/height 無効化など) を見逃す。
Future Action: headed/Playwright 検証の前に必ず cargo build -p gwt --bin gwt を実行し、起動後 curl <url>/launch-controls.js 等で最新マーカー (今回は makeSwitch) が配信されているか確認してから UI 駆動する。headless-browser-check skill の『freshly edited code はビルドしてから』に従う。toggle 等のカスタムコントロールは getBoundingClientRect と elementFromPoint で実寸・ヒット対象を検証し、見た目だけでなくクリック可能性まで Playwright で確認する。

## 2026-05-30 — frontend/backend が別ブランチに分かれた機能は cherry-pick -n で結合ビルド検証してから revert する

Type: lesson
Context: SPEC-2014 で frontend のスライダー (自分のブランチ) と Ultracode reasoning level の backend (別 Agent の work/20260529-0217, d72415b4e) が別ブランチに分かれていた。自分のクリーンブランチ単独では Ultracode が出ず user が『対応できていない』と指摘。データ駆動の主張 (max=stops.length-1) だけでは不十分で、end-to-end の実機証明が必要だった。
Learning: 別 Agent の push 済み commit を git fetch → git cherry-pick -n <sha> で自分の working tree に重ねれば、両者を結合したビルドで end-to-end を実機検証できる (別ファイルなら競合なし)。検証後は git reset --hard HEAD + 新規ファイル rm で完全 revert し、相手の作業を自分のブランチにコミットしない。今回 my slider + their backend で Claude Opus に Ultracode 6 段描画・ドラッグで ultracode コミット・Sonnet 非表示 (version gating 2.1.158>=2.1.154) を確認できた。
Future Action: frontend/backend が別 Agent・別ブランチに分かれる機能では、(1) 役割境界 (別ファイル) を Board で確認、(2) 相手の push 済み backend を cherry-pick -n で重ねた結合ビルドで end-to-end を Playwright 実駆動検証、(3) git reset --hard で revert、の手順で『自分の担当部分が相手の成果と結合して動く』ことを証明してから PR にする。データ駆動の理屈だけで完了報告しない。

## 2026-05-29 — index 検索が pane で使えない時はハッシュ env export を疑う (GWT_REPO_HASH/GWT_WORKTREE_HASH)

Type: lesson
Context: File Index が使われていないとの報告。実機調査で index 自体は構築済み・最新だが、agent pane env に GWT_REPO_HASH/GWT_WORKTREE_HASH が無く、skill の検索コマンドが runner で {ok:false, --db-path is required} となり全スコープ失敗していた (#2933 / SPEC-1939 US-2 AC-11)。
Learning: GWT_PROJECT_ROOT は worktree 解決後の with_project_root() で再注入されるが、ハッシュは LaunchConfigBuilder::build() 時点 (working_dir=None) でしか挿入を試みず後段注入点が無いため欠落していた。chroma_index_runner は main() で --repo-hash 非空のときだけ v2 dispatch に入る設計で、空だとレガシー経路の --db-path 必須に落ちる。
Future Action: ハッシュ系 env は GWT_PROJECT_ROOT と同じ with_project_root に同伴させる。runner は --repo-hash/--worktree-hash 未指定でも --project-root から自己導出する (Rust の repo_hash::normalize_origin_url / worktree_hash とバイト一致で移植)。索引デバッグはまず実機で空ハッシュ再現→正しいハッシュで成功確認→env export 経路を追う順で行う。

## 2026-05-29 — VPN 越し gwt アクセスは loopback 固定 bind が直接の原因 — Phase 4 partial で --bind/--port を復活

Type: lesson
Context: リモート機で起動した gwt に VPN 越しで届かない、と user が報告。実証で ping は通る (RTT 8-9ms) が TCP target port は timeout (silent drop)。コード読みで main.rs:6352 が hardcoded loopback bind であることを確認。SPEC #2920 FR-013 / README は --bind/--port を約束しているが Phase 4 未着手だった。
Learning: VPN 越し / LAN 別端末からの gwt アクセス不可は、loopback bind が直接の原因である可能性が高い。RST ではなく timeout なら loopback または firewall drop。TCP RST なら別の犯人 (service down 等) を疑う。手元 PC からの ping/nc/curl と remote 機の lsof|ss listener 確認で 1 ターンで切り分けられる。一時的な workaround は SSH local port forward (ssh -L <port>:127.0.0.1:<port> user@host)。
Future Action: 同種の質問が出たら、まず (1) ping (2) nc/curl (3) リモートで lsof|ss listener の 3 ステップで切り分けを user に依頼するか、可能なら手元 PC から自分で実行する。loopback 固定が確定したら、Phase 4 partial 実装 (SPEC #2920 の TrayArgs/parse_tray_argv 経路) を案内する。--no-tray / --no-open は parser 受け入れるが no-op の状態を明示する。

## 2026-05-30 — Launch Wizard の installed_version は production で常に None

Type: lesson
Context: Ultracode gating を selected_agent().installed_version >= 2.1.154 で実装したが、手動 GUI 検証で常に非表示だった。load_agent_options が build_agent_options(Vec::new(), ...) を呼ぶため AgentOption.installed_version は production で常に None。AgentDetector::detect_all() は production で Launch Wizard に配線されていない (tests のみ)。
Learning: Launch Wizard は render 時に installed agent version を保持しない。installed_version に依存する gating は常に false になり、自動テストは fixture で version を埋めるため通ってしまう (テスト緑でも実挙動と乖離)。
Future Action: Launch Wizard で installed version 依存の判定が必要な場合は wizard-open 時に検出して context へ格納する (例: claude_ultracode_supported() を context.ultracode_supported に格納)。render hot path で subprocess/IO しない。version 依存 feature は自動テストに加え実 GUI で必ず確認する。

## 2026-06-01 — xterm fontFamily に CSS var() を渡すと canvas 測定が 10px sans-serif に化けて見切れる

Type: lesson
Context: ターミナル文字の縦見切れを lineHeight 1.2→1.28→1.35 と上げ続けても直らなかった (#2903 系譜)。xterm.js 6.0.0 は OffscreenCanvas で ctx.font=`${fontSize}px ${fontFamily}` を設定しセル高を測定するが、fontFamily が 'var(--font-mono), …' だった。
Learning: Canvas 2D の ctx.font は CSS custom property を解決できず、var() を含む font 文字列は丸ごと無効として無視され ctx.font は初期値 '10px sans-serif' のままになる (Playwright 実機計測: varString.normalized='10px sans-serif' boxHeight=10、resolved --font-mono='14px JetBrains Mono Variable…' boxHeight=18 で JBM ground truth と一致)。一方 DOM 行は style.fontFamily で var() を解決し 18px の JBM を描画するため、測定(10)<描画(18) の恒久的不一致で overflow:hidden 行が glyph を切る。lineHeight 倍率は誤った base を倍率するため何度上げても収束しない。
Future Action: xterm の fontFamily オプションには var() を含めない。getComputedStyle(:root).getPropertyValue('--font-mono') で解決した実フォントスタックを渡し、測定フォント==描画フォントに揃える。canvas/OffscreenCanvas に渡す font 文字列全般で CSS 変数を使わない。

## 2026-06-01 — リモートブラウザでは Open Project(rfd) がフリーズ。reopen_recent_project でパス指定 open する

Type: lesson
Context: 新ビルド検証で隔離HOMEのgwtを別プロセス起動しChromeから操作。Open Projectクリックで UI がフリーズ（HTTPサーバーは200で生存）。
Learning: open_project_dialog_events は rfd::FileDialog::pick_folder() を同期呼び出し(crates/gwt/src/app_runtime/mod.rs:4659)。ブラウザサーバーを別プロセス起動して別アプリ(Chrome)経由で操作すると、ネイティブダイアログを前面化できずブロックすることがある。ReopenRecentProject{path}->open_project_path_events はダイアログ不要でパスから開ける。
Future Action: リモートブラウザでプロジェクトを開く必要がある時はOpen Projectを使わず、WS(/ws)へ {kind:'reopen_recent_project', path:'<repo>'} を送る(Playwright page.evaluateでWebSocketを開いて送信)。Open Projectフリーズ自体は別Issue候補。

## 2026-05-31 — startup auto-resume: linked worktree の agent session が workspace-home tab に紐付かず resume されない (#2942)

Type: lesson
Context: 前回終了していないセッションの復元が Stopped のまま起動されない。当初 open_project 経路未配線と誤診したが、実フローは ~/.gwt/session.json の tab 復元 (bootstrap 経路) であり open_project_path は通らない。
Learning: auto_resume_tab_id_for_session が project_scope_hash 一致のみでタブ照合していたが、gwt 管理レイアウトでは workspace home (親 project_root) と linked worktree で repo_hash/scope_hash が異なる (例: 親=b19aac, worktree=99a866) ため worktree 由来の agent session を親 tab に紐付けられず queue されなかった。session 状態の正本は session-state.json ではなく ~/.gwt/session.json (gwt_session_state_path)。
Future Action: worktree とプロジェクトの関連付けは repo_hash/project_scope_hash 比較ではなく gwt_git::worktree::main_worktree_root() の一致で判定する。復元/resume バグ調査では、静的推測でなくライブで各ゲートの発火 (DEBUGQ ログ) を確認して実際に skip しているゲートを特定してから修正する。

## 2026-06-01 — PR Gate: ユーザーの曖昧な質問を視覚検証 confirmed と解釈して PR を先走り作成しない

Type: lesson
Context: #2942 で、ユーザーが視覚検証スクショ送信後に『何が残っているのですか？』と質問。これを承認と解釈して PR #2947 を作成したが、明示的な『OK/問題なし』は未取得だった（PR Gate 手順違反、PR #2857 と同型）。
Learning: 『何が残っているのか』『これで合っているのか』等の曖昧な質問・確認要求は User Verification Result: confirmed ではない。PR Gate は『confirmed』または『n/a』、もしくはユーザーが明示的に skip を選んだ場合のみ満たされる。解釈による前倒しは違反。
Future Action: PR create/update は、ユーザーが literal に『OK / 問題なし / confirmed / skip 承認』を述べるまで実行しない。曖昧な質問には『PR 作成には明示的な OK が必要』と返し、承認を待つ。誤って作成したら即 [DO NOT MERGE — user verification pending] をタイトルに付与しブロック comment、confirmed 後にタイトル復元。

## 2026-06-01 — 実環境 GUI 検証: GWT.app と共存起動するには debug ビルドで single-instance lock を一時無効化＋実 HOME（claude 認証は Keychain）

Type: lesson
Context: #2942 で、隔離 HOME の gwt は claude 認証が通らず（token は macOS Keychain にあり HOME 非依存だが、隔離 HOME 起動の claude は 'Not logged in' / 'No conversation found' になる）クリーンな視覚検証ができなかった。実 HOME は per-user single-instance tray lock（main.rs 6273、--no-tray でも無条件）で GWT.app と共存できず即終了。
Learning: 実環境でクリーンに認証された GUI 検証をするには、(1) debug ビルドで single-instance lock を一時 cfg(debug_assertions) skip（別パス debug-coexist で handle 取得）し GWT.app と共存、(2) 実 HOME で起動して実 ~/.claude + Keychain 認証を効かせる。ただし実 HOME 起動は session.json の全タブ・全ペインを resume するため、他の稼働中エージェントのセッションも claude --resume で二重起動し干渉しうる。検証専用変更はコミットしない。
Future Action: GUI の実環境視覚検証が必要で GWT.app を閉じられない場合: debug-only の lock-skip を一時適用→実 HOME で起動→目視→即停止→lock-skip を revert。他エージェントの session を巻き込むため最短時間で停止する。検証コードは PR に含めない（revert 必須）。debug ビルドでの恒久 lock-skip は別 Issue で検討。

## 2026-06-01 — gwt GUI 修正の視覚検証は HOME 隔離で dev build を起動し lsof で配信プロセスを確認する

Type: lesson
Context: #2948 検証で ./target/debug/gwt を起動したら single-instance lock($HOME/.gwt キー)で既存 tray インスタンス(installed /Applications/GWT.app)を検知し、既存 URL(53425)を表示して自身は終了していた。lsof で 53425 の listener が installed app(修正なし)と判明。ユーザーに『その URL に修正は入っていないのでは』と的確に指摘された。
Learning: 2つ目の gwt は single-instance lock(main.rs: gui_single_instance + cli::tray::lock, どちらも gwt_home=$HOME/.gwt キー)により installed app へ defer し、自分のバックエンドを配信しない。dev build を独立起動するには HOME を隔離(例: worktree 内 ./.gwt-verify-home)して gwt_home を分離する。隔離 HOME は ~/.bun ~/.npm cache も空=cold になるので cold 再現検証にも使える。プロジェクトは ReopenRecentProject(任意パス)で WS 経由で開け、Start Work は git origin remote を要求する。関連: 同 single-instance lock を debug build の lock-skip + 実 HOME で回避する手法も別 entry にあり(認証が必要な検証向け)。
Future Action: GUI 修正の視覚検証で dev build を案内する前に、必ず HOME 隔離起動し『lsof -nP -iTCP:<port> -sTCP:LISTEN』で listener PID=自分の ./target/debug/gwt であることを確認してから URL を共有する。installed app と同居する素の起動は修正が反映されない。

## 2026-06-01 — リリース中に origin/develop が他Agentマージで移動した場合は ff 後に CHANGELOG/version を再生成する

Type: lesson
Context: /release 実行中、pull 後に別Agentが PR #2950 を develop にマージし origin/develop が前進。最初のリリースコミット(古い develop ベース)は #2950 の fix を含まず、push も non-fast-forward で不成立だった。
Learning: リリースコミットは push 直前時点の origin/develop に直接乗っている必要がある。origin が動いたら release commit を reset → origin/develop に --ff-only → version/CHANGELOG を git-cliff で再生成 → 再コミット、で新規マージ分を取り込む。背景 git push は完了報告が遅延するため push 後に必ず origin/develop == HEAD を fetch 確認してから成功宣言する。
Future Action: /release の push 前に git fetch + 'HEAD~1 == origin/develop' を確認し、不一致なら reset+ff+再生成。push 後も fetch で origin/develop が release commit と一致するまで成功を宣言しない。

## 2026-06-01 — Claude Code latest should prefer npx over bunx on non-Windows

Type: lesson
Context: Claude Code @anthropic-ai/claude-code@latest failed to launch from GWT when selected as latest because bunx one-shot package execution resolved dependencies but left Claude Code's postinstall-managed native binary as a stub, producing could not determine executable / EEXIST style launch failures.
Learning: For built-in Claude Code npm-backed launches on non-Windows, npx --yes is the reliable package runner while bunx remains a fallback; Codex and custom Bunx flows should keep their existing bunx-first behavior.
Future Action: When changing package-runner launch code, add agent-aware tests that cover Claude Code npx preference, bunx fallback, and Codex/custom Bunx compatibility before touching production runner selection.

## 2026-06-01 — Agent terminal overlays must stay disabled

Type: lesson
Context: A user-reported Agent window showed a white foreground terminal overlay after prior work intended raw TTY display. The leftover pre-output error path still toggled .terminal-overlay.visible.
Learning: When the product decision says Agent terminals show raw TTY, do not keep exception paths for running or pre-output error overlays. Status details belong in chrome/logs/banners, not xterm foreground overlays.
Future Action: For terminal UI changes, assert shouldShowOverlay is false for Agent status details and check both running and error paths before claiming the overlay is removed.

## 2026-06-01 — Removed gwt serve guidance must not remain in agent skills

Type: lesson
Context: The local headless-browser-check skill still instructed agents to run the removed gwt serve command, which caused a failed verification launch after SPEC #2920 removed that route.
Learning: Generated or local skills can outlive product CLI changes. When a command is removed, update the skills, README, diagnostics, live-test comments, and memory entries that agents may reuse, not only production code and tests.
Future Action: Before using a remembered launch command for gwt UI verification, search the current checkout for the command and prefer the canonical README/runtime usage hint; never run gwt serve or gwt --headless except in removal regression tests.

## 2026-06-01 — Pre-PTY launch errors must enter the terminal transcript

Type: failure-pattern
Context: Agent TTY overlay had been intentionally disabled, but Launch completion errors before PTY spawn were only sent as TerminalStatus detail. The UI then showed an Error badge while xterm remained blank.
Learning: When a process fails before a PTY exists, status/detail chrome is not enough. Emit a normal terminal_output diagnostic and replay it through terminal_snapshot on frontend reconnect; do not reintroduce foreground overlays.
Future Action: For any future launch/startup failure path that can happen before PTY output, add tests for both immediate TerminalOutput and reconnect TerminalSnapshot visibility.

## 2026-06-01 — Seed project tabs for isolated HOME visual verification

Type: failure-pattern
Context: SPEC-2785 status strip visual verification launched target/debug/gwt with a temporary HOME to avoid the user production GWT.app tray lock. The server was reachable, but the UI stopped at Open Project because the isolated HOME had an empty ~/.gwt/session.json.
Learning: A reachable fresh gwt URL is not enough for visual verification when HOME is isolated. If the target UI surface requires an opened project, the isolated session.json must contain the current checkout project tab before asking the user to verify.
Future Action: For headless-browser-check or fresh checkout UI verification with temporary HOME, pre-seed ~/.gwt/session.json with the current worktree project tab, then verify via browser automation that the page is past Open Project and the requested UI surface is visible before sharing the URL.

## 2026-06-01 — frontend root module route parity

Type: lesson
Context: SPEC-1919 added /terminal-copy-shortcut.js as a root module imported by crates/gwt/web/app.js. Frontend unit verification first failed at the Playwright embedded routes coverage because ROOT_MODULES did not include the new file.
Learning: When adding a root-level web module imported by app.js, keep three contracts in sync: crates/gwt/src/embedded_web.rs asset registry, scripts/run-frontend-unit-tests.sh coverage, and crates/gwt/playwright/tests/_helpers/embedded-frontend.ts ROOT_MODULES.
Future Action: Before final verification for app.js root imports, run scripts/run-frontend-unit-tests.sh and check the Playwright embedded route parity test instead of assuming the Rust embedded registry is sufficient.

## 2026-06-01 — gwt crate は bin/lib で別クレートルート: 共有モジュールは bin から gwt:: で参照

Type: lesson
Context: main.rs(bin gwt) は独自の mod ツリー(app_runtime, board_view 等)を持ち、共有モジュールは gwt::<mod> で参照する。一方 `cli/*` は lib.rs 配下で crate::<mod> が正しい。
Learning: gwt crate は [[bin]] gwt=src/main.rs と lib.rs が別クレートルート。bin 側ソース(app_runtime/board_view)からは lib のモジュールを gwt:: で参照し、lib 側ソース(cli/*)は crate:: で参照する。
Future Action: lib に新規 pub mod を足して bin から使う場合は gwt::<mod>、lib 内から使う場合は crate::<mod> を使い分ける。コンパイル前に呼び出し元が bin/lib どちらかを確認する。

## 2026-06-02 — Board provider tests read the global ~/.gwt/config.toml

Type: lesson
Context: board_provider::provider() reads Settings::load() (global config) on every call. cli::board + cli::hook::board_reminder + cli::env unit tests exercise provider() indirectly. dirs::home_dir() (used by Settings::global_config_path) ignores HOME/USERPROFILE env on Windows (dirs 6), so these tests CANNOT be isolated from the machine config via ScopedEnvVar HOME/USERPROFILE.
Learning: If the dev machine's config.toml has [board] provider = slack/teams, ~33 board lib tests fail with 'Slack is not signed in' because provider() resolves a remote provider with no token. This is collateral, not a code defect. Attempting env-based HOME isolation in these tests caused ordering-dependent regressions (env::set_var races across parallel/sequential tests).
Future Action: Keep the dev machine ~/.gwt/config.toml at provider = local while running 'cargo test -p gwt'. Switch to slack/teams only for manual GUI e2e, then switch back. Do NOT add ScopedEnvVar HOME guards to board tests to work around it. Configure Slack/Teams via the Settings UI (BackendEvent UpdateBoardProviderConfig), not by editing config.toml.

## 2026-06-02 — Slack OAuth v2 authorize URL requires comma-separated scopes and no response_type

Type: lesson
Context: gwt Board Slack provider sign-in (SPEC-2963). build_authorize_url in crates/gwt/src/board_remote/oauth.rs originally joined scopes with a space and unconditionally added response_type=code for all providers.
Learning: Slack OAuth v2 (/oauth/v2/authorize) needs scope as a COMMA-separated list; a space-separated list is parsed as one invalid scope and Slack fails with 'No scopes requested'. Slack also does NOT define response_type (it nudges toward the OIDC sign-in flow); omit it. Microsoft/Teams is the opposite: space-separated scopes + response_type=code required. Also: Slack DOES accept `http://127.0.0.1` loopback redirect URLs (no https forced), but the redirect_uri must match the registered URL EXACTLY incl. host (127.0.0.1 != localhost), port, /oauth/callback path, no trailing slash, and the app must be reinstalled after changing Redirect URLs/scopes.
Future Action: For any OAuth provider, branch the scope separator and response_type by provider (Slack: comma + no response_type; MS: space + response_type=code). When a desktop app uses a loopback redirect, the port must be STABLE/pre-registered — gwt's default ephemeral server port breaks OAuth, so OAuth needs a fixed callback port (follow-up). Tell users to register 127.0.0.1 (not localhost), click Save URLs, and Reinstall to Workspace.

## 2026-06-02 — Slack bot must join the channel before conversations.history / chat.postMessage

Type: lesson
Context: gwt Board SlackProvider (SPEC-2963). After OAuth sign-in succeeds and the xoxb- bot token is stored, reading the Board surfaced 'slack conversations.history error: not_in_channel' for the configured default channel.
Learning: A Slack bot token with channels:history/channels:read/chat:write still cannot read history or post to a channel unless the bot is a MEMBER of that channel. conversations.history returns not_in_channel and chat.postMessage fails until the bot joins. chat:write.public would allow posting to public channels without joining, but reading history always requires membership.
Future Action: Tell users to invite the gwt bot to the target channel (/invite @gwt, or channel Integrations > Add apps) after sign-in. Optional enhancement: on not_in_channel, have SlackProvider call conversations.join (needs channels:join scope added to SLACK_SCOPES + re-auth) and retry, for public channels only.

## 2026-06-02 — OAuth redirect port only matters during sign-in, not ongoing Board ops

Type: lesson
Context: gwt Board remote provider (SPEC-2963). The fixed OAuth callback port (default 8765) is used to build redirect_uri for authorize + code->token exchange.
Learning: redirect_uri/port is only consumed during the interactive sign-in flow (authorize request + oauth token exchange). After the access token is stored, all Board read/write use the Bearer token with no redirect, so changing/losing the port does NOT break an existing session. Slack bot tokens (xoxb-) typically never expire (no refresh_token). Microsoft/Teams tokens expire but refresh uses grant_type=refresh_token with NO redirect_uri, so refresh needs no port either. The port is needed again only for a fresh sign-in (sign-out -> sign-in, or after refresh_token revocation).
Future Action: When reasoning about the fixed-port requirement: it must be stable/registerable at sign-in time only. Post-auth port changes are safe for existing sessions; only the next sign-in needs the new redirect URL registered. Do not over-engineer port stability for ongoing operation.

## 2026-06-02 — Teams Entra app must be public client (Mobile/desktop), not Web

Type: lesson
Context: gwt Teams Board provider OAuth (SPEC-2963). Audited against MS docs before user E2E.
Learning: Teams uses delegated auth-code + PKCE with NO client_secret, so the Entra app MUST register the `http://127.0.0.1:8765/oauth/callback` redirect under the 'Mobile and desktop applications' (public client) platform with 'Allow public client flows'=Yes. Registering under 'Web' makes Entra treat it as a confidential client and the secret-less token exchange fails with AADSTS invalid_client. The Azure portal may reject http-loopback in the redirect textbox; add via app Manifest replyUrlsWithType type=InstalledClient, and note loopback port is ignored for matching (`http://127.0.0.1/oauth/callback` matches any port). The signed-in user must be a member of the target team/channel or Graph returns 403 (Teams analogue of Slack not_in_channel). Graph 'list channel messages' is NOT a metered/protected API (Teams APIs unmetered since 2025-08-25) - no Microsoft approval gate; do not confuse with channel.getAllMessages export.
Future Action: When documenting/supporting Teams OAuth: emphasize public-client/Mobile-desktop registration + Allow public client flows + 127.0.0.1 exact host + membership. gwt code is correct (response_mode=query pinned, contentType=text, system/deleted filtered, 403 self-diagnoses).

## 2026-06-02 — Teams Board provider E2E verified against real Microsoft Graph

Type: lesson
Context: gwt SPEC-2963 Teams provider. User registered Entra public-client app (client_id 164f7884..., tenant 0ff7b59c...), signed in via gwt; agent verified E2E.
Learning: Teams delegated OAuth + Graph post/read/reply works end-to-end: gwt posts a channel message via TeamsProvider, read back via GET /teams/{team}/channels/{chan}/messages matches, and --parent reply lands as a Graph reply with replyToId=parent. channel_id can be @thread.skype (older) as well as @thread.tacv2; gwt split_channel(team/channel) handles it. Note: gwt does NOT request User.Read, so Graph /me returns Authorization_RequestDenied — this is expected and does NOT mean the token is invalid; ChannelMessage.Send/Read.All operations succeed. The fixed OAuth callback port (8765) + Mobile/desktop public-client Entra registration worked.
Future Action: All three Board providers (Local/Slack/Teams) are now E2E-verified. For Teams verification: check post/read/reply via Graph directly, not /me (which needs User.Read gwt doesn't request).

## 2026-06-03 — gwt bin tests read real ~/.gwt/config.toml board.provider (cfg(test) seam is lib-only)

Type: lesson
Context: SPEC-2963 検証中、cargo test -p gwt --bin gwt の app_runtime board テスト8件が config.toml の provider=teams で失敗。board_provider::current_kind() の cfg(test) thread-local override(default Local)は gwt LIB を --test ビルドした時のみ有効。bin(main.rs/app_runtime)テストは LIB を通常依存としてリンクするため override が効かず、Settings::load().board.provider(実機 config)を読む。Windows の dirs 6 は HOME/USERPROFILE を無視するため ScopedEnvVar による隔離も効かない。
Learning: bin crate のテストは LIB の #[cfg(test)] seam に到達できない。machine の ~/.gwt/config.toml に board.provider=slack/teams が設定されていると bin board テストが remote provider を使い失敗する。CI は board.provider 未設定→default Local なので緑。
Future Action: gwt の board 関連 bin テストをローカル実行する前に ~/.gwt/config.toml の [board] provider を local(または未設定)にする。lib テスト(cargo test -p gwt --lib)は cfg(test) override で hermetic なので config 非依存。恒久対策が必要なら current_kind() に non-test でも効く env override seam を入れて bin テストで設定する案を検討。

## 2026-06-01 — Isolated HOME live checks must avoid Start Work auth traps

Type: lesson
Context: During SPEC-2008 manual visual verification, current checkout was launched with isolated HOME to avoid tray lock collisions. The empty or partially linked HOME caused Start Work to attempt git push origin origin/develop:refs/heads/work/... without usable GitHub credentials, yielding fatal: could not read Username for `https://github.com`: terminal prompts disabled before the intended scroll check.
Learning: For UI verification that only needs an existing worktree, do not drive Start Work in an isolated HOME unless Git/GitHub credentials are fully available and noninteractive. Prefer seeding session.json for the current worktree and using OpenActiveWorkLaunchWizard on the existing branch, or explicitly link required agent/git config while avoiding remote branch creation.
Future Action: When preparing manual verification URLs with isolated HOME, document that the user should not use Start Work/Open Project for the check, close any failed remote-branch windows, and provide a pre-launched running agent or deterministic fixture for the exact UI behavior under test.

## 2026-06-01 — Claude Code agent scrollback requires no-flicker off

Type: lesson
Context: SPEC-2008 agent window scroll regression: CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1 alone still left Claude Code 2.1.159 in a fullscreen/no-flicker style renderer where xterm viewport scrollHeight equaled clientHeight and wheel did not move visible history.
Learning: Claude Code may keep virtual scroll history inside its own renderer unless CLAUDE_CODE_NO_FLICKER=0 is also set. In this mode xterm viewport metrics may still look non-scrollable, so verify by observing screen content change after wheel input and by checking process env.
Future Action: For future Claude Code terminal scroll fixes, set and test both CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1 and CLAUDE_CODE_NO_FLICKER=0 while preserving explicit user/custom overrides; live verification should include process env plus wheel-driven content change.

## 2026-06-01 — Isolated HOME Claude verification needs claude.json

Type: lesson
Context: During SPEC-2008 live debug verification, isolated HOME linked ~/.claude but initially omitted ~/.claude.json, causing Claude Code to show login/setup instead of the authenticated agent UI.
Learning: Claude Code authentication/config in this environment depends on ~/.claude.json as well as ~/.claude. Linking only the directory can produce a misleading launch blocker unrelated to the feature under test.
Future Action: When preparing isolated HOME live GUI verification for Claude Code, link both ~/.claude and ~/.claude.json before launching gwt, then confirm the agent reaches the normal TUI before testing the target behavior.

## 2026-06-01 — Debug browser Open Project can require killing the verification gwt process

Type: lesson
Context: During SPEC-2008 user visual verification, the user clicked Open Project in a browser-served debug URL. The current checkout gwt process still answered HTTP 200, but Open Project had entered the native rfd FileDialog path and normal TERM did not stop the process; INT plus KILL was needed before starting a fresh URL.
Learning: In browser-served verification, Open Project is not a safe recovery path. It can leave the validation server in a partially blocked native-dialog state even when HTTP health checks pass.
Future Action: For future debug-browser verification, pre-open the target project and tell the user not to click Open Project. If they do, restart only the verification gwt process from the current checkout and issue a fresh URL; do not treat HTTP 200 as proof the user tab is usable.

## 2026-06-01 — Claude fullscreen TUI wheel needs PageUp/PageDown fallback

Type: lesson
Context: SPEC-2008 agent window scroll regression persisted after setting CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1 and CLAUDE_CODE_NO_FLICKER=0. Live debug URL showed wheel reached .xterm-screen and was not canvas-prevented, but xterm metrics stayed baseY=0 and scrollHeight=clientHeight, so there was no normal scrollback for wheel to move.
Learning: Claude Code can render as a fullscreen TUI with internal scroll even when launch env asks for classic/no-alt behavior. In that state, xterm scrollback metrics are the wrong success criterion; wheel must be translated to the same terminal input that PageUp/PageDown sends, scoped only to agent presets and only when normal xterm scrollback is absent.
Future Action: For future agent terminal scroll issues, measure wheel delivery, xterm baseY/scrollHeight, and PageUp/PageDown behavior separately. If baseY=0 but PageUp changes the TUI, add or verify an agent-only wheel-to-PageUp/PageDown fallback instead of continuing viewport reflow fixes.

## 2026-06-01 — Terminal reconnect snapshots must replay scrollback

Type: lesson
Context: SPEC-2008 Phase 26.F: user could scroll only after maximizing the agent window because fresh frontend ready / reconnect snapshots contained only the current vt100 visible screen.
Learning: A normal-size agent window with xterm baseY=0 can mean the backend snapshot did not replay scrollback; maximizing can create misleading xterm scrollback through redraw and hide the real source of the bug.
Future Action: For terminal scroll bugs, measure baseY/viewportY on fresh connect and verify backend snapshot composition includes scrollback before changing wheel routing or resize behavior.

## 2026-06-01 — SPEC section edits must stay sequential

Type: workflow
Context: During SPEC-2013 Phase 6, parallel gwtd issue spec --edit calls on spec/plan/tasks caused one section update to be overwritten and required sequential re-application.
Learning: Same-issue SPEC section edits share one issue body and can race if run in parallel.
Future Action: For one SPEC issue, read, patch, write, and re-read each section sequentially; never use multi_tool_use.parallel for gwtd issue spec --edit.

## 2026-06-01 — Agent title updates must resolve canonical Project State root

Type: lesson
Context: SPEC-2359 Phase W-10: gwtd workspace update --agent-session was writing title/focus into the linked worktree Project State root while the live GUI watched the Workspace Home Project State root.
Learning: Do not use an agent worktree path as the implicit Project State identity. Persist Session.project_state_root during GUI launch, route CLI/hook reads and writes through that canonical root, and repair old split same-session projection data by updated_at.
Future Action: Before changing Agent title, Workspace, hook, or Project State behavior, add a regression test with a Workspace Home project root and a linked worktree agent so canonical-root and worktree-root writes cannot diverge again.

## 2026-06-01 — Fresh browser checks must never share production gwt URLs

Type: lesson
Context: User corrected the headless-browser-check workflow after it printed an existing tray-resident production URL. The desired verification URL must come from the modified checkout's own freshly launched server.
Learning: Browser verification skills for gwt must isolate HOME/USERPROFILE, launch the current checkout's target/debug/gwt with --no-tray --no-open, seed session.json for the checkout, and reject any URL reported after an existing tray instance warning.
Future Action: When providing a gwt verification URL, use the browser-check workflow and prove the URL comes from the fresh process's GWT_BROWSER_URL_FILE plus HTTP 200 before sharing it.

## 2026-06-01 — Fresh browser verification should not use Start Work unless credentials are proven

Type: lesson
Context: During gwt-fresh-browser-check, the user saw a failed Claude Code window because the isolated HOME Start Work path tried to create remote branch origin/work/20260601-1042 and git push failed with terminal prompts disabled.
Learning: Fresh browser checks isolate HOME and set GIT_TERMINAL_PROMPT=0, so Start Work can fail on GitHub HTTPS authentication even when the app under test is otherwise fine. Verification should avoid Start Work unless the feature under test requires it and branch creation credentials are preflighted.
Future Action: For gwt fresh UI checks, seed the target project/window or launch on the current branch path; if a failed remote-branch Agent window appears, close it and treat it as verification setup noise rather than feature evidence.

## 2026-06-01 — Project-local skills do not need repository prefix

Type: workflow
Context: The browser verification skill was renamed to `gwt-fresh-browser-check` even though it lives inside this gwt repository's project-local skill set. The user corrected that `gwt-*` prefixes are redundant for gwt development skills.
Learning: For project-local skills, the repository context already supplies the namespace. Use concise action/domain names and keep the directory name, frontmatter name, and in-skill title aligned.
Future Action: Before naming or renaming a project-local skill, check whether the skill location already implies the repository scope; avoid redundant repository prefixes such as `gwt-*` unless the user explicitly requests one.

## 2026-06-01 — Use concise browser-check skill name

Type: workflow
Context: After removing the project-local gwt- prefix, the skill was still named fresh-browser-check. The user clarified that browser-check is sufficient.
Learning: When the skill's behavior already says it must launch a fresh isolated server, the public skill name does not need to include implementation qualifiers like fresh. Prefer the concise user-facing trigger name.
Future Action: Name this browser verification skill browser-check in project-local skill directories, with freshness and isolation requirements documented inside SKILL.md rather than encoded in the skill name.

## 2026-06-01 — Hidden attribute can be overridden by component display rules

Type: lesson
Context: SPEC-2009 Branches notice hotfix: .branch-notice used display:grid, so a hidden notice still rendered as an empty red band after branch detail checking completed.
Learning: When a component class sets display explicitly, hidden elements need an explicit selector such as .component[hidden] { display: none; } and a visual regression contract, because the class rule can override the UA hidden style.
Future Action: For UI surfaces with reusable notice/banner components, add hidden-state display contracts in both static CSS tests and browser/UI tests whenever the component sets display.

## 2026-06-01 — FrontendReady must replay nullable singleton tombstones

Type: lesson
Context: Launch Wizard close recovery after heavy Launch Agent processing / Event Hub queue overflow investigation on 2026-06-01.
Learning: A successful mutation can emit a close event, but bounded ClientHub overflow may disconnect a slow WebView before it receives the frame. Reconnect recovery must be authoritative for latest-wins nullable singleton state; omitting None/tombstone payloads leaves stale frontend UI.
Future Action: When adding nullable latest-wins frontend state, make FrontendReady reply with both Some(current state) and None(tombstone), and add RED reconnect-sync coverage before relying on one-shot broadcast close events.

## 2026-06-01 — User-accepted visual verification for rare load-only UI failures

Type: lesson
Context: Launch Wizard close recovery after reconnect depended on an extreme terminal load condition that was hard for the user to reproduce manually after live E2E RED/GREEN coverage existed.
Learning: When a UI failure is rare and load-dependent, deterministic E2E evidence can be the strongest practical proof; if the user explicitly accepts that evidence, record the acceptance in the SPEC instead of keeping the work blocked on manual reproduction.
Future Action: For future rare load-only UI bugs, add a deterministic E2E that models the lost or delayed event path, record RED/GREEN evidence, then ask the user whether that evidence is sufficient when manual reproduction is impractical.

## 2026-06-01 — Seed session for isolated GUI verification

Type: lesson
Context: SPEC-2920 tray/About verification used an isolated HOME. With an empty ~/.gwt/session.json, the app opened the Open Project picker instead of the intended checkout surface.
Learning: Isolated GUI verification that must land on an in-project surface needs a seeded session.json pointing at the checkout under test; otherwise the verification can be blocked before the changed UI is reachable.
Future Action: Before sharing a manual GUI verification URL from a temp HOME, seed ~/.gwt/session.json with the target checkout tab and verify the served URL reaches the intended screen.

## 2026-06-02 — gwt-register-spec: register phase/complete は owned --spec id を使う（start --spec 0 なら 0）

Type: lesson
Context: register start --spec 0 で owner_spec=0。その後 register phase --spec <real-n> は "state owns SPEC-Some(0), got --spec <n>" で拒否される。CLI は phase で placeholder→実 id の rebind をしない（SKILL.md の "bind the real spec id" 文言と挙動が不一致）。
Learning: 実 SPEC Issue は issue spec create / issue spec <n> --edit spec で正しく作成・投入される。register の lifecycle 記録だけが owned id に紐づく。phase/complete は --spec 0（owned id）で実行すれば milestone 記録と stop-block 解除ができる。
Future Action: gwt-register-spec を register start --spec 0 で開始した場合、phase/complete は新 issue 番号ではなく --spec 0 を使う。または issue 番号確定後に register start を実 id で 1 回だけ実行する。

## 2026-06-02 — SPEC-2970 Usage 検証: 実 Claude /api/oauth/usage は 200 で取得可・gwt 二重起動は GWT_FORCE_NEW_INSTANCE=1

Type: lesson
Context: Provider Usage 機能の視覚検証で、(1) 既存 gwt が single-instance lock を持つため検証用 2 つ目を起動できなかった、(2) Claude account usage が取れるか不明だった、(3) 検証用 session を seed する際 session.toml の agent_id 形式でつまづいた。
Learning: (1) 2つ目の gwt 検証インスタンスは GWT_FORCE_NEW_INSTANCE=1 と --no-tray --no-open で起動できる（HOME 隔離 + CODEX_HOME=実ディレクトリ で実 Codex を読ませる）。(2) 実トークン(Keychain `security find-generic-password -s "Claude Code-credentials" -w` の claudeAiOauth.accessToken)で `GET https://api.anthropic.com/api/oauth/usage` は HTTP 200。応答は five_hour/seven_day/seven_day_sonnet を含み、resets_at は RFC3339 `+00:00` オフセット、seven_day_opus 等は null、未知の sub-window キー多数。(3) Session の agent_id は serde adjacently-tagged (`#[serde(tag="type",content="value")]`) なので toml では `agent_id = { type = "Codex" }`。
Future Action: GUI 視覚検証で本番 gwt と衝突する場合は GWT_FORCE_NEW_INSTANCE=1 + 隔離 HOME + CODEX_HOME 実ディレクトリで起動する。Claude usage パーサは null sub-window と +00:00 オフセットと未知キーを許容する defensive parse を維持する。

## 2026-06-02 — Claude usage: macOS は Keychain の token が live、~/.claude/.credentials.json は stale/expired のことがある

Type: lesson
Context: SPEC-2970 で Claude account usage が 401 auth expired になった。原因は resolve_claude_creds が .credentials.json を先に読み、その accessToken が expiresAt 過去で失効していたため。Keychain (security find-generic-password -s "Claude Code-credentials" -w) の token は live で同 endpoint が 200 を返す。
Learning: macOS では Keychain が live token の真実。resolve は Keychain 優先 → file fallback にする。ただし GUI でない detached プロセスから security を叩くと keychain ACL prompt が応答できず失敗し得る（実 GUI アプリは初回 Always Allow で解決）。headless 検証では CLAUDE_CONFIG_DIR を worktree 内一時ディレクトリに向け、Keychain から取り出した最新 token を .credentials.json として置けば file fallback で 200 を再現できる。claude_home は CLAUDE_CONFIG_DIR env を尊重する。
Future Action: Claude token は Keychain 優先・file fallback。失効 access token のときは将来 refreshToken での更新も検討。検証時は CLAUDE_CONFIG_DIR + 一時 .credentials.json で実データ再現し、token file は確認後に削除する。

## 2026-06-02 — Removing a derivation path: check sibling reminder guards for the same is_unassigned early-return

Type: lesson
Context: SPEC-2359 W-11: removed the UserPromptSubmit prompt→title derivation. Unassigned Start Work agents then got no title at all because board_reminder::agent_title_summary_missing still had an is_unassigned() early-return that suppressed the title reminder. The derivation path had already dropped that guard (US-46/FR-179) but the reminder path had not.
Learning: When you remove one code path that handled a case (e.g. derivation for unassigned agents), grep for the SAME guard (is_unassigned / affiliation early-returns) in sibling paths (reminders, sync) that must now cover the case. A guard that was harmless while the derivation existed becomes a silent gap once it is removed.
Future Action: After deleting a path that produced some state, search for every other gate keyed on the same condition (e.g. grep is_unassigned) and confirm each still behaves correctly without the deleted path.

## 2026-06-02 — browser-check of hook-driven agent behavior needs keychain symlink + GWT_HOOK_BIN

Type: lesson
Context: Verifying SPEC-2359 W-11 title behavior in an isolated browser-check instance hit 3 env-only blockers: (1) Start Work git push failed because the macOS login keychain lives at $HOME/Library/Keychains and the isolated HOME had none; (2) materialized agent hooks resolved to the installed /Applications/GWT.app gwtd (old code) not the rebuilt target/debug/gwtd; (3) my standalone CLI hook sim io-errored on the daemon-forward step which only works inside the launched agent.
Learning: Isolated-HOME browser-check needs: symlink $CHECK_HOME/Library/Keychains -> $HOME/Library/Keychains so osxkeychain can auth git push; set GWT_HOOK_BIN=<repo>/target/debug/gwtd so new worktrees' hooks run the rebuilt binary; verify agent behavior via the projection (CHECK_HOME/.gwt/projects/*/current.json) and the Claude transcript, not a standalone CLI hook invocation (daemon-forward step needs the launch env).
Future Action: When browser-check must exercise agent hooks against edited Rust, symlink the keychain into the isolated HOME, launch with GWT_HOOK_BIN pointing at target/debug/gwtd, and confirm outcomes by reading the projection + transcript.

## 2026-06-02 — gwt-managed skill ファイル編集は dual-mirror + force-add

Type: workflow
Context: gwt-fix-issue SKILL.md 強化で新規 references/closure-comment.md を追加した際、git status に出ず原因調査した。
Learning: `.claude/skills/gwt-*` と `.codex/skills/gwt-*` は .git/info/exclude で除外されており、新規ファイルは untracked 扱い。既存 tracked ファイル(SKILL.md 等)の編集は通常反映される。.codex は distribute.rs が embedded .claude を逐語コピーするが tracked-path 保護で上書きされない手動ミラーで、参照パスのみ .codex/ に書き換える。
Future Action: skill 編集時は .claude と .codex の両ミラーを同一コミットで更新し、新規 managed skill ファイルは git add -f で tracked 化する。SKILL.md 内の自己参照パスは mirror 側で .codex/ prefix にする。

## 2026-06-02 — SPEC-2970 Claude usage は opt-in 既定 OFF が consent 正。default-on は外部送信を同意なしに発火させる

Type: lesson
Context: Provider Usage 実装で Claude account 既定を ON にしたところ Codex 自動レビューが P1 指摘: [usage] 未設定の既存ユーザーが GUI 接続直後に opt-in 同意なしで Keychain 読取 + Anthropic /api/oauth/usage 送信を受ける。承認済み SPEC の同意モデルは『Claude アカウント枠のみ opt-in』だった。
Learning: 外部送信/資格情報読取を伴う機能の既定値は『承認済み SPEC の consent 契約』に従う。デバッグ中の『既定で見たい』要望で default-on にすると spec と矛盾し privacy regression になる。フラグgate は呼び出し経路の最前段(early-return)に置き、未同意時は資格情報にも通信にも一切触れないことを敵対的監査で確認する。per-session のローカル読取は opt-in 不要(FR-017)。
Future Action: consent を伴う設定の既定は false(opt-in)。SPEC FR の同意モデルとコード default を必ず一致させ、UI 説明文(Settings hint)・FR 全箇所の『既定で有効』表現も同時に掃き出す。視覚検証は隔離 HOME(未 opt-in 状態)で『Enable in Settings』が既定表示されることを確認する。

## 2026-06-02 — 最大化など per-client 表示状態を共有ジオメトリに broadcast すると複数クライアントでチラつく

Type: lesson
Context: SPEC-2008 の最大化で、ユーザー検証中に激しいチラつきが発生。調査の結果、syncMaximizedWindowsToViewport が各クライアントの可視領域に合わせて共有の最大化ジオメトリへ maximize_window 補正を broadcast しており、異なる viewport サイズの 2 クライアント（検証用 MCP ブラウザ1200px + ユーザーのブラウザ810px）が同時接続するとジオメトリを往復させ続けた。inset 値に依存しない既存設計問題。
Learning: 最大化の塗りつぶしのような per-client な表示状態を shared workspace geometry に書き戻して全クライアントに broadcast すると、サイズの異なるクライアント間で ping-pong してチラつく。各クライアントが visibleBounds から fill をローカル計算してローカル適用し、共有するのは maximized フラグだけにすると解消する。さらに、エージェント検証時に検証用ブラウザ(Playwright/MCP)を開いたままユーザーに視覚確認を依頼すると、それが第2クライアントになり multi-client バグを誘発し『回帰』に見える。
Future Action: (1) 最大化/ズーム追従など viewport 依存の表示はローカル描画にし、shared geometry へ補正を broadcast しない。(2) ユーザーに視覚確認を依頼する前に、検証用ブラウザ(MCP/Playwright)を必ず閉じて単一クライアントにする。複数クライアント挙動を確認したい場合は意図的に別サイズで開く。

## 2026-06-02 — Release push が coverage gate で落ちたら新規マージcoードへtest追加で回復する

Type: lesson
Context: /release で develop push 時、pre-push hook の cargo llvm-cov gate が 89.90% < 90% で失敗。release commit 自体は version/CHANGELOG のみだが、直近 squash merge された feature コード (usage モジュール) のテスト不足が原因。PR squash merge は local pre-push hook を経由しないため負債が develop に蓄積し、release push で初めて顕在化する。
Learning: scripts/check-coverage-threshold.mjs が target/coverage-summary.json から filtered line coverage を算出。未カバーは network/async 経路と fs-walk 経路に集中。fs-walk 経路 (rollouts_modified_since / transcripts_modified_since / read_*_consumption) は tempdir で安全 (parallel-safe, env 非変更) にテストでき、複数モジュールを横断的にカバーできる。env 変数を変える test は --test-threads=1 でない通常 test で flaky。
Future Action: release push が coverage gate で落ちたら --no-verify でバイパスせず、coverage-summary.json を node で解析して missing 行が多い新規ファイルを特定し、tempdir ベースの pure/fs-walk test を追加して >=90% を回復してから push する。閾値ギリギリ (margin <0.2%) の場合は run 間変動で再 flake しうるため余裕を持たせる。

## 2026-06-03 — 新しい singleton/lock 機構を足す時は既存のエスケープハッチ(env override)を引き継ぐ

Type: lesson
Context: SPEC #2920 の per-user tray single-instance lock (cli/tray/lock.rs::acquire, main.rs:6283) が、レガシー gui_single_instance ロックの GWT_FORCE_NEW_INSTANCE エスケープハッチを引き継がず、debug でも 2 つ目のインスタンスを起動できなかった。ユーザーは env で回避できるはずと認識していた。
Learning: front-door に新しいロック層を追加すると、旧ロックが honor していた env override が暗黙に失効する。tray lock は force 時に PID スコープの forced_lock_path を使い正規ロックを汚さず共存させる形で修正した。
Future Action: singleton/lock/exclusive 系の機構を新設・置換する時は、既存ロックの bypass/override(特に GWT_FORCE_NEW_INSTANCE 等の env)を新経路にも必ず移植し、TDD で回避経路を固定する。

## 2026-06-03 — 視覚検証で GWT_FORCE_NEW_INSTANCE 共存インスタンスを使わない(共有 ~/.gwt が不安定)

Type: lesson
Context: SESSIONS 一覧削除の視覚確認のため GWT_FORCE_NEW_INSTANCE=1 で dev ビルドを GWT.app と並行起動したが、同じ ~/.gwt を共有するため issues 再インデックスがループし、dev 側 usage poller が provider_usage を配信せず USAGE セルが hidden のまま(accounts 空判定)になった。WebSocket/telemetry は正常だった。
Learning: 2 つの gwt インスタンスが同一 ~/.gwt を共有すると stateful subsystem(index/usage poller)が競合し live 検証が不安定になる。単一インスタンス(GWT.app)では browser からでも usage は正常表示される。
Future Action: GUI の視覚検証は単一インスタンスで行う。dev を確認したい時は既存インスタンスを終了して dev を唯一の tray-resident として起動する。共存は lock 検証用に留め、live データ表示の検証には使わない。

## 2026-06-04 — worktree 列挙は main_worktree_root を先に解決する

Type: lesson
Context: File Tree の SELECT WORKTREE ピッカーが workspace home (子に bare repo を持つが home 自体は git work tree でないレイアウト) で 'failed to list worktrees: ... not a git repository ... .git' (exit 128) で失敗した。enumerate_worktrees が git worktree list を tab.project_root で直接実行していたのが原因。
Learning: gwt は 'workspace home without default worktree' レイアウトをサポートしており、project_root が git work tree でない場合がある。git worktree list はその home から実行すると失敗する。main_worktree_root は first_child_bare_repository で child-bare を解決できるので、worktree 列挙系は必ず main_worktree_root を先に解決してから WorktreeManager に渡す。
Future Action: git worktree list / WorktreeManager::new を新規に呼ぶコードを書くときは、渡す repo_root が linked worktree / 通常 repo / workspace-home(child-bare) のいずれでも動くよう main_worktree_root で解決してから渡す。

## 2026-06-04 — board_audience テストは並行実行で flaky (workspace projection 共有状態)

Type: lesson
Context: gwt-managed worktree 内で cargo test -p gwt --all-features を並行(default threads)実行すると board_audience::tests の gui_default_scope_reads_projection_from_disk / post_audience_for_session_attaches_workspace_and_respects_broadcast が実行ごとに異なるテスト・assertion 行で落ちる。--test-threads=1 では全 PASS。
Learning: これらのテストは tempdir を repo_path に渡すが、workspace projection の save/load が共有可変状態 (~/.gwt 配下の project-state や env 由来) で並行競合し、projection 読み出しが None/All に化ける。worktree_inventory など無関係な変更の PR 検証でも踏むため、変更起因の regression と誤認しやすい。
Future Action: cargo test 並行実行で board_audience が落ちたら、まず --test-threads=1 で再実行して flaky か変更起因かを切り分ける。flaky の場合は変更の diff が board_audience/workspace_projection に触れていないことを確認し pre-existing として分離報告する。テスト分離の根本修正は別 Issue で扱う。

## 2026-06-04 — Host launch fallback executable must reuse platform-aware runner resolution, not hardcode bare names

Type: failure-pattern
Context: Issue #2981: bunx probe 失敗後の host package-runner fallback が bare "npx" をハードコードしており、Windows では CreateProcess が POSIX shim(npx) を spawn できず program not found で PTY 開始前に失敗。primary runner は package_runner_candidates で npx.cmd を優先(SPEC-1921 FR-080)していたが fallback だけがこの解決をバイパスしていた。
Learning: spawn する executable は platform で解決形が異なる(Windows は .cmd 必須)。fallback/secondary 経路で runner 名をハードコードすると primary の Windows-aware 解決(find_package_runner_in_path)を取りこぼし、Windows 限定 bug を生む。
Future Action: launch/spawn 系で runner executable を選ぶ箇所は必ず find_package_runner_in_path 系の platform-aware 解決を経由する。新しい fallback を足すときは bare 名のハードコードを禁止し、未解決時のみ bare 名へフォールバックする。

## 2026-06-04 — Workspace→Work 統合は FR-334 決定済みだが実装未完了（agent guidance 含む）

Type: lesson
Context: SPEC-2359 FR-334 で canonical naming=Work と決定済み。だが WorkspaceProjection struct(850+行)/workspace_projection.rs/gwtd workspace CLI/.gwt/workspace/ storage/WorkspaceState protocol、特に coordination_guidance.rs:24,79 の agent guidance が今も「Workspace を update しろ」と指示している。
Learning: enum/UI label だけ Work に統一され、domain/CLI/storage/protocol/agent guidance が未統一の technical debt。agent guidance が残る限り agent は Workspace 用語で作業し続けるため、命名統一の核は coordination_guidance.rs。
Future Action: Work 系の作業前に coordination_guidance.rs と命名統一の残作業を確認する。FR-334 完遂は SPEC-2359 Phase W-12 (US-66/FR-357/358) で扱う。

## 2026-06-04 — Work 概念モデル = agent session + lifecycle（Board とは分離維持）

Type: lesson
Context: gwt-discussion で Work UI/UX 全面再設計に合意。Work=agent session 単位(1 agent:1 Work)、lifecycle Active/Paused/Done/Discarded、手動 close まで一覧に残す、Canvas 専用 surface 集約(サイドバー Active Works 撤去)、cleanup は worktree のみ削除、repo-local 追跡で永続コア/揮発ランタイムの 2 層。
Learning: Work current state と Board は責務分離(event log vs snapshot、shared vs project-scoped、author vs owner)。統合せず board_refs/board_entry_id でリンクする。FR-008 と 2026-05-07 lesson(Board を current state に流用するな、live 照合せよ)を維持。
Future Action: Work state を Board に統合しようとしない。Active 判定は必ず live session/window 照合を挟む。設計の正本は SPEC-2359 Phase W-12。

## 2026-06-06 — repo-local tracked ファイルは --show-toplevel(現 worktree)に解決する。--git-common-dir は bare repo を返す

Type: lesson
Context: SPEC-2359 W-12 Slice 5b で gwt_repo_local_work_dir が resolve_main_worktree_root(git rev-parse --git-common-dir)を使い、gwt の workspace-home layout(親が bare repo gwt.git を持つ)では bare repo dir(gwt.git)に解決。bare repo は working tree を持たないため .gwt/work/events.jsonl/memory.md が tracked されず gwt.git/.gwt/work/ に書かれ、committed の <worktree>/.gwt/work/ と不一致になった。
Learning: git-tracked な repo-local ファイルの配置先は必ず git rev-parse --show-toplevel(現在の worktree の working tree root)で解決する。--git-common-dir / main_worktree_root は linked worktree で共有 git dir(しばしば bare)を返し working tree が無いので tracked file には使えない。worktree 横断の共有は filesystem 共有でなく git commit + merge=union で行う(各 worktree が自分の .gwt/work/ を持つ)。
Future Action: repo-local tracked ファイルの path helper は --show-toplevel ベースにする。CLI 書込み先と committed 場所が一致するか git ls-files/git show で確認する。

## 2026-06-04 — Board/storage tests must share env lock

Type: lesson
Context: SPEC-1974 Board storage split fix exposed flaky tests that write project-scoped coordination/workspace data while HOME/USERPROFILE point at the developer machine.
Learning: Tests that call project-scoped storage helpers must isolate HOME/USERPROFILE and use the crate-wide env_test_lock/env_lock, not a local Mutex, because cargo test runs modules in parallel.
Future Action: Before adding tests around Board, workspace projection, or project-scoped paths, acquire the existing crate-wide env lock and point HOME/USERPROFILE at a tempdir.

## 2026-06-04 — 使用量調査は credentials で課金体系を先に確定し transcript は model別+メイン/サブ分離で集計する

Type: workflow
Context: 「Claude Code の使用量が怪しい」調査で Explore サブエージェントが『月$8,462の従量課金』『cacheを5分TTLで毎ターン破壊』『cache_read 11億/per-turn 966k』『無限リトライループ・264回暴走起動』と報告したが全て実データで反証された。実際は ~/.claude/.credentials.json が subscriptionType=max / rateLimitTier=default_claude_max_20x で ANTHROPIC_API_KEY 未設定（ドル従量課金ゼロ・Maxサブスク枠）、cacheは ephemeral_1h TTL、11億はメインとサブエージェント(subagents/配下)の二重計上、retryは有限(100ms bootstrap完了待ち)、264回は1791行の2日間テスト集中だった。真因は Opus 4.8 + 1M context(per-turn最大995k)常用 × 長大セッション × subagent多用で gwt が Opus の81%。
Learning: 使用量/コスト調査でサブエージェントの定量報告は誇張・二重計上・誤前提を含みやすく鵜呑み厳禁。(1)課金体系は ~/.claude/.credentials.json の subscriptionType/rateLimitTier と ANTHROPIC_API_KEY 有無で先に確定する(サブスク枠かAPI従量かで結論が真逆)。(2)transcript集計はメインセッションとサブエージェント(subagents/*.jsonl)を分離し各assistant messageを1回だけ model別 group_by する。(3)per-turnの input+cache_read が context window(200k/1M)を超えたら集計バグのサイン。
Future Action: 使用量調査では結論前に自分で credentials 種別確認と jq による model別/二重計上排除の集計を実行し、エージェントの数値と断定を一次データで裏取りする。

## 2026-06-06 — Guard SPEC section edits against empty stdin writes

Type: lesson
Context: While completing SPEC-1939 Phase 34, a perl pipeline used backtick-delimited text and failed before producing content, but gwtd issue spec 1939 --edit tasks -f - still consumed empty stdin and wrote a 0-byte tasks section. The section was restored from the cached artifact comment before handoff.
Learning: Piping generated content directly into gwtd issue spec --edit is unsafe when the generator can fail; pipefail does not prevent the receiving command from accepting empty stdin.
Future Action: For SPEC section rewrites, generate to a temporary file first, assert expected byte/line count and required anchors, show tail/readback, and only then pass the verified file to gwtd issue spec --edit.

## 2026-06-06 — Codex managed hook は tool-use event の session_id 欠落で fail-closed にしない

Type: lesson
Context: Codex の PreToolUse/PostToolUse hook が毎回 exit code 1。runtime_state::validated_hook_agent_session_id が Codex セッションで CODEX_THREAD_ID 未設定かつ payload に session_id 無しのとき HookError::InvalidEvent を返していた。Codex は SessionStart では id を渡すが tool-use event では渡さないため毎回失敗。agent_session_id は session .toml に永続化済みなのに hook 全体を落としていた。live-event 経路 (daemon_runtime) は同条件で既に fail-open だった (commit c2c83469b で混入, SPEC-2077 ドメイン)。
Learning: Provider 由来 (Codex) の hook payload フィールドは event ごとに有無が変わる。必須化して fail-closed にすると、ツール呼び出しごとに exit 1 がユーザーに露出する。永続済みメタデータ (exact_resume_session_id) があるなら fail-open + 既存値再利用が正しい。診断ログも persisted id がある通常ケースでは出さず、shipped behavior と分離する (2026-05-07 lesson と一致)。
Future Action: hook handler に必須フィールド検査を追加するときは、(1) gwt 自身の不変条件 (GWT_SESSION_ID) だけ fail-closed、(2) provider 供給値の欠落は fail-open し persisted session metadata を破壊しない、(3) regression test で persisted id 保持と exit 0 を固定、(4) 診断ログは fallback 不能時のみ。runtime_state と daemon_runtime の両経路を必ず揃える。

## 2026-06-04 — Fresh browser checks must seed agent windows, not rely on Start Work

Type: lesson
Context: SPEC-2012 visual verification was blocked twice because the isolated browser-check HOME had no Git HTTPS credentials and the user attempted Start Work/Launch flows that created remote work branches.
Learning: For UI verification that needs an Agent window, prepare the fresh checkout with a current-branch Agent window before handing off the URL. For Claude Code startup auto-resume, the seeded session must include exact agent_session_id plus lifecycle evidence such as last_hook_event_at or last_completed_stop_at; otherwise it is filtered out.
Future Action: When using browser-check for Agent-window behavior, never ask the user to Start Work in the isolated HOME. Seed or launch the required current-branch Agent window first, verify with headless/browser checks that no branch-auth error is present, then share the URL.

## 2026-06-04 — Fresh Claude verification can be blocked by existing live Work agent focus

Type: lesson
Context: SPEC-2012 visual verification needed a Claude Code window in an isolated browser-check HOME. Launch Wizard normal mode appeared to close successfully but did not create Claude while a live Codex pane was already assigned to the same Work.
Learning: spawn_agent_window_with_placement first focuses any existing live agent for the same worktree/branch without checking the requested agent type. In fresh verification environments, this can make Add Agent/Launch Agent look like a no-op when switching from Codex to Claude.
Future Action: When browser-check needs a specific second agent for the same Work, either close the existing fresh-check agent pane first or explicitly verify the product supports multiple agents before using Launch Wizard as the setup path.

## 2026-06-08 — 遅れたブランチの test-file マージは ours+append だけでは develop の base-test 修正を取りこぼす

Type: lesson
Context: SPEC-2359 W-12 / SPEC-2356 ブランチ (develop 76 commit 遅れ) のマージで operator-chrome-structure.test.mjs が 1400 行の単一巨大 conflict 化。ours(私の119テスト)を土台に develop の新規 perf テスト33個を append する戦略を取ったが、develop が base テスト 'Launch wizard runtime confirmation' を contract 変更 (showConfirm 追加) していたのに ours が旧版を保持し、merged app.js (develop の launch wizard) と不一致で1件 RED になった。
Learning: ours+append-theirs'-new-tests 戦略は『theirs が base テストを変更し ours が触っていない』ケースを取りこぼす。新規テスト名 (comm -13 base theirs) は拾えても、同名 base テストの body 変更は拾えない。全テストランナーを oracle にすれば contract 不一致は RED として顕在化するので、append 後に必ず full suite を回し、RED の base テストは theirs 版へ swap する。obsolete 化したテスト (削除済み関数参照) のみ除外する。
Future Action: 大きく遅れたブランチの test-file conflict は (1) theirs を土台に git apply --3way で ours の diff を当てるか、(2) ours+append 後に full suite を oracle にして RED を theirs 版へ swap する。どちらでも append/swap 後に full frontend+cargo suite で 0 fail を確認してから commit する。

## 2026-06-08 — browser-check の session.json seed: recent_projects は RecentProjectEntry 構造体配列 (string 配列は起動 panic)

Type: lesson
Context: merged build で fresh 隔離インスタンスを起動した際、seed した session.json の recent_projects を文字列配列 ['<repo>'] にしたら 'app runtime: invalid type: string, expected struct RecentProjectEntry' で起動 panic (main.rs:6356)。RecentProjectEntry は persistence.rs:122 で { path, title, kind } 構造体。develop 側でスキーマが string→struct に変わっていた。
Learning: browser-check で session.json を seed する時、recent_projects は { path, title, kind } の構造体配列。空 [] にすればプロジェクトタブ (tabs) だけでアプリに着地でき安全。古い string 配列形式は新ビルドで起動を落とす。これは seed バグであり製品バグではない。
Future Action: browser-check の seed では recent_projects は [] にするか persistence.rs の RecentProjectEntry 現行 shape をその場で確認してから書く。起動 panic 時は seed スキーマ不一致をまず疑い、CHECK_HOME/.gwt/session.json を現行 struct 定義と突き合わせる。

## 2026-06-07 — Branches Resume 不可の真因は scope 照合ではなく in-memory session cache の陳腐化（初期診断の訂正）

Type: lesson
Context: Issue #2995: Branches/Work からの Resume が「ときどき」できず再起動でも直らない。初期診断では collect_quick_start_entries_from_sessions (quick_start.rs) の WorktreePathScope 完全一致がバグと推定したが誤りだった。実コード追跡で、Branches availability/resolution の実ゲートは public quick_start_entries_from_sessions の QuickStartRepoScope（完全一致 OR repo_hash OR main_worktree_root の3段階、#2546 で既に正しい）であり、collect_ の WorktreePathScope は worktree_path を repo_path に上書き後に呼ばれるためバイパスされていた。実環境検証（git -C <workspace home> rev-parse --git-common-dir が fatal → first_child_bare_repository が bare child gwt.git を発見 → detect_repo_hash で repo identity 99a8660247f5bc49 を解決）で、workspace-home project_root でも repo_hash 一致で正しく match することを確認。
Learning: 真因は launch_wizard_cache.sessions（起動時1回ロード、spawn 時のみ per-window 部分更新、sessions_dir watcher 無し）の陳腐化。hook CLI (cli/hook/runtime_state.rs → persist_agent_session_id) が agent 起動後に書く agent_session_id を GUI cache が観測しないため、同一プロセス内で launch→stop したセッションが Branches から resume 不可になる。Work picker は projection+disk 都度ロードのため影響が小さく「Branches が多い」と一致。gwt は daemon/tray 常駐（SPEC-2077/2920）でプロセスが生き続けるため window 再起動では cache が再ロードされず「再起動でも直らない」とも一致。修正は availability(spawn_branch_load_async) と resolution(latest_resumable_branch_session) を sessions_dir から disk-fresh に読むだけ（既存の正しい QuickStartRepoScope を再利用）。
Future Action: 「resume できない」系バグでは matching を疑う前に (1) どの関数が実ゲートか call graph を確定（wrapper が後段 scope をバイパスしていないか）、(2) 実データ・実 git レイアウトで matching が本当に false になるか empirical に検証、(3) データソースの鮮度（in-memory cache vs disk、watcher 有無、常駐プロセス寿命）を疑う。推測で大改修に入る前に再現を取る。

## 2026-06-08 — Branches degraded banner の真因は WS queue-overflow eviction + branch list が reconnect replay から欠落

Type: lesson
Context: Branches で 'BRANCH DETAIL CHECK INTERRUPTED / SAFETY UNKNOWN' が頻発し読めない、を調査。WebView↔localhost の WS で heartbeat 無し。
Learning: 切断はネットワーク断ではなく、各 client の 64-slot 送信キュー(embedded_server.rs CLIENT_QUEUE_CAPACITY)が terminal output 等で飽和→try_send 失敗→サーバーが client を evict することで発生(設計通りの backpressure)。failLoadingBranchesOnConnectionLoss(app.js) が interrupted notice を立て全 row を Safety unknown 化。核心の欠陥は build_frontend_sync_events(app_runtime/mod.rs) が Workspace/Terminal/LaunchWizard を reconnect replay するのに branch_entries を含めないため Branches だけ自己回復できず手動 Refresh まで固まる点。Branches/cleanup の SPEC owner は SPEC-2009(新規作成不要)。
Future Action: GUI surface が切断後に固まる/古いままの不具合は (1) build_frontend_sync_events の replay 対象に当該 surface が含まれるか (2) 切断トリガは embedded_server.rs の queue-overflow eviction を疑う。SPEC routing は gwt-search で既存 owner(SPEC-2009 等)を必ず確認してから新規作成を検討する。

## 2026-06-08 — 検証中にユーザーの実 repo で git branch -d main を実行し local main を誤削除した

Type: lesson
Context: Phase C 視覚検証中、main が BLOCKED な理由を確認するため 'git が refuse するはず' と誤想定して git branch -d main を実 repo で実行した。
Learning: git branch -d は merged branch を実際に削除する（refuse しない）。bare repo の HEAD→main は worktree checkout ではないため git は main の削除をブロックしない（実際に削除された）。よって Phase C の current_head ブロックは bare repo の symbolic HEAD を誤検出している。復元は元 SHA で 'git branch main <sha>' により可能（origin/main とは別 commit だったため exact SHA 復元が必須だった）。
Future Action: 調査/検証中はユーザーの実 repo で破壊的 git コマンド（branch -d/-D, push --delete, worktree remove 等）を絶対に実行しない。ブランチ削除可否は read-only 検査（git worktree list で checkout 有無、symbolic-ref で HEAD、show-ref）だけで判定する。branch_list の current_head 判定は bare symbolic HEAD と実 worktree checkout を区別すべき。
## 2026-06-08 — 概念変更要望では SPEC 末尾の最新 Phase を実読してから設計判断する

Type: lesson
Context: gwt-discussion で『Work と Branches を統合/分離』要望を受けた。explore agent の要約と user の『Work=Branch』前提だけに従うと、SPEC-2359 を Work=branch identity に差し戻す方向（option A・大規模）に進みかけた。
Learning: SPEC-2359 の spec section 末尾（4日前の Phase W-12 / US-60〜66）を実読すると、Work=agent session(1 agent:1 Work, FR-348) へ意図的に再設計済みで Work=branch(W-8/US-54) は supersede 済みだった。この矛盾を user に提示した結果、判断が option A → option B（presentation のみ統合・W-12 identity 維持）に変わり、4日前確定の設計を覆さずに要望を満たせた。
Future Action: 既存概念の変更要望では、対象 SPEC section の『最新/末尾の Update Phase / Supersede note』を必ず実読し、user 前提や explore 要約と現行正本が食い違わないか確認してから設計判断する。recent supersede note を見落とさない。

## 2026-06-08 — SPEC テキストと実装の drift: Work は code 上 branch 由来 multi-agent (W-8) で W-12(1:1) は未実装

Type: lesson
Context: SPEC-2359 W-13 実装で、SPEC 最新セクション W-12 (2026-06-04, FR-348 '1 agent:1 Work') を維持する前提で branch 背骨 UI を作ったが、ユーザーが視覚チェックで『Work を主導に・Work に複数 agent』と指摘。
Learning: 実コード active_work_items_from_projection (mod.rs:2186) は active_work_agent_work_id→canonical_work_id(branch 由来) で agent を grouping し、1 Work に複数 agent を持たせる W-8 モデルを実装している。ActiveWorkItemView.agents は Vec。W-12 の '1 agent:1 Work' は SPEC テキストのみで未実装のドリフト。SPEC の最新 Update セクションを読んでも実装の真実とは限らない。
Future Action: UI 設計前に SPEC のモデル記述だけで判断せず、対応する projection 構築コード（active_work_items_from_projection / canonical_work_id 等）を読んで実装の実態を確認する。SPEC text と code が食い違う場合は実装を真実とし、drift を明示してユーザーに確認する。

## 2026-06-08 — Work/Workspace 一覧の土台はライブ active_work_projection ではなく永続 WorkspaceWorkItem(W-12)

Type: lesson
Context: 「Work surface 作り直し」で、ユーザーの『Work』をライブ active_work_projection(稼働中 agent から導出・停止で消える非永続)と誤解し、agent 一覧を spine にして何度も方向を外した(branch 背骨→Active Works→Option A→…と4回以上やり直し)。
Learning: ユーザーの Work(=Workspace)は永続概念: 1 Start Work 単位、branch と 1:1 だがローカル branch 消失でも永続、own id、.gwt/work/events.jsonl で git 追跡、lifecycle Active/Paused/Done/Discarded、複数 session を順番に束ね(active は 1 つ)、linked SPEC/Issue/PR、Board スレッドを内包。= develop の WorkspaceWorkItem(W-12)。一覧は is_incomplete(Active+Paused)。ライブ projection は agent が無いと空で、ユーザー視覚検証も不能になる。
Future Action: Work/Workspace 一覧 UI は最初に『永続 WorkspaceWorkItem store(load_workspace_work_items / .gwt/work/events.jsonl, develop W-12)』を土台にする。Work は agent/branch の有無に依存しない永続概念だと最初に確認。SPEC の最新フェーズ(W-12)を実装の正本とし、UI を projection ではなく永続 store に接続する。

## 2026-06-09 — Avoid wall-clock upper bounds for async coalesce tests

Type: failure-pattern
Context: pre-pr verification for PR #3001 repeatedly failed in the full suite on app_runtime::persist_dispatcher::tests::suppresses_identical_snapshot_while_pending while isolated runs passed.
Learning: The test asserted an elapsed wall-clock upper bound that included scheduler and disk latency, so suite load could fail the test even when duplicate enqueue suppression was correct.
Future Action: For async coalescing and background-worker tests, assert internal state transitions or use deterministic test harnesses instead of wall-clock upper bounds for completion time.

## 2026-06-09 — Lock HOME readers in parallel Rust tests

Type: lesson
Context: PR #3001 の Linux Test (Rust) で workspace_projection::tests::resolve_workspace_id_for_session_returns_none_when_session_missing が save_workspace_projection(...).unwrap() の ENOENT で失敗した。
Learning: 同じ test binary 内で HOME を一時ディレクトリに差し替えるテストがある場合、HOME を変更しないテストでも gwt_home()/gwt_workspace_*_for_repo_path() を読むなら env_lock が必要。読み手が lock を取らないと、差し替えテストの TempDir drop と write_atomic の create/open が競合する。
Future Action: HOME/USERPROFILE/XDG_CONFIG_HOME/GIT_CONFIG_GLOBAL など process-wide env から path を導くテストを追加・変更するときは、env を変更する側だけでなく読む側にも crate::test_support::env_lock() を適用する。

## 2026-06-09 — Installed app/runtime staleness can hide CPU regressions

Type: failure-pattern
Context: SPEC-1939 Phase 67 investigated >100% CPU in the installed GWT.app while a fresh target/debug/gwt checkout was idle.
Learning: Current-checkout smoke passing is insufficient when the running installed app uses an older binary/runtime runner; compare installed/current hashes, runtime manifest runner hash, child runner processes, and recent log storm counters.
Future Action: For future CPU or log-storm reports, run gwtd diagnostics cpu --json against the live machine before declaring the checkout fixed, and explicitly record whether installed app/runtime remain stale.

## 2026-06-10 — 新モデル追加時は per-model default effort も Claude Code docs で確認する

Type: lesson
Context: PR #3007 で Fable 5 をモデル候補に追加した際、opus と同じ reasoning ladder (xHigh default) を共有させたが、Codex レビューで Fable 5 の Claude Code 既定 effort は high だと指摘された。検証の結果 Opus 4.8 / Sonnet 4.6 の既定も high で、gwt の xHigh/medium 既定は旧バージョン (Opus 4.7 時代) の stale な引き継ぎだった。
Learning: Claude Code の既定 effort はモデル世代ごとに変わる (4.7=xhigh, 4.8/Fable5/Sonnet4.6=high)。モデル候補の追加・ラベル更新時にラベルだけ追従して既定値の追従が漏れると、ユーザーが意図せず高コスト effort で起動する。
Future Action: launch_wizard のモデル一覧や既定値を更新する際は https://code.claude.com/docs/en/model-config#adjust-effort-level の "The default effort is ..." を必ず確認し、CLAUDE_*_REASONING_OPTIONS の is_default と説明文 "(... default)" を同時に更新する。

## 2026-06-10 — effort 既定はハードコードせず Auto（非 export）で Claude Code に委譲する

Type: decision
Context: PR #3009 で既定 effort を docs の high に揃えた直後、Codex レビューが「AWS platform では opus→Opus 4.7（既定 xhigh）に解決されるため high 固定は不一致」と指摘。provider・モデル世代ごとに既定が異なるため、gwt 側のハードコードはどの値でも何処かで stale になる。
Learning: Claude の effort 既定は reasoning=auto（CLAUDE_CODE_EFFORT_LEVEL 非 export）にして Claude Code 自身の per-model 既定に委譲するのが構造的な解。値の追従更新が不要になり、stale 既定バグのクラスごと消える。
Future Action: launch オプションの「既定値」を gwt 側に持たせる前に、CLI 側に既定解決を委譲できるか（フラグ/環境変数を渡さない選択肢）を先に検討する。

## 2026-06-10 — wizard slider の E2E は focusout で interaction guard を解放してから assert する

Type: failure-pattern
Context: launch-wizard-controls-live.spec.ts の ArrowRight→summary assert が常に失敗。WS 送信・backend 適用は正常で、frontend の wizardInteractionGuard（SPEC-2014 2026-05-29）が slider focus 中の launch_wizard_state 再レンダリングを focusout まで defer していた。Playwright の press は focus を残すため summary が永遠に古いままになり、後続 probe の echo も全て deferred に飲まれて「backend が死んだ」ように見えた。
Learning: guard は <select> と .launch-range__input の focus/pointer 中に activate され focusout/Escape で release される。slider 操作後の backend 反映を assert する E2E は blur() などで guard を先に解放する必要がある。
Future Action: wizard の <select>/slider を操作する E2E・自動検証では、操作後に blur または別要素クリックを挟んでから backend 反映を assert する。「アクションが無視される」症状を見たら interaction guard の defer を最初に疑う。

## 2026-06-10 — Windows npx probe timeout is inconclusive

Type: failure-pattern
Context: SPEC-1921 Phase 67: Claude Code host launch showed a pre-PTY error when the fallback npx --version probe timed out, while the same npx command succeeded manually afterward.
Learning: A Windows npx probe timeout without a verified npm _npx corruption signature can be cold install/extraction latency and should not be treated as a deterministic launch-blocking failure.
Future Action: For package-runner probes, keep deterministic repair/error handling for corrupted _npx or immediate unrelated failures, but continue to terminal launch with npx --yes when the only fallback probe signal is timeout.
