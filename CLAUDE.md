# CLAUDE.md

このファイルは、このリポジトリでコードを扱う際のガイダンスを提供します。

## 開発指針

### 🛠️ 技術実装指針

- **設計・実装は複雑にせずに、シンプルさの極限を追求してください**
- **ただし、ユーザビリティと開発者体験の品質は決して妥協しない**
- 実装はシンプルに、開発者体験は最高品質に
- GUI操作の直感性と効率性を技術的複雑さより優先
- **変更は外科的に行い、影響範囲を最小限にする。** 必要な箇所だけに手を入れ、新たなバグを持ち込まない
- **非自明な変更では、実装前に「もっともシンプルでエレガントな解」を比較し、採用理由を1行で明示する。**
- **場当たり的な修正（ワークアラウンド）を禁止する。** 必ず根本原因を特定してから修正すること。原因が不明な場合はログ・テスト・コードを調査し、推測で修正しない

### 🧩 Tauri GUI ガイドライン

- デスクトップGUI は Tauri v2 + Svelte 5 + xterm.js
- バックエンド: Rust (gwt-core + gwt-tauri)
- フロントエンド: Svelte 5 + TypeScript + Vite (gwt-gui/)
- ターミナルエミュレーション: xterm.js v6
- UIアイコンはGUIに適したアイコン（SVG / Unicode シンボル等）を使用する

### 📝 設計ガイドライン

- 設計に関するドキュメントには、ソースコードを書かないこと

## 開発品質

### 完了条件

- エラーが発生している状態で完了としないこと。必ずエラーが解消された時点で完了とする。
- 変更対象に応じた検証（テスト / lint / 型チェック）を実行し、成功を確認してから完了とする。
- 実行不能な検証がある場合は、未実施理由・代替確認・残リスクを明示する。未検証のまま「完了」と報告しない。
- **完了報告前のセルフチェックリスト（必須）:**
  - [ ] 対象の `gwt-spec` Issue が最新状態に更新されているか
  - [ ] 全テスト通過・lint / 型チェック成功
  - [ ] 未実装・TODO が残っていないか
  - [ ] コミット＆プッシュ済みか

## 開発ワークフロー

### 実装前ワークフロー（必須）

> 🚨 **エージェントは、以下のワークフローを完了するまでプロダクションコードの実装に着手してはならない。**

#### 1. 仕様策定（feat / fix / refactor 対象）

- 新機能・バグ修正・リファクタリングの実装前に、GitHub Issue（`gwt-spec` ラベル）を作成する。Issue 番号 = SPEC ID
- Issue body のセクション構造に従い、最低限以下を含める:
  - ユーザーシナリオとテスト（受け入れシナリオ）
  - 機能要件（FR-*）
  - 成功基準
- Issue body の `## Plan`、`## Tasks` セクションも策定してから実装に入る
- 通常の GitHub Issue から開始する場合は、Issue 分析ワークフローにより直接修正・既存SPEC更新・新規SPEC作成のどれかを決定する
- 新規 SPEC を明示的に起票する場合は SPEC 登録ワークフローで `gwt-spec` Issue を作成する
- 対象の `gwt-spec` Issue が確定した後は SPEC 管理ワークフローに従って Spec/Plan/Tasks を更新し、実装進行を管理する
- 仕様策定時のユーザーインタビューでは以下を遵守する:
  - 表面的・ありきたりな質問を避け、技術実装・UX・トレードオフに踏み込んだ質問をする
  - 1回で終わらず、仕様が十分に詰まるまで継続的にインタビューする
  - 既存の `gwt-spec` Issue が存在しないか必ず確認してから、新規作成・更新を判断する

#### 2. TDD（テストファースト）

- 仕様の受け入れシナリオに基づき、**実装コードより先にテストコードを書く**
- Rust: `crates/*/tests/` または `#[cfg(test)]` モジュール内にテストを追加
- Frontend: `gwt-gui/src/**/*.test.ts` にテストを追加（vitest + @testing-library/svelte）
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
- **変更規模の大小に関わらず `feat` / `fix` / `refactor` は仕様策定（`gwt-spec` Issue）・TDD を省略しない。** 「軽微だから省略」は禁止。適用除外は `docs:` / `chore:` / typo修正 / CLAUDE.md更新のみ

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

- このリポジトリのローカル検証・実行は Cargo + Tauri CLI を使用する
- ビルド: `cargo tauri build`
- 開発: `cargo tauri dev`
- テスト: `cargo test`
- Lint: `cargo clippy --all-targets --all-features -- -D warnings`
- フォーマット: `cargo fmt`
- フロントエンドチェック: `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json`

### フロントエンド実行前セットアップ（gwt-gui）

- `gwt-gui` の依存はこの配下で管理されており、未インストールだと `vitest` / `@tsconfig/svelte` が見つからないエラーになります。  
  まず `cd gwt-gui && pnpm install` を実行してください（Node 依存の初回/クリーン環境用）。
- `vitest` を実行する場合は `cd gwt-gui && pnpm test` を使います。  
  ファイルを限定する場合は `cd gwt-gui && pnpm test src/lib/components/Sidebar.test.ts src/lib/components/WorktreeSummaryPanel.test.ts` のように指定します。

### フロントエンド E2E（Playwright）手順

- `gwt-gui/e2e/` 配下の WebUI E2E は Playwright で実行します（`playwright.config.ts` の Chromium 設定を使用）。
- 依存が未取得の場合は `cd gwt-gui && pnpm install` の後、初回のみブラウザバイナリを取得します。
  - `cd gwt-gui && pnpm exec playwright install chromium`
- E2E 実行コマンド:
  - `cd gwt-gui && pnpm test:e2e`
- Playwright 側のローカルサーバー起動は自動です（`http://127.0.0.1:4173`）。必要なら個別実行で絞り込みます。
  - `cd gwt-gui && pnpm exec playwright test e2e/open-project-smoke.spec.ts`

## コミュニケーションガイドライン

- 回答は必ず日本語
- GUIのユーザー向け表示は英語のみ（日本語の文言を表示しない）
- ログ（`~/.gwt/logs/` 等）はこの環境から直接参照できる前提で対応すること
- ログ参照の指示があれば、この環境から直接読み取って調査すること

## ドキュメント管理

- ドキュメントはREADME.md/README.ja.mdに集約する
- 仕様・要件は GitHub Issue（`gwt-spec` ラベル）に記載する。Issue の close で管理

### README.md / README.ja.md に必ず記載する内容

- 利用者向けの導線: インストール方法、起動方法、基本操作、主要機能の使い方
- 利用前提: サポートOS、初期設定（例: AI 機能を使う場合の設定）
- 開発者向けの最小情報: 前提環境、ビルド/開発手順、テスト実行方針（`pnpm test`, E2Eなど）
- 配布情報: リリース/バイナリ資産の取得先、バージョン取得方法
- 代表的な画面操作: よく使う画面遷移や一般的なトラブル時の案内（再現しやすく簡潔）
- 変更が設計判断を必要とする場合の案内: 重要仕様の所在（GitHub Issue `gwt-spec` ラベルへの参照）
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
  - Tauri ビルド（.dmg/.msi/.AppImage）を GitHub Release にアップロード

## パッケージ公開状況

| プラットフォーム | 確認コマンド |
| -------------- | ----------- |
| GitHub Release | `gh release list --repo akiojin/gwt --limit 1` |

## 使用中の技術
- Rust 2021 Edition (stable) + Tauri v2, portable-pty, serde, tokio
- Svelte 5 + TypeScript + Vite 6
- xterm.js v6 (@xterm/xterm, @xterm/addon-fit, @xterm/addon-web-links)
- ローカルファイルと Git メタデータ（DB なし）

## プロジェクト構成

```text
├── Cargo.toml          # ワークスペース設定
├── crates/
│   ├── gwt-core/       # コアライブラリ（Git操作・PTY管理・設定）
│   └── gwt-tauri/      # Tauri v2 バックエンド（コマンド・状態管理）
├── gwt-gui/            # Svelte 5 フロントエンド（UI・xterm.js）
│   ├── src/
│   │   ├── lib/components/  # UIコンポーネント
│   │   ├── lib/terminal/    # xterm.jsラッパー
│   │   └── lib/types.ts     # TypeScript型定義
│   └── package.json
└── package.json        # Tauri開発用スクリプト
```

<!-- BEGIN gwt managed skills -->
## Available Skills & Commands (gwt)

Skills are located in `.claude/skills/<name>/SKILL.md`.
Commands can be invoked as `/gwt:<command-name>`.

### Issue & SPEC Management

| Skill | Command | Description |
|-------|---------|-------------|
| gwt-issue-register | `/gwt:gwt-issue-register` | Register new GitHub work items from a request. Search existing Issues and `gwt-spec` Issues first, reuse a clear existing owner when possible, otherwise create a plain GitHub Issue or continue into the SPEC workflow. Use as the main entrypoint for new Issue/SPEC registration requests. |
| gwt-issue-resolve | `/gwt:gwt-issue-resolve` | Resolve an existing GitHub Issue end-to-end. Analyze the issue, decide whether it should be fixed directly, merged into an existing gwt-spec issue, or promoted to a new spec issue, and continue toward resolution. Use `gwt-issue-register` for brand-new work registration. |
| gwt-issue-search | `/gwt:gwt-issue-search` | Semantic search over GitHub gwt-spec Issues using vector embeddings. Use before creating or updating any spec issue. |
| gwt-spec-register | `/gwt:gwt-spec-register` | Create a new GitHub Issue-first SPEC container when no existing canonical SPEC fits. Seed the Issue body as an artifact index plus a `spec.md` comment, then continue into SPEC orchestration unless the user explicitly asks for register-only behavior. |
| gwt-spec-clarify | `/gwt:gwt-spec-clarify` | Clarify an existing `gwt-spec` by resolving `[NEEDS CLARIFICATION]` markers, tightening user stories, and locking acceptance scenarios before planning. Use directly or through `gwt-spec-ops`. |
| gwt-spec-plan | `/gwt:gwt-spec-plan` | Generate planning artifacts for an existing `gwt-spec`: `plan.md`, `research.md`, `data-model.md`, `quickstart.md`, and `contracts/*`, including a constitution check against `memory/constitution.md`. Use directly or through `gwt-spec-ops`. |
| gwt-spec-tasks | `/gwt:gwt-spec-tasks` | Generate `tasks.md` for an existing `gwt-spec` from `spec.md` and `plan.md`, grouped by phase and user story with exact file paths, `[P]` parallel markers, and test-first ordering. Use directly or through `gwt-spec-ops`. |
| gwt-spec-analyze | `/gwt:gwt-spec-analyze` | Analyze a `gwt-spec` artifact set for completeness and consistency across `spec.md`, `plan.md`, `tasks.md`, and supporting artifacts. Detect missing traceability, unresolved clarifications, and constitution gaps before implementation, and distinguish auto-fixable gaps from true decision blockers. |
| gwt-spec-ops | — | GitHub Issue-first SPEC orchestration. Use an existing or newly created `gwt-spec` issue to stabilize `spec.md`, `plan.md`, `tasks.md`, analysis gates, and then continue into implementation without stopping at normal handoff boundaries. |
| gwt-spec-implement | `/gwt:gwt-spec-implement` | Implement an existing `gwt-spec` end-to-end from `tasks.md`. Execute test-first tasks, update progress artifacts, and keep PR work moving until the SPEC is done. |

### PR Management

| Skill | Command | Description |
|-------|---------|-------------|
| gwt-pr | `/gwt:gwt-pr` | Create or update GitHub Pull Requests with the gh CLI, including deciding whether to create a new PR or only push based on existing PR merge status. Use when the user asks to open/create/edit a PR, generate a PR body/template, or says 'open a PR/create a PR/gh pr'. Defaults: base=develop, head=current branch (same-branch only; never create/switch branches). |
| gwt-pr-check | `/gwt:gwt-pr-check` | Check GitHub PR status with the gh CLI, including unmerged PR detection and post-merge new-commit detection for the current branch. |
| gwt-pr-fix | `/gwt:gwt-pr-fix` | Inspect GitHub PR for CI failures, merge conflicts, update-branch requirements, reviewer comments, change requests, and unresolved review threads. Autonomously fix high-confidence blockers, reply to ALL reviewer comments with action taken or reason for not addressing, then resolve threads. Ask the user only for ambiguous conflicts or design decisions. |

### Utilities

| Skill | Command | Description |
|-------|---------|-------------|
| gwt-project-index | `/gwt:gwt-project-index` | Semantic search over project source files using vector embeddings. Use to find files related to a feature, bug, or concept. |
| gwt-agent-communication | `/gwt:gwt-agent-communication` | Agent↔Assistant consultation protocol for PM-mode orchestration. |
| gwt-spec-to-issue-migration | — | Migrate legacy spec sources to artifact-first GitHub Issue specs. Supports local `specs/SPEC-*` directories and body-canonical `gwt-spec` Issues using the bundled migration script. |

### Recommended Workflow

See each skill's SKILL.md for detailed instructions:

1. **Register work** → `gwt-issue-register`
2. **Resolve an existing issue** → `gwt-issue-resolve`
3. **Create or select SPEC** → `gwt-spec-register` / `gwt-spec-ops`
4. **Clarify / plan / tasks / analyze** → `gwt-spec-ops`
5. **Implement SPEC tasks** → `gwt-spec-implement`
6. **Open PR** → `gwt-pr`
7. **Fix CI / reviews** → `gwt-pr-fix`
<!-- END gwt managed skills -->
