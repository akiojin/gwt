# タスク: エラーポップアップ・ログ出力システム

**入力**: `/specs/SPEC-e66acf66/` からの設計ドキュメント
**前提条件**: plan.md、spec.md、research.md、data-model.md、contracts/error-codes.md、quickstart.md

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（US1〜US7）
- 説明に正確なファイルパスを含める

## 依存関係マップ

```text
US3 (ログ出力) ─────┬─► US1 (ポップアップ表示) ─► US2 (ログ遷移)
                   │                          │
基盤 (ErrorCode) ──┴─► US5 (キュー) ──────────┴─► US4 (コピー)
                                               │
                                               └─► US6 (マウス)
                                               │
                                               └─► US7 (サジェスチョン)
```

## Lint最小要件

- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo fmt --check`
- `cargo test`

## パス規約

- **gwt-core**: `crates/gwt-core/src/`
- **gwt-cli**: `crates/gwt-cli/src/tui/`

---

## フェーズ1: 基盤（共有インフラストラクチャ）

**目的**: ErrorCode体系とサジェスチョンの基盤を構築

### 基盤タスク

- [x] **T001** [P] [共通] 既存の `crates/gwt-core/src/error.rs` にErrorCategory, GwtError, error_messages()が実装済み
- [x] **T002** [P] [共通] 既存の `crates/gwt-core/src/errors.toml` にErrorCode（E1xxx〜E9xxx）が定義済み
- [x] **T003** [P] [共通] 既存の `crates/gwt-core/src/error.rs` にErrorCategory enumが定義済み（Git, Worktree, Config, Agent, WebApi, Internal）
- [x] **T004** [共通] 既存の `crates/gwt-core/src/error.rs` にcode(), category()メソッドが実装済み
- [x] **T005** [P] [共通] `crates/gwt-core/src/errors.toml` に[suggestions]セクションを追加し、全エラーコードにサジェスチョンを定義
- [x] **T006** [共通] `crates/gwt-core/src/error.rs` にsuggestions()メソッドを追加
- [x] **T007** [共通] `cargo clippy` と `cargo test` でエラーがないことを確認

---

## フェーズ2: ユーザーストーリー3 - 全エラーのログ出力 (優先度: P0)

**ストーリー**: エラーが発生すると、ポップアップ表示とは別に、ログファイルに記録される。ログにはエラーコード、カテゴリ、メッセージ、詳細が含まれる。

**価値**: エラーの永続化と後からのデバッグを可能にする

### ログ出力タスク

- [x] **T101** [US3] `crates/gwt-core/src/logging/logger.rs` にlog_gwt_error()とlog_error_message()関数を追加
- [x] **T102** [US3] `crates/gwt-cli/src/tui/app.rs` のapply_entry_contextでlog_error_message()を呼び出し
- [x] **T103** [US3] T101の後に `crates/gwt-cli/src/tui/app.rs` のGitコマンド失敗箇所でlog_error()を呼び出し（tracing::warnで既存実装あり）
- [x] **T104** [US3] T101の後に `crates/gwt-cli/src/tui/app.rs` のプロファイル保存失敗箇所でlog_error()を呼び出し（tracing::warnで既存実装あり）
- [x] **T105** [US3] `crates/gwt-cli/src/tui/app.rs` のLaunchUpdate::Failedとエージェント起動失敗でlog_error_message()を呼び出し
- [ ] **T106** [US3] ログファイル（~/.gwt/logs/gwt.jsonl.YYYY-MM-DD）にエラーが記録されることを手動確認

**✅ MVP1チェックポイント**: US3完了後、全エラーがログファイルに記録される

---

## フェーズ3: ユーザーストーリー1 - エラーポップアップの表示 (優先度: P0)

**ストーリー**: 開発者がWorktree作成、Gitコマンド実行、プロファイル設定保存などの操作でエラーが発生した場合、画面中央にエラーポップアップが表示される。

**価値**: エラーをユーザーに確実に通知する基本機能

### ErrorState拡張タスク

- [x] **T201** [US1] `crates/gwt-cli/src/tui/screens/error.rs` のErrorStateにcode, suggestions等のフィールドが既存で定義済み
- [x] **T202** [US1] `crates/gwt-cli/src/tui/screens/error.rs` にErrorState::from_gwt_error()コンストラクタを追加
- [x] **T203** [US1] `crates/gwt-cli/src/tui/screens/error.rs` のrender_error()でエラーコードをタイトルに表示（既存実装を確認）

### ポップアップUI強化タスク

- [x] **T204** [US1] `crates/gwt-cli/src/tui/screens/error.rs` のrender_error()で詳細セクションのスクロール実装済み
- [x] **T205** [US1] `crates/gwt-cli/src/tui/screens/error.rs` のrender_error()でポップアップ幅を最大70文字に制限済み

### キーボード操作タスク

- [x] **T206** [US1] `crates/gwt-cli/src/tui/app.rs` で[Enter]キーでポップアップを閉じる処理を実装
- [x] **T207** [US1] `crates/gwt-cli/src/tui/app.rs` で[Esc]キーでポップアップを閉じる処理を実装
- [ ] **T208** [US1] エラーポップアップが正しく表示・閉じられることを手動確認

**✅ MVP2チェックポイント**: US1完了後、エラーポップアップが正しく表示される

---

## フェーズ4: ユーザーストーリー2 - ログ画面へのジャンプ (優先度: P0)

**ストーリー**: エラーポップアップ表示中に[l]キーを押すと、ログビューアー画面に遷移し、該当エラーが選択された状態で表示される。

**価値**: エラーの詳細な履歴や文脈を確認するためのナビゲーション

### ログ遷移タスク

- [x] **T301** [US2] `crates/gwt-cli/src/tui/app.rs` のMessage::Charで[l]キーでログ画面へ遷移する処理を追加
- [ ] **T302** [US2] `crates/gwt-cli/src/tui/screens/logs.rs` にselect_latest_error()メソッドを追加（最新エラーを選択状態にする）- 未実装（オプショナル）
- [ ] **T303** [US2] `crates/gwt-cli/src/tui/app.rs` でログ画面遷移時にselect_latest_error()を呼び出し - 未実装（オプショナル）
- [ ] **T304** [US2] エラーポップアップから[l]キーでログ画面に遷移することを手動確認

**✅ MVP3チェックポイント**: US2完了後、P0機能（ポップアップ、ログ出力、ログ遷移）が完成

---

## フェーズ5: ユーザーストーリー5 - 複数エラーのキュー処理 (優先度: P1)

**ストーリー**: エラーポップアップ表示中に新たなエラーが発生した場合、キューに追加され、現在のポップアップを閉じると次のエラーが表示される。

**価値**: 連続エラー発生時のUX向上

### ErrorQueueタスク

- [x] **T401** [US5] `crates/gwt-cli/src/tui/screens/error.rs` にErrorQueue構造体を追加（VecDeque<ErrorState>）
- [x] **T402** [US5] `crates/gwt-cli/src/tui/screens/error.rs` にErrorQueue::push(), dismiss_current(), current(), position_string()を実装
- [x] **T403** [US5] `crates/gwt-cli/src/tui/app.rs` のModelでerror_queue: ErrorQueueに変更
- [x] **T404** [US5] `crates/gwt-cli/src/tui/app.rs` のエラー発生箇所でerror_queue.push()を使用するよう変更
- [x] **T405** [US5] `crates/gwt-cli/src/tui/screens/error.rs` のrender_error_with_queue()でキュー位置をタイトルに表示
- [x] **T406** [US5] `crates/gwt-cli/src/tui/app.rs` のEnterキーハンドラでdismiss_current()を呼び出し
- [ ] **T407** [US5] 複数エラーが発生した際にキューで順次表示されることを手動確認

**✅ MVP4チェックポイント**: US5完了後、複数エラーのキュー処理が動作

---

## フェーズ6: ユーザーストーリー7 - サジェスチョン表示 (優先度: P1)

**ストーリー**: エラーポップアップには、エラー解決のためのサジェスチョン（具体的なコマンド例を含む）が表示される。

**価値**: ユーザーが問題を自己解決できるよう支援

### サジェスチョン表示タスク

- [x] **T501** [US7] `crates/gwt-core/src/error.rs` のGwtError::suggestions()でget_suggestions相当の機能を実装
- [x] **T502** [US7] `crates/gwt-cli/src/tui/screens/error.rs` のrender_error_internal()でサジェスチョンセクションを箇条書きで表示（既存実装）
- [ ] **T503** [US7] エラー発生時にサジェスチョンが正しく表示されることを手動確認

**✅ MVP5チェックポイント**: US7完了後、サジェスチョンが表示される

---

## フェーズ7: ユーザーストーリー4 - クリップボードコピー (優先度: P1)

**ストーリー**: エラーポップアップ表示中に[c]キーを押すと、エラー情報がJSON形式でクリップボードにコピーされる。

**価値**: バグレポートや問題共有のための便利機能

### クリップボードタスク

- [x] **T601** [US4] `crates/gwt-cli/Cargo.toml` にarboard = "3"が既に依存関係に存在
- [x] **T602** [US4] `crates/gwt-cli/src/tui/screens/error.rs` にErrorState::to_json()メソッドを実装
- [x] **T603** [US4] `crates/gwt-cli/src/tui/app.rs` のMessage::Charで[c]キーでarboard::Clipboardを使用してコピー
- [x] **T604** [US4] `crates/gwt-cli/src/tui/app.rs` で[c]キーでto_json()を呼び出しクリップボードにコピー
- [x] **T605** [US4] `crates/gwt-cli/src/tui/app.rs` でコピー成功時に「Error copied to clipboard」ステータスメッセージを表示
- [ ] **T606** [US4] [c]キーでエラー情報がクリップボードにコピーされることを手動確認

**✅ MVP6チェックポイント**: US4完了後、クリップボードコピーが動作

---

## フェーズ8: ユーザーストーリー6 - マウス操作サポート (優先度: P1)

**ストーリー**: エラーポップアップはマウス操作に対応し、フッターのショートカットをクリックで実行、詳細セクションをホイールでスクロールできる。

**価値**: マウスユーザーへのアクセシビリティ向上

### マウス操作タスク

- [x] **T701** [US6] `crates/gwt-cli/src/tui/app.rs` にhandle_error_mouse()を追加し、クリックでポップアップを閉じる処理を実装
- [x] **T702** [US6] `crates/gwt-cli/src/tui/screens/error.rs` のrender_error()でフッターに[l] Logs [c] Copyショートカットを表示（既存実装）
- [x] **T703** [US6] Event::Mouseでエラー画面のマウスイベントをhandle_error_mouse()にルーティング
- [x] **T704** [US6] handle_error_mouse()でScrollUp/ScrollDownイベントで詳細セクションのスクロールを実装
- [ ] **T705** [US6] マウス操作（クリック、スクロール）が正しく動作することを手動確認

**✅ MVP7チェックポイント**: US6完了後、マウス操作が動作

---

## フェーズ9: 統合とポリッシュ

**目的**: すべてのストーリーを統合し、品質を確保

### 統合タスク

- [x] **T801** [統合] 主要エラー発生箇所（apply_entry_context, LaunchUpdate::Failed, agent起動失敗）でlog_error_message()を追加
- [x] **T802** [統合] `cargo clippy --all-targets --all-features -- -D warnings` をローカルで完走
- [x] **T803** [統合] `cargo fmt` を実行しフォーマットを適用
- [x] **T804** [統合] `cargo test` を実行し315テストが通ることを確認
- [ ] **T805** [統合] エッジケース（長いメッセージ、空キュー、クリップボード失敗）をテスト

### コミット・プッシュ

- [ ] **T806** [統合] 変更をConventional Commits形式でコミット（feat: add error popup and log output system）
- [ ] **T807** [統合] `bunx commitlint --from HEAD~1 --to HEAD` でコミットメッセージを検証
- [ ] **T808** [統合] 変更をプッシュ

---

## タスク凡例

**優先度**:
- **P0**: 最も重要 - 基本機能に必要（US1, US2, US3）
- **P1**: 重要 - 完全な機能に必要（US4, US5, US6, US7）

**依存関係**:
- **[P]**: 並列実行可能
- **T###の後に**: 指定タスク完了後に実行

**ストーリータグ**:
- **[US1]**: エラーポップアップ表示
- **[US2]**: ログ画面遷移
- **[US3]**: ログ出力
- **[US4]**: クリップボードコピー
- **[US5]**: キュー処理
- **[US6]**: マウス操作
- **[US7]**: サジェスチョン
- **[共通]**: 全ストーリー共有
- **[統合]**: 複数ストーリーにまたがる

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化

## サマリー

| フェーズ | ストーリー | タスク数 | 並列可能 |
|---------|----------|---------|---------|
| 1 | 基盤 | 7 | T001, T002, T003, T005 |
| 2 | US3 ログ出力 | 6 | なし |
| 3 | US1 ポップアップ | 8 | なし |
| 4 | US2 ログ遷移 | 4 | なし |
| 5 | US5 キュー | 7 | なし |
| 6 | US7 サジェスチョン | 3 | なし |
| 7 | US4 コピー | 6 | T601 |
| 8 | US6 マウス | 5 | なし |
| 9 | 統合 | 8 | なし |
| **合計** | | **54** | |

**推奨MVP範囲**: フェーズ1〜4（US3, US1, US2）= 25タスク
