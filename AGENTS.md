# AGENTS.md

このファイルは、このリポジトリでコードを扱う際のガイダンスを提供します。

## 適用範囲

- **この AGENTS.md は gwt リポジトリ専用**のローカル運用ルールであり、gwt が開く任意プロジェクト向けの汎用 Agent 指示ではない。
- gwt を使って他プロジェクトを開発する場合、そのプロジェクト自身の `AGENTS.md` / `CLAUDE.md` / README 等を優先する。
- gwt 共通の Agent 運用（Board/Work 更新、Start Work / Launch materialization、branch/worktree 操作の禁止）は、3 つの注入経路で配信する: managed hooks（SessionStart/UserPromptSubmit/Stop reminder）+ generated guidance（`.claude/skills/gwt-coordination/SKILL.md` および `.codex/skills/gwt-coordination/SKILL.md`）+ launch context（`GWT_SESSION_ID` 等）。canonical source は `crates/gwt-skills/src/coordination_guidance.rs` の 1 箇所。重複ドリフト防止のため、Board/Work の operational content（kind taxonomy、audience selection、body template、tool-unit post 禁止など）を AGENTS.md に複製しない。詳細な投稿手順は generated guidance 経由で agent に届く。
- **gwt 専用機能は「機能」として実装する。** gwt は他プロジェクトの開発にも利用される汎用ツールだが、gwt 専用の運用・挙動・ルールが必要になった場合、memory・session note・口頭運用などの一時的な手段でしのぎ続けない。気づいた時点で Issue として登録し、skill / hooks / runtime のいずれかの恒久機能として実装対象にする（memory は恒久実装が着地するまでのつなぎに限る）。

## エージェント運用原則

- **Classify before acting:** ユーザーの指摘・指示・修正を受けたら、着手前に「gwt 機能」か「このリポジトリ固有の運用」かを判定し、結果と根拠を報告に明記する。
  「他プロジェクトを gwt で開いたときにも必要か」「`gwt-pm` / `gwt-coordination` や skill / hooks / runtime / `gwtd` operation の契約に影響するか」を判定基準とする。
  gwt 機能の Issue 化・恒久実装は、既存の「gwt 専用機能は『機能』として実装する」と「Report gwt Friction to the PM」に従う。判断に迷う場合も gwt 機能として PM に提示する。
  gwt リポジトリの CI・レビュー・リリース慣習に限定される運用は AGENTS.md、仕様は該当 SPEC、利用者向け説明は「ドキュメント管理」に従い README.md / README.ja.md に記載する（運用ルールは README に入れない）。
- **Plan Mode Default:** 非自明な作業、3ステップ以上のタスク、設計判断を含む変更では、実装前に Plan を作成する。途中で前提が崩れた場合は、作業を止めて Plan を更新してから再開する。
- **Self-Improvement Loop:** ユーザー修正、レビュー指摘、失敗から得た再発防止策や再利用可能な判断は `gwtd` JSON operation `memory.add` でマシンローカルの work-notes memory（`~/.gwt/projects/<repo-hash>/work-notes/memory.md`、SPEC-3214）に記録し、同種の作業を始める前に確認する。repo-local `.gwt/work/memory.md` / `tasks/memory.md` / `tasks/lessons.md` は読み取り fallback / legacy alias として扱う。
- **Report gwt Friction to the PM:** gwt 自体の摩擦・機能ギャップは Board で PM に報告し、PM が `gwt-register-issue` で起票する。agent は自分で upstream に Issue を作らない（詳細な投稿手順は generated `gwt-coordination` SKILL.md が配信する）。
- **PM 機能の修正は PM 自身が実装して着地させる:** PM が動作するために必要な機能の修正は、実装エージェントに委譲せず、**PM 自身が PM worktree で実装し、PR を出してマージする**。対象は PM guidance / PM skill、PM が呼ぶ `issue.monitor.*` / `pm.*` operation、PM が裁定に使う Board・escalation 面、および PM が作業を観測・順序付け・決着できなくなる欠陥。理由は順序にある — **動けない PM は、それを直すエージェントを steer できない**ため、委譲するとデッドロックする。それ以外は従来どおり実装エージェントが担当する。判断に迷う場合は「その修正なしで艦隊を steer できるか」で判定し、できないなら PM の担当とする。
- **Skill-First Workflow:** 作業開始時に利用可能なスキルを確認し、要求に適合するスキルがある場合は積極的に使用する。検索、調査、Issue/SPEC 運用、設計議論、実装、PR 管理では手動運用より先にスキル適用を検討する。
- **Skill Authoring Language:** スキルを新規作成・更新する場合、`SKILL.md`、テンプレート、説明文などスキル本体の内容は英語で記述する。通常の対話や補足説明は日本語でよいが、スキル定義の正本は英語とする。
- **Verification Before Done:** 完了を宣言する前に、変更対象に応じたテスト、lint、型チェック、ログ確認、差分確認を実施し、スタッフエンジニアが承認できる状態かを基準にセルフレビューする。
- **Subagent Strategy:** 独立した調査、分析、実装、テスト整備はサブエージェントに分割し、メインのコンテキストを不要な詳細で汚さない。担当範囲、完了条件、検証観点を明示して責務を重複させない。
- **Demand Elegance:** 非自明な変更では、力技で実装する前に 2〜3 のアプローチを比較し、もっともシンプルで保守しやすい案を選ぶ。単純な修正では過剰設計しない。
- **Autonomous Bug Fixing:** バグ対応では、まず再現手順、ログ、失敗テスト、関連コードを自律的に調査し、原因特定、修正、再発防止確認まで進める。不可逆な仕様判断やプロダクト判断だけをユーザーに確認する。
- **Investigation-First Discussion:** 実装中に以下のシグナルを検知した場合、実装を一時停止して調査と議論に入る:
  - generator やテンプレートを変更したが、生成される実ファイル（settings.local.json 等）を実際に確認していない
  - SPEC の acceptance scenario と実装の実際の挙動が一致しない
  - 実装が SPEC の `tasks` section / `tasks.md` artifact に記載されていないファイルに触れようとしている
  - テストが通ったが、手動で検証すると期待と異なる結果になる
  - migration / 互換性パスの条件分岐が新形式を網羅していない
  - 変更の下流影響（何が壊れるか）、上流前提（何が先に必要か）、同時変更境界（何を一緒に変えないと中間状態で壊れるか）を分析していない

  調査手順: コードを読む → 依存関係を洗い出す → 実行して試す → 結果をユーザーに提示 → 判断を仰ぐ。「進めて」と言われるまでは議論を続ける。明示的に議論モードに入りたい場合は `gwt-discussion` を使用する。

## 最小十分アプローチ（Execution Discipline）

現在のタスクを**最小十分なアプローチ**で完了させる。過剰設計をしない。計画は厚くてよいが、実行は軽くあること。必要だと証明できない設計は入れない。必要だと証明できないテストは足さない。

### Workflow

1. コードに触る前に要求を理解する。コードを変えてから意図を推測しない。
2. 計画フェーズは高い推論を使ってよい。実行フェーズは軽量に保つ。**セッション全体を通して最大推論で走らせない。**
3. 既定では複数エージェントを同時に立てない。まず単独で 1 タスクを終わらせ、その後で分割が有効かを判断する。
4. タスクが実際に必要とするスキルだけを使う。重い手順を伴うスキルを不要に有効化しない。
5. 実行前に最小限の Plan を作る。Plan には Goal / Non-goals / Acceptance criteria / What stays untouched を含める。

### Failure Modes（避けるべき失敗）

1. 意図を理解せず表面だけ直した。
2. 1 つの根本修正で済むところに、パッチ・互換レイヤ・二重実装を積み上げた。
3. 稀なケースのために過剰設計し、日常のメンテナンスを高くつかせた。
4. 前提が誤ったまま推論を重ねた。誤った出発点は正しい推論では直らない。
5. コードを直接読むべき場面で検索や推測に頼った。
6. 「テスト追加」を口実にスコープ拡大・抽象化・見栄えの水増しをした。

### Action Boundaries

1. 着手前に「ユーザーが実際に求めていること / 今回のスコープ / 明示的なスコープ外 / 完了の定義」を言い直す。
2. 不可逆な操作はユーザー確認を先に取る。可逆な操作（revert・restore・テスト実行・diff 確認・読み取り分析）は確認不要。
3. 次を始めた自分に気づいたら止まり、より小さい Plan に切り替える: タスクに不要な抽象・設定レイヤの追加 / 将来の可能性のための先回り設計 / 制約を満たすための制約の積み増し / 無関係ファイルの広範な変更 / 旧ロジック温存のための二重実装 / テスト追加を理由にした作り込み。

### テストの範囲規律

テストは**今回の変更の受け入れ**に仕えるものであり、それ以外の目的で増やさない。TDD（RED を先に作る）・カバレッジ 90% 維持・GUI 変更の headed E2E といった本リポジトリの必須ゲートはそのまま適用した上で、次を守る:

1. まず変更に関連する既存テストを走らせる。既存テストで変更の正しさが証明できるなら新規テストは足さない。
2. 新規テストは「既存テストが覆えない挙動変更」または「ユーザーの明示要求」がある場合に限る。
3. 完全網羅のためのテスト拡大・無関係モジュールへの後追いテスト・snapshot 行列やパラメタライズ格子の量産をしない。
4. 今回の要求が求めていない境界をテストしない。緑のテストを更なる抽象化の口実にしない。
5. テスト追加前に自問する: このテストはどの受け入れ要件を検証するか / 無ければ既存テストはこのリグレッションを見逃すか / 実装より単純か。テストコードが実装より長く複雑なら過剰設計として扱う。

### テスト衛生ゲート（test hygiene gate）

テストの flake を事後に 1 件ずつ直す運用は機能しなかったため（SPEC #4551）、
既知の flake 機序は `crates/gwt-core/tests/test_hygiene_test.rs` が
**書いた時点で落とす**。テストコードを書く前に次を守る:

1. **壁時計に依存しない。** テストコード中の `Duration::from_millis(N)` は
   `N >= 100` のみ許可する。`from_micros` / `from_nanos` は不可。
   飽和した CI runner のスケジューリング遅延は数十 ms 単位で出るため、
   100ms 未満の実時間で順序を assert する前提は原理的に固定できない。
   deadline を「待つ」のではなく「経過を観測してから応答する」形にする。
2. **process 全体の状態は施錠して触る。** テスト関数内の `env::set_var` /
   `env::remove_var` / `env::set_current_dir` は、同じ関数内で
   `gwt_core::test_support::env_lock()` / `env_test_lock()` を取得するか、
   `ScopedEnvVar` / `ScopedGwtHome` で包む。
3. **正当な例外は理由付きで宣言する。** 違反行またはその直前行に
   `// test-hygiene: allow-short-duration <理由>` もしくは
   `// test-hygiene: allow-unlocked-env <理由>` を置く。理由本文が空なら通らない。
4. **`test_hygiene_baseline.txt` に行を足さない。** これは SPEC #4551 導入時点の
   既存違反を据え置いた一覧で、縮む一方の台帳である。違反を直したら該当行を削除する
   （残っているとゲートが落ちる）。一括整理の後は
   `cargo test -p gwt-core --test test_hygiene_test -- --ignored regenerate_the_baseline`
   で再生成できる。

### モデル配分（Model Allocation）

- 要求の明確化・Plan のレビュー: より強いモデルを使う。
- コードの記述・変更・テスト実行: 中〜低推論、もしくは軽量な実行モデルを使う。
- 実行モデルがアーキテクチャを積み上げ始めた、あるいはスコープを広げ始めたら止まる。最小の Plan を書き直してからやり直す。

### 完了前チェック（最小十分の観点）

- 意図と受け入れ条件を言い直したか / 解は最大ではなく最小十分か / Non-goals を明示したか
- 推測でなくコードを直接読んだか / 変更ファイルは最小集合か / 関連する既存テストを走らせたか
- 要求されていないシナリオのテストを足していないか / diff は小さく、余計なファイル・デバッグ残骸がないか
- 「完了に見せるための余計な仕事」をしていないか

## 開発指針

### 🛠️ 技術実装指針

- **設計・実装は複雑にせずに、シンプルさの極限を追求してください**
- **ただし、ユーザビリティと開発者体験の品質は決して妥協しない**
- 実装はシンプルに、開発者体験は最高品質に
- TUI 操作の直感性と効率性を技術的複雑さより優先
- **変更は外科的に行い、影響範囲を最小限にする。** 必要な箇所だけに手を入れ、新たなバグを持ち込まない
- **非自明な変更では、実装前に「もっともシンプルでエレガントな解」を比較し、採用理由を1行で明示する。**
- **場当たり的な修正（ワークアラウンド）を禁止する。** 必ず根本原因を特定してから修正すること。原因が不明な場合はログ・テスト・コードを調査し、推測で修正しない

### 🧩 GUI/TUI ガイドライン

- デスクトップ GUI は WebView (wry + tao) + axum WebSocket サーバー
- フロントエンド: HTML/JS/CSS (xterm.js でターミナル描画)
- バックエンド: Rust (`gwt` + `gwt-core` / `gwt-agent` / `gwt-skills` / `gwt-github` などのドメインクレート)
- ターミナルエミュレーション: vt100 crate
- UI アイコンは Unicode シンボルを使用する
- GUI/CSS 変更では Operator Design System (`crates/gwt/web/styles/tokens.css` と typography tokens) を必ず使用し、新規 UI CSS に raw hex / rgb / rgba 色や独自 palette を直書きしない。必要な token が無い場合は、理由を明示して token 追加を検討する
- Modal / dialog / overlay を追加・変更する場合は、共有 primitive (`.modal-backdrop` / `.modal-shell` / `.modal-header` / `.modal-body` / `.modal-footer`) と WAI-ARIA dialog 契約を使用し、独自の fixed/absolute overlay shell を作らない
- 新規・変更 UI surface では、Operator token 使用と共有 primitive 準拠が崩れやすい箇所に frontend contract test / embedded web test を追加・更新してから実装する

### 🔒 ブランチ保護ルール

- develop ブランチへの直接コミット / push は許可する。ただし共有ブランチなので、`origin/develop` を取り込んだ上で fast-forward を維持し、履歴を壊さないこと
- SPEC 策定・ブレインストーミングは develop 上のエージェントで行える。必要に応じて feature ブランチを使ってもよいが、develop でのコミット / push も禁止しない

### 📝 設計ガイドライン

- 設計に関するドキュメントには、ソースコードを書かないこと

## 開発品質

### 完了条件

- エラーが発生している状態で完了としないこと。必ずエラーが解消された時点で完了とする。
- 変更対象に応じた検証（テスト / lint / 型チェック）を実行し、成功を確認してから完了とする。
- 実行不能な検証がある場合は、未実施理由・代替確認・残リスクを明示する。未検証のまま「完了」と報告しない。
- gwtプロジェクトでは、単体テスト・結合テスト・E2Eテストを含む全体のテストカバレッジを 90% 以上で維持すること。
- **GUI / フロントエンドに影響する変更では、必ず Headed ブラウザ（実 Chromium）による E2E テストを実施すること。** headless・linkedom・unit テストのみの検証で完了としない。`browser-check` skill の隔離起動（checkout の `target/debug/gwt` + fresh HOME）を用い、dark / light 両テーマの表示確認と console / page error が無いことまで確認してから完了とする。
- **完了報告前のセルフチェックリスト（必須）:**
  - [ ] 対象の SPEC (GitHub Issue `gwt-spec` label) が最新状態に更新されているか
  - [ ] 全テスト通過・lint / 型チェック成功
  - [ ] 未実装・TODO が残っていないか
  - [ ] コミット＆プッシュ済みか

### Ready PR Gate（Ready 運用）

> 🚨 **Draft PR は廃止する（ユーザー裁定 2026-09-16）。PR は常に Ready で作成し、auto-merge を有効にする。**

- **Draft PR を作成しない。** `pr.create` は常に非 draft で行い、既存の Draft を見つけたら `pr.ready` で Ready 化する。「まだ途中だから Draft」という運用は行わない。
- **すべての PR に `auto-merge` を有効にする。** CI が緑になった時点で着地させる。配信の可否は CI の必須チェック 9 件が判定する。
- この裁定により、**配信可否の唯一のゲートは CI になる。** 「未完了だから Draft に留める」という緩衝は無くなるので、**PR のスコープを最初から単独で配信可能な大きさに切ること**が以前より重要になる。大きすぎる変更は 1 本の PR に詰めず分割する。
- 単独で配信可能とは、既存機能を壊さず、ユーザーに見える中途半端な挙動を出さず、rollback / follow-up 境界を PR 本文で説明できる状態を指す。残件がある場合は PR 本文に後続タスクとして明記し、**Draft に倒すのではなく follow-up Issue を立てる。**
- PR 作成前に `gwt-verify --mode pre-pr` の `Overall: PASS`、`User Verification Result` の確定、PR 本文 checklist 完了を確認する。**これは Ready 化の条件ではなく PR 作成の条件になった。**
- **検証が通らない変更は PR を作らない。** Draft という逃げ道が無くなったため、「とりあえず Draft で出して CI を見る」ことはできない。ローカルで検証してから PR を作る。
- **起動経路は実行記録から判定する（Issue #4217 FR-002）。** `execution.status` の `launch_route` が `autonomous` なら自動実行、`manual` または不明なら手動起動として扱う。`GWT_AUTONOMOUS_EXECUTION` は legacy シグナルであり、**設定されていることは autonomous の証拠になるが、設定されていないことは manual の証拠にならない**。この env は「プロジェクトが unattended mode を opt-in したか」でのみ書かれるため、Issue Monitor 起動でも未設定になり、実際に 2 窓が「手動起動」と誤判定して視覚検証待ちで停止した（#3777 / #3697）。
- **自動実行（`launch_route: autonomous`）では、実装・自動検証・Ready PR Gate を満たしたら Ready PR を作成し、既存の CI 自動マージまで完結させる（Issue #4326）。** ユーザーへ視覚確認を依頼せず、UI surface の有無にかかわらず `User Verification Result: n/a (autonomous)` を記録する。agent 自身の確認を人間の `confirmed` と偽ってはならない。
- **旧 `deferred (autonomous execution)` は移行互換として扱う。** 既存の自動実行 PR は本文を書き換えず、fresh な検証証跡と他の Ready Gate 条件を満たせば `pr.ready` / 非 draft の `pr.create` を実行できる。`pr.list` の `deferred_user_verification` は新旧の自動実行値では `false`、manual / 一般の deferred では `true` とする。本文を hydrate していない場合はフィールドを省略する。
- **自動実行では、人間の視覚確認の不在を理由に `execution.blocked` を打ってはならない。** 自動テスト・実 headed E2E・CI の失敗、既知 blocker、他の Ready Gate 未達は解消してから進む。`execution.blocked` は一時停止ではなく terminal であるため、一時的な検証待ちにも使用しない。
- GUI / フロントエンド変更では、`browser-check` による checkout + fresh HOME の隔離起動を使い、実 Chromium の headed E2E で dark / light 両テーマ、console / page error ゼロ、変更した機能の挙動を検証する。`verify.run` の `params.headed_e2e_commands` に `params.commands` 内の Playwright コマンドを完全一致で指定する。gwtd が `--headed` と組込 reporter を付加し、両テーマの実測 PASS 件数を記録する。script 経由の場合は追加の Playwright 引数を転送できること。
- 自動実行の UI Ready Gate は、同じ fresh 検証記録にある実測 headed PASS 証跡も必須とする。`Agent Visual Check: pass | fail(<reason>) | n/a (no UI surface)` は **`User Verification Result` とは別の行に**記録し、自己申告の pass のみを証跡としない。console / page error と機能 assertion は E2E 自体で確認する。
- Gate を満たさない場合は Draft のまま維持するか、Ready 化せず No Action として報告する。

## 開発ワークフロー

### 実装前ワークフロー（必須）

> 🚨 **エージェントは、以下のワークフローを完了するまでプロダクションコードの実装に着手してはならない。**

#### 1. 仕様策定（feat / fix / refactor 対象）

> 🚨 **既存 SPEC の検索が最優先。新規 SPEC の作成は、該当する既存 SPEC が存在しないことを確認した後の最終手段である。**

##### Step 1: 既存 SPEC を検索する（必須）

- 実装に入る前に、`gwt-search` で関連する既存 SPEC と Issue を必ず検索する
- 検索クエリは対象機能のキーワードを 2〜3 パターン試す（日本語・英語両方）
- `gwt-search` では JSON payload の `scopes:["specs"]` で SPEC を、`scopes:["issues"]` で Issue を絞り込める

##### Step 2: 既存 SPEC が見つかった場合 → 既存 SPEC Issue を更新する

- 該当 GitHub Issue (`gwt-spec` label) の `spec` section に不足しているユーザーストーリー・機能要件・受け入れシナリオを追加する
- `plan` section に新しいフェーズや実装ステップを追加する
- `tasks` section に新しいタスクを追加する
- SPEC section の読み書きは `gwtd` JSON operations `issue.spec.section` と `issue.spec.edit` を正本経路にする
- 同一 SPEC Issue の section 更新は逐次実行し、更新後に対象 section を読み直して parse 可能であることを確認する
- 対象の SPEC が確定した後は SPEC 管理ワークフローに従って実装進行を管理する

##### Step 3: 既存 SPEC が見つからない場合のみ → 新規 SPEC を作成する

- `gwt-discussion` を使って investigation-first で議論し、必要なら DDD ベースで SPEC 設計まで進める（調査 → ドメイン分析 → SPEC 登録/更新 → 仕様明確化）
- SPEC 登録は **`gwt-register-issue` の design-required 登録モード**で行う。gwt-discussion の Action Bundle で `Register Spec` を選択し、title + body file を渡せば、validation → JSON operation `issue.spec.create` → `issue.spec.edit` → roundtrip 検証を安全に実行する。`gwt-register-spec` は 1 release cycle の alias として残す。legacy create-body transport を直接使うと section マーカー漏れで空 SPEC が作成される（SPEC #2780 で発生、work-notes memory 参照）
- GitHub Issue (`gwt-spec` label) として作成する `spec` section には最低限以下を含める（design-required 登録 validation が強制する 8 セクション）:
  - 背景 / ユビキタス言語
  - ユーザーシナリオと受け入れシナリオ
  - 機能要件（FR-\*）
  - 成功基準（検証コマンドと期待結果。Issue Monitor は読まない）
  - 受け入れ基準（`- [ ] AC-N:` 形式のチェックリスト。Issue Monitor の autonomous gate が読む唯一の場所。`auto-merge` ラベル付きで欠けていると `issue.create` / `issue.spec.create` / `issue.spec.edit` が拒否する）
  - Out of Scope / Related Artifacts
- `gwt-plan-spec` で `plan` / `tasks` section も策定してから実装に入る
- 新規 SPEC を作成した場合でも、エージェントは自分で新規ブランチや Worktree を作成しない。実装に進む場合は、承認済み SPEC と `gwt-plan-spec` の成果物に基づき、現在起動されている branch/worktree で作業する。
- Git 環境の作成が必要な場合は、ユーザー操作に基づく gwt の Start Work / Launch materialization が担当する。

##### 共通ルール

- 通常の GitHub Issue から開始する場合は、既存 Issue なら `gwt-execute #N`、新規 work intake なら `gwt-register-issue` により direct / design-required / standalone のどれかへ進める
- 仕様策定時のユーザーインタビューでは以下を遵守する:
  - 表面的・ありきたりな質問を避け、技術実装・UX・トレードオフに踏み込んだ質問をする
  - 1回で終わらず、仕様が十分に詰まるまで継続的にインタビューする

#### 2. TDD（テストファースト）

- `gwt-execute` を使って TDD ベースで実装する（design-gated / direct / standalone mode）。旧 `gwt-build-spec` / `gwt-fix-issue` は transition alias として扱う
- 仕様の受け入れシナリオに基づき、**実装コードより先にテストコードを書く**
- Rust: `crates/*/tests/` または `#[cfg(test)]` モジュール内にテストを追加
- テストが RED（失敗）状態であることを確認してから実装に進む

#### 適用除外

以下の変更は仕様策定・TDD を省略できる。`fix:` タイプのバグ修正は適用除外に含めず、原因調査 → 仕様/SPEC 確認 → TDD → 再発防止確認の流れで扱う:

- `docs:` / `chore:` タイプの変更（ドキュメント修正、CI設定、依存更新など）
- 1行程度の明白な typo 修正
- AGENTS.md / CLAUDE.md / README.md の更新のみの変更

### Plan / Execute / Verify（必須）

- 中規模以上の作業（複数ファイル変更、仕様判断を伴う変更、原因調査が必要な不具合修正）では、実装前に短い Plan を作成する。
- Plan には最低限「何を変えるか」「どう検証するか」を含め、実装中に前提が崩れたら Plan を更新してから再開する。
- 不具合修正は、再現手順の確立 → 原因特定 → 修正 → 再発防止確認までを1サイクルで完了する。
- 仕様選択が不可逆な場合、またはプロダクト判断が必要な場合のみユーザーへ確認し、それ以外は自律的に進める。

### タスクトラッキング（tasks/）

- 中規模以上の作業では `tasks/todo.md` をローカル作業ログとして使用する。存在しない場合は作成し、Plan と進捗チェックボックスを管理する。
- `tasks/todo.md` には「背景」「実装ステップ」「検証結果」を残し、作業に合わせて更新する。ただし `tasks/todo.md` は version 管理しない。恒久的に残すべき内容は GitHub Issue / PR / README 等へ転記する。
- 再発防止に値する失敗、レビュー指摘、設計判断、agent workflow correction は JSON operation `memory.add` でマシンローカルの work-notes memory に `Type` / `Context` / `Learning` / `Future Action` の形式で記録する。legacy lessons alias も同じ work-notes memory に追記される。
- 同種の作業を始める前に work-notes memory を確認し、既知の失敗を繰り返さない。発見導線として `gwt-search` の JSON payload `scopes:["memory"]` または `/gwt:gwt-memory-search "<query>"` を使い、関連 memory が見つかった場合はその再発防止策を再利用する（SPEC #2805）。

### サブエージェント活用（並列化）

- 独立した作業単位（例: Rust修正、GUI修正、テスト整備）に分割できる場合はサブエージェントで並列実行する。
- 各サブエージェントには担当範囲・完了条件・検証観点を明示し、責務を重複させない。
- 統合担当は最終的に全変更を再レビューし、競合解消と統合検証を実施してから完了とする。

### 基本ルール

- 指示を受けた場合、まず既存実装・関連ドキュメント（AGENTS.md/CLAUDE.md/README.md）を確認し、必要なら先に更新する。
- 作業（タスク）を完了したら、変更点を日本語でコミットログに追加して、コミット＆プッシュを必ず行う
- 完了報告には、実行した検証コマンドと結果（成功/失敗、未実施項目）を必ず含める
- 作業（タスク）は、最大限の並列化をして進める
- `git rebase -i origin/main` はLLMでの失敗率が高いため禁止（必要な場合は人間が手動で整形すること）
- 作業（タスク）は、忖度なしで進める
- **エージェントはユーザーからの明示的な指示なく新規ブランチの作成・削除・切り替えを行ってはならない。`git checkout -b`、`git switch -c`、`git branch -D`、`git worktree add/remove` は禁止。Worktree は起動ブランチで作業を完結する設計であり、必要な Git 環境作成は gwt の Start Work / Launch materialization が行う。**
- 「進めて」等の承認指示は、承認済みタスクを自律的に完了まで進める指示である。不要な中間確認を挟まず、完了まで一気に進める
- **変更規模の大小に関わらず `feat` / `fix` / `refactor` は仕様策定（GitHub Issue-backed SPEC）・TDD を省略しない。** 「軽微だから省略」は禁止。適用除外は `docs:` / `chore:` / typo修正 / AGENTS.md / CLAUDE.md / README.md 更新のみ

### PR 作成ルール（必須）

> 🚨 **手動起動（ユーザーが自分で始めた作業）では、エージェントは、ユーザーの視覚検証結果が `confirmed` になる前に PR を `create` / `update` してはならない。自動実行（autonomous launch）では、以下の「自動実行時の扱い」に従い視覚検証を要求しない。**

- 手動起動では `gwt-verify --mode pre-pr` の **`User Verification Result`** が `confirmed` または `n/a`（UI 影響が無い変更で視覚検証不要な場合に限る）のいずれかになるまで PR 作成・更新を行わない。`pending` / 未確認のまま JSON operations `pr.create` / `pr.edit` を呼ばない。
- **自動実行時の扱い（ユーザー裁定 2026-09-14、Issue #4326）:** `execution.status` の `launch_route` が `autonomous` なら、ユーザーへ視覚確認を依頼せず、URL も出さない。`User Verification Result: n/a (autonomous)` と独立した `Agent Visual Check` を記録し、上記 Ready PR Gate の fresh 自動検証と UI 時の実測 headed 証跡が PASS したら、Ready PR 作成から CI 自動マージまで進める。旧 deferred 本文も同じ証跡で Ready にできる。自動実行で質問ツールを呼ぶと owner Issue が needs_human で park されるため、視覚確認待ちで実行を止めない。この扱いは起動経路による事実であり、`skipped(<reason>)` や `confirmed` へ書き換えない。
- （手動起動時）ユーザーが視覚検証できない状態（例: Open Project picker のクリックがブロックされている、splash から進めない、サーバーが起動しない 等）に遭遇した場合、エージェントの独断で `skipped(<reason>)` に倒さない。**まずブロッカーの根本原因を特定して解消し、ユーザーが実際に視覚確認できる状態を再現してから verification を依頼する**。
- `skipped(<reason>)` を許容するのは、ユーザーが `AskUserQuestion` 等で明示的に "Skip — proceed to PR" を選択した場合のみ。エージェントが「自動テスト全 PASS だから skip 妥当」と判断して skip するのは禁止。
- 「進めて」「OK」等の承認指示は、**既に verification 結果を持つ作業**を完了まで進める指示であり、verification 自体の skip 承認ではない。verification 動線がブロックされている時に「進めて」と言われた場合は、ブロッカー解消の作業を進める指示として解釈する。
- 万が一誤って PR を作成してしまった場合、即座に PR タイトルへ `[DO NOT MERGE — user verification pending]` を付与し、ブロック comment を投稿してマージを物理的に阻止する。verification が `confirmed` になってからタイトルを戻す。
- 過去事例: PR #2857（SPEC-2809）で `User Verification Result: skipped(reason: develop 側 picker regression)` をエージェントが独断で倒して PR を作成したのは skill 違反だった。原因は picker click-blocking という visualization blocker をエージェントが解消せずに skip に倒したこと。今後は同じ skip 判断を繰り返さない。

### コミットメッセージポリシー

> 🚨 **コミットログはリリースワークフローがバージョン判定に使用する唯一の真実であり、ここに齟齬があるとリリースバージョン・CHANGELOG 生成が即座に破綻します。commitlint を素通りさせることは絶対に許されません。**

- バージョン判定とリリースノート生成を Conventional Commits から自動化しているため、コミットメッセージは例外なく Conventional Commits 形式（`feat:`/`fix:`/`docs:`/`chore:` ...）で記述する。
- コミットを作成する前に、変更内容と Conventional Commits の種別（`feat`/`fix`/`docs` など）が 1 対 1 で一致しているかを厳格に突き合わせる。バージョン種別（major/minor/patch）がこの判定で決まるため、嘘の種類を付けた瞬間にバージョン管理が壊れる。
- ローカルでは `bunx commitlint --from HEAD~1 --to HEAD` などで必ず自己検証し、CI の commitlint に丸投げしない。エラーが出た状態で push しない。
- `feat:` はマイナーバージョン、`fix:` はパッチになる。**`type!:` と本文の `BREAKING CHANGE:` footer はバージョンを決めない**（Issue #4373）。marker を付けても `bump=auto` は minor 止まりで、marker は Prepare Release のログと Release PR 本文に情報として残るだけである。
- **メジャーバージョン昇格はユーザー（リリース起動者）が Prepare Release で `bump=major` を明示した場合のみ。** agent（PM を含む）が独断で `!` や `BREAKING CHANGE:` を書いてメジャーを狙ってはならない。互換性に影響する変更は commit 本文・PR 本文に説明として書き、昇格の要否はユーザーの裁定に委ねる。
- 1コミットで複数タスクを抱き合わせない。変更内容とコミットメッセージの対応関係を明確に保ち、解析精度を担保する。
- `chore:` や `docs:` などリリース対象外のタイプでも必ずプレフィックスを付け、曖昧な自然文だけのコミットメッセージを禁止する。
- コミット前に commitlint ルール（subject 空欄禁止・100文字以内など）を自己確認し、CI での差し戻しを防止する。

### ローカル検証/実行ルール（Rust）

- このリポジトリのローカル検証・実行は Cargo を使用する
- checkout のコードを実行する `gwtd` JSON operation（`execution.*` / `workspace.*` / `build.*` / `verify.*` / checkout で追加した operation）は、タスクまたはセッションの初回実行前に checkout root で `cargo build -p gwt --bin gwtd` を実行し、以後は `<checkout-root>/target/debug/gwtd` を明示的に使用する。`GWT_BIN_PATH` や `PATH` 上のバイナリは、version が一致する場合や checkout より新しい場合も checkout source と同じ実装であることを保証しないため、これらの operation には使用しない
- 読み取りの `issue.*` / `pr.*` / `board.*` / `search` は installed gwtd（`GWT_BIN_PATH` / PATH）で実行してよい。Issue / PR / Board の状態を知るためだけに build や lease を待たない
- 初回の `cargo build -p gwt --bin gwtd` は lease 不要の bootstrap step であり heavy な検証コマンドではない。順序は build → `verify.plan` → `verify.run` とし、lease を保持・待機したまま build しない（正本は `coordination_guidance.rs` の「gwtd bootstrap order」、生成 gwt-coordination / gwt-verify / gwt-search SKILL.md と同一文面）
- ビルド: `cargo build -p gwt --bin gwt --bin gwtd`
- 開発: `cargo run -p gwt --bin gwt`
- テスト: `cargo nextest run -p gwt-core -p gwt --all-features --test-threads=1`（`cargo install cargo-nextest --locked --version 0.9.146` で導入）。`.config/nextest.toml` により単一テストを120秒で失敗させ、後続を継続する。doctest は `cargo test --workspace --all-features --doc`。同一process内の競合調査・nightly flake検出には従来の `cargo test` を使うが、per-test timeoutは適用されない。
- このrepoの canonical matrix は上記nextestコマンドを明示 `verify.plan` に登録する。汎用deriveはcargo testを導出するため、そのまま流用しない。test-gh-guard付きbinaryの復旧用に `cargo build -p gwt --bin gwtd` を最後に含める。
- カバレッジ: `node scripts/coverage-summary.mjs --output-path target/coverage-summary.json -- --workspace --all-features` の後に `node scripts/check-coverage-threshold.mjs target/coverage-summary.json 90 --scope "crates/(gwt-core|gwt)/"` と `... 80 --scope-exclude "crates/(gwt-core|gwt)/"`（CI の coverage.yml と同一）。`cargo llvm-cov` を直接呼ぶと、raw profile の切り詰めがテスト失敗と区別できない FAIL になる（Issue #4628）
- Lint: `cargo clippy --all-targets --all-features -- -D warnings`
- フォーマット: `cargo fmt`
- GUI のユーザー確認が必要な実装では、ビルド済みなら `target/debug/gwt`、未ビルドなら `cargo run -p gwt --bin gwt` で起動し、標準出力の `gwt browser URL: http://127.0.0.1:<port>/` をユーザーに共有する。共有前に `curl -fsS -I <URL>` などで HTTP 200 を確認し、ユーザーが同じ URL で手動確認できる状態にする。
- 「デバッグ用サーバーを起動して」等の依頼は **`browser-check` skill**（`.claude/skills/browser-check/SKILL.md`）の手順に従う。production の `GWT.app` や既存 gwt インスタンスの URL を共有せず、この checkout の `target/debug/gwt` を隔離 HOME（fresh home + `~/.gwt/runtime` symlink + credential/`.docker` symlink + `session.json` seed）で `--no-tray --no-open` 起動し、`GWT_BROWSER_URL_FILE` から得た URL を HTTP 200 確認後に共有する。検査完了の連絡を受けたらプロセスを停止する。

## コミュニケーションガイドライン

- 回答は必ず日本語
- TUI のユーザー向け表示は英語のみ（日本語の文言を表示しない）
- ログ（`~/.gwt/logs/` 等）はこの環境から直接参照できる前提で対応すること
- ログ参照の指示があれば、この環境から直接読み取って調査すること

### Board / Work 運用ガイダンスの所在

Board / Work の operational rules（投稿 kind、audience selection、body template、
tool-unit post 禁止 など）は AGENTS.md には書かない。canonical source は
`crates/gwt-skills/src/coordination_guidance.rs` のみで、そこから 2 つの経路に配信される:

- **Generated guidance**: `.claude/skills/gwt-coordination/SKILL.md` および
  `.codex/skills/gwt-coordination/SKILL.md` に gwt materialization 時に書き込まれる
  自動配信スキル。target project の `AGENTS.md` / `CLAUDE.md` に Board 記述が無くても
  gwt-managed worktree であれば必ず適用される。
- **Managed hook reminder**: SessionStart / UserPromptSubmit / Stop hook で
  `board_reminder.rs` が動的注入する reminder text。`GWT_SESSION_ID` 環境変数が
  設定された session で発火する。

Board は coordination/history log、Work は current state という分離は維持する。
新しい kind や mention 構文の更新は canonical source を編集して再 materialize する。
AGENTS.md には複製しない（複製ドリフトが SPEC-1935 で問題化したため）。

## ドキュメント管理

- ドキュメントはREADME.md/README.ja.mdに集約する
- 仕様・要件は **GitHub Issue (`gwt-spec` ラベル)** に記載する。読み書きは `gwtd` JSON operations `issue.spec.*` 経由、ローカルキャッシュは `~/.gwt/cache/issues/`

### README.md / README.ja.md に必ず記載する内容

- 利用者向けの導線: インストール方法、起動方法、基本操作、主要機能の使い方
- 利用前提: サポートOS、初期設定（例: AI 機能を使う場合の設定）
- 開発者向けの最小情報: 前提環境、ビルド/開発手順、テスト実行方針（`cargo test` など）
- 配布情報: リリース/バイナリ資産の取得先、バージョン取得方法
- 代表的な画面操作: よく使う画面遷移や一般的なトラブル時の案内（再現しやすく簡潔）
- 変更が設計判断を必要とする場合の案内: 重要仕様の所在 (GitHub Issue `gwt-spec` ラベル、JSON operation `issue.spec.read` でアクセス)
- `CLAUDE.md` の運用ルールや内部実装ガイドは README に入れない
- 英語版/日本語版の内容は同等レベルを保つ（順序・見出しは対応させる）

## コードクオリティガイドライン

- マークダウンファイルはmarkdownlintでエラー及び警告がない状態にする
- コミットログはcommitlintに対応する

## 開発ガイドライン

- 既存のファイルのメンテナンスを無視して、新規ファイルばかり作成するのは禁止。既存ファイルを改修することを優先する。

## ドキュメント作成ガイドライン

- README.mdには設計などは書いてはいけない。プロジェクトの説明やディレクトリ構成などの説明のみに徹底する。設計などは、適切なファイルへのリンクを書く。

## パッケージ公開状況

| プラットフォーム | 確認コマンド |
| -------------- | ----------- |
| GitHub Release | `gh release list --repo akiojin/gwt --limit 1` |

## 使用中の技術

- Rust 2021 Edition (stable) + vt100, portable-pty, serde, tokio, axum, wry/tao, xterm.js (GUI terminal)
- GitHub Issue cache、ローカルファイル、Git メタデータ、ChromaDB / multilingual-e5 semantic index

## プロジェクト構成

```text
├── Cargo.toml          # ワークスペース設定
├── crates/
│   ├── gwt/            # GUI フロントエンド + gwtd JSON envelope CLI (WebView GUI)
│   ├── gwt-core/       # コアライブラリ（coordination / workspace / index など）
│   ├── gwt-agent/      # エージェント検出・起動・セッション管理
│   ├── gwt-skills/     # 組込スキル / 管理対象アセット配布
│   ├── gwt-github/     # GitHub Issue SOT for SPEC 管理 (SPEC-12)
│   └── ...             # AI / Git / Docker / terminal / config などのドメインクレート
└── scripts/            # 開発 / 検証 / リリース補助スクリプト
```

**SPEC 管理**: SPEC は `gwt-spec` ラベル付き GitHub Issue として格納される (#1930 SPEC-12 参照)。
読み取りは JSON operations `issue.spec.read` / `issue.spec.section`、書き込みは
`issue.spec.edit`、一覧は `issue.spec.list`。
ローカルキャッシュは `~/.gwt/cache/issues/` で UI レイヤーの唯一の真実
(一方向フロー: GitHub API → cache → UI、SPEC-12 FR-022)。

以下のスキル一覧は実運用の優先導線であり、該当するスキルがある場合は積極的に使用すること。

<!-- BEGIN gwt managed skills -->
## Available Skills & Commands (gwt)

Skills are located in `.claude/skills/<name>/SKILL.md`.
Commands can be invoked as `/gwt:<command-name>`.

### Public Task Entry Points

| Skill | Command | Description |
|-------|---------|-------------|
| gwt-register-issue | `/gwt:gwt-register-issue` | Register new work from a bug report, enhancement idea, docs task, or rough request. Creates a plain Issue or a design-required `gwt-spec` Issue after duplicate search. |
| gwt-execute | `/gwt:gwt-execute` | Execute an Issue-backed Work Item (`#N`) or approved standalone task through TDD, verification, and PR handoff. |
| gwt-fix-issue | `/gwt:gwt-fix-issue` | Transition alias to `gwt-execute #N` for existing Issue prompts. |
| gwt-discussion | `/gwt:gwt-discussion` | Investigate ideas, spec gaps, and implementation concerns. Updates `spec` / `plan` when discussion stabilizes and returns an action bundle for the next step. |
| gwt-plan-spec | `/gwt:gwt-plan-spec` | Generate or refresh `plan` / `tasks` and related planning artifacts for a SPEC. |
| gwt-build-spec | `/gwt:gwt-build-spec` | Transition alias to `gwt-execute #N` for approved SPEC prompts. |
| gwt-verify | `/gwt:gwt-verify` | Environment-aware verification. Classifies changed surfaces and runs the correct matrix (cargo for Rust crates, Bun/Node helpers for frontend JS, Playwright only for WebView/browser UI, release scripts only for release-system changes). Called from `gwt-execute` verify phase (`--mode full`) and `gwt-manage-pr` before PR create/update (`--mode pre-pr`). |
| gwt-manage-pr | `/gwt:gwt-manage-pr` | Create, inspect, update, or unblock a PR through one visible PR lifecycle entrypoint. |
| gwt-arch-review | `/gwt:gwt-arch-review` | Scan codebase architecture: domain boundaries (DDD), module depth (Ousterhout), testability, and agent-friendliness. Generates prioritized improvement report. Closes the feedback loop back to gwt-discussion. |

### Search & Agent Management

| Skill | Command | Description |
|-------|---------|-------------|
| gwt-search | `/gwt:gwt-search` | Unified semantic search over SPECs, GitHub Issues, project source files, and docs using ChromaDB. Uses JSON payload `scopes` filters and resolves `gwtd` through the managed skill contract. Mandatory preflight before gwt-discussion, gwt-register-issue, and visible owner routing decisions. |
| gwt-agent | `/gwt:gwt-agent` | Unified agent pane management through JSON operations `pane.list`, `pane.read`, and `pane.close`. Auto-detects mode: no args → list panes; pane ID → read output; stop/close + pane ID → stop pane. Use Board for agent-to-agent communication. |

### Recommended Workflow

```text
gwt-register-issue
          ↓
     gwt-discussion → gwt-plan-spec → gwt-execute → gwt-manage-pr
          ↑                                                |
          └────────────────── gwt-arch-review ─────────────┘
```

1. **Register new work** → `gwt-register-issue` (plain Issue か `gwt-spec` design-required Issue かを決める)
2. **Execute an existing Issue** → `gwt-execute #N` (Issue 起点で direct / design-gated / standalone を自動選択する)
3. **Discuss and shape the work** → `gwt-discussion` (investigation → design clarification → action bundle)
4. **Plan implementation** → `gwt-plan-spec` (SDD architecture → tasks)
5. **Build with TDD** → `gwt-execute` (Red-Green-Refactor → verification via `gwt-verify`)
6. **Verify changes** → `gwt-verify` (surface→matrix selection; Playwright only for WebView/browser UI)
7. **Manage PRs** → `gwt-manage-pr` (create, check, or fix; requires `gwt-verify --mode pre-pr` PASS)
8. **Review architecture** → `gwt-arch-review` (analysis → improvement proposals)
<!-- END gwt managed skills -->
