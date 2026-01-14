# SPEC-1d62511e: TypeScript/Bun から Rust への完全移行

## 概要

Git Worktree Manager (gwt) を TypeScript/Bun から Rust へ完全移行する。
シングルバイナリ配布、パフォーマンス向上、長期保守性の確保を目指す。

## ステータス

- **作成日**: 2026-01-11
- **最終更新**: 2026-01-14
- **ステータス**: In Progress
- **優先度**: High

## 背景と動機

### 現状

- **言語**: TypeScript 5.8
- **ランタイム**: Bun >= 1.0
- **CLI UI**: OpenTUI + SolidJS
- **Web UI**: Fastify + React 19
- **配布**: npm パッケージ (@akiojin/gwt)

### 移行の目的

1. **パフォーマンス向上**: Git操作・ファイルシステム処理の高速化
2. **シングルバイナリ配布**: Node.js/Bun依存なしで動作する単一実行ファイル
3. **長期保守性**: 型安全性・メモリ安全性による堅牢なコードベース
4. **技術検証**: Rust エコシステムの実践的習得

## 技術決定事項

### コア技術選定

| 用途 | 選定 | 理由 |
| ---- | ---- | ---- |
| Git操作 | **gix + フォールバック** | gixをメインに、未実装機能は外部gitコマンド |
| TUI状態管理 | **Elmアーキテクチャ** | Model-Update-View、グローバル状態を明示的管理 |
| 画面状態保持 | **スタック保持** | 画面遷移時に前画面の状態を保持（スクロール位置等） |
| Async TUI | **ratatui-async-template** | tokio::select!でイベント統合 |
| エラー処理 | **thiserrorのみ** | 細粒度、全エラーを型定義 |
| エラー粒度 | **細粒度** | 各エラーケースを別々のバリアント |
| エラーコード | **数値・カテゴリ別** | E1xxx=Git, E2xxx=Worktree, E3xxx=Config |
| エラーメッセージ | **バイナリ埋め込み** | include_str!でコンパイル時埋め込み |
| 非同期 | **全面async** | tokioランタイムで全て非同期 |
| WASM | **Leptos** | Fine-grained reactivity、CSRのみ |
| 設定形式 | **TOML移行** | 既存JSON自動変換 |
| セッション保存 | **TOML** | 人間が読みやすい形式 |
| ログ形式 | **JSON Lines + スパン** | Pino互換 + 関数トレース情報 |
| テスト | **テンポラリリポジトリ** | テストごとにgit init、終了後削除 |

### TUI設計

| 用途 | 選定 | 理由 |
| ---- | ---- | ---- |
| 大量データ | **遅延読み込み** | スクロールに応じて追加ロード |
| オフライン | **グレースフルデグレード** | ヘッダーに[OFFLINE]アイコン表示 |
| クラッシュ復旧 | **クリーンアップ** | 次回起動時に中途半端なWorktreeを検出・削除 |
| Agent待機 | **ブロッキング待機** | Agent終了までgwtもブロック |
| 並行作業 | **マルチインスタンス** | Worktree単位でflock/LockFileロック |
| キーバインド | **現行維持** | 矢印キー中心、カスタマイズ不可 |
| マウス | **不要** | キーボードのみ |
| シグナル | **Ctrl+C二度押し** | 一度目は無視、二度目でクリーンアップ終了 |
| 非対話モード | **不要** | TUIのみ |
| カラースキーム | **端末依存** | Ratatuiデフォルト |
| ソート順 | **現行維持** | 既存実装のロジックを移植 |
| プレフィックス | **固定** | feature/, bugfix/, hotfix/, release/ |

### ビルド・配布

| 用途 | 選定 | 理由 |
| ---- | ---- | ---- |
| MSRV | **最新stable** | 新機能を活用 |
| ビルド最適化 | **ビルド速度優先** | デフォルト設定、サイズは気にしない |
| クロスコンパイル | **ネイティブランナー** | GitHub ActionsのOS別ランナー |
| バイナリサイズ | **制限なし** | 機能優先 |
| WASM配布 | **バイナリ埋め込み** | include_bytes!でシングルバイナリ |
| 配布先 | **GitHub Releases, Homebrew, crates.io, npm** | npm はpostinstallでGH Releasesからダウンロード |
| ベンチマーク | **criterionで実装** | CIで回帰検出 |

### Web UI

| 用途 | 選定 | 理由 |
| ---- | ---- | ---- |
| 認証 | **なし** | localhostのみバインド |
| SSR | **CSRのみ** | Axumは API+静的ファイル配信 |

### その他

| 用途 | 選定 | 理由 |
| ---- | ---- | ---- |
| マイグレーション | **自動変換** | 初回起動時にJSON→TOML変換 |
| ログローテーション | **日数のみ** | 7日保持、サイズ制限なし |
| ヘルプ | **clap自動生成** | シンプルに |
| git依存 | **必須要件** | 起動時チェック、なければ終了 |
| デバッグ | **環境変数+フラグ** | RUST_LOG と --debug の両方 |

## 要件

### 機能要件

#### FR-001: Git操作

- [x] リポジトリ検出・ルート取得
- [x] ブランチ一覧取得・作成・削除
- [x] リモート操作（fetch, pull, push）
- [x] Fast-Forward Pull
- [x] 未コミット/未プッシュ検出
- [x] ブランチ乖離状態（divergence）検出
- [x] 外部gitコマンド必須チェック（起動時）

#### FR-002: Worktree管理

- [x] Worktree一覧取得
- [x] Worktree作成・削除
- [x] パス生成（`.worktrees/{branch-name}`）
- [x] 保護ブランチ判定（main/master/develop）
- [x] Worktreeパス修復機能
- [x] クリーンアップ候補判定
- [x] 中途半端なWorktreeの自動クリーンアップ（起動時）
- [x] Worktree単位のファイルロック（flock）

#### FR-003: CLI TUI

- [x] フルスクリーンTUI（Ratatui + ratatui-async-template）
- [x] Elmアーキテクチャ（Model-Update-View）
- [x] 画面スタック保持（戻る時に状態復元）
- [x] ブランチ一覧画面（メイン）
  - [x] 遅延読み込み（1000+ブランチ対応）
  - [x] 現行ソート順維持
- [x] Worktree作成ウィザード
- [x] 削除確認ダイアログ
- [x] エラー表示画面
- [x] テキスト入力画面
- [x] 選択肢提示画面
- [x] プロファイル管理画面
- [x] 環境変数管理画面
- [x] 設定画面
- [x] ログ表示画面
- [x] ヘルプオーバーレイ
- [x] キーボードショートカット（現行維持：矢印キー中心）
- [x] オフライン表示（ヘッダーに[OFFLINE]アイコン）
- [x] Ctrl+C二度押し終了（一度目無視、二度目でクリーンアップ終了）

#### FR-004: Web UI

- [x] Axum Webサーバー（localhostのみ、認証なし）
- [x] REST API（worktrees, branches, sessions, config）
- [x] WebSocket（端末通信）
  - [x] WebSocket接続時にPTYセッションを作成し、接続中のみプロセスを維持する
  - [x] クライアント入力はPTYへ転送し、PTY出力はWebSocketへストリームする
  - [x] 端末リサイズは専用メッセージで通知し、PTYサイズへ反映する
  - [x] セッション終了/切断時は子プロセスとPTYを確実に破棄する
  - [x] 端末出力/エラーは英語メッセージで通知する
- [x] Leptos フロントエンド（CSRのみ、WASM埋め込み）
- [x] 端末エミュレーション（xterm.jsでWebSocket接続、入力/出力/リサイズ対応）
- [x] Worktree/Branchの作成・削除操作をWeb UIから実行可能
- [ ] システムトレイ統合（Windows）

#### FR-005: Coding Agent統合

- [x] Claude Code起動（continue/resume対応）
- [x] Codex CLI起動（推論レベル設定）
- [x] Gemini CLI起動
- [x] セッション管理（ID保存・履歴）
- [x] 環境変数渡し
- [x] Agent終了までブロッキング待機

#### FR-006: 設定管理

- [x] 設定ファイル読み込み（TOML形式、.gwt.toml）
- [x] 既存JSON設定の自動TOML変換
- [x] 環境変数サポート（GWT_*）
- [x] プロファイル機能
- [x] セッション保存・復元（TOML形式）
- [x] 既存JSONセッションの自動TOML変換

#### FR-007: ログシステム

- [x] JSON Lines形式（Pino互換）
- [x] スパン情報追加（関数名、ファイル:line）
- [x] カテゴリ別ログ
- [x] ログローテーション（7日保持）
- [x] ログ閲覧機能

#### FR-008: GitHub統合

- [x] PR情報取得
- [x] マージ状態確認
- [ ] 自動クリーンアップ候補判定
- [ ] オフライン時はグレースフルデグレード

#### FR-009: エラーハンドリング

- [x] thiserrorによる細粒度エラー型
- [x] カテゴリ別エラーコード（E1xxx=Git, E2xxx=Worktree, E3xxx=Config...）
- [x] エラーメッセージのバイナリ埋め込み（include_str!）

#### FR-010: デバッグ

- [x] RUST_LOG環境変数でログレベル制御
- [x] --debugフラグでデバッグモード有効化

### 非機能要件

#### NFR-001: 互換性

- [x] 既存設定ファイル（.gwt.json）の自動TOML変換
- [x] 既存セッションファイルの自動TOML変換
- [x] 既存ログ形式（JSON Lines）の維持 + スパン拡張
- [x] コマンドライン引数の互換性（serve, --help, --version）
- [x] キーバインドの完全互換

#### NFR-002: パフォーマンス

- [ ] 起動時間: < 100ms
- [ ] ブランチ一覧取得: < 500ms（1000ブランチ）
- [ ] メモリ使用量: < 50MB（通常操作時）
- [x] WASM最適化（wasm-opt: `z`）
- [ ] criterionによるベンチマーク（CIで回帰検出）

#### NFR-003: 配布

- [ ] シングルバイナリ（Linux, macOS, Windows）
- [ ] GitHub Releases
- [ ] Homebrew tap
- [ ] crates.io（cargo install gwt）
- [ ] npm（postinstallでGitHub Releasesからダウンロード）

#### NFR-004: 品質

- [ ] 統合テスト重視
- [ ] テンポラリGitリポジトリでのテスト
- [ ] CI/CD（GitHub Actions、ネイティブランナー）

#### NFR-005: 前提条件

- [x] git コマンドが必須（起動時チェック）
- [x] Rust最新stable（MSRV設定なし）

#### NFR-006: リポジトリ運用

- [x] ローカル専用ファイル（`*.local.*`）はGitで追跡せず、`.gitignore`で除外する

## アーキテクチャ

```
gwt-rust/
├── Cargo.toml
├── crates/
│   ├── gwt-core/           # コアロジック（Git, Worktree, Config, Logging, Error）
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── git/        # gix + フォールバック
│   │   │   ├── worktree/   # Worktree管理 + ロック
│   │   │   ├── config/     # TOML設定 + マイグレーション
│   │   │   ├── logging/    # JSON Lines + スパン
│   │   │   ├── error/      # thiserror + エラーコード
│   │   │   └── agent/      # Coding Agent起動
│   │   └── Cargo.toml
│   │
│   ├── gwt-cli/            # CLI TUI
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── app/        # Elmアーキテクチャ
│   │   │   ├── screens/    # 各画面
│   │   │   ├── components/ # 共通コンポーネント
│   │   │   └── handlers/   # イベントハンドラ
│   │   └── Cargo.toml
│   │
│   ├── gwt-web/            # Web Server (Axum)
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── routes/     # REST API
│   │   │   ├── websocket/  # 端末通信
│   │   │   └── pty/        # PTY管理
│   │   └── Cargo.toml
│   │
│   └── gwt-frontend/       # Web Frontend (Leptos CSR)
│       ├── src/
│       │   ├── lib.rs
│       │   ├── pages/
│       │   └── components/
│       └── Cargo.toml
│
├── benches/                # criterionベンチマーク
├── tests/
│   ├── integration/        # 統合テスト（テンポラリリポジトリ使用）
│   └── e2e/
└── messages/               # エラーメッセージ定義
    └── errors.toml
```

## エラーコード体系

| 範囲 | カテゴリ | 例 |
| ---- | -------- | --- |
| E1xxx | Git操作 | E1001: ブランチが見つからない |
| E2xxx | Worktree | E2001: Worktree作成失敗 |
| E3xxx | 設定 | E3001: 設定ファイル解析エラー |
| E4xxx | ログ | E4001: ログファイル書き込み失敗 |
| E5xxx | Agent | E5001: Agent起動失敗 |
| E6xxx | Web | E6001: サーバー起動失敗 |
| E9xxx | 一般 | E9001: 予期しないエラー |

## 移行フェーズ

### Phase 1: 基盤構築

- プロジェクト構造セットアップ
- gwt-core クレート実装
  - Git操作（gix + フォールバック）
  - Worktree管理（ロック含む）
  - 設定管理（TOML + マイグレーション）
  - ログシステム（JSON Lines + スパン）
  - エラー型（thiserror + コード）
- 基本的なCLI（clap）
- git存在チェック

### Phase 2: CLI TUI

- Ratatui + ratatui-async-template構成
- Elmアーキテクチャ実装
- 画面スタック（状態保持）
- 全画面の実装
- キーバインド（現行互換）
- オフライン表示
- Ctrl+C二度押し終了

### Phase 3: Coding Agent統合

- Claude Code起動
- Codex CLI起動
- Gemini CLI起動
- セッション管理
- ブロッキング待機

### Phase 4: Web UI

- Axum サーバー
- REST API
- WebSocket
- Leptos フロントエンド（CSR、WASM埋め込み）
- システムトレイ（Windows）

### Phase 5: 品質・配布

- 統合テスト（テンポラリリポジトリ）
- criterionベンチマーク
- CI/CD構築（ネイティブランナー）
- 配布（GitHub Releases, Homebrew, crates.io, npm）

## リスクと対策

| リスク | 影響 | 対策 |
| ------ | ---- | ---- |
| gixの機能不足 | High | 外部gitコマンドでフォールバック（git必須要件） |
| Leptos学習コスト | Medium | CSRのみに限定、シンプルに |
| WASM埋め込みでバイナリ肥大化 | Low | サイズ制限なしと決定済み |
| 既存機能の再現漏れ | High | 機能マトリクスで追跡、統合テスト重視 |

## 成功基準

1. 全機能がRustで動作
2. シングルバイナリで配布可能（WASM埋め込み）
3. 既存設定・ログとの互換（自動変換）
4. TypeScript版と同等以上のパフォーマンス（ベンチマークで検証）
5. 統合テストでカバー

## 参考資料

- [Ratatui](https://ratatui.rs/)
- [ratatui-async-template](https://github.com/ratatui-org/templates)
- [gitoxide](https://github.com/Byron/gitoxide)
- [Axum](https://github.com/tokio-rs/axum)
- [Leptos](https://leptos.dev/)
- [thiserror](https://github.com/dtolnay/thiserror)
- [criterion](https://github.com/bheisler/criterion.rs)
