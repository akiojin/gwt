# AGENTS.md

このファイルは、このリポジトリでコードを扱う際のガイダンスを提供します。

## エージェント運用原則

- **Plan Mode Default:** 非自明な作業、3ステップ以上のタスク、設計判断を含む変更では、実装前に Plan を作成する。途中で前提が崩れた場合は、作業を止めて Plan を更新してから再開する。
- **Self-Improvement Loop:** ユーザー修正、レビュー指摘、失敗から得た再発防止策は `tasks/lessons.md` に記録し、同種の作業を始める前に確認する。
- **Verification Before Done:** 完了を宣言する前に、変更対象に応じたテスト、lint、型チェック、ログ確認、差分確認を実施し、スタッフエンジニアが承認できる状態かを基準にセルフレビューする。
- **Subagent Strategy:** 独立した調査、分析、実装、テスト整備はサブエージェントに分割し、メインのコンテキストを不要な詳細で汚さない。担当範囲、完了条件、検証観点を明示して責務を重複させない。
- **Demand Elegance:** 非自明な変更では、力技で実装する前に 2〜3 のアプローチを比較し、もっともシンプルで保守しやすい案を選ぶ。単純な修正では過剰設計しない。
- **Autonomous Bug Fixing:** バグ対応では、まず再現手順、ログ、失敗テスト、関連コードを自律的に調査し、原因特定、修正、再発防止確認まで進める。不可逆な仕様判断やプロダクト判断だけをユーザーに確認する。

## 開発指針

### 🛠️ 技術実装指針

- **設計・実装は複雑にせずに、シンプルさの極限を追求してください**
- **ただし、ユーザビリティと開発者体験の品質は決して妥協しない**
- 実装はシンプルに、開発者体験は最高品質に
- TUI 操作の直感性と効率性を技術的複雑さより優先
- **変更は外科的に行い、影響範囲を最小限にする。** 必要な箇所だけに手を入れ、新たなバグを持ち込まない
- **非自明な変更では、実装前に「もっともシンプルでエレガントな解」を比較し、採用理由を1行で明示する。**
- **場当たり的な修正（ワークアラウンド）を禁止する。** 必ず根本原因を特定してから修正すること。原因が不明な場合はログ・テスト・コードを調査し、推測で修正しない

### 🧩 TUI ガイドライン

- デスクトップ TUI は ratatui + crossterm
- バックエンド: Rust (gwt-core + gwt-tui)
- ターミナルエミュレーション: vt100 crate
- UI アイコンは Unicode シンボルを使用する

### 🔒 ブランチ保護ルール

- **develop ブランチへの直接コミットは禁止。** pre-commit hook によりブロックされる。作業は必ず feature ブランチで行い、PR 経由で develop にマージする
- SPEC 策定・ブレインストーミングは develop 上のエージェントで行えるが、コミットは feature/feature-{N} ブランチに切り替えてから実行する

### 📝 設計ガイドライン

- 設計に関するドキュメントには、ソースコードを書かないこと

## 開発品質

### 完了条件

- エラーが発生している状態で完了としないこと。必ずエラーが解消された時点で完了とする。
- 変更対象に応じた検証（テスト / lint / 型チェック）を実行し、成功を確認してから完了とする。
- 実行不能な検証がある場合は、未実施理由・代替確認・残リスクを明示する。未検証のまま「完了」と報告しない。
- **完了報告前のセルフチェックリスト（必須）:**
  - [ ] 対象の SPEC（`specs/SPEC-{N}/`）が最新状態に更新されているか
  - [ ] 全テスト通過・lint / 型チェック成功
  - [ ] 未実装・TODO が残っていないか
  - [ ] コミット＆プッシュ済みか

## 開発ワークフロー

### 実装前ワークフロー（必須）

> 🚨 **エージェントは、以下のワークフローを完了するまでプロダクションコードの実装に着手してはならない。**

#### 1. 仕様策定（feat / fix / refactor 対象）

> 🚨 **既存 SPEC の検索が最優先。新規 SPEC の作成は、該当する既存 SPEC が存在しないことを確認した後の最終手段である。**

##### Step 1: 既存 SPEC を検索する（必須）

- 実装に入る前に、`gwt-spec-search` で関連する既存 SPEC を必ず検索する
- 検索クエリは対象機能のキーワードを 2〜3 パターン試す（日本語・英語両方）
- `gwt-issue-search` でも関連 Issue を確認する

##### Step 2: 既存 SPEC が見つかった場合 → 既存 SPEC を更新する

- 該当 SPEC の `spec.md` に不足しているユーザーストーリー・機能要件・受け入れシナリオを追加する
- `plan.md` に新しいフェーズや実装ステップを追加する
- `tasks.md` に新しいタスクを追加する
- `metadata.json` の status/phase を必要に応じて更新する（例: `done` → `in-progress` に戻す）
- 対象の SPEC が確定した後は SPEC 管理ワークフローに従って実装進行を管理する

##### Step 3: 既存 SPEC が見つからない場合のみ → 新規 SPEC を作成する

- SPEC 登録ワークフローでローカル `specs/SPEC-{N}/` ディレクトリを作成する（N = 連番 SPEC ID）
- SPEC ディレクトリ内の `spec.md` に最低限以下を含める:
  - ユーザーシナリオとテスト（受け入れシナリオ）
  - 機能要件（FR-\*）
  - 成功基準
- `plan.md`、`tasks.md` も策定してから実装に入る
- 新規 SPEC を作成した場合、現在のブランチでは実装に入らず、SPEC に基づく別ブランチ（Worktree）で実装する
- 現在のコンバセーションでは SPEC 登録までで完了とする

##### 共通ルール

- 通常の GitHub Issue から開始する場合は、Issue 分析ワークフローにより直接修正・既存SPEC更新・新規SPEC作成のどれかを決定する
- 仕様策定時のユーザーインタビューでは以下を遵守する:
  - 表面的・ありきたりな質問を避け、技術実装・UX・トレードオフに踏み込んだ質問をする
  - 1回で終わらず、仕様が十分に詰まるまで継続的にインタビューする

#### 2. TDD（テストファースト）

- 仕様の受け入れシナリオに基づき、**実装コードより先にテストコードを書く**
- Rust: `crates/*/tests/` または `#[cfg(test)]` モジュール内にテストを追加
- テストが RED（失敗）状態であることを確認してから実装に進む

#### 適用除外

以下の変更は仕様策定・TDD を省略できる:

- `fix:` タイプのバグ修正（原因調査 → 修正 → 再発防止確認の Plan/Execute/Verify で管理する）
- `docs:` / `chore:` タイプの変更（ドキュメント修正、CI設定、依存更新など）
- 1行程度の明白な typo 修正
- CLAUDE.md / README.md の更新のみの変更

### Plan / Execute / Verify（必須）

- 中規模以上の作業（複数ファイル変更、仕様判断を伴う変更、原因調査が必要な不具合修正）では、実装前に短い Plan を作成する。
- Plan には最低限「何を変えるか」「どう検証するか」を含め、実装中に前提が崩れたら Plan を更新してから再開する。
- 不具合修正は、再現手順の確立 → 原因特定 → 修正 → 再発防止確認までを1サイクルで完了する。
- 仕様選択が不可逆な場合、またはプロダクト判断が必要な場合のみユーザーへ確認し、それ以外は自律的に進める。

### タスクトラッキング（tasks/）

- 中規模以上の作業では `tasks/todo.md` をローカル作業ログとして使用する。存在しない場合は作成し、Plan と進捗チェックボックスを管理する。
- `tasks/todo.md` には「背景」「実装ステップ」「検証結果」を残し、作業に合わせて更新する。ただし `tasks/todo.md` は version 管理しない。恒久的に残すべき内容は GitHub Issue / PR / README 等へ転記する。
- 再発防止に値する失敗やレビュー指摘は `tasks/lessons.md` に「事象 / 原因 / 再発防止策」の形式で記録する。
- 同種の作業を始める前に `tasks/lessons.md` を確認し、既知の失敗を繰り返さない。

### サブエージェント活用（並列化）

- 独立した作業単位（例: Rust修正、GUI修正、テスト整備）に分割できる場合はサブエージェントで並列実行する。
- 各サブエージェントには担当範囲・完了条件・検証観点を明示し、責務を重複させない。
- 統合担当は最終的に全変更を再レビューし、競合解消と統合検証を実施してから完了とする。

### 基本ルール

- 指示を受けた場合、まず既存実装・関連ドキュメント（README/CLAUDE.md）を確認し、必要なら先に更新する。
- 作業（タスク）を完了したら、変更点を日本語でコミットログに追加して、コミット＆プッシュを必ず行う
- 完了報告には、実行した検証コマンドと結果（成功/失敗、未実施項目）を必ず含める
- 作業（タスク）は、最大限の並列化をして進める
- `git rebase -i origin/main` はLLMでの失敗率が高いため禁止（必要な場合は人間が手動で整形すること）
- 作業（タスク）は、忖度なしで進める
- **エージェントはユーザーからの明示的な指示なく新規ブランチの作成・削除を行ってはならない。Worktreeは起動ブランチで作業を完結する設計。**
- 「進めて」等の承認指示は、承認済みタスクを自律的に完了まで進める指示である。不要な中間確認を挟まず、完了まで一気に進める
- **変更規模の大小に関わらず `feat` / `fix` / `refactor` は仕様策定（ローカル SPEC）・TDD を省略しない。** 「軽微だから省略」は禁止。適用除外は `docs:` / `chore:` / typo修正 / CLAUDE.md更新のみ

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
- ビルド: `cargo build -p gwt-tui`
- 開発: `cargo run -p gwt-tui`
- テスト: `cargo test -p gwt-core -p gwt-tui`
- Lint: `cargo clippy --all-targets --all-features -- -D warnings`
- フォーマット: `cargo fmt`

## コミュニケーションガイドライン

- 回答は必ず日本語
- TUI のユーザー向け表示は英語のみ（日本語の文言を表示しない）
- ログ（`~/.gwt/logs/` 等）はこの環境から直接参照できる前提で対応すること
- ログ参照の指示があれば、この環境から直接読み取って調査すること

## ドキュメント管理

- ドキュメントはREADME.md/README.ja.mdに集約する
- 仕様・要件は `specs/SPEC-{N}/` のローカルファイルに記載する。`metadata.json` の status で管理

### README.md / README.ja.md に必ず記載する内容

- 利用者向けの導線: インストール方法、起動方法、基本操作、主要機能の使い方
- 利用前提: サポートOS、初期設定（例: AI 機能を使う場合の設定）
- 開発者向けの最小情報: 前提環境、ビルド/開発手順、テスト実行方針（`cargo test` など）
- 配布情報: リリース/バイナリ資産の取得先、バージョン取得方法
- 代表的な画面操作: よく使う画面遷移や一般的なトラブル時の案内（再現しやすく簡潔）
- 変更が設計判断を必要とする場合の案内: 重要仕様の所在（`specs/SPEC-{N}/` ディレクトリへの参照）
- `CLAUDE.md` の運用ルールや内部実装ガイドは README に入れない
- 英語版/日本語版の内容は同等レベルを保つ（順序・見出しは対応させる）

## コードクオリティガイドライン

- マークダウンファイルはmarkdownlintでエラー及び警告がない状態にする
- コミットログはcommitlintに対応する

## 開発ガイドライン

- 既存のファイルのメンテナンスを無視して、新規ファイルばかり作成するのは禁止。既存ファイルを改修することを優先する。

## ドキュメント作成ガイドライン

- README.mdには設計などは書いてはいけない。プロジェクトの説明やディレクトリ構成などの説明のみに徹底する。設計などは、適切なファイルへのリンクを書く。

## リリースワークフロー

- feature/\* ブランチは develop への PR を作成し、オーナー承認後にマージする。develop で次回リリース候補を蓄積する。
- **main への PR は develop からのみ許可。** それ以外のブランチ（feature/\*、release-\* 等）から main への直接 PR は禁止。CI（`pr-source-check.yml`）でも拒否される。
- `/release` コマンドで Release PR を作成:
  - Conventional Commits を解析してバージョン自動判定（feat→minor, fix→patch, !→major）
  - git-cliff で CHANGELOG.md を更新
  - Cargo.toml, package.json のバージョンを更新
  - develop → main への PR を作成（リリースブランチは作成しない）
- Release PR が main にマージされると `.github/workflows/release.yml` が以下を自動実行:
  - タグ・GitHub Release を作成
  - ビルド済みバイナリを GitHub Release にアップロード

## パッケージ公開状況

| プラットフォーム | 確認コマンド |
| -------------- | ----------- |
| GitHub Release | `gh release list --repo akiojin/gwt --limit 1` |

## 使用中の技術

- Rust 2021 Edition (stable) + ratatui, crossterm, vt100, portable-pty, serde, tokio
- ローカルファイルと Git メタデータ（DB なし）

## プロジェクト構成

```text
├── Cargo.toml          # ワークスペース設定
├── crates/
│   ├── gwt-core/       # コアライブラリ（Git操作・PTY管理・設定）
│   └── gwt-tui/        # ratatui TUI フロントエンド
├── specs/              # ローカル SPEC 管理（SPEC-{N}/）
│   └── SPEC-*/         # 各 SPEC のアーティファクト（spec.md, plan.md, tasks.md 等）
└── package.json        # 開発用スクリプト
```

<!-- BEGIN gwt managed skills -->
## Available Skills & Commands (gwt)

Skills are located in `.claude/skills/<name>/SKILL.md`.
Commands can be invoked as `/gwt:<command-name>`.

### Issue & SPEC Management

| Skill | Command | Description |
|-------|---------|-------------|
| gwt-issue-register | `/gwt:gwt-issue-register` | Register new GitHub work items from a request. Search existing Issues and SPECs first, reuse a clear existing owner when possible, otherwise create a plain GitHub Issue or continue into the SPEC workflow. Use when user says "register an issue", "create a new issue", "file a bug", "add feature request", or asks to track new work items. |
| gwt-issue-resolve | `/gwt:gwt-issue-resolve` | Resolve an existing GitHub Issue end-to-end. Analyze the issue, decide whether it should be fixed directly, merged into an existing SPEC, or promoted to a new SPEC, and continue toward resolution. Use when user says "resolve this issue", "fix issue #N", "progress this issue", or brings a GitHub Issue URL to be worked on. |
| gwt-issue-search | `/gwt:gwt-issue-search` | Semantic search over all GitHub Issues using vector embeddings. Use when user says "search issues", "find related issues", "check for duplicates", or asks which issue owns a scope. Mandatory preflight before gwt-spec-register, gwt-spec-ops, gwt-issue-register, and gwt-issue-resolve. |
| gwt-spec-brainstorm | `/gwt:gwt-spec-brainstorm` | Cross-agent pre-SPEC intake for rough ideas. Interview the user one question at a time, search existing Issues and SPECs first, then route automatically to an existing Issue or SPEC, a new SPEC, or a plain Issue. Use when user says "brainstorm this before writing a spec", starts from a title-level request, asks whether something should become a SPEC, or asks whether an existing SPEC should be updated. |
| gwt-spec-register | `/gwt:gwt-spec-register` | Create a new local SPEC directory when no existing canonical SPEC fits. Create specs/SPEC-{id}/ with metadata.json + spec.md, then continue into SPEC orchestration. Use when user says "create a new spec", "register a spec", "new SPEC for this feature", or asks to start a spec from scratch. |
| gwt-spec-clarify | `/gwt:gwt-spec-clarify` | Clarify an existing SPEC by resolving [NEEDS CLARIFICATION] markers, tightening user stories, and locking acceptance scenarios before planning. Use when user says "clarify this spec", "resolve clarifications", "tighten the spec", or when spec.md has unresolved markers. |
| gwt-spec-plan | `/gwt:gwt-spec-plan` | Generate plan.md, research.md, data-model.md, quickstart.md, and contracts/* planning artifacts for an existing SPEC, including a constitution check. Use when user says "plan this spec", "generate a plan", "create planning artifacts", or when a clarified spec.md needs implementation planning. |
| gwt-spec-tasks | `/gwt:gwt-spec-tasks` | Generate tasks.md for an existing SPEC from spec.md and plan.md, grouped by phase and user story with exact file paths, [P] parallel markers, and test-first ordering. Use when user says "generate tasks", "create tasks.md", "break down the plan into tasks", or when plan.md is ready for task decomposition. |
| gwt-spec-deepen | `/gwt:gwt-spec-deepen` | Interactively deepen an existing SPEC's specifications and detail its tasks through a two-phase workshop: analysis report of deepening points, then focused deep-dive on user-selected points. Use when user says "deepen this spec", "dig deeper", "challenge assumptions", "detail the tasks", or wants to explore alternatives and hidden requirements. |
| gwt-spec-analyze | `/gwt:gwt-spec-analyze` | Analyze a SPEC artifact set for completeness and consistency across spec.md, plan.md, tasks.md, and supporting artifacts. Detect missing traceability, unresolved clarifications, and constitution gaps before implementation. Use when user says "analyze this spec", "check spec completeness", "run the analysis gate", or before starting implementation on a spec. |
| gwt-spec-ops | — | Local SPEC directory orchestration. Use an existing or newly created SPEC directory to stabilize spec.md, plan.md, tasks.md, analysis gates, and then continue into implementation. Use when user says "run spec workflow", "orchestrate this spec", "stabilize the spec", or asks to drive a spec end-to-end from clarification through implementation. |
| gwt-spec-implement | `/gwt:gwt-spec-implement` | Implement an existing SPEC end-to-end from tasks.md. Execute test-first tasks, update progress artifacts, and keep PR work moving until the SPEC is done. Use when user says "implement this spec", "start implementation", "execute the tasks", or when a spec has passed the analysis gate and is ready for coding. |
| gwt-spec-search | `/gwt:gwt-spec-search` | Semantic search over local SPEC files (specs/SPEC-{N}/) using vector embeddings. Use when user says "search specs", "find related specs", "check for duplicate specs", or asks which spec owns a scope. Mandatory preflight before gwt-spec-register and gwt-spec-ops. |

### PR Management

| Skill | Command | Description |
|-------|---------|-------------|
| gwt-pr | `/gwt:gwt-pr` | Create or update GitHub Pull Requests with the gh CLI, preferring REST-first gh api flows. Use when the user asks to open/create/edit a PR, generate a PR body/template, or says "open a PR/create a PR/gh pr". Defaults: base=develop, head=current branch. |
| gwt-pr-check | `/gwt:gwt-pr-check` | Check GitHub PR status with REST-first PR lookups, including unmerged PR detection and post-merge new-commit detection. Use when user says "check PR status", "is the PR merged?", "PR state", or asks about the current branch's pull request progress. |
| gwt-pr-fix | `/gwt:gwt-pr-fix` | Inspect GitHub PR for CI failures, merge conflicts, reviewer comments, and unresolved review threads. Autonomously fix high-confidence blockers and reply to ALL reviewer comments. Use when user says "fix CI", "fix the PR", "CI is failing", "resolve PR blockers", or after creating/pushing a PR when CI failures are detected. |

### Agent Pane Management

| Skill | Command | Description |
|-------|---------|-------------|
| gwt-agent-discover | `/gwt:gwt-agent-discover` | List active agent panes with their IDs, agent types, branches, and statuses. Use when user says "list panes", "what agents are running?", "show active agents", or when discovering available panes before dispatch. |
| gwt-agent-read | `/gwt:gwt-agent-read` | Read the scrollback tail of an agent pane to check progress and status. Use when user says "check pane output", "read agent output", "what is the agent doing?", or when monitoring agent progress. |
| gwt-agent-send | `/gwt:gwt-agent-send` | Send key input to a specific agent pane or broadcast to all panes. Use when user says "send to pane", "dispatch to agent", "broadcast instructions", or when dispatching tasks to agents. |
| gwt-agent-lifecycle | `/gwt:gwt-agent-lifecycle` | Stop an agent pane when escalation is needed or the agent is stuck. Use when user says "stop the agent", "close pane", "escalation needed", or when managing pane lifecycle. |

### Utilities

| Skill | Command | Description |
|-------|---------|-------------|
| gwt-project-search | `/gwt:gwt-project-search` | Semantic search over project source files using vector embeddings. Use to find files related to a feature, bug, or concept. |
| gwt-spec-to-issue-migration | — | Migrate GitHub Issue-based specs to local SPEC directories. Supports reverse migration from gwt-spec Issues to local specs/SPEC-{id}/ directories using the bundled migration script. |

### Recommended Workflow

See each skill's SKILL.md for detailed instructions:

1. **Brainstorm a rough request** → `gwt-spec-brainstorm`
2. **Register work** → `gwt-issue-register`
3. **Resolve an existing issue** → `gwt-issue-resolve`
4. **Create or select SPEC** → `gwt-spec-register` / `gwt-spec-ops`
5. **Clarify / plan / tasks / analyze** → `gwt-spec-ops`
6. **Implement SPEC tasks** → `gwt-spec-implement`
7. **Open PR** → `gwt-pr`
8. **Fix CI / reviews** → `gwt-pr-fix`
<!-- END gwt managed skills -->
