# Lessons Learned

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
