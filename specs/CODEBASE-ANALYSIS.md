# コードベース解析ドキュメント

## 目的

新SPEC体系構築のための基礎資料。現行コード・旧TUI・GUI版の3世代を比較し、実装済み/未実装/レガシーを明確にする。

---

## 1. バージョン遷移

| 世代 | バージョン | フロントエンド | 技術スタック |
|------|-----------|--------------|-------------|
| 旧TUI | v6.0.0〜v6.30.3 | gwt-cli内蔵TUI | Ratatui 0.30 + Crossterm 0.29 |
| GUI版 | v7.0.0〜v7.13.3 | gwt-tauri + gwt-gui | Tauri v2 + Svelte 5 |
| 現行TUI | v8.17.2+ | gwt-tui | Ratatui(latest) + Crossterm(latest) + vt100 |

- GUI版は commit `18764cdb` で完全削除（358ファイル、約145,784行）
- Unity時代(v5以前)の名残がSPECに残存（TextMeshPro, XtermSharp, Agent Canvas）

---

## 2. 旧TUI (v6.30.3) 機能一覧

### 画面構成（21画面）

| 画面 | 説明 | 現行TUIでの対応 |
|------|------|----------------|
| Branch List | ブランチ一覧（カテゴリ表示、ワークツリー状態、PR情報、安全性表示） | ✅ Branches画面 |
| Wizard | ステップ式エージェント起動（QuickStart→Agent→Model→Version→Mode→Branch） | ✅ Wizard画面 |
| Agent Mode | エージェントチャットUI（ステータスバー、チャットパネル、入力パネル） | ❌ 廃止（PTY直接表示に変更） |
| Git View | ファイル差分表示（Staged/Unstaged/Untracked、コミット履歴） | ❌ 廃止（外部ツール委任） |
| Clone Wizard | リポジトリクローン（URL入力→タイプ選択→進捗表示） | ✅ Clone Wizard |
| Settings | 設定管理（General/Worktree/Web/Agent/Custom/Env/AI） | ✅ Settings画面 |
| Profiles | プロファイル管理 | ✅ Profiles（Settings内） |
| Environment | 環境変数編集 | ✅ Settings内EnvEdit |
| Logs | ログビューア（レベルフィルタ、JSON出力） | ✅ Logs画面 |
| Pane List | tmuxペイン一覧（ブランチ、エージェント、稼働時間） | ❌ 廃止（セッション管理に統合） |
| Split Layout | マルチエージェントレイアウト | ✅ グリッド/最大化レイアウト |
| Docker Progress | コンテナ起動進捗（5段階ステージ表示） | ❌ TUI未統合 |
| Service Select | Docker Composeサービス選択 | ❌ TUI未統合 |
| Confirm | Yes/Noダイアログ | ✅ Confirm画面 |
| Error | エラーキュー表示 | ✅ Error画面 |
| Migration Dialog | 旧worktreeメソッドからの移行 | ❌ 不要（移行完了） |
| Worktree Create | ワークツリー作成ステップ | ✅ Wizard内に統合 |
| Help | ヘルプオーバーレイ（セクション別キーバインド一覧） | ✅ ヘルプ画面（Ctrl+G,?） |
| Port Select | Dockerポート競合解決 | ❌ TUI未統合 |
| AI Wizard | AI設定フロー | ❌ 廃止（Settings内に統合） |
| SpecKit Wizard | SPEC生成ウィザード（Clarify→Specify→Plan→Tasks→Done） | ✅ SpecKit Wizard |

### 旧TUI固有機能

| 機能 | 説明 | 現行TUIでの状態 |
|------|------|----------------|
| AIブランチ名提案 | AI生成のブランチ名候補（Input→Loading→Select→Error） | ⚠️ Wizard内で対応（専用UIなし） |
| セッション変換 | エージェント間でセッション変換（バージョン選択、npm registry連携） | ⚠️ バックエンドあり、TUI未統合 |
| コラボレーションモード | Codex v0.91.0+の機能フラグ | ⚠️ 未確認 |
| ツールセッション履歴 | ブランチごとの最終使用エージェント記録 | ✅ AgentHistoryStore |
| Goneブランチ追跡 | upstream削除済みブランチの検出・表示 | ⚠️ 部分実装 |
| ダブルクリック検出 | 500msウィンドウのダブルクリック（worktree選択） | ❌ 未実装 |

### 旧TUIのキーバインド

| キー | アクション |
|------|----------|
| Up/k, Down/j | カーソル移動 |
| PageUp/PageDown | ページ移動 |
| Home/g, End/G | 先頭/末尾 |
| Enter | 選択/確認 |
| d | ワークツリー削除 |
| s | ソートモード切替 |
| r | リフレッシュ |
| u | Claude Code hooks再登録 |
| v | ペイン表示切替 |
| ?/F1 | ヘルプ表示 |
| / | 検索/フィルタ |
| Tab | 次のセクション/タブ |
| Esc | 閉じる/キャンセル/戻る |
| q | 終了 |
| 1/2/3/4 | 画面切替 |

---

## 3. GUI版 (v7.x) で追加された機能

| 機能 | 説明 | 現行TUIでの状態 |
|------|------|----------------|
| Agent Canvas | Figmaスタイルのタイル配置（パン/ズーム/ドラッグ） | ❌ 廃止（Elm Architectureに置換） |
| OS通知 | Tauri経由のOS通知（バックグラウンドイベント） | ❌ 廃止 |
| 自動アップデート | Tauri自動アップデート（.dmg/.msi配布） | ❌ GitHub Release + bunx/npxに変更 |
| PRダッシュボード | PR状態・CI結果・マージ状態の統合表示 | ❌ TUI未統合（バックエンドあり） |
| AIセッションサマリー | エージェント出力の定期的AI要約 | ❌ 明示的延期（バックエンド98KB実装済み） |
| Issue/SPECパネル | マネジメントパネル内の検索付き一覧 | ✅ Issues/SPECsタブ |

### Unity時代の名残（SPECに残存）

| 概念 | 出現SPEC | 説明 |
|------|---------|------|
| TextMeshPro / TextMeshProUGUI | SPEC-1541 | Unity UIテキストレンダリング |
| MonoBehaviour | SPEC-1541 | Unityコンポーネント基底クラス |
| XtermSharp | SPEC-1541 | C#ターミナルアダプタ |
| Agent Canvas | SPEC-1654, 1768, 1770 | Figmaスタイルタイルシステム |
| uGUI | SPEC-1768 | Unity UIフレームワーク |
| TMP_InputField / ScrollRect | SPEC-1541 | Unity入力コンポーネント |

---

## 4. 現行TUI (ratatui) 実装状況

### アーキテクチャ

```
Elm Architecture (Model → Message → Update → View)

gwt-tui (31ファイル, ~13.4K LOC)
├── main.rs          # エントリポイント
├── app.rs           # Update + View関数 (~2,800 LOC)
├── model.rs         # 中央状態 (~1,200 LOC)
├── message.rs       # メッセージenum
├── event.rs         # イベントループ
├── renderer.rs      # VT100→ratatui変換
├── config/          # 起動設定
├── input/
│   ├── keybind.rs   # Ctrl+Gプレフィックスシステム
│   └── voice.rs     # 音声入力状態
├── screens/
│   ├── branches.rs      # ブランチ一覧 (52KB)
│   ├── wizard.rs        # エージェント起動 (89KB)
│   ├── settings.rs      # 設定管理 (77KB)
│   ├── specs.rs         # SPEC一覧 (47KB)
│   ├── logs.rs          # ログビューア (38KB)
│   ├── versions.rs      # バージョン一覧 (34KB)
│   ├── issues.rs        # Issue一覧 (26KB)
│   ├── error.rs         # エラー表示
│   ├── confirm.rs       # 確認ダイアログ
│   ├── agent_pane.rs    # エージェント色表示
│   ├── clone_wizard.rs  # クローンウィザード
│   ├── speckit_wizard.rs# SPECキットウィザード
│   └── branch_session_selector.rs
└── widgets/
    ├── markdown.rs      # Markdownレンダリング
    ├── progress_modal.rs
    ├── status_bar.rs
    ├── tab_bar.rs
    └── terminal_view.rs
```

### 状態モデル

```
Model
├── active_layer: Initialization | Main | Management
├── session_tabs: Vec<SessionTab>     # Shell/Agentセッション
├── session_layout_mode: Grid | Maximized
├── management_tab: Branches|Specs|Issues|Profiles|Versions|Settings|Logs
├── 各画面状態: BranchListState, IssuePanelState, SpecsState...
├── オーバーレイ: error_queue, confirm, progress, wizard...
├── PTY: PaneManager + vt100パーサー + TerminalViewportState
└── バックグラウンド: branch_list/management_dataプリロード
```

### 実装済み機能 ✅

| カテゴリ | 機能 | テスト |
|---------|------|-------|
| **ナビゲーション** | ブランチファースト、7タブ管理、レイヤー切替 | ✅ |
| **セッション管理** | マルチセッション、グリッド/最大化、Shell/Agent | ✅ |
| **ターミナル** | vt100+ratatui、ANSI 256色、スクロールバック、テキスト選択 | ✅ |
| **キーバインド** | Ctrl+Gプレフィックス、17テストケース | ✅ |
| **Wizard** | 11ステップ起動フロー、QuickStart、セッション再開 | ✅ |
| **ブランチ** | カテゴリ表示、ワークツリー状態、PR情報、セッション数 | ✅ |
| **Issue** | GitHub同期、詳細表示、ブランチリンク、検索 | ✅ |
| **SPEC** | ローカルspecs/*一覧、Markdown詳細、起動エントリ | ✅ |
| **設定** | 6カテゴリ、カスタムエージェントCRUD | ✅ |
| **プロファイル** | 作成/編集/削除、環境変数、プロファイル切替 | ✅ |
| **Versions** | Gitタグ一覧 | ✅ |
| **Logs** | ログビューア、フィルタ | ✅ |
| **Codex hooks** | 確認フロー、起動キュー | ✅ |
| **Clone** | リポジトリクローンウィザード | ✅ |
| **SpecKit** | SPEC生成ウィザード | ✅ |
| **エラー** | エラーキュー、オーバーレイ表示 | ✅ |

### 未実装機能 ❌

| 機能 | バックエンド | 優先度 | 備考 |
|------|------------|--------|------|
| **PRダッシュボード** | ✅ PrStatus完備 | P1 | TUI内の表示UI未作成 |
| **AIセッションサマリー** | ✅ 98KB実装済み | P2 | SPEC-1776で明示的延期 |
| **Docker/DevContainer UI** | ✅ DockerManager完備 | P2 | 検出・ポート管理・起動UIなし |
| **フル通知システム** | ❌ | P1 | ステータスバー+モーダル+エラーキュー+構造化ログ |
| **音声入力TUI統合** | ✅ VoiceBackend trait | P2 | 状態管理のみ、キャプチャ/転写未接続 |
| **ファイル貼り付け** | ❌ | P2 | Ctrl+G,p でパス文字列注入 |
| **ヘルプオーバーレイ拡張** | ⚠️ ShowHelp存在 | P2 | コードからの自動収集未実装 |
| **SPECs→Agent起動** | ❌ | P2 | Shift+Enterでの起動フロー |
| **SPECセマンティック検索** | ❌ | P2 | ChromaDB連携 |
| **バージョン一覧キャッシュ** | ❌ | P1 | 起動時に直近10件をキャッシュ |
| **埋込スキル管理** | ❌ | P1 | gwt-pr-check等の登録・管理UI |
| **AIブランチ命名** | ✅ suggest_branch_name() | P2 | Wizard統合のみ |
| **セッション変換UI** | ✅ SessionConverter | P2 | バックエンドあり、TUI未統合 |

### テスト状況

| コンポーネント | テスト数 | 状況 |
|--------------|---------|------|
| gwt-tui全体 | 434 | ✅ 充実 |
| keybind.rs | 17 | ✅ 網羅的 |
| branches.rs | ~100+ | ✅ |
| settings.rs | ~80+ | ✅ |
| wizard.rs | ~50+ | ✅ |
| issues.rs | ~50+ | ✅ |
| specs.rs | ~50+ | ✅ |
| gwt-core統合 | 4ファイル | ✅ |
| ベンチマーク | 3スイート | ✅ |

---

## 5. 既存SPECとの乖離マップ

### 旧GUI/Unity前提で現実と乖離しているSPEC

| SPEC | 問題 |
|------|------|
| SPEC-1541 | TextMeshPro/Unity前提の5層アーキテクチャ。現行はvt100+ratatui |
| SPEC-1654 | Agent Canvas/Tile前提。現行はElm Architecture |
| SPEC-1651 | Tauri OS通知前提。現行はTUI内通知 |
| SPEC-1768 | GUI Agent Canvas用。現行TUIでは不要 |
| SPEC-1770 | Agent Canvas操作前提。現行はCtrl+Gプレフィックス |

### 実装済みだがSPECと不整合のあるもの

| SPEC | 状況 |
|------|------|
| SPEC-1776 | 大部分実装済みだがタスクは全て未チェック |
| SPEC-1648 | セッション保存は実装済みだがSPECは旧設計のまま |
| SPEC-1646 | エージェント検出は実装済みだがSPECは旧設計のまま |
| SPEC-1645 | 設定画面は実装済みだがSPECは旧設計のまま |

### 価値ある独自情報を持つSPEC

| SPEC | 独自情報 |
|------|---------|
| SPEC-1354 | Issue artifact-first reconstruct契約、API安定性要件 |
| SPEC-1785 | SPECs画面→Agent起動の詳細仕様（12タスク、50+ FR） |
| SPEC-1786 | hooks.jsonマージアルゴリズム（31タスク、20完了） |
| SPEC-1787 | ワークスペース初期化、SPEC-firstワークフロー |

---

## 6. レガシーコードのクリーンアップ対象

| ファイル | 問題 | 優先度 |
|---------|------|--------|
| package.json | "Tauri desktop GUI"の記述、tauri dev/buildスクリプト | 高 |
| gwt-core/tests/no_direct_git_command.rs:79,104 | 削除済みgwt-tauriディレクトリのスキャン | 高 |
| gwt-core/src/update.rs | テストデータ内の"tauri.msi" URL | 低 |
| gwt-core/src/agent/launch.rs | "Extracted from gwt-tauri"コメント | 低 |

---

## 7. 旧TUI→現行TUIの主要な設計変更

| 観点 | 旧TUI (v6.30.3) | 現行TUI |
|------|-----------------|---------|
| クレート構成 | gwt-cli内蔵 | 独立gwt-tuiクレート |
| アーキテクチャ | 画面切替ベース(1/2/3/4キー) | Elm Architecture(Model→Message→Update→View) |
| レイヤー | なし（単一画面） | 2レイヤー（Main=セッション、Management=タブ） |
| エージェント表示 | Agent Mode画面（チャットUI） | PTY直接表示（ターミナル出力そのまま） |
| Git表示 | Git View画面（差分・コミット） | 廃止（エージェント/外部ツール委任） |
| マルチセッション | tmuxペイン連携 | ネイティブPTY + グリッド/最大化レイアウト |
| タブ構成 | 4固定画面 | 7管理タブ + Nセッションタブ |
| プレフィックスキー | なし | Ctrl+G（2秒タイムアウト） |
