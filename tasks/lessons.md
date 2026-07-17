# Lessons

## Recovery fixture classification must be explicit

- 事象: current-format Execution のテスト fixture が `session_kind=None` のまま残り、legacy ambiguity として正しく Attention に分類された結果、自動復元テストが広範囲に失敗した。
- 原因: 新しい Recovery SOT の fail-closed 分類を実装した一方、current-format の正例 fixture に lane metadata を明示していなかった。
- 再発防止策: current-format の正例は `session_kind` と `is_ephemeral` を必ず明示する。metadata が欠けた fixture は legacy 専用テストとして Attention と診断理由を検証する。

## Shared environment locks must preserve the first failure

- 事象: 環境変数を共有するテストが一度 panic すると mutex が poison され、後続テストが同じ `env lock` エラーで大量に失敗して根因を隠した。
- 原因: test-only の排他ロック取得に `expect` を使い、poison 後の安全な直列実行よりも連鎖 panic を選んでいた。
- 再発防止策: test-only の共有環境 mutex は `PoisonError::into_inner` で回復し、最初の失敗を唯一の根因として残す。環境値は各 test guard で必ず復元する。

## Client-side self-only checks are not authorization

- 事象: `gwtd pane send` は client 側で自 Session のみを選んでいたが、内部 WebSocket は全 Agent 共通 bearer を受け入れ、raw event の `session_id` をそのまま runtime へ渡していたため、別 Session の PTY へ入力できた。
- 原因: capability を接続 principal に変換せず、caller-supplied identifier と client helper の検証を server-side authorization の代わりにしていた。
- 再発防止策: mutation endpoint は opaque capability を canonical project と Session の server-side principal に束縛し、dispatch 前に対象との一致を強制する。回帰テストは別 project / Session の capability を使い、list、read、close、input、初期 snapshot のすべてで越境を拒否することを確認する。

## Process-wide broadcasts bypass scoped capabilities

- 事象: pane 接続時の mutation は Session 単位に制限しても、process-global な frontend broadcast と初期 snapshot から別 project の workspace / terminal 情報を受信できた。
- 原因: 接続認証だけを分離し、接続後に配送される read model と event fan-out を principal の project scope で絞っていなかった。
- 再発防止策: capability の scope は mutation だけでなく read、snapshot、broadcast、close の全経路に適用する。agent 接続には専用 DTO と filtered reply を使い、browser 向け global broadcast へ参加させない。

## Host loopback URLs are not container reachability contracts

- 事象: Host で有効な `127.0.0.1` bridge URL を Docker/Podman の agent 環境へそのまま渡す設計では、container の loopback が host process を指さず managed hook/pane control channel に到達できなかった。
- 原因: URL validator、launch-time rewrite、listener bind、生成 Compose override を個別に考え、runtime ごとの network namespace を一つの契約として検証していなかった。
- 再発防止策: Host / Docker / Podman ごとに予約済み alias と listener 到達性を定義し、URL 変換、allowlist、Docker `extra_hosts`、実生成ファイルを一組の契約テストで確認する。token は URL / argv に含めず環境変数だけで渡す。

## Recovery proof must cover every managed lane consistently

- 事象: Intake では cold start 時の supervisor-stop 証明から Ready recovery を Interrupted に遷移できたが、同じ Recovery SOT を使う Execution は対象外になり、再起動後だけ復元候補から消えた。
- 原因: 中断証明と startup claim の条件が `Intake + ephemeral` に埋め込まれ、Recovery Session の種類とユーザー操作による停止意図を別々に扱っていなかった。
- 再発防止策: Recovery SOT の proof / claim / exact-resume guard は Intake と Execution の両 lane に適用し、worktree 再作成だけを Intake 専用に保つ。provider の自然停止は復元フラグを維持し、明示 Stop / Close は自動復帰フラグだけを落とす回帰テストを置く。

## Structured operation fields must outrank shell text

- 事象: Intake checkpoint の許可判定が command 全体の substring を検索していたため、JSON payload の説明文に operation 名が含まれるだけで別操作として誤認できた。
- 原因: shell command を transport として扱う境界で、構造化 JSON の root `operation` field と任意の user text を分離していなかった。
- 再発防止策: 埋め込まれた JSON root を bounded parser で走査し、単一の `operation` field だけを authority とする。複数 field は fail closed、互換 marker は専用 comment 形式だけに限定し、payload substring では許可しない。

## Generated runtime files must prove ownership before replacement

- 事象: container bridge 用 Compose override を固定名で生成すると、同名のユーザー管理ファイルを上書きする危険があり、生成内容だけのテストでは実際の `compose -f` 順序も保証できなかった。
- 原因: generator の出力と launch invocation を一つの契約として検証せず、managed artifact の ownership marker を書き込み前提に含めていなかった。
- 再発防止策: managed file は専用名と marker を持たせ、既存内容が未所有なら書き込みを拒否する。ユーザー override を保持し、base → user → managed の実 argv と Docker / Podman 差分を契約テストで検証する。

## Attachment limits must apply before durable staging

- 事象: recovery attachment の store 側上限だけでは、HTTP multipart upload が先に一時ファイルへ無制限に staging され、拒否前に disk を消費できた。
- 原因: 永続化 API の制限を request ingress の制限と同一視し、下流 validation より前の byte path を分析していなかった。
- 再発防止策: request body、multipart field、temporary staging、RecoveryStore の各境界に同じ bounded policy を適用し、上限超過は publish 前に拒否して一時 artifact を残さない。

## Timeout contract tests need scheduler-safe margins

- 事象: compose exec が generic Docker command より長い timeout を使う回帰テストが、全 workspace テスト中の高負荷時だけ 500ms を超えて失敗した。
- 原因: production timeout の選択を検証するテストが、短い wall-clock deadline を host scheduler の性能測定としても使っていた。
- 再発防止策: timeout 種別そのものを直接検証し、subprocess 成功確認には production の差を保った十分な余裕を与える。短い deadline は timeout 発生そのものを検証する専用テストに限定する。

## Coordinated session pre-reads must use the writer lock

- 事象: Recovery 情報を協調更新する前の Session 読み込みが、Windows で別 writer の remove/rename 区間に重なると一時的な `NotFound` になり、並行 metadata 更新テストが失敗した。
- 原因: 最終書き込みだけを Session lock で保護し、更新判断に使う事前読み込みは同じ atomic replacement 契約の外に置いていた。
- 再発防止策: read-modify-write の事前読み込みも writer と同じ Session lock で直列化する。Windows の置換区間を含む並行更新テストを繰り返し実行し、履歴保持と TOML の parseability を同時に確認する。

## Isolated child-process tests must clear runtime routing overrides

- 事象: 一時 HOME を指定した `gwtd` 統合テストが、親 Intake から継承した `GWT_SESSION_RUNTIME_PATH` を優先し、fixture ではなく実 Session ledger を読んで失敗した。hook forward URL も継承されたままだった。
- 原因: HOME / USERPROFILE と Session ID だけを隔離し、保存先を上書きする runtime path と live daemon への転送先を child environment から除去していなかった。
- 再発防止策: child process を使う hermetic test は HOME だけでなく Session runtime path と hook forward URL/token も明示的に除去する。開発中の実 `GWT_*` 環境を残した状態でも対象テストを実行する。

## A marker write does not prove child-process completion

- 事象: background refresh テストが marker の出現直後に削除し、Windows の `cmd.exe` が追記ハンドルをまだ保持している短い区間で sharing violation になった。
- 原因: 「子プロセスが marker を作成した」と「子プロセスが marker を閉じて終了した」を同じ同期点として扱っていた。
- 再発防止策: child completion を待てない silent-path テストでは、marker の存在確認後に writer lock 解放を bounded retry で同期する。固定 sleep や一度だけの削除で scheduler timing を仮定しない。

## Rebase conflict resolution must preserve process-launch policy

- 事象: `origin/develop` との rebase で Docker 起動処理を統合した際、型 import を残すために `std::process::Command` を採用し、全テストは通ったが Windows GUI の console 抑止を強制する clippy で失敗した。
- 原因: 競合箇所の機能的な呼び出し順だけを確認し、develop 側で追加された process-launch policy と `hidden_command` の適用範囲を再確認していなかった。
- 再発防止策: process 起動を含む rebase 競合では、統合後に `Command::new` の残存を検索し、対象 crate のテストに加えて workspace clippy を必ず再実行する。
