> **🔄 TUI MIGRATION (SPEC-1776)**: This SPEC has been updated for the gwt-tui migration. Assistant tools now operate within the TUI shell integration context, not the Tauri runtime.

# Assistant 組み込みツールシステム

## Background

gwt-tui の Assistant は PTY ベースで動作し、Shell タブ内でユーザーと対話する。Assistant の全機能（SPEC 生成、Agent 管理、Git 操作、コード参照、GitHub 連携、セッション管理）は Rust で実装された内蔵ツールセットを通じて実行される。

gwt-spec-ops スキル（Claude Code 用）とは独立した、**Assistant 専用の内蔵ツールセット**として Rust（gwt-core）で実装する。これは Assistant の AI セッション内で直接呼び出せるツール形式として設計し、gwt-tui の PTY ランタイム上で動作する。

## User Stories

| ID | シナリオ | 優先度 |
|----|---------|--------|
| US-1 | Assistant がユーザーの指示を受けて、内蔵ツールでコードベースを分析し、適切な Agent 配置を決定する | P0 |
| US-2 | Assistant がインタビュー形式で要件を聞き取り、内蔵 SPEC ツールで構造化された SPEC を生成し、GitHub Issue として作成する | P0 |
| US-3 | Assistant が内蔵 Git ツールで worktree を作成し、Agent を配置して作業を開始させる | P0 |
| US-4 | Assistant が内蔵 GitHub ツールで Issue を読み取り、タスク分解を行う | P0 |
| US-5 | Assistant が内蔵 PTY ツールで Agent の作業状況を監視し、進捗を把握する | P0 |
| US-6 | Assistant が PR マージ済みを検知し、内蔵 Git ツールで worktree 削除を提案する | P0 |
| US-7 | Assistant が既存 SPEC との整合性チェックを内蔵ツールで自動実行する | P1 |
| US-8 | Assistant が実装中の知見に基づき、SPEC 更新を内蔵ツールで自律的に提案する | P1 |

### US-1 詳細

1. ユーザーが Shell タブの入力フィールドから「この Issue を解決して」と指示する
2. Assistant のセッションがツール呼び出しで `github_read_issue` を実行する
3. gwt-core のツールハンドラが GitHub API を呼び出し、Issue 内容を取得
4. Assistant が結果を受け取り、次に `codebase_search` で関連コードを特定
5. Assistant が `agent_assign` でタスクを Agent に割り当てる

### US-2 詳細

1. ユーザーが「～の機能を追加したい」と Assistant に伝える
2. Assistant が `spec_start_interview` を呼び出してインタビューモードに入る
3. Assistant がユーザーに質問を投げる（Shell タブに表示）
4. ユーザーの回答を受けて `codebase_read_file` / `github_list_specs` で既存コードと SPEC を分析
5. Assistant が `spec_generate_section` で各セクションを段階的に生成
6. ユーザー確認後、`spec_create_issue` で GitHub Issue を自動作成

### US-3 詳細

1. Assistant がタスク分解結果に基づき `git_worktree_create` を呼び出す
2. gwt-core が `git worktree add` を実行し、結果を返す
3. Assistant が `agent_hire` で Agent プロセスを起動（新しい Agent タブとして表示）
4. Assistant が `pty_send_keys` で Agent に初期指示を送信

### US-6 詳細

1. Assistant の監視サイクルで `github_get_pr_status` を定期呼び出し
2. PR マージ済み + CI green を検知
3. Assistant が `assistant_propose_action` で「worktree 削除」をユーザーに提案（Shell タブに表示）
4. ユーザー承認後、`git_worktree_remove` で削除実行

## Functional Requirements

| ID | 要件 | 関連US |
|----|------|--------|
| FR-001 | Assistant のセッションに登録する全ツールを定義として実装する | 全US |
| FR-002 | ツールハンドラを Rust（gwt-core）で実装する | 全US |
| FR-003 | ツール実行結果を構造化 JSON で返却する（エラー時もエラー構造体で返す） | 全US |
| FR-004 | ツールカテゴリ: **コード参照ツール**（file read, symbol search, directory list, grep）を実装する | US-1 |
| FR-005 | ツールカテゴリ: **Git 操作ツール**（worktree create/remove, push, status, diff, log）を実装する。force push / rebase は禁止 | US-3, US-6 |
| FR-006 | ツールカテゴリ: **GitHub 連携ツール**（Issue CRUD, PR create/read/merge, Label 管理）を実装する | US-4, US-6 |
| FR-007 | ツールカテゴリ: **Agent 管理ツール**（hire/fire, assign task, get status, list agents）を実装する | US-1, US-3 |
| FR-008 | ツールカテゴリ: **PTY 監視ツール**（send keys, capture scrollback, get output since）を実装する | US-5 |
| FR-009 | ツールカテゴリ: **SPEC 管理ツール**（start interview, generate section, create/update issue, consistency check）を実装する | US-2, US-7, US-8 |
| FR-010 | ツールカテゴリ: **セッション管理ツール**（save state, restore state, list sessions）を実装する | - |
| FR-011 | ツールカテゴリ: **ユーザー対話ツール**（propose action, ask question, notify）を実装する | US-6 |
| FR-012 | 各ツールに権限レベル（read-only / write / destructive）を設定し、destructive 操作はユーザー承認を必須とする | US-6 |
| FR-013 | ツール実行のタイムアウト（デフォルト 30 秒、Git/GitHub 操作は 60 秒）を設定する | 全US |
| FR-014 | ツール定義は起動時に一括生成し、セッション中は固定とする | 全US |
| FR-015 | ツール実行ログ（ツール名、引数、実行時間、結果サイズ）を記録する | 全US |
| FR-016 | Git 操作ツールの権限スコープ: worktree ライフサイクル全体。**force push / rebase ツールは定義自体を持たない** | US-3, US-6 |
| FR-017 | SPEC 生成ツールは gwt-spec-ops テンプレートに準拠した出力を生成する | US-2 |
| FR-018 | SPEC 整合性チェックツールは全既存 SPEC を取得し、重複・矛盾を検出して構造化レポートを返す | US-7 |

## Non-Functional Requirements

| ID | 要件 |
|----|------|
| NFR-001 | 単一ツール実行のレイテンシ: ローカル操作は 500ms 以内、外部 API は 5s 以内 |
| NFR-002 | ツール定義の総数は 40 個以下に抑える |
| NFR-003 | ツールハンドラは async（tokio）で実装し、TUI スレッドをブロックしない |
| NFR-004 | ツール実行エラーは例外を投げず、エラー構造体として返却する |

## Success Criteria

| ID | 基準 | 検証方法 |
|----|------|----------|
| SC-001 | 全ツールカテゴリが定義される | ユニットテスト: ツールレジストリの全カテゴリ存在確認 |
| SC-002 | ツール呼び出しが Rust ハンドラで正しく実行され構造化結果が返る | ユニットテスト: 各ツールのモック実行 → 結果検証 |
| SC-003 | destructive 操作でユーザー承認フローが発動する | ユニットテスト: destructive ツール呼び出し → 承認要求確認 |
| SC-004 | force push / rebase ツールが存在しない | ユニットテスト: レジストリに禁止ツールが含まれないことを確認 |
| SC-005 | SPEC 生成ツールが gwt-spec-ops テンプレート準拠の出力を生成する | ユニットテスト |
| SC-006 | ツール実行タイムアウトが正しく動作する | ユニットテスト |
| SC-007 | SPEC 整合性チェックが重複・矛盾を検出する | ユニットテスト |
