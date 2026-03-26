### 技術コンテキスト

**影響モジュール・Assembly:**

```
Gwt.Core.asmdef          # コアサービス (PTY, Git, GitHub, Data, Error Handling)
  ├── references: なし (外部依存のみ)
Gwt.Agent.asmdef          # エージェント管理 + Lead
  ├── references: Gwt.Core
Gwt.Studio.asmdef         # スタジオ + エンティティ + HUD
  ├── references: Gwt.Core, Gwt.Agent
Gwt.AI.asmdef             # AI API + 音声
  ├── references: Gwt.Core
Gwt.Infra.asmdef          # Docker, Build, Index
  ├── references: Gwt.Core
Gwt.Lifecycle.asmdef      # プロジェクトLC + マルチプロジェクト
  ├── references: Gwt.Core, Gwt.Studio
Gwt.Tests.Editor.asmdef   # EditMode テスト
  ├── references: 全 asmdef
Gwt.Tests.Runtime.asmdef  # PlayMode テスト
  ├── references: 全 asmdef
```

**アーキテクチャ概要:**

```
Unity Application
├── Core Layer (DI: VContainer, Service + UseCase パターン)
│   ├── Project Service (プロジェクト開閉・作成・移行: #1557)
│   ├── Git Service (git CLI wrapper: #1543)
│   ├── GitHub Service (gh CLI wrapper: #1544)
│   ├── PTY Service (Pty.Net, Unity 内子プロセス管理: #1540)
│   ├── Agent Service (process management: #1545)
│   ├── Session Service (~/.gwt/ persistence, JSON: #1542)
│   ├── AI Service (OpenAI-compatible API, 全8機能: #1550)
│   ├── Migration Service (Rust版からの自動データ移行: #1556)
│   ├── Docker Service (container management: #1552)
│   ├── Error Handling Service (リトライ・通知・グレースフルデグラデーション基盤)
│   └── Multi-Project Service (プロジェクト切替: #1558)
├── Orchestration Layer
│   ├── Lead→Agent 協調（常時アクティブ: #1549）
│   ├── Single Agent Mode
│   └── Manual Terminal Mode
├── Studio Layer (URP 2D Lighting, シングルウィンドウ: #1546)
│   ├── Studio Generator (repo → top-down ¾ view studio)
│   ├── Entity System (desks=worktrees, characters=agents, markers=issues: #1547)
│   ├── Camera Controller (top-down ¾ view)
│   ├── Atmosphere System (CI status → lighting/mood)
│   └── Interaction System (click, overlay, command)
├── Terminal Layer (#1541)
│   ├── TerminalEmulator (自前 ANSI パーサー + ターミナルバッファ)
│   └── TextMeshPro Renderer (TerminalEmulator バッファ描画)
├── Lead Layer
│   ├── Lead Character (キャラクター性あり: #1549)
│   ├── TTS (ローカル TTS: #1551)
│   └── STT (ローカル Qwen3-ASR: #1551)
├── Gamification Layer (#1555)
│   ├── Achievement System
│   ├── Studio Level System
│   └── Badge System
└── UI Layer (シングルウィンドウ内: #1548)
    ├── HUD (always-visible Lead input, RTS-style console, notifications)
    ├── Terminal Renderer (TerminalEmulator + TextMeshPro)
    ├── Overlay Panels (diff, commits, stash, forms, details)
    ├── Settings Menu (game-style ESC menu, i18n: EN/JA via Unity Localization)
    └── GFM Markdown Renderer (自前完全実装)
```

**サブ SPEC 一覧:**

```
#1539 アーキテクチャ（親 SPEC）
├── コアサービス層
│   ├── #1540 PTY 管理
│   ├── #1541 ターミナルエミュレータ
│   ├── #1542 データ永続化
│   ├── #1543 Git 操作
│   └── #1544 GitHub 連携
├── エージェント層
│   ├── #1545 エージェント管理
│   └── #1549 Lead オーケストレーション
├── スタジオ層
│   ├── #1546 スタジオ（ワールド生成）
│   ├── #1547 エンティティシステム
│   └── #1548 HUD & UI
├── AI/音声層
│   ├── #1550 AI API 統合
│   └── #1551 音声インタラクション
├── インフラ層
│   ├── #1552 Docker/DevContainer
│   ├── #1553 ビルド・配布
│   └── #1554 プロジェクトインデックス
├── ライフサイクル層
│   ├── #1557 プロジェクトライフサイクル
│   └── #1558 マルチプロジェクト切替
└── Phase 5（後回し）
    ├── #1555 ゲーミフィケーション
    ├── #1556 データマイグレーション
    └── #1560 サウンド
```

### 実装アプローチ

Rust/Tauri/Svelte から Unity 6 (C#) への全面移行を、サブSPEC単位の段階的実装で進める。VContainer + UniTask による DI/非同期基盤を先に構築し、各サービス層を独立して実装・テスト可能な構成とする。

**選定理由**: モノリシックな一括移行ではなく、サブSPEC分割による段階的実装を選択。各サブSPECが独立してテスト・検証可能であり、リスクを局所化できるため。

### フェーズ分割

1. **Phase 1: 基盤構築** — Unity プロジェクト基盤（VContainer, UniTask, Assembly Definitions, Service + UseCase パターン）
2. **Phase 2: コアサービス** — Git, GitHub, PTY, Session, Data Persistence, Error Handling の C# 実装
3. **Phase 3: ターミナル** — 自前 TerminalEmulator + TextMeshPro によるターミナルエミュレータ
4. **Phase 4: スタジオ・UI** — トップダウン ¾ ビュースタジオ、エンティティシステム、HUD/UI、Lead 協調
5. **Phase 5: 拡張機能** — ゲーミフィケーション、データマイグレーション、サウンド、AI統合、音声

### テスト責務分離

| テスト種別 | 対象 | ツール |
|-----------|------|-------|
| EditMode テスト | サービス単体、UseCase ロジック、データ変換 | NUnit + NSubstitute |
| PlayMode テスト | UI + シーン統合、MonoBehaviour、入力操作 | NUnit + Unity Test Framework |
| 統合テスト | 実プロセス起動、ファイルI/O、外部コマンド | PlayMode + プロセス管理 |

### 設計判断メモ

| 項目 | 決定 | 理由 |
|------|------|------|
| 3D vs トップダウン ¾ ビュー | トップダウン ¾ ビュー | Game Dev Story スタイル。3Dモデル不要でピクセルアートで十分 |
| ゴッドゲーム vs スタジオ | 開発スタジオメタファー | デスク=Worktree、キャラ=Agent の直感的マッピング |
| ミーティングルーム | 不採用 | Lead 入力は常時表示 HUD で十分 |
| 掲示板 | 不採用（浮遊 Issue マーカーに変更） | ゲームライクな表現。Issue=浮遊する!マーク |
| サウンド | 後期フェーズに延期 | コア機能を優先 |
| サウンドアセット | **AI 生成（Suno/Udio 等）** | 市販アセットではなく AI 生成で統一 |
| ゲーミフィケーション | 別 SPEC で策定 | コア機能完成後に着手。データ基盤は Git+GitHub+gwt アクティビティ |
| プロジェクト名 | gwt（変更なし） | 既存ブランドを維持 |
| アセット調達 | LimeZu エコシステム + Unity Asset Store（docs/pixelart.md 参照） | ピクセルアートアセットパック |
| C# アーキテクチャ | 薄い Service + UseCase パターン | VContainer で DI。過剰抽象化を避けシンプルさを追求 |
| ターミナルエンジン | **自前 TerminalEmulator 実装** | XtermSharp は不採用。ANSI パーサー + バッファを自前実装（既に実装で変更済み） |
| データ永続化フォーマット | **JSON に完全統一** | TOML 完全廃止、agents.toml 互換も廃止、Tomlyn 依存不要 |
| ウィンドウ管理 | シングルウィンドウ完結 | ゲームらしさを重視。マルチウィンドウは不採用 |
| PTY プロセス管理 | Unity 内子プロセス管理 | デーモン分離不要。プロジェクト切替時もセッション維持。**MVP で Process → Pty.Net 移行必須** |
| キー保管 | **平文保存（~/.gwt/config）を維持（明示的判断）** | 暗号化は現時点では不要と判断 |
| オフライン対応 | 非対応 | ネットワーク接続前提 |
| テスト戦略 | 全層テスト + CI | カバレッジ 90% 以上を目標 |
| 自動更新 | **GitHub Release API 自前実装** | Sparkle/WinSparkle は不採用。プラットフォーム横断で統一可能 |
| キーバインド | フォーカスベース切替 | コンフリクト回避 |
| Lead TTS/STT | 音声会話対応 | ローカル Qwen3-ASR + ローカル TTS |
| 「Project Mode」名称 | 廃止（機能は維持） | Lead→Agent 協調は常時アクティブ |
| ローカライゼーション | **Unity Localization（EN + JA）、デフォルト言語は OS 設定追従** | ESC メニューから切替。パッケージ標準で多言語対応 |
| クラッシュ復旧 | スタジオ自動復元 | ~/.gwt/ のデータから状態復元 |
| エラーハンドリング | **基盤サービス一元化** | リトライ3回指数バックオフ、3段階通知、グレースフルデグラデーション |
| テレメトリ | **オプトインクラッシュレポート** | GitHub Issue 自動作成。プライバシー配慮でオプトイン |
| アクセシビリティ | **当面非対応** | MVP ではスコープ外（明示的判断） |
| GFM マークダウンレンダリング | **自前完全実装** | 将来パッケージ化前提。既存パッケージは要件不足 |
| エージェント同時稼働上限 | **スタジオレベル連動** | 初期3体→レベルアップで5→10→無制限。ゲーミフィケーション連携 |

### アセットスタック

詳細は `docs/pixelart.md` を参照。コア構成（約 $153）:

| レイヤー | アセット | 用途 | 関連 SPEC |
|---------|---------|------|----------|
| グラフィック（キャラ） | LimeZu Modern Interiors ($1.50〜) | キャラジェネレータ、16x16/32x32/48x48 | #1547 |
| グラフィック（環境） | LimeZu Modern Office ($5) | オフィスタイルセット（300+スプライト） | #1546 |
| UI 素材 | Pixel UI & HUD ($5) | 700+スプライト | #1548 |
| アニメーション | BoneToPix ($27) | 3D→ドット絵アニメ自動変換 | #1547 |
| レンダリング | Unity 2D Pixel Perfect (無料) | ピクセルパーフェクトカメラ | #1546 |
| ゲーミフィケーション | Shiny Stats ($18) + SBLS ($10) + SOAP ($35) | スタッフ成長・状態管理 | #1555 |
| セーブ/ロード | Easy Save 3 ($59) | 堅牢なセーブシステム | #1542 |
| サウンド | **AI 生成（Suno/Udio 等）** | ピクセルアート風 BGM + SE | #1560 |
