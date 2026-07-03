# AGENTS.md

このファイルは、このリポジトリでコードを扱う際のガイダンスを提供します。

## 適用範囲

- **この AGENTS.md は gwt リポジトリ専用**のローカル運用ルールであり、gwt が開く任意プロジェクト向けの汎用 Agent 指示ではない。
- gwt を使って他プロジェクトを開発する場合、そのプロジェクト自身の `AGENTS.md` / `CLAUDE.md` / README 等を優先する。
- gwt 共通の Agent 運用（Board/Work 更新、Start Work / Launch materialization、branch/worktree 操作の禁止）は、3 つの注入経路で配信する: managed hooks（SessionStart/UserPromptSubmit/Stop reminder）+ generated guidance（`.claude/skills/gwt-coordination/SKILL.md` および `.codex/skills/gwt-coordination/SKILL.md`）+ launch context（`GWT_SESSION_ID` 等）。canonical source は `crates/gwt-skills/src/coordination_guidance.rs` の 1 箇所。重複ドリフト防止のため、Board/Work の operational content（kind taxonomy、audience selection、body template、tool-unit post 禁止など）を AGENTS.md に複製しない。詳細な投稿手順は generated guidance 経由で agent に届く。

## エージェント運用原則

- **Plan Mode Default:** 非自明な作業、3ステップ以上のタスク、設計判断を含む変更では、実装前に Plan を作成する。途中で前提が崩れた場合は、作業を止めて Plan を更新してから再開する。
- **Self-Improvement Loop:** ユーザー修正、レビュー指摘、失敗から得た再発防止策や再利用可能な判断は `gwtd` JSON operation `memory.add` でマシンローカルの work-notes memory（`~/.gwt/projects/<repo-hash>/work-notes/memory.md`、SPEC-3214）に記録し、同種の作業を始める前に確認する。repo-local `.gwt/work/memory.md` / `tasks/memory.md` / `tasks/lessons.md` は読み取り fallback / legacy alias として扱う。
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
- **完了報告前のセルフチェックリスト（必須）:**
  - [ ] 対象の SPEC (GitHub Issue `gwt-spec` label) が最新状態に更新されているか
  - [ ] 全テスト通過・lint / 型チェック成功
  - [ ] 未実装・TODO が残っていないか
  - [ ] コミット＆プッシュ済みか

### Ready PR Gate（Draft / Ready 運用）

- `feat` / `fix` / `refactor` の途中成果は **Draft PR** のみ許可する。未完了・未検証・受け入れ未達・既知 blocker ありの変更は **Ready PR 禁止**。
- Ready PR は、その PR スコープが**単独で配信可能**であり、残件が配信 blocker ではない後続タスクとして明確な場合だけ許可する。
- 単独で配信可能とは、既存機能を壊さず、ユーザーに見える中途半端な挙動を出さず、rollback / follow-up 境界を PR 本文で説明できる状態を指す。
- Draft PR は CI / 共有 / 早期レビュー用とし、PR 本文に未完了項目、既知 blocker、Remaining acceptance を明記する。Draft PR で完了や配信可能性を主張しない。
- Ready 化前に `gwt-verify --mode pre-pr` の `Overall: PASS`、`User Verification Result` の確定、PR 本文 checklist 完了、既知 blocker なしを確認する。
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
- SPEC 登録は **`gwt-register-spec` skill 経由** で行う（SPEC-2784）。gwt-discussion の Action Bundle で `Register Spec` を選択し、title + body file を渡せば、skill が validation → JSON operation `issue.spec.create` → `issue.spec.edit` → roundtrip 検証を安全に実行する。legacy create-body transport を直接使うと section マーカー漏れで空 SPEC が作成される（SPEC #2780 で発生、work-notes memory 参照）
- GitHub Issue (`gwt-spec` label) として作成する `spec` section には最低限以下を含める（gwt-register-spec の validation が強制する 7 セクション）:
  - 背景 / ユビキタス言語
  - ユーザーシナリオと受け入れシナリオ
  - 機能要件（FR-\*）
  - 成功基準
  - Out of Scope / Related Artifacts
- `gwt-plan-spec` で `plan` / `tasks` section も策定してから実装に入る
- 新規 SPEC を作成した場合でも、エージェントは自分で新規ブランチや Worktree を作成しない。実装に進む場合は、承認済み SPEC と `gwt-plan-spec` の成果物に基づき、現在起動されている branch/worktree で作業する。
- Git 環境の作成が必要な場合は、ユーザー操作に基づく gwt の Start Work / Launch materialization が担当する。

##### 共通ルール

- 通常の GitHub Issue から開始する場合は、`gwt-fix-issue` または `gwt-register-issue` により直接修正・既存SPEC更新・新規SPEC作成のどれかを決定する
- 仕様策定時のユーザーインタビューでは以下を遵守する:
  - 表面的・ありきたりな質問を避け、技術実装・UX・トレードオフに踏み込んだ質問をする
  - 1回で終わらず、仕様が十分に詰まるまで継続的にインタビューする

#### 2. TDD（テストファースト）

- `gwt-build-spec` を使って TDD ベースで実装する（SPEC モードまたはスタンドアロンモード）。既存 Issue 起点の修正は `gwt-fix-issue` を優先する
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

> 🚨 **エージェントは、ユーザーの視覚検証結果が `confirmed` になる前に PR を `create` / `update` してはならない。**

- `gwt-verify --mode pre-pr` の **`User Verification Result`** が `confirmed` または `n/a`（UI 影響が無い変更で視覚検証不要な場合に限る）のいずれかになるまで PR 作成・更新を行わない。`pending` / 未確認のまま JSON operations `pr.create` / `pr.edit` を呼ばない。
- ユーザーが視覚検証できない状態（例: Open Project picker のクリックがブロックされている、splash から進めない、サーバーが起動しない 等）に遭遇した場合、エージェントの独断で `skipped(<reason>)` に倒さない。**まずブロッカーの根本原因を特定して解消し、ユーザーが実際に視覚確認できる状態を再現してから verification を依頼する**。
- `skipped(<reason>)` を許容するのは、ユーザーが `AskUserQuestion` 等で明示的に "Skip — proceed to PR" を選択した場合のみ。エージェントが「自動テスト全 PASS だから skip 妥当」と判断して skip するのは禁止。
- 「進めて」「OK」等の承認指示は、**既に verification 結果を持つ作業**を完了まで進める指示であり、verification 自体の skip 承認ではない。verification 動線がブロックされている時に「進めて」と言われた場合は、ブロッカー解消の作業を進める指示として解釈する。
- 万が一誤って PR を作成してしまった場合、即座に PR タイトルへ `[DO NOT MERGE — user verification pending]` を付与し、ブロック comment を投稿してマージを物理的に阻止する。verification が `confirmed` になってからタイトルを戻す。
- 過去事例: PR #2857（SPEC-2809）で `User Verification Result: skipped(reason: develop 側 picker regression)` をエージェントが独断で倒して PR を作成したのは skill 違反だった。原因は picker click-blocking という visualization blocker をエージェントが解消せずに skip に倒したこと。今後は同じ skip 判断を繰り返さない。

### コミットメッセージポリシー

> 🚨 **コミットログはリリースワークフローがバージョン判定に使用する唯一の真実であり、ここに齟齬があるとリリースバージョン・CHANGELOG 生成が即座に破綻します。commitlint を素通りさせることは絶対に許されません。**

- バージョン判定とリリースノート生成を Conventional Commits から自動化しているため、コミットメッセージは例外なく Conventional Commits 形式（`feat:`/`fix:`/`docs:`/`chore:` ...）で記述する。
- コミットを作成する前に、変更内容と Conventional Commits の種別（`feat`/`fix`/`docs` など）が 1 対 1 で一致しているかを厳格に突き合わせる。バージョン種別（major/minor/patch）がこの判定で決まるため、嘘の種類を付けた瞬間にバージョン管理が壊れる。
- ローカルでは `bunx commitlint --from HEAD~1 --to HEAD` などで必ず自己検証し、CI の commitlint に丸投げしない。エラーが出た状態で push しない。
- `feat:` はマイナーバージョン、`fix:` はパッチ、`type!:` もしくは本文の `BREAKING CHANGE:` はメジャー扱いになる。 breaking change を含む場合は例外なく `!` か `BREAKING CHANGE:` を記載し、破壊的変更を認識させる。
- 1コミットで複数タスクを抱き合わせない。変更内容とコミットメッセージの対応関係を明確に保ち、解析精度を担保する。
- `chore:` や `docs:` などリリース対象外のタイプでも必ずプレフィックスを付け、曖昧な自然文だけのコミットメッセージを禁止する。
- コミット前に commitlint ルール（subject 空欄禁止・100文字以内など）を自己確認し、CI での差し戻しを防止する。

### ローカル検証/実行ルール（Rust）

- このリポジトリのローカル検証・実行は Cargo を使用する
- ビルド: `cargo build -p gwt --bin gwt --bin gwtd`
- 開発: `cargo run -p gwt --bin gwt`
- テスト: `cargo test -p gwt-core -p gwt --all-features`
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
| gwt-register-issue | `/gwt:gwt-register-issue` | Register new work from a bug report, enhancement idea, docs task, or rough request. Decides plain Issue vs SPEC after duplicate search. |
| gwt-fix-issue | `/gwt:gwt-fix-issue` | Resolve an existing GitHub Issue by number or URL. Carries clear direct-fix work through implementation and routes to SPEC design only when needed. |
| gwt-discussion | `/gwt:gwt-discussion` | Investigate ideas, spec gaps, and implementation concerns. Updates `spec` / `plan` when discussion stabilizes and returns an action bundle for the next step. |
| gwt-plan-spec | `/gwt:gwt-plan-spec` | Generate or refresh `plan` / `tasks` and related planning artifacts for a SPEC. |
| gwt-build-spec | `/gwt:gwt-build-spec` | Implement an approved SPEC or approved standalone task with TDD, verification, and PR handoff. |
| gwt-verify | `/gwt:gwt-verify` | Environment-aware verification. Classifies changed surfaces and runs the correct matrix (cargo for Rust crates, Bun/Node helpers for frontend JS, Playwright only for WebView/browser UI, release scripts only for release-system changes). Called from `gwt-build-spec` Phase 3 (`--mode full`) and `gwt-manage-pr` before PR create/update (`--mode pre-pr`). |
| gwt-manage-pr | `/gwt:gwt-manage-pr` | Create, inspect, update, or unblock a PR through one visible PR lifecycle entrypoint. |
| gwt-arch-review | `/gwt:gwt-arch-review` | Scan codebase architecture: domain boundaries (DDD), module depth (Ousterhout), testability, and agent-friendliness. Generates prioritized improvement report. Closes the feedback loop back to gwt-discussion. |

### Search & Agent Management

| Skill | Command | Description |
|-------|---------|-------------|
| gwt-search | `/gwt:gwt-search` | Unified semantic search over SPECs, GitHub Issues, project source files, and docs using ChromaDB. Uses JSON payload `scopes` filters and resolves `gwtd` through the managed skill contract. Mandatory preflight before gwt-discussion, gwt-register-issue, and gwt-fix-issue. |
| gwt-agent | `/gwt:gwt-agent` | Unified agent pane management through JSON operations `pane.list`, `pane.read`, and `pane.close`. Auto-detects mode: no args → list panes; pane ID → read output; stop/close + pane ID → stop pane. Use Board for agent-to-agent communication. |

### Recommended Workflow

```text
gwt-register-issue / gwt-fix-issue
          ↓
     gwt-discussion → gwt-plan-spec → gwt-build-spec → gwt-manage-pr
          ↑                                                |
          └────────────────── gwt-arch-review ─────────────┘
```

1. **Register new work** → `gwt-register-issue` (plain Issue か SPEC かを決める)
2. **Fix an existing Issue** → `gwt-fix-issue` (Issue 起点で直接修正か SPEC 化かを決める)
3. **Discuss and shape the work** → `gwt-discussion` (investigation → design clarification → action bundle)
4. **Plan implementation** → `gwt-plan-spec` (SDD architecture → tasks)
5. **Build with TDD** → `gwt-build-spec` (Red-Green-Refactor → verification via `gwt-verify`)
6. **Verify changes** → `gwt-verify` (surface→matrix selection; Playwright only for WebView/browser UI)
7. **Manage PRs** → `gwt-manage-pr` (create, check, or fix; requires `gwt-verify --mode pre-pr` PASS)
8. **Review architecture** → `gwt-arch-review` (analysis → improvement proposals)
<!-- END gwt managed skills -->
