# タスク: Ink UI内蔵仮想ターミナル機能

**入力**: `/specs/SPEC-6d501fd0/` からの設計ドキュメント
**前提条件**: plan.md（必須）、spec.md（ユーザーストーリー用に必須）、research.md、data-model.md、quickstart.md

**テスト**: このプロジェクトはTDDアプローチを採用します（CLAUDE.md: "Spec Kitを用いたSDD/TDDの絶対遵守を義務付ける"）

**構成**: タスクはユーザーストーリーごとにグループ化され、各ストーリーの独立した実装とテストを可能にします。

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（例: US1、US2、US3）
- 説明に正確なファイルパスを含める

## Commitlintルール

- コミットメッセージは件名のみを使用し、空にしてはいけません（`commitlint.config.cjs`の`subject-empty`ルール）
- 件名は100文字以内に収めてください（`subject-max-length`ルール）
- タスク生成時は、これらのルールを満たすコミットメッセージが書けるよう変更内容を整理してください

## フェーズ1: セットアップ（共有インフラストラクチャ）

**目的**: プロジェクトの初期化と依存関係のインストール

### セットアップタスク

- [ ] **T001** [P] [共通] `bun add node-pty` でnode-ptyをインストール
- [ ] **T002** [P] [共通] `bun add -d @types/node-pty` でnode-ptyの型定義をインストール
- [ ] **T003** [P] [共通] `src/pty/` ディレクトリを作成
- [ ] **T004** [P] [共通] `src/ui/components/parts/` ディレクトリが存在することを確認（存在しない場合は作成）

## フェーズ2: 基盤（すべてのストーリーのブロッキング前提条件）

**目的**: すべてのユーザーストーリーで使用される基本的な型定義とPTY管理機能

### 型定義

- [ ] **T101** [P] [基盤] `src/pty/types.ts` にPtyProcess型を定義（pid: number, ptyInstance: IPty, status: 'running' | 'paused' | 'stopped', exitCode?: number, errorMessage?: string）
- [ ] **T102** [P] [基盤] `src/ui/types.ts` にTerminalSession型を追加（id: string, branch: string, tool: 'claude-code' | 'codex-cli', mode: ExecutionMode, worktreePath: string, startTime: Date, endTime?: Date, skipPermissions: boolean）
- [ ] **T103** [P] [基盤] `src/ui/types.ts` にTerminalOutput型を追加（id: string, sessionId: string, timestamp: Date, content: string, isError: boolean）

### PTYマネージャー基本実装

- [ ] **T104** [基盤] T101の後に `src/pty/PtyManager.test.ts` にPtyManager.spawn()のテストを作成
- [ ] **T105** [基盤] T104の後に `src/pty/PtyManager.ts` にPtyManagerクラスとspawn()メソッドを実装
- [ ] **T106** [基盤] T105の後に `src/pty/PtyManager.test.ts` にPtyManager.write()のテストを作成
- [ ] **T107** [基盤] T106の後に `src/pty/PtyManager.ts` にwrite()メソッドを実装
- [ ] **T108** [基盤] T107の後に `src/pty/PtyManager.test.ts` にPtyManager.kill()のテストを作成
- [ ] **T109** [基盤] T108の後に `src/pty/PtyManager.ts` にkill()メソッドを実装

## フェーズ3: ユーザーストーリー1 - コンテキスト情報付きAIツール実行 (優先度: P1)

**ストーリー**: 開発者がブランチを選択し、AIツール（Claude CodeまたはCodex CLI）を起動すると、アプリケーションのUI内にターミナル画面が表示される。画面上部には、現在のブランチ名、選択したツール名、実行モード、作業ディレクトリパスが常に表示され、開発者は自分がどの環境で作業しているかを一目で確認できる。ターミナル領域では、AIツールとシームレスに対話でき、入力した内容と出力結果がリアルタイムで表示される。

**価値**: この機能の最も基本的な価値を提供する。現在のUIでは、AIツール起動時にUIが消えてしまい、コンテキスト情報が失われる問題を解決する。開発者は作業環境を常に認識しながらAIツールを使用できる。

### Reactフック実装

- [ ] **T201** [US1] T109の後に `src/ui/hooks/usePtyProcess.test.ts` にusePtyProcess()フックの基本テストを作成
- [ ] **T202** [US1] T201の後に `src/ui/hooks/usePtyProcess.ts` にusePtyProcess()フックを実装（spawn, outputBuffer, kill）

### ターミナル出力コンポーネント

- [ ] **T203** [P] [US1] `src/ui/components/parts/TerminalOutput.test.tsx` にTerminalOutputコンポーネントのテストを作成（ANSI制御コード処理を含む）
- [ ] **T204** [US1] T203の後に `src/ui/components/parts/TerminalOutput.tsx` にTerminalOutputコンポーネントを実装（ANSI制御コードによる色付きテキストとフォーマット表示に対応）

### ターミナル画面コンポーネント

- [ ] **T205** [US1] T202とT204の後に `src/ui/components/screens/TerminalScreen.test.tsx` にTerminalScreenの基本レンダリングテストを作成
- [ ] **T206** [US1] T205の後に `src/ui/components/screens/TerminalScreen.tsx` にTerminalScreenコンポーネントを実装（ヘッダー表示）
- [ ] **T207** [US1] T206の後に `src/ui/components/screens/TerminalScreen.test.tsx` にPTY起動のテストを追加
- [ ] **T208** [US1] T207の後に `src/ui/components/screens/TerminalScreen.tsx` にPTY起動ロジックを追加（useEffect）
- [ ] **T209** [US1] T208の後に `src/ui/components/screens/TerminalScreen.test.tsx` に出力表示のテストを追加
- [ ] **T210** [US1] T209の後に `src/ui/components/screens/TerminalScreen.tsx` に出力表示ロジックを追加

### キーボード入力処理

- [ ] **T211** [US1] T210の後に `src/ui/components/screens/TerminalScreen.test.tsx` に通常キー入力のテストを追加
- [ ] **T212** [US1] T211の後に `src/ui/components/screens/TerminalScreen.tsx` にuseInput()フックで通常キー入力をPTYに転送

### App.tsx統合

- [ ] **T213** [US1] T212の後に `src/ui/components/App.test.tsx` にTerminalScreen遷移のテストを追加
- [ ] **T214** [US1] T213の後に `src/ui/components/App.tsx` のhandleModeSelect()を修正してTerminalScreenに遷移
- [ ] **T215** [US1] T214の後に `src/ui/components/App.tsx` のrenderScreen()にTerminalScreenのケースを追加

### AIツール終了処理

- [ ] **T216** [US1] T215の後に `src/ui/components/screens/TerminalScreen.test.tsx` にAIツール終了のテストを追加
- [ ] **T217** [US1] T216の後に `src/ui/components/screens/TerminalScreen.tsx` にPTY終了イベントハンドリングを追加
- [ ] **T218** [US1] T217の後に `src/ui/components/screens/TerminalScreen.tsx` に終了後のonBack()呼び出しを実装

### 統合テスト

- [ ] **T219** [US1] T218の後に `src/ui/__tests__/acceptance/terminal.acceptance.test.tsx` にUS1の受け入れシナリオテストを作成

**✅ MVP1チェックポイント**: US1完了後、この機能は独立した価値を提供可能

## フェーズ4: ユーザーストーリー2 - AIツール実行の制御とログ保存 (優先度: P2)

**ストーリー**: 開発者は、ターミナル画面でAIツールを実行中に、特定のキー操作で実行を制御できる。処理を中断したい場合はCtrl+Cで即座に終了し、一時的に停止したい場合はCtrl+Zで停止・再開できる。また、AIツールとの対話履歴を後で確認したい場合、Ctrl+Sで出力ログをファイルに保存できる。

**価値**: 基本的な実行機能の上に、より高度な操作性を提供する。長時間実行されるタスクの制御や、後で参照するためのログ保存が可能になり、開発者の生産性が向上する。

### Ctrl+C（中断）実装

- [ ] **T301** [P] [US2] `src/ui/components/screens/TerminalScreen.test.tsx` にCtrl+C押下のテストを追加
- [ ] **T302** [US2] T301の後に `src/ui/components/screens/TerminalScreen.tsx` にCtrl+Cハンドリングを実装（kill() + onBack()）

### Ctrl+Z（一時停止/再開）実装

- [ ] **T303** [US2] T109の後に `src/pty/PtyManager.test.ts` にpause()メソッドのテストを追加
- [ ] **T304** [US2] T303の後に `src/pty/PtyManager.ts` にpause()メソッドを実装（Unix系のみ、SIGSTOP）
- [ ] **T305** [US2] T304の後に `src/pty/PtyManager.test.ts` にresume()メソッドのテストを追加
- [ ] **T306** [US2] T305の後に `src/pty/PtyManager.ts` にresume()メソッドを実装（Unix系のみ、SIGCONT）
- [ ] **T307** [US2] T306の後に `src/ui/hooks/usePtyProcess.ts` にpause()とresume()をエクスポート
- [ ] **T308** [US2] T307の後に `src/ui/components/screens/TerminalScreen.test.tsx` にCtrl+Z押下のテストを追加
- [ ] **T309** [US2] T308の後に `src/ui/components/screens/TerminalScreen.tsx` にCtrl+Zハンドリングを実装（pause/resumeトグル）

### Ctrl+S（ログ保存）実装

- [ ] **T310** [P] [US2] `src/ui/types.ts` にLogFile型を追加
- [ ] **T311** [US2] T310の後に `src/ui/utils/logSaver.test.ts` にsaveLog()関数のテストを作成
- [ ] **T312** [US2] T311の後に `src/ui/utils/logSaver.ts` にsaveLog()関数を実装（.logs/ディレクトリ作成、ファイル書き込み）
- [ ] **T313** [US2] T312の後に `src/ui/components/screens/TerminalScreen.test.tsx` にCtrl+S押下のテストを追加
- [ ] **T314** [US2] T313の後に `src/ui/components/screens/TerminalScreen.tsx` にCtrl+Sハンドリングを実装（saveLog()呼び出し）

### 統合テスト

- [ ] **T315** [US2] T314の後に `src/ui/__tests__/acceptance/terminal.acceptance.test.tsx` にUS2の受け入れシナリオテストを追加

**✅ MVP2チェックポイント**: US2完了後、機能は拡張された価値を提供

## フェーズ5: ユーザーストーリー3 - 全画面表示による視認性向上 (優先度: P3)

**ストーリー**: 開発者がF11キーを押すと、ヘッダーとフッターが非表示になり、ターミナル領域が画面全体に拡大される。長い出力を確認する際や、より多くの情報を一度に表示したい場合に、画面スペースを最大限に活用できる。

**価値**: 基本機能と制御機能が実装された後の、UI体験の改善機能。必須ではないが、大量の出力を扱う場合のユーザビリティを向上させる。

### F11（全画面切替）実装

- [ ] **T401** [P] [US3] `src/ui/components/screens/TerminalScreen.test.tsx` にF11押下のテストを追加
- [ ] **T402** [US3] T401の後に `src/ui/components/screens/TerminalScreen.tsx` にisFullscreen状態を追加
- [ ] **T403** [US3] T402の後に `src/ui/components/screens/TerminalScreen.tsx` にF11ハンドリングを実装（isFullscreenトグル）
- [ ] **T404** [US3] T403の後に `src/ui/components/screens/TerminalScreen.tsx` にヘッダー/フッター表示の条件分岐を実装

### 統合テスト

- [ ] **T405** [US3] T404の後に `src/ui/__tests__/acceptance/terminal.acceptance.test.tsx` にUS3の受け入れシナリオテストを追加

**✅ 完全な機能**: US3完了後、すべての要件が満たされます

## フェーズ6: 統合とポリッシュ

**目的**: すべてのストーリーを統合し、プロダクション準備を整える

### エラーハンドリング

- [ ] **T501** [統合] `src/ui/components/screens/TerminalScreen.test.tsx` にPTY起動失敗のテストを追加
- [ ] **T502** [統合] T501の後に `src/ui/components/screens/TerminalScreen.tsx` にPTY起動エラーハンドリングを実装（FR-011対応）
- [ ] **T503** [統合] T502の後に `src/ui/components/screens/TerminalScreen.test.tsx` にPTY異常終了のテストを追加
- [ ] **T504** [統合] T503の後に `src/ui/components/screens/TerminalScreen.tsx` にPTY異常終了ハンドリングを実装
- [ ] **T505** [統合] T504の後に `src/ui/utils/logSaver.test.ts` にディスク容量不足のテストを追加
- [ ] **T506** [統合] T505の後に `src/ui/utils/logSaver.ts` にディスク容量不足エラーハンドリングを実装
- [ ] **T506a** [P] [統合] `src/ui/components/screens/TerminalScreen.test.tsx` にPTY利用不可時のテストを追加（FR-012対応）
- [ ] **T506b** [統合] T506aの後に `src/ui/components/screens/TerminalScreen.tsx` にPTY利用不可時のフォールバック処理を実装（警告表示と従来動作）

### プラットフォーム固有の調整

- [ ] **T507** [P] [統合] `src/pty/PtyManager.ts` にプラットフォーム判定（process.platform）を追加
- [ ] **T508** [統合] T507の後に `src/ui/components/screens/TerminalScreen.tsx` にWindows環境でのCtrl+Z無効化ロジックを追加
- [ ] **T509** [統合] T508の後に `src/ui/components/screens/TerminalScreen.tsx` のフッターにプラットフォーム別のアクション表示を実装

### パフォーマンス最適化

- [ ] **T510** [P] [統合] `src/ui/hooks/usePtyProcess.ts` に出力バッファリング（16ms毎の更新）を実装
- [ ] **T511** [P] [統合] `src/ui/hooks/usePtyProcess.ts` にスクロールバッファ制限（MAX_OUTPUT_LINES = 10000）を実装

### パフォーマンス測定

- [ ] **T511a** [P] [統合] `src/ui/__tests__/performance/terminal-startup.test.ts` に起動時間測定テストを作成（SC-001: 5秒以内）
- [ ] **T511b** [P] [統合] `src/ui/__tests__/performance/input-latency.test.ts` にキー入力レイテンシ測定テストを作成（SC-002: 100ms以内）
- [ ] **T511c** [P] [統合] `src/ui/__tests__/performance/output-latency.test.ts` に出力表示レイテンシ測定テストを作成（SC-003: 200ms以内）
- [ ] **T511d** [P] [統合] `src/ui/__tests__/performance/terminal-exit.test.ts` に終了時間測定テストを作成（SC-005: 1秒以内）
- [ ] **T511e** [P] [統合] `src/ui/__tests__/performance/log-save.test.ts` にログ保存速度測定テストを作成（SC-006: 1MB < 1秒）
- [ ] **T511f** [P] [統合] `src/ui/__tests__/acceptance/ansi-visual.test.tsx` にANSI制御コード表示の視覚的検証テストを作成（SC-007対応）

### CI/CD検証

- [ ] **T512** [統合] `.github/workflows/test.yml` に合わせて `bun run type-check` をローカルで実行し、失敗時は修正
- [ ] **T513** [統合] `.github/workflows/test.yml` に合わせて `bun run lint` をローカルで実行し、失敗時は修正
- [ ] **T514** [統合] `.github/workflows/test.yml` に合わせて `bun run test` をローカルで実行し、失敗時は修正
- [ ] **T515** [統合] `.github/workflows/test.yml` に合わせて `bun run test:coverage` をローカルで実行し、失敗時は修正
- [ ] **T516** [統合] `.github/workflows/test.yml` に合わせて `bun run build` をローカルで実行し、失敗時は修正
- [ ] **T517** [統合] `.github/workflows/lint.yml` に合わせて `bun run format:check` をローカルで実行し、失敗時は修正
- [ ] **T518** [統合] `.github/workflows/lint.yml` に合わせて `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore` をローカルで実行し、失敗時は修正

### ドキュメント

- [ ] **T519** [P] [ドキュメント] `README.md` にTerminalScreen機能の説明を追加
- [ ] **T520** [P] [ドキュメント] `README.ja.md` にTerminalScreen機能の説明を追加

## タスク凡例

**優先度**:

- **P1**: 最も重要 - MVP1に必要
- **P2**: 重要 - MVP2に必要
- **P3**: 補完的 - 完全な機能に必要

**依存関係**:

- **[P]**: 並列実行可能
- **T###の後に**: 指定タスクの後に実行

**ストーリータグ**:

- **[US1]**: ユーザーストーリー1
- **[US2]**: ユーザーストーリー2
- **[US3]**: ユーザーストーリー3
- **[共通]**: すべてのストーリーで共有
- **[基盤]**: すべてのストーリーのブロッキング前提条件
- **[統合]**: 複数ストーリーにまたがる
- **[ドキュメント]**: ドキュメント専用

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
- **スキップしたタスク**: 理由と共に文書化

## タスク統計

- **総タスク数**: 80
- **Phase 1 (Setup)**: 4タスク
- **Phase 2 (Foundational)**: 9タスク
- **Phase 3 (US1)**: 19タスク
- **Phase 4 (US2)**: 15タスク
- **Phase 5 (US3)**: 5タスク
- **Phase 6 (Polish)**: 28タスク（エラーハンドリング8、プラットフォーム調整3、最適化2、測定6、CI/CD検証7、ドキュメント2）

## 並列実行の機会

### Phase 1 (Setup)

- T001, T002, T003, T004 は並列実行可能

### Phase 2 (Foundational)

- T101, T102, T103 は並列実行可能

### Phase 3 (US1)

- T203 はT202の完了を待たずに開始可能

### Phase 4 (US2)

- T301, T303, T310 は並列実行可能（異なるファイル）

### Phase 5 (US3)

- T401 は単独で開始可能

### Phase 6 (Polish)

- T506a, T507, T510, T511, T511a-f, T519, T520 は並列実行可能（異なるファイル）

## 依存関係グラフ

```text
Phase 1: Setup
  T001, T002, T003, T004 [並列]
    ↓
Phase 2: Foundational
  T101, T102, T103 [並列]
    ↓
  T104 → T105 → T106 → T107 → T108 → T109
    ↓
Phase 3: US1 (P1)
  T201 → T202
  T203 → T204 [並列]
    ↓
  T205 → T206 → T207 → T208 → T209 → T210 → T211 → T212
    ↓
  T213 → T214 → T215
    ↓
  T216 → T217 → T218
    ↓
  T219
    ↓
Phase 4: US2 (P2)
  T301 → T302
  T303 → T304 → T305 → T306 → T307 → T308 → T309 [並列]
  T310 → T311 → T312 → T313 → T314
    ↓
  T315
    ↓
Phase 5: US3 (P3)
  T401 → T402 → T403 → T404
    ↓
  T405
    ↓
Phase 6: Polish
  T501 → T502 → T503 → T504 → T505 → T506
  T506a → T506b [並列]
  T507 → T508 → T509 [並列]
  T510, T511 [並列]
  T511a, T511b, T511c, T511d, T511e, T511f [並列]
    ↓
  T512 → T513 → T514 → T515 → T516 → T517 → T518
  T519, T520 [並列]
```

## 実装戦略

### MVP First

**MVP1 (US1のみ)**: 基本的なターミナル機能

- セットアップ（Phase 1）
- 基盤（Phase 2）
- US1実装（Phase 3）
- 推定工数: 2-3日

**MVP2 (US1+US2)**: 制御とログ保存機能追加

- US2実装（Phase 4）
- 推定工数: +1-2日

**完全版 (US1+US2+US3)**: 全画面モード追加

- US3実装（Phase 5）
- 推定工数: +0.5-1日

**プロダクション準備 (すべて+Polish)**: エラーハンドリング、最適化、ドキュメント

- 統合とポリッシュ（Phase 6）
- 推定工数: +1-2日

### 独立したデリバリー

各ユーザーストーリーは独立してテスト・デリバリー可能：

- **US1完了後**: ブランチをマージして基本機能をリリース可能
- **US2完了後**: 制御機能を追加リリース可能
- **US3完了後**: UI改善を追加リリース可能

### TDDアプローチ

すべてのタスクはTDD（Test-Driven Development）に従います：

1. テストを先に書く（Red）
2. テストが失敗することを確認
3. 最小限の実装でテストをパス（Green）
4. リファクタリング（Refactor）

各機能の実装タスクには対応するテストタスクが先行しています。

## 注記

- 各タスクは1時間から1日で完了可能であるべき
- より大きなタスクはより小さなサブタスクに分割
- ファイルパスは正確で、プロジェクト構造と一致させる
- 各ストーリーは独立してテスト・デプロイ可能
- TDDアプローチに従い、テストファーストで実装
- Commitlintルールに従い、コミットメッセージは100文字以内
