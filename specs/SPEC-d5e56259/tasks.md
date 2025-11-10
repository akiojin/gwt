# タスク: Web UI機能の追加

**入力**: `/specs/SPEC-d5e56259/` からの設計ドキュメント
**前提条件**: plan.md、spec.md、research.md、data-model.md、contracts/

**テスト**: この機能ではテストタスクを含みますが、CLAUDE.mdの「Spec Kit準拠のTDD」要件に従います。

**構成**: タスクはユーザーストーリーごとにグループ化され、各ストーリーの独立した実装とテストを可能にします。

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（例: US1、US2、US3）
- 説明に正確なファイルパスを含める

## Commitlintルール

- コミットメッセージは件名のみを使用し、空にしてはいけません（`commitlint.config.cjs`の`subject-empty`ルール）。
- 件名は100文字以内に収めてください（`subject-max-length`ルール）。
- タスク生成時は、これらのルールを満たすコミットメッセージが書けるよう変更内容を整理してください。

## Lint最小要件

- `.github/workflows/lint.yml` に対応するため、以下のチェックがローカルで成功することをタスク完了条件に含めてください。
  - `bun run format:check`
  - `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore`
  - `bun run lint`

## フェーズ1: セットアップ（共有インフラストラクチャ）

**目的**: プロジェクトの初期化とアーキテクチャ整理

### セットアップタスク

- [X] **T001** [P] 既存の`src/ui/`を`src/cli/ui/`に移動し、CLIとWeb UIを分離
- [ ] **T002** [P] 共通ビジネスロジックを`src/core/`に抽出（git.ts, worktree.ts, claude.ts, codex.ts, launcher.ts, services/, repositories/）
- [ ] **T003** `src/index.ts`に`serve`サブコマンド分岐ロジックを追加（CLI/Web判定）
- [X] **T004** [P] Web UIの依存関係をpackage.jsonに追加（Fastify 5, @fastify/websocket, @fastify/static, node-pty, xterm.js, Vite, TanStack Query, Zustand）
- [ ] **T005** [P] `src/web/`ディレクトリ構造を作成（server/, client/）
- [ ] **T006** [P] `src/types/api.ts`に共通型定義を作成（Branch, Worktree, AIToolSession, CustomAITool）

## フェーズ2: 基盤（ブロッキング前提条件）

**目的**: すべてのユーザーストーリーで必要な基盤機能

### バックエンド基盤

- [ ] **T101** `src/web/server/index.ts`にFastifyサーバーの基本セットアップを実装
- [ ] **T102** T101の後に `src/web/server/index.ts`に静的ファイル配信（@fastify/static）を追加
- [ ] **T103** [P] `src/web/server/routes/health.ts`にヘルスチェックエンドポイント（GET /api/health）を実装
- [ ] **T104** [P] `src/web/server/pty/PTYManager.ts`にPTYマネージャークラスを実装（spawn, write, resize, kill）
- [ ] **T105** [P] `src/web/server/websocket/TerminalHandler.ts`にWebSocketハンドラーを実装（PTY ↔ WebSocket橋渡し）

### フロントエンド基盤

- [ ] **T106** [P] `src/web/client/vite.config.ts`にVite設定を作成
- [ ] **T107** [P] `src/web/client/src/main.tsx`にReactエントリーポイントを作成
- [ ] **T108** [P] `src/web/client/src/App.tsx`にルーティング設定（React Router）を実装
- [ ] **T109** [P] `src/web/client/src/lib/api.ts`にREST APIクライアント（fetch wrapper）を実装
- [ ] **T110** [P] `src/web/client/src/lib/websocket.ts`にWebSocketクライアントを実装

## フェーズ3: ユーザーストーリー1 - Webサーバーの起動とアクセス (優先度: P1)

**ストーリー**: 開発者は`claude-worktree serve`コマンドを実行してローカルでWebサーバーを起動し、ブラウザから`http://localhost:3000`にアクセスしてWorktree管理画面を表示できる。

**価値**: Web UIの基盤。サーバーが起動しなければ他の機能は使用できない。

### コマンド実装

- [ ] **T201** [US1] `src/index.ts`の`main()`関数で`serve`引数を検出し、`startWebServer()`を呼び出すロジックを実装
- [ ] **T202** [US1] T201の後に `src/web/server/index.ts`の`startWebServer()`関数を実装（ポート3000でリッスン、Ctrl+Cでシャットダウン）
- [ ] **T203** [US1] T202の後に ポート競合時のエラーハンドリングを`src/web/server/index.ts`に追加

### 静的ページ配信

- [ ] **T204** [P] [US1] `src/web/client/public/index.html`に基本的なHTMLテンプレートを作成
- [ ] **T205** [US1] T202の後に `src/web/server/index.ts`で`@fastify/static`を設定し、`src/web/client/dist/`を配信
- [ ] **T206** [P] [US1] `src/web/client/src/pages/Home.tsx`にホーム画面を作成（「Web UI is running」メッセージ表示）

### ビルドスクリプト

- [ ] **T207** [P] [US1] package.jsonに`dev:server`, `dev:client`, `build:server`, `build:client`スクリプトを追加
- [ ] **T208** [P] [US1] `bin/claude-worktree.js`を更新し、`serve`引数をサポート

### テスト（US1）

- [ ] **T209** [P] [US1] `tests/web/server/index.test.ts`にサーバー起動のユニットテストを作成
- [ ] **T210** [P] [US1] `tests/e2e/server-startup.test.ts`にE2Eテスト（Playwright）: サーバー起動 → ブラウザアクセス → ページ表示

**✅ MVP1チェックポイント**: US1完了後、`claude-worktree serve`でWebサーバーが起動し、ブラウザでアクセス可能

## フェーズ4: ユーザーストーリー2 - ブランチ一覧の表示とWorktree作成 (優先度: P1)

**ストーリー**: 開発者はブラウザでブランチ一覧を見て、作業したいブランチを選択し、クリック操作でWorktreeを作成できる。

**価値**: Worktree管理の核心機能。ユーザーはマウス操作で直感的にWorktreeを作成できる。

### REST API実装

- [ ] **T301** [P] [US2] `src/web/server/routes/branches.ts`に`GET /api/branches`エンドポイントを実装（`src/core/git.ts`の`getAllBranches()`を使用）
- [ ] **T301-1** [P] [US2] `src/web/server/routes/branches.ts`に`GET /api/branches/:branchName`エンドポイントを実装（特定ブランチの詳細情報取得、URLデコード処理を含む）
- [ ] **T302** [P] [US2] `src/web/server/routes/worktrees.ts`に`GET /api/worktrees`エンドポイントを実装（`src/core/worktree.ts`の関数を使用）
- [ ] **T303** [P] [US2] `src/web/server/routes/worktrees.ts`に`POST /api/worktrees`エンドポイントを実装（Worktree作成）
- [ ] **T304** [P] [US2] `src/web/server/routes/worktrees.ts`に`DELETE /api/worktrees/:path`エンドポイントを実装（Worktree削除）

### フロントエンド実装

- [ ] **T305** [P] [US2] `src/web/client/src/hooks/useBranches.ts`にTanStack Query hooksを実装（branches取得、キャッシング）
- [ ] **T305-1** [P] [US2] `src/web/client/src/hooks/useBranch.ts`にTanStack Query hookを実装（特定ブランチ詳細取得、ブランチ名のURLエンコード/デコード処理）
- [ ] **T306** [P] [US2] `src/web/client/src/hooks/useWorktrees.ts`にTanStack Query hooksを実装（worktrees取得、作成、削除）
- [ ] **T307** [P] [US2] `src/web/client/src/components/BranchList.tsx`にブランチ一覧コンポーネントを作成（検索・フィルター機能付き、各ブランチをリンクとして表示、URLエンコード処理）
- [ ] **T308** [P] [US2] `src/web/client/src/components/BranchDetail.tsx`にブランチ詳細コンポーネントを作成（ブランチ情報表示、Worktree作成ボタン、AI Tool起動ボタン）
- [ ] **T309** [P] [US2] `src/web/client/src/pages/Home.tsx`にホームページ（ブランチ一覧）を作成
- [ ] **T309-1** [P] [US2] `src/web/client/src/pages/BranchDetailPage.tsx`にブランチ詳細ページを作成（`/:branchName`ルート、URLデコード処理）
- [ ] **T309-2** [US2] T108の後に `src/web/client/src/App.tsx`のルーティング設定に`/`（ホーム）と`/:branchName`（ブランチ詳細）を追加

### エラーハンドリング

- [ ] **T310** [US2] 既存Worktreeがあるブランチのエラーハンドリングを`src/web/server/routes/worktrees.ts`に追加
- [ ] **T311** [P] [US2] `src/web/client/src/components/ErrorToast.tsx`にトースト通知コンポーネントを作成

### テスト（US2）

- [ ] **T312** [P] [US2] `tests/web/server/routes/branches.test.ts`にブランチAPIのユニットテストを作成（一覧取得と詳細取得、URLデコード処理のテスト）
- [ ] **T313** [P] [US2] `tests/web/server/routes/worktrees.test.ts`にWorktree APIのユニットテストを作成
- [ ] **T314** [P] [US2] `tests/e2e/worktree-creation.test.ts`にE2Eテスト: ブランチ一覧表示 → ブランチリンククリック → ブランチ詳細ページ表示 → Worktree作成 → 成功メッセージ（スラッシュを含むブランチ名のテストを含む）

**✅ MVP2チェックポイント**: US2完了後、ブラウザからブランチ詳細ページでWorktreeを作成・削除可能

## フェーズ5: ユーザーストーリー3 - AI Toolの起動とリアルタイム出力表示 (優先度: P1)

**ストーリー**: 開発者はブラウザでAI Toolを選択して起動し、ターミナル風の画面でリアルタイムに出力を確認し、キーボード入力でインタラクティブに操作できる。

**価値**: Web UIの最大の価値。ブラウザからターミナル操作を行うことで、リモート環境での作業やタブ管理が容易になる。

### セッション管理API

- [ ] **T401** [P] [US3] `src/web/server/services/SessionManager.ts`にセッション管理クラスを実装（Map<sessionId, PTY>）
- [ ] **T402** [P] [US3] `src/web/server/routes/sessions.ts`に`POST /api/sessions/start`エンドポイントを実装（AI Tool起動、PTY spawn）
- [ ] **T403** [P] [US3] `src/web/server/routes/sessions.ts`に`GET /api/sessions`エンドポイントを実装（セッション履歴取得）
- [ ] **T404** [P] [US3] `src/web/server/routes/sessions.ts`に`DELETE /api/sessions/:sessionId`エンドポイントを実装（セッション終了、PTY kill）

### WebSocket統合

- [ ] **T405** [US3] `src/web/server/websocket/TerminalHandler.ts`にWebSocketエンドポイント`/ws/terminal/:sessionId`を実装
- [ ] **T406** [US3] T405の後に PTY出力 → WebSocket送信ロジックを`src/web/server/websocket/TerminalHandler.ts`に追加
- [ ] **T407** [US3] T405の後に WebSocket受信 → PTY入力ロジックを`src/web/server/websocket/TerminalHandler.ts`に追加
- [ ] **T408** [US3] T405の後に リサイズメッセージハンドリングを`src/web/server/websocket/TerminalHandler.ts`に追加

### フロントエンド（xterm.js）

- [ ] **T409** [P] [US3] `src/web/client/src/components/Terminal.tsx`にxterm.jsコンポーネントを作成
- [ ] **T410** [US3] T409の後に `src/web/client/src/components/Terminal.tsx`にWebSocket接続ロジックを追加
- [ ] **T411** [US3] T409の後に `src/web/client/src/components/Terminal.tsx`にキーボード入力ハンドリング（term.onData）を追加
- [ ] **T412** [US3] T409の後に `src/web/client/src/components/Terminal.tsx`にリサイズハンドリング（term.onResize, xterm-addon-fit）を追加
- [ ] **T413** [P] [US3] `src/web/client/src/pages/Terminal.tsx`にターミナルページを作成

### AI Tool選択UI

- [ ] **T414** [P] [US3] `src/web/client/src/components/AIToolSelector.tsx`にAI Tool選択コンポーネントを作成（Claude Code, Codex CLI, Custom）
- [ ] **T415** [P] [US3] `src/web/client/src/components/ExecutionModeSelector.tsx`にモード選択コンポーネントを作成（normal/continue/resume）
- [ ] **T416** [US3] T414, T415の後に `src/web/client/src/pages/LaunchAITool.tsx`にAI Tool起動ページを作成

### エラーハンドリングと再接続

- [ ] **T417** [US3] WebSocket切断時の再接続ロジックを`src/web/client/src/components/Terminal.tsx`に追加
- [ ] **T418** [US3] PTYエラー（spawn失敗、crash）のハンドリングを`src/web/server/pty/PTYManager.ts`に追加

### テスト（US3）

- [ ] **T419** [P] [US3] `tests/web/server/pty/PTYManager.test.ts`にPTYマネージャーのユニットテストを作成
- [ ] **T420** [P] [US3] `tests/web/server/websocket/TerminalHandler.test.ts`にWebSocketハンドラーのユニットテストを作成
- [ ] **T421** [P] [US3] `tests/e2e/ai-tool-launch.test.ts`にE2Eテスト: AI Tool起動 → ターミナル表示 → 入出力動作

**✅ MVP3チェックポイント**: US3完了後、ブラウザからAI Toolを起動し、インタラクティブに操作可能（コア機能完成）

## フェーズ6: ユーザーストーリー4 - Worktreeの削除と管理 (優先度: P2)

**ストーリー**: 開発者はブラウザから既存のWorktree一覧を確認し、不要なWorktreeを削除できる。また、マージ済みPRのWorktreeを一括クリーンアップできる。

**価値**: Worktreeのライフサイクル管理。ディスク容量の管理が容易になる。

### API拡張

- [ ] **T501** [P] [US4] `src/web/server/routes/worktrees.ts`に`POST /api/worktrees/cleanup/merged`エンドポイントを実装（`src/core/worktree.ts`の`getMergedPRWorktrees()`を使用）
- [ ] **T502** [US4] T304の後に `DELETE /api/worktrees/:path`に保護ブランチチェック（main/master/develop）を追加

### フロントエンド実装

- [ ] **T503** [P] [US4] `src/web/client/src/components/WorktreeManager.tsx`にWorktree管理コンポーネントを作成（一覧表示、削除ボタン）
- [ ] **T504** [US4] T503の後に `src/web/client/src/components/WorktreeManager.tsx`に削除確認ダイアログを追加
- [ ] **T505** [P] [US4] `src/web/client/src/components/CleanupMergedWorktrees.tsx`にマージ済みクリーンアップコンポーネントを作成
- [ ] **T506** [P] [US4] `src/web/client/src/pages/WorktreeManagement.tsx`にWorktree管理ページを作成

### テスト（US4）

- [ ] **T507** [P] [US4] `tests/e2e/worktree-deletion.test.ts`にE2Eテスト: Worktree削除 → 確認ダイアログ → 削除完了

**✅ チェックポイント**: US4完了後、Worktreeの完全なライフサイクル管理が可能

## フェーズ7: ユーザーストーリー5 - カスタムAI Toolの設定管理 (優先度: P2)

**ストーリー**: 開発者はブラウザから`~/.claude-worktree/tools.json`の内容を編集し、カスタムAI Toolを追加・削除・変更できる。

**価値**: 拡張性。ユーザーが独自のAI Toolを登録できる。

### API実装

- [ ] **T601** [P] [US5] `src/web/server/routes/config.ts`に`GET /api/config`エンドポイントを実装（tools.json読み込み）
- [ ] **T602** [P] [US5] `src/web/server/routes/config.ts`に`PUT /api/config`エンドポイントを実装（tools.json保存、バリデーション）

### フロントエンド実装

- [ ] **T603** [P] [US5] `src/web/client/src/components/CustomToolForm.tsx`にカスタムツール編集フォームを作成（name, command, executionType, args, env）
- [ ] **T604** [P] [US5] `src/web/client/src/components/CustomToolList.tsx`にカスタムツール一覧コンポーネントを作成（追加・編集・削除）
- [ ] **T605** [US5] T603, T604の後に `src/web/client/src/pages/ConfigManagement.tsx`に設定管理ページを作成

### バリデーション

- [ ] **T606** [US5] `src/web/server/routes/config.ts`にJSON構文バリデーション、必須フィールドチェックを追加

### テスト（US5）

- [ ] **T607** [P] [US5] `tests/web/server/routes/config.test.ts`に設定APIのユニットテストを作成
- [ ] **T608** [P] [US5] `tests/e2e/config-management.test.ts`にE2Eテスト: カスタムツール追加 → 保存 → AI Tool選択画面で表示

**✅ チェックポイント**: US5完了後、カスタムAI Toolの管理が可能

## フェーズ8: ユーザーストーリー6 - Git操作のビジュアル化 (優先度: P3)

**ストーリー**: 開発者はブラウザでブランチツリーを視覚的に確認し、各ブランチのマージ状態や差分を視覚的に理解できる。

**価値**: UXを大幅に向上。複雑なブランチ構造の理解が容易になる。

### フロントエンド実装

- [ ] **T701** [P] [US6] `src/web/client/src/components/BranchTree.tsx`にブランチツリー可視化コンポーネントを作成（d3.jsまたはmermaid.js使用）
- [ ] **T702** [P] [US6] `src/web/client/src/components/BranchDetails.tsx`にブランチ詳細コンポーネントを作成（コミット履歴、差分）
- [ ] **T703** [US6] T701, T702の後に `src/web/client/src/pages/BranchVisualization.tsx`にブランチ可視化ページを作成

### テスト（US6）

- [ ] **T704** [P] [US6] `tests/e2e/branch-visualization.test.ts`にE2Eテスト: ブランチツリー表示 → ブランチクリック → 詳細表示

**✅ 完全な機能**: US6完了後、すべての要件が満たされます

## フェーズ9: 統合とポリッシュ

**目的**: すべてのストーリーを統合し、プロダクション準備を整える

### 統合

- [ ] **T801** エンドツーエンドの統合テストを実行（全ユーザーストーリーのフロー）
- [ ] **T802** エッジケースハンドリングを追加（ポート競合、ディスク容量不足、大量ブランチ、WebSocket再接続）
- [ ] **T803** `.github/workflows/test.yml`に合わせて`bun run type-check` / `bun run lint` / `bun run test` / `bun run test:coverage` / `bun run build`をローカルで完走させ、失敗時は修正
- [ ] **T804** `.github/workflows/lint.yml`に合わせて`bun run format:check`と`bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore`をローカルで完走させ、失敗時は修正

### ドキュメント

- [ ] **T805** [P] `README.md`にWeb UI使用方法セクションを追加（`claude-worktree serve`コマンド、ブラウザアクセス）
- [ ] **T806** [P] `CLAUDE.md`にWeb UI開発ガイドラインを追加（ビルドコマンド、開発サーバー起動）
- [ ] **T807** [P] `docs/web-ui.md`に詳細ドキュメントを作成（アーキテクチャ、API仕様、トラブルシューティング）

### パフォーマンス最適化

- [ ] **T808** [P] ブランチ一覧のページネーションまたは仮想スクロール（react-window）を`src/web/client/src/components/BranchList.tsx`に追加
- [ ] **T809** [P] Git操作のキューイング（p-queue）を`src/web/server/routes/`の各エンドポイントに追加

### デプロイメント準備

- [ ] **T810** `package.json`の`build`スクリプトを更新し、CLIとWeb UIを一括ビルド
- [ ] **T811** `bin/claude-worktree.js`を最終確認し、CLI/Web両方が動作することを検証

## タスク凡例

**優先度**:
- **P1**: 最も重要 - MVP1〜MVP3に必要
- **P2**: 重要 - 完全な機能に必要
- **P3**: 補完的 - UX向上

**依存関係**:
- **[P]**: 並列実行可能
- **T00X の後に**: 依存関係あり、順次実行

**ストーリータグ**:
- **[US1]**: ユーザーストーリー1（Webサーバー起動）
- **[US2]**: ユーザーストーリー2（ブランチ一覧・Worktree作成）
- **[US3]**: ユーザーストーリー3（AI Tool起動）
- **[US4]**: ユーザーストーリー4（Worktree削除・管理）
- **[US5]**: ユーザーストーリー5（カスタムツール設定）
- **[US6]**: ユーザーストーリー6（Git可視化）

## 依存関係グラフ（ユーザーストーリーレベル）

```
Phase 1 (Setup)
    ↓
Phase 2 (Foundational)
    ↓
Phase 3 (US1: サーバー起動) ← MVP1
    ↓
Phase 4 (US2: ブランチ・Worktree) ← MVP2
    ↓
Phase 5 (US3: AI Tool起動) ← MVP3（コア機能完成）
    ↓
Phase 6 (US4: Worktree管理) ← 独立（US2に依存）
    ↓
Phase 7 (US5: 設定管理) ← 独立（US3に依存）
    ↓
Phase 8 (US6: Git可視化) ← 独立（US2に依存）
    ↓
Phase 9 (統合・ポリッシュ)
```

## 並列実行の機会

### フェーズ1（セットアップ）
- T001, T002, T004, T005, T006 を並列実行可能

### フェーズ2（基盤）
- T103, T104, T105 を並列実行可能（バックエンド）
- T106, T107, T108, T109, T110 を並列実行可能（フロントエンド）

### フェーズ3（US1）
- T204, T206, T207, T208 を並列実行可能

### フェーズ4（US2）
- T301, T302, T303, T304 を並列実行可能（REST API）
- T305, T306, T307 を並列実行可能（フロントエンド）

### フェーズ5（US3）
- T401, T402, T403, T404 を並列実行可能（セッション管理）
- T414, T415 を並列実行可能（AI Tool選択UI）

### フェーズ6〜8（US4〜US6）
- 各ユーザーストーリーは独立しているため、並列開発可能（リソースがあれば）

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
- **スキップしたタスク**: 理由と共に文書化

## 実装戦略

### MVP優先アプローチ

1. **MVP1（Phase 3完了）**: Webサーバー起動、ブラウザアクセス → **即座にデプロイ可能**
2. **MVP2（Phase 4完了）**: ブランチ一覧、Worktree作成 → **基本的なWorktree管理が可能**
3. **MVP3（Phase 5完了）**: AI Tool起動、ターミナル操作 → **コア機能完成、実用可能**
4. **完全版（Phase 9完了）**: 全機能実装、プロダクション準備完了

### インクリメンタルデリバリー

各MVPチェックポイントで：
- 完全に動作する機能を提供
- ユーザーに価値を提供
- フィードバックを収集し、次のMVPに反映

## 注記

- 各タスクは1時間から1日で完了可能であるべき
- より大きなタスクはより小さなサブタスクに分割
- ファイルパスは正確で、プロジェクト構造と一致させる
- テストタスクはCLAUDE.mdの「Spec Kit準拠のTDD」要件に従う
- 各ストーリーは独立してテスト・デプロイ可能
