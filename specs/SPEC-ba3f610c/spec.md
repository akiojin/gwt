# 機能仕様: プロジェクトモード（Project Mode）

**仕様ID**: `SPEC-ba3f610c`
**作成日**: 2026-01-22
**更新日**: 2026-02-27
**ステータス**: 更新済み
**カテゴリ**: GUI
**入力**: 3層エージェントアーキテクチャ（Lead / Coordinator / Worker）によるプロジェクト統括。Lead（PM相当）がプロジェクトのゴールを保持し、対話モード（質問への回答）と計画モード（要件収集→整理→仕様化→委譲）で動作する。Coordinator（オーケストレーター）がWorker管理・CI監視・修正ループを行い、Worker（ペルソナベースの専門エージェント）がWorktree内で実際の実装を行う。ユーザーはプロジェクト概要または具体的な機能要求のどちらでもLeadに伝えられる。

## アーキテクチャ概要

### 3層エージェント構造

| 層 | 内部名 | UI表示名 | 役割 | 実行場所 |
|---|---|---|---|---|
| PM | PM | Lead | プロジェクト統括・対話モード（質問回答）・計画モード（要件収集→整理→仕様化→委譲）・進捗管理・段階的委譲 | gwt内蔵AI |
| Orchestrator | Orchestrator | Coordinator | Issue単位のタスク管理・CI監視・修正ループ・Worker管理・ペルソナ選定 | GUI内蔵ターミナルペイン |
| Worker | Worker | **ペルソナの役割名**（例: "Frontend Dev", "Researcher"）。未設定時デフォルト: "Worker" | 実際のコード実装・テスト・PR作成。ペルソナにより専門性が異なる | GUI内蔵ターミナルペイン |

### 各層の責務

**Lead（PM）**:

- **プロジェクトのゴールを保持**し、全体像を理解する
- ユーザーからのプロジェクト概要または具体的な機能要求を受け取る
- **2つの動作モード**を持つ:

**対話モード（常時）**: ユーザーの質問に答える

- 「〜の機能はどうなっている？」→ コードベースを調べて回答
- 「現在の進捗は？」→ Issue/タスク状況を報告
- 「〜の実装方法は？」→ 既存コードを分析して提案
- Leadは `read_file`, `search_code`, `list_directory` 等のツールで自律的に調査

**計画モード（要件収集→実行）**: システム構築のフロー

1. **収集フェーズ**: ユーザーが自由に要望を投げ込む。Leadは否定せず全て受け止め、確認質問で深掘り。「もうない」「以上」等でフェーズ終了
2. **整理フェーズ**: 収集した要望を大項目（Epic）・中項目（Story）に構造化して提示。構造の確認を取る
3. **仕様化フェーズ**: 承認後、各項目をGitHub Issue化（spec/plan/tasks/tddセクション付き）
4. **委譲フェーズ**: Coordinatorを起動し、GitHub Issue番号と推奨ペルソナ情報を渡す

- **GitHub Issueベースの仕様管理**: 要件→仕様→計画→タスク→TDDをGitHub Issueに記録
- GitHub Issue作成・GitHub Project登録・ステータス管理
- ユーザーへの計画提示と承認取得
- Coordinatorの起動・停止・再起動の判断
- 全体進捗の把握（PRマージ状況・CI状況含む）とユーザーへの報告
- 段階的委譲に基づく自律的判断
- Worktreeは持たない（コードを書かないため）

**プロジェクト知識の構築**:

- Leadは初回起動時とセッション開始時にリポジトリをスキャンし、プロジェクト構造・技術スタック・既存機能を把握する
- 知識はセッションに永続化

**Leadの核心的価値**:

- ユーザーがプロジェクト全容を覚えておく必要がない。Leadがプロジェクトの「記憶」「ナレッジベース」として機能する。

**Coordinator（Orchestrator）**:

- Leadから受け取った仕様・タスクに基づくWorker管理
- タスク内容を分析し、最適なWorkerペルソナを選定
- Worktree作成とWorkerへのタスク割り当て
- Worker間の依存関係管理（Git merge等）
- 成果物検証（テスト実行）とPR作成
- **CI結果の監視と修正ループ**（CI失敗 → Worker修正指示 → 再プッシュ → CI再実行）
- 失敗時のリトライ・代替アプローチ判断
- Claude Code Agent Team等のチームセッションとして動作可能
- Worktreeは持たない（コードを書かないため）

**Worker**:

- **Worktree内**での個別タスクの実装（コーディング・テスト・コミット）
- ペルソナに基づく専門性を持つ（Frontend Dev, Backend Dev, Researcher等）
- Worktree単位で管理・表示される
- 完了報告（Hook / GWT_TASK_DONE / プロセス終了）

### エンティティ関係

| 関係 | カーディナリティ | 説明 |
|---|---|---|
| Project : Lead | 1 : 1 | 1プロジェクトに1つのLeadが常駐 |
| Project : Issue | 1 : N | Leadが要件定義からIssueを生成 |
| Issue : Coordinator | 1 : 1 | 各IssueにCoordinatorを1つ起動 |
| Issue : GitHub Issue | 1 : 1 | 各IssueにGitHub Issueが対応（仕様・計画・タスクを格納） |
| Issue : Task | 1 : N | GitHub Issue内のタスク定義からタスクを生成 |
| Task : Worker | 1 : N | 1タスクを複数Workerが並列実装可 |
| Worker : Worktree | 1 : 1 | 各Workerが専用Worktreeで作業 |
| Worker : Persona | N : 1 | 各WorkerにペルソナをCoordinatorが割り当て |

### プロセス分離

- Lead、Coordinator、Workerはそれぞれ**独立したLLMセッション**として動作する。
- 各層は独立したプロセスであり、上位層の障害は下位層に影響しない。
- 層間の通信はPTY直接通信（send_keys系）を基盤とし、スキルとして公開する。
- 複数のCoordinatorが**並列に起動可能**（リソースが許す限り）。

### エンドツーエンドフロー

1. ユーザーがプロジェクト概要**または**具体的な機能要求をLeadに伝える（収集フェーズ開始）
2. Leadが要望を全て受け止め、確認質問で深掘りする。ユーザーが「以上」等でフェーズ完了
3. Leadが要望を大項目（Epic）・中項目（Story）に構造化して提示する（整理フェーズ）
4. ユーザーが構造を確認・修正する
5. Leadが要件をIssue単位に分割し、各IssueについてGitHub Issueを作成する（仕様・計画・タスク・TDDをIssueに記録）
6. LeadがGitHub Projectに各Issueを登録する
7. Leadがユーザーに**プロジェクト全体の計画**を提示し承認を得る
8. 承認後、Leadが各Issueに対してCoordinatorを内蔵ターミナルペインで起動し、**GitHub Issue番号**と推奨ペルソナ情報を渡す（複数Coordinator並列起動可）
9. 各CoordinatorがGitHub Issueから仕様・タスクを読み取り、タスク内容に基づいて最適なペルソナを選定し、Worktreeを作成してWorkerを起動する（1 Task = N Worker = N Worktree可）
10. CoordinatorがWorker完了検出 → テスト検証 → PR作成を行う
11. CoordinatorがCI結果を監視し、失敗時はWorkerに修正を指示する（自律修正ループ）
12. Leadが全Coordinatorの進捗を把握し、ユーザーに定期報告する
13. 各Issue完了（全タスク完了・CI全パス）後、LeadがGitHub Issueをクローズし、Projectステータスを更新する
14. 全Issue完了後、プロジェクト完了とする

### プロジェクト管理の統一先

- プロジェクトとして対応すべき全てのアイテムは**GitHub Issue**に統一する。
- Leadが作成したIssueは**GitHub Project**に登録し、フェーズ管理を行う。
- GitHub Issue ↔ PR の紐付けにより、要件から成果物までのトレーサビリティを確保する。
- Project上のステータス遷移: draft → ready → planned → ready-for-dev → in-progress → done

### gwtの提供価値

gwtは以下を統合的に提供するプラットフォームとして機能する：

- 複数のAgent Teamセッション（Coordinator）の統括管理
- 各セッションへのWorktree自動割り当て
- 成果物のPR経由統合
- GitHub Issueベースの仕様駆動開発（issue_specツール連携）
- CI監視と自律的修正ループ
- プロジェクト全体の可視化

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - モード切り替えと基本対話 (優先度: P1)

開発者がGUIのタブバーで`Project Mode`タブを選ぶと、Leadとの対話画面が表示される。ユーザーは自由形式でタスクを入力し、Leadがタスクを分析・Coordinatorの起動計画を応答する。

**この優先度の理由**: プロジェクトモードへの入口であり、Leadとの対話がすべての機能の基盤となる。

**独立したテスト**: `Project Mode`タブを開き、タスクを入力して応答を受け取ることで検証できる。

**受け入れシナリオ**:

1. **前提条件** ブランチモードが表示されている、**操作** `Project Mode`タブを選択、**期待結果** プロジェクトモード画面に切り替わり、Leadチャット画面が表示される
2. **前提条件** プロジェクトモード画面が表示されている、**操作** `Settings`や`Version History`など別タブを選択、**期待結果** ブランチモードに戻る
3. **前提条件** AI設定が無効（エンドポイントまたはモデル未設定）、**操作** プロジェクトモードに切り替え、**期待結果** 設定促進メッセージが表示される
4. **前提条件** プロジェクトモード画面が表示されている、**操作** 「認証機能を実装して」と入力、**期待結果** Leadがタスク分析とCoordinator起動計画を応答
5. **前提条件** プロジェクトモードでタスクが実行中、**操作** `Tab`でブランチモードに切り替え、**期待結果** タスクはバックグラウンドで継続、ブランチモードに切り替わる

---

### ユーザーストーリー 2 - GitHub Issue仕様管理と計画承認 (優先度: P1)

Leadがユーザーの入力を受けて要件定義を行い、GitHub Issueに仕様・計画・タスク・TDDを記録する。完成した計画をユーザーに提示し、承認後にCoordinatorを起動してGitHub Issue番号を渡す。

**この優先度の理由**: GitHub Issueベースの仕様管理がプロジェクトモードの核心機能であり、Coordinator/Worker実行の前提条件となる。

**独立したテスト**: タスクを入力し、LeadがGitHub Issueに仕様・計画・タスク・TDDを記録し、承認後にCoordinatorが起動されることを確認する。

**承認方式**: 計画一括承認。Leadがタスク計画全体を提示し、ユーザーが1回承認すれば、以降のCoordinator起動・WT作成・Worker起動・テスト検証・PR作成はすべてLead/Coordinatorが自律実行する。ユーザーは対話画面でいつでも介入・中止が可能。

**受け入れシナリオ**:

1. **前提条件** ユーザーがプロジェクト概要または機能要求を入力、**操作** Leadが要件を明確化する質問を行う、**期待結果** ユーザーとの対話で要件が確定する
2. **前提条件** 要件が確定、**操作** LeadがGitHub Issueに仕様・計画・タスク・TDDを記録、**期待結果** GitHub Issueに仕様セクション（spec/plan/tasks/tdd）が登録される
3. **前提条件** GitHub Issueの仕様4セクションが揃う、**操作** Leadが計画全体をユーザーに提示、**期待結果** ユーザーが承認/拒否できる
4. **前提条件** ユーザーが計画を承認、**操作** LeadがCoordinatorを起動、**期待結果** Coordinatorが内蔵ターミナルペインで起動され、GitHub Issue番号を受け取る
5. **前提条件** ユーザーが計画を拒否、**操作** 拒否理由を入力、**期待結果** Leadが計画を再策定する

---

### ユーザーストーリー 3 - Worker起動と実装 (優先度: P1)

Coordinatorがworktreeを作成し、Worker（Claude Code等）をGUI内蔵ターミナルペインで起動する。Workerはタスクを実装し、完了を報告する。

**この優先度の理由**: Workerへの指示がタスク実行の実体であり、プロジェクトモードの価値を実現する核心機能。

**独立したテスト**: タスク実行を開始し、WorkerがGUI内蔵ターミナルペインで起動してプロンプトを受け取ることを確認する。

**受け入れシナリオ**:

1. **前提条件** worktreeが作成済み、**操作** タスク実行開始、**期待結果** GUI内蔵ターミナルペインでWorker（Claude Code等）が起動
2. **前提条件** Workerが起動、**操作** Coordinatorがプロンプト送信、**期待結果** PTY直接通信でプロンプトが入力される
3. **前提条件** プロンプト送信、**操作** プロンプト内容を確認、**期待結果** タスク指示と完了指示が含まれる
4. **前提条件** 複数Workerを起動、**操作** 並列実行、**期待結果** 各Workerが別々のGUI内蔵ターミナルペインで動作

---

### ユーザーストーリー 4 - Worker完了検出 (優先度: P1)

CoordinatorがWorkerの完了を検出する。Claude CodeはHookのStop経由、他のエージェントはGUI内蔵ターミナルの複合方式（プロセス終了監視 + スクロールバック出力パターン監視 + アクティビティ監視）で検出する。

**この優先度の理由**: 完了検出なしにはタスクの進行管理と次のステップへの移行ができない。

**独立したテスト**: Workerがタスクを完了し、Coordinatorが完了を検出して次のアクションに移行することを確認する。

**受け入れシナリオ**:

1. **前提条件** Claude Codeが動作中、**操作** タスク完了でqを押す、**期待結果** Hook経由で完了が検出される
2. **前提条件** Claude Code以外のWorkerが動作中、**操作** プロセスが終了、**期待結果** ペイン終了検出で完了が検出される
3. **前提条件** スクロールバック出力パターン監視、**操作** 特定の完了パターンが出力、**期待結果** `capture_scrollback_tail`で完了が検出される
4. **前提条件** 複合方式での監視、**操作** いずれかの条件を満たす、**期待結果** 完了が検出され、Coordinatorに通知される

---

### ユーザーストーリー 5 - 成果物検証と統合（PR経由） (優先度: P2)

Worker完了後、Coordinatorがテスト実行による自動検証を行い、パスした場合にPRを作成する。CoordinatorがCI結果を監視し、失敗時はWorkerに修正を指示する自律修正ループを実行する。

**この優先度の理由**: 成果物の品質検証と統合はフルサイクルに必須だが、基本的なタスク実行が動作してからの改善項目。

**独立したテスト**: 複数タスク完了後、テスト検証を経てPRが作成され、CIがパスすることを確認する。

**受け入れシナリオ**:

1. **前提条件** Workerがタスク完了、**操作** Coordinatorがテスト実行を指示、**期待結果** Workerが当該worktreeでテスト（`cargo test`等）を実行する
2. **前提条件** テストがパス、**操作** 成果物統合フェーズ開始、**期待結果** worktreeからPRが作成される
3. **前提条件** テストが失敗、**操作** Coordinatorが検出、**期待結果** Workerに修正を指示し再テスト（最大3回まで）
4. **前提条件** PRが作成された、**操作** CoordinatorがCI結果を監視、**期待結果** CI失敗時にWorkerへ修正指示を自律的に発行し再プッシュ→CI再実行する（最大3回）
5. **前提条件** マージ時にコンフリクト発生、**操作** Coordinatorが検出、**期待結果** Workerにコンフリクト解決を指示

---

### ユーザーストーリー 6 - 障害ハンドリングと層間独立性 (優先度: P2)

各層が独立して動作し、上位層の障害が下位層に影響しない。Workerの失敗はCoordinatorが、Coordinatorの失敗はLeadが対処する。

**この優先度の理由**: 堅牢性に必要だが、基本フローが動作してからの改善項目。

**独立したテスト**: 各層の障害シナリオを再現し、他層が独立して動作し続けることを確認する。

**受け入れシナリオ**:

1. **前提条件** Workerがエラー終了、**操作** Coordinatorが検出、**期待結果** Coordinatorがリトライ/代替/相談を判断（Lead承認不要）
2. **前提条件** Coordinatorがクラッシュ、**操作** Leadが検出、**期待結果** Leadが自律的にCoordinatorを再起動（人間承認不要）。実行中のWorkerは現タスクを続行
3. **前提条件** LeadのLLM API障害、**操作** API応答なし、**期待結果** Coordinator/Workerは独立して続行。Lead復旧後に状態を再取得
4. **前提条件** 複数回の失敗、**操作** Coordinatorが判断、**期待結果** ユーザーに相談または代替アプローチを提案

---

### ユーザーストーリー 7 - セッション永続化と再開 (優先度: P2)

プロジェクトモードのセッション状態（全層の状態、タスク一覧、進捗、会話履歴）を完全永続化する。gwtを再起動しても途中のセッションを再開できる。

**この優先度の理由**: 長時間タスクの中断・再開に必須だが、基本フローが動作してからの改善項目。

**受け入れシナリオ**:

1. **前提条件** タスク実行中、**操作** gwtを終了、**期待結果** 全層のセッション状態が`~/.gwt/sessions/`に保存される
2. **前提条件** セッションが保存されている、**操作** gwtを再起動、**期待結果** 前回のセッションを再開できる
3. **前提条件** セッション再開、**操作** 継続実行、**期待結果** 中断前の状態から継続できる

---

### ユーザーストーリー 8 - 直接アクセスと層間対話 (優先度: P2)

ユーザーはLeadとの対話を基本としつつ、Coordinator/Workerにも直接アクセスできる。Workerにはターミナル直接操作、Coordinator/Leadにはチャットで対話する。

**この優先度の理由**: 上級ユーザーの柔軟な操作に必要だが、Lead単一窓口で基本機能は動作する。

**受け入れシナリオ**:

1. **前提条件** Workerが動作中、**操作** Workerのターミナルペインを選択してキー入力、**期待結果** Worker端末に直接入力される
2. **前提条件** Coordinatorが動作中、**操作** Coordinatorのチャットに「タスクXを優先して」と入力、**期待結果** Coordinatorが指示を受けて対応する
3. **前提条件** Lead画面表示中、**操作** ダッシュボードでIssueを展開、**期待結果** Coordinator詳細と配下Worker一覧が表示される

---

### ユーザーストーリー 9 - コンテキスト管理（要約圧縮） (優先度: P3)

タスクが大規模になりLLMのコンテキストウィンドウを超える可能性がある場合、完了タスクの情報を要約圧縮してコンテキストを管理する。

**受け入れシナリオ**:

1. **前提条件** 対話が長くなりコンテキストが大きくなる、**操作** 継続して対話、**期待結果** 完了タスクの情報が要約圧縮される

---

### ユーザーストーリー 10 - Skill/Plugin登録スコープ選択 (優先度: P2)

ユーザーとして、Codex/Claude Code/Gemini向けのSkill/Plugin登録先を`User`/`Project`/`Local`から選択したい。環境に応じて共有範囲を切り替え、個人設定・リポジトリ共有設定・ローカル専用設定を使い分けられるようにする。

**この優先度の理由**: チーム運用ではProject共有設定、個人運用ではUser設定、検証用途ではLocal設定が必要になり、登録先固定では運用要件を満たせない。

**独立したテスト**: 設定画面でスコープを切り替え、`repair skill registration`実行時に各エージェントの登録先ファイル/ディレクトリが選択どおりになることを確認すれば検証できる。

**受け入れシナリオ**:

1. **前提条件** デフォルトスコープが`User`、**操作** Skill Registration Repairを実行、**期待結果** Codex/Geminiは`~/.codex/skills`/`~/.gemini/skills`、Claude Codeは`~/.claude/settings.json`側に登録される
2. **前提条件** デフォルトスコープが`Project`、**操作** Repairを実行、**期待結果** `<repo>/.codex/skills`/`<repo>/.gemini/skills`と`<repo>/.claude/settings.json`に登録される
3. **前提条件** デフォルトスコープが`Local`、**操作** Repairを実行、**期待結果** `<repo>/.codex/skills.local`/`<repo>/.gemini/skills.local`と`<repo>/.claude/settings.local.json`に登録される
4. **前提条件** Agent別上書きで`Claude=Project`, `Codex=User`, `Gemini=Local`、**操作** Repairを実行、**期待結果** 各エージェントが個別スコープに従って登録される
5. **前提条件** Scopeを変更後、**操作** Skill Registration Statusを表示、**期待結果** Status判定とmissing項目が選択中スコープ基準で表示される

---

### ユーザーストーリー 11 - Worker ペルソナ管理 (優先度: P2)

ユーザーがSettings画面でWorkerペルソナを管理（追加/編集/削除）できる。各ペルソナにはrole_label、タグ、エージェントタイプ、システムプロンプト追加テキストを設定できる。

**この優先度の理由**: ペルソナによるWorkerの専門性制御はプロジェクトモードの品質向上に寄与するが、デフォルトペルソナで基本動作は可能。

**独立したテスト**: Settings画面でペルソナのCRUD操作を行い、保存・復元・削除が正常に動作することを確認する。

**受け入れシナリオ**:

1. **前提条件** Settings画面表示、**操作** Worker Personasセクションを開く、**期待結果** 組み込みデフォルト含む全ペルソナのカード一覧が表示される
2. **前提条件** ペルソナ一覧表示、**操作** 「Add Persona」をクリック、**期待結果** ペルソナ作成モーダルが表示される
3. **前提条件** モーダル表示、**操作** 各フィールドを入力してSave、**期待結果** 新ペルソナがグローバルまたはプロジェクトスコープに保存される
4. **前提条件** ペルソナ一覧表示、**操作** 既存ペルソナのEditボタン、**期待結果** 編集モーダルが表示され更新可能
5. **前提条件** ペルソナ一覧表示、**操作** Deleteボタン→確認ダイアログで確定、**期待結果** ペルソナが削除される（組み込みは削除不可）

---

### エッジケース

- Coordinator起動中にGUIウィンドウが閉じられた場合、どう復旧するか？
- 同一ファイルを複数Workerが同時に編集しようとした場合、コンフリクト検出のタイミングは？（→ 依存関係のあるタスクはGit merge時に検出、独立タスクはPR統合時に検出）
- LeadのLLM API呼び出しがタイムアウトした場合、Coordinator/Workerはどうなるか？（→ 独立続行）
- セッション復元時に参照していたworktreeが削除されていた場合、どう対処するか？
- テスト検証で3回連続失敗した場合のタスク状態遷移（→ Failed + ユーザー通知）
- Coordinatorがクラッシュした場合、配下のWorkerはどうなるか？（→ 現タスク続行、新規指示なし）
- 大規模プロジェクトでCoordinatorが多数並列起動した場合のリソース管理は？

## 詳細仕様 *(必須)*

### 3層通信プロトコル

#### PTY直接通信（スキル化）

- 層間通信の基盤は既存のPTY直接通信（`send_keys_to_pane`, `send_keys_broadcast`, `capture_scrollback_tail`）を使用する。
- これらのツールは`agent_tools.rs`から**完全移行**し、Codex/Geminiは`~/.codex/skills`/`~/.gemini/skills`へ、Claude Codeはgwtプラグイン（marketplace）経由で提供する。
- Claude Code向けプラグイン（`gwt-integration`）にはHook転送を同梱し、`UserPromptSubmit`/`PreToolUse`/`PostToolUse`/`Notification`/`Stop`を`gwt-tauri hook <Event>`へ転送する。
- Hook転送はベストエフォート実行とし、Hook転送失敗でClaude Code本体の実行をブロックしてはならない。
- `~/.claude/settings.json`への手動Hook登録ダイアログを前提にしてはならず、起動時の自動登録（plugin setup）を正とする。
- Skill/Plugin登録先は`User`/`Project`/`Local`のスコープ選択に従って解決する。
- Lead/CoordinatorはスキルとしてPTY通信を呼び出す。

#### 通信方向と手段

| 方向 | 手段 | 即時性 |
|---|---|---|
| Lead → Coordinator | PTY直接通信（send_keys） | 即時 |
| Coordinator → Worker | PTY直接通信（send_keys） | 即時 |
| Worker → Coordinator | Hook Stop（Claude Code）/ 出力パターン / プロセス終了 | ハイブリッド |
| Coordinator → Lead | ハイブリッド: 重要イベント（完了/失敗）はTauriイベント（`agent-status-changed`）、途中経過はLeadがscrollback読み取り（`capture_scrollback_tail`）で取得 | ハイブリッド |
| Lead → ユーザー | GUIチャット | 即時 |
| ユーザー → Lead/Coordinator/Worker | チャット / ターミナル直接操作 | 即時 |

#### 完了検出の階層

- **Worker → Coordinator**: Claude CodeはHook Stop、他はGWT_TASK_DONEパターン / プロセス終了
- **Coordinator → Lead**: Coordinator完了時はTauriイベント（`agent-status-changed`）+ scrollback確認

### Lead（PM）の段階的委譲

Leadは以下の範囲で人間承認なしに自律的に判断・実行できる：

**自律実行可能（承認不要）**:

- タスクの実行順序変更
- 並列度の調整
- 失敗タスクのリトライ指示
- Coordinatorの再起動
- Workerの差し替え
- CI失敗報告受信時のCoordinatorへの修正方針指示

**人間承認が必要**:

- 実装方針の変更
- タスク計画の大幅な変更
- 新規機能の追加
- リリース判断・トリガー

### Lead（PM）の常駐性

- **ハイブリッド方式**: 基本はイベント駆動、アクティブセッション中は定期ポーリングも併用。
- イベント駆動のトリガー:
  - Worker完了検出（Coordinator経由）
  - Coordinator完了/失敗検出
  - ユーザーからのチャット入力
  - CI結果の変更（GitHub Actionsステータス変更）
  - セッション開始（初回タスク入力）
- アクティブセッション中の定期ポーリング:
  - 2分間隔でCoordinator/Workerの状態をチェック
  - CI結果の定期確認
- イベント間はLead LLMコール不要（アイドル状態）。

### Lead実行基盤

- Leadは**gwt内蔵AI**として動作する（現在のMaster Agent相当を拡張）。
- gwt自身がLLMを呼び出し、GitHub Issue仕様管理・対話ループ・Coordinator管理を実行する。
- チャットUIで統一的なUXを提供する。

### LeadStatus

```text
Idle          // 待機中
Collecting    // ユーザーから要望を収集中
Organizing    // 収集した要望を構造化中
WaitingApproval // 計画の承認待ち
Specifying    // GitHub Issue作成中
Orchestrating // Coordinator起動・管理中
Thinking      // 質問への回答を思考中
Error         // エラー状態
```

### Lead用ツール拡張

既存ツール（PTY通信 + issue_spec）に加え:

- `read_file`: プロジェクトファイル読み取り（リポジトリ理解用）
- `list_directory`: ディレクトリ一覧（構造把握用）
- `search_code`: コード検索（既存実装把握用）
- `get_git_status`: Git状態確認
- `list_personas`: 利用可能なWorkerペルソナ一覧取得

### Coordinator起動と管理

- CoordinatorはGUI内蔵ターミナルペインで起動する（ユーザーが出力を見られる）。
- Coordinatorの**作業ディレクトリ（cwd）はリポジトリルート**とする。コードは書かないが、specs/やgit操作へのアクセスが必要なため。
- **1 Issue = 1 Coordinator**で固定。各Issueに専用のCoordinatorを起動する。
- 複数のCoordinatorは**並列に起動可能**（リソースが許す限り）。
- CoordinatorはClaude Code Agent Team等のチームセッションとして動作可能。
- Coordinatorの起動コマンドと引数はgwtが生成する。
- CoordinatorにはGitHub Issue番号と推奨ペルソナ情報を渡し、Coordinator自身がissue_specツール（スキル）でIssueを読んでタスクを把握する。
- Coordinatorはタスク内容を分析し、必要なスキルタグを判定して最適なペルソナを選定する。Leadからの推奨ペルソナがある場合はそれを優先する。

### CI監視と自律修正ループ

- **CoordinatorがPR作成後のGitHub Actionsの結果を監視する**（Leadではなく、Coordinatorの責務）。
- CI失敗時の自律修正フロー:
  1. CoordinatorがCI失敗を検出（`gh pr checks`等で確認）
  2. CoordinatorがWorkerに修正タスクを指示
  3. Workerが修正を実行する
  4. Worker修正完了 → コミット・プッシュ
  5. CI再実行 → Coordinatorが結果を監視
  6. 成功するまで繰り返し（最大3回まで）
- 3回連続CI失敗の場合はCoordinatorがLeadに報告し、Leadがユーザーに通知して人間判断を仰ぐ。
- LeadはCoordinatorから報告されるCI結果を全体進捗として把握する（直接監視はしない）。
- リリース判断・トリガーはLeadの責務外（人間が行う）。

### AI設定未構成時の扱い

- AI設定が有効とみなされる条件は「endpointとmodelが設定済みで、AIClientの初期化に成功すること」。
- AI設定が無効の場合でもプロジェクトモード画面は表示するが、送信入力は無効化する。
- 画面内に英語のエラーメッセージ（例: "AI settings are required"）と、既存のAI設定ウィザードへ遷移する導線を表示する。

### GitHub Issueベースの仕様管理

- gwtは**GitHub Issue**を仕様・計画・タスクの一元管理先として使用する。ローカルファイルベースのSpec Kit（`specs/SPEC-xxx/`）は使用しない。
- 仕様管理の基盤は既存の**issue_specツール群**（`upsert_spec_issue`, `get_spec_issue`, `append_spec_contract_comment`, `upsert_spec_issue_artifact`, `list_spec_issue_artifacts`, `delete_spec_issue_artifact`, `sync_spec_issue_project`）を使用する。
- これらのツールはSkillインターフェース経由で統一提供され、MCP Bridgeは使用しない。
- **issue_specツールはCodex/Gemini向けローカルSkill + Claude Code向けgwtプラグインとして公開**し、ブランチモードの各エージェントからも利用可能にする。
- GitHub Issueに記録するセクション:
  - **spec**: 仕様（ユーザーストーリー・受け入れ条件・機能要件）
  - **plan**: 実装計画（フェーズ構成・技術設計）
  - **tasks**: タスクリスト（依存関係・並列性）
  - **tdd**: テスト戦略とテストケース
- **モード横断**で利用可能:
  - プロジェクトモード: Leadが自動的にGitHub Issueを作成・更新
  - ブランチモード: 各エージェントがissue_specスキルでGitHub Issueを参照・更新

### GitHub Issue仕様管理ワークフロー（プロジェクトモード）

- **Leadが仕様管理ワークフロー全体を実行する**（Coordinatorは完成済みGitHub Issue番号を受け取る）:
  1. **要件収集フェーズ**: ユーザーの入力（プロジェクト概要 or 機能要求）+ リポジトリディープスキャンをもとに、Leadがユーザーに質問して要件を明確化する（clarify）
  2. **GitHub Issue作成**: Leadが要件をIssue単位に分割し、各IssueをGitHub Issueとして作成する
  3. **仕様記録**: Leadが各GitHub Issueに仕様セクション（spec）を記録する（`upsert_spec_issue_artifact`）
  4. **計画記録**: 仕様に基づいてLeadが計画セクション（plan）をGitHub Issueに記録する
  5. **タスク記録**: Leadがタスクセクション（tasks）をGitHub Issueに記録する
  6. **TDD記録**: Leadがテスト戦略（tdd）をGitHub Issueに記録する
  7. **一括承認**: Leadが仕様概要 + 計画 + タスク一覧 + TDD要約をチャットでユーザーに提示し、一括承認を得る
  8. **Coordinator起動**: 承認後、LeadがCoordinatorを起動し、GitHub Issue番号を渡す
  9. **実行**: CoordinatorがGitHub Issueからタスクを読み取り、各Workerに割り当てて自律実行する
- LeadはGitHub Issueに仕様4セクション（spec/plan/tasks/tdd）が揃うまでCoordinatorを起動してはならない。

### タスク分割の入出力仕様

- タスク生成はLeadがGitHub Issueのtasksセクションに記録する。
- CoordinatorはGitHub Issueから読み取ったタスク定義の各タスクにWorkerを割り当てる。
- **1 Task = N Worker = N Worktree**: 1つのタスクを複数Workerで並列実装できる。
- 割り当て時の判断:
  - 大きなタスクはさらに分割し、複数Worker+Worktreeで並列実行する
  - 独立したタスク間は別々のWorktree+Workerに割り当てる
  - 依存関係のあるタスクは同一Worktreeで順次実行、またはGit merge経由で連携する
- worktree_strategyはCoordinatorがLLMで判断する（`new`または`shared`）。

### Worktree/ブランチ命名と作成ルール

- ブランチ名は必ず`agent/`プレフィックスを付ける。
- タスク名からブランチ名を生成する際のルール:
  - 英小文字化する
  - 空白/連続スペースは`-`に置換する
  - `/`と`\`は`-`に置換する
  - 記号は除去し、英数字・ハイフン・アンダースコアのみ許可する
  - 長さは64文字以内とする
- 既に同名ブランチまたは同名worktreeパスが存在する場合は`-2`,`-3`の連番を付与する。
- worktree作成パスは`{repo_root}/.worktrees/{sanitized_branch_name}`を使用する。
- ブランチの起点は「プロジェクトモード開始時点の現在ブランチ」とする。

### Worker起動プロンプト規約

- プロンプトには必ず完了指示を含める（Claude Code: 「完了したらqで終了」、他: 「GWT_TASK_DONEを出力」）。
- プロンプトの豊かさはCoordinatorがLLMで**アダプティブに判断**する。
- すべてのWorkerプロンプトには、CLAUDE.mdから抽出したコーディング規約を含める。
- すべてのWorkerプロンプトには、ペルソナのsystem_additionを含める。

### Worker ペルソナ/プリセットシステム

#### ペルソナの定義

```toml
# ~/.gwt/personas/frontend-specialist.toml

[persona]
id = "frontend-specialist"
name = "Frontend Specialist"
role_label = "Frontend Dev"  # UI表示名
description = "Expert in modern web frontend development"
tags = ["frontend", "svelte", "react", "css", "typescript"]
agent_type = "claude"  # claude / codex / gemini

[prompt]
system_addition = """
You are a frontend specialist. Focus on:
- Component architecture and reusability
- Accessibility (WCAG 2.1 AA)
- Performance optimization
- Responsive design
"""

[options]
auto_mode_flag = "--dangerously-skip-permissions"
additional_args = []
```

#### ペルソナ例

| id | name | role_label | tags |
|---|---|---|---|
| `frontend-specialist` | Frontend Specialist | Frontend Dev | frontend, svelte, react, css |
| `backend-engineer` | Backend Engineer | Backend Dev | backend, rust, api, database |
| `unity-engineer` | Unity Engineer | Unity Dev | unity, csharp, 3d, game |
| `ue-engineer` | UE Engineer | UE Dev | unreal, cpp, blueprint, game |
| `graphic-artist` | Graphic Artist | Artist | graphics, ui-design, svg, assets |
| `researcher` | Technical Researcher | Researcher | research, analysis, docs |
| `fullstack-dev` | Fullstack Developer | Fullstack Dev | frontend, backend, devops |
| `devops-engineer` | DevOps Engineer | DevOps | ci, cd, docker, infrastructure |

#### ストレージ設計

- グローバルプリセット: `~/.gwt/personas/*.toml`
- プロジェクト上書き: `<repo>/.gwt/personas/*.toml` (optional)
- 組み込みデフォルト: gwt バイナリに同梱（ペルソナ例テーブルの全8種）
- 優先順位: プロジェクト > グローバル > 組み込み（同じidの場合）

#### Coordinator の Worker 選定ロジック

1. タスク内容を分析し、必要なスキルセット（tags）を判定
2. 利用可能なペルソナの中からtagsマッチ度でスコアリング
3. 最適なペルソナを選定してWorkerを起動
4. 同じペルソナの複数インスタンスも可（並列実行）
5. Leadが「推奨ペルソナ」を指定することも可能

#### エンティティ関係の更新

```text
変更前: Developer : AgentType (Claude/Codex/Gemini)
変更後: Worker : Persona (Frontend Specialist, Backend Engineer, etc.)
         Persona : AgentType (Claude/Codex/Gemini)
```

### システムプロンプト設計

現在の `PROJECT_MODE_SYSTEM_PROMPT`（40行の静的文字列）を動的に拡張:

```text
[静的部分]
- 役割定義: プロジェクト管理者としての振る舞い
- 対話モード指示: 質問に答える際はツールで調査
- 計画モード指示: 収集→整理→仕様化→委譲のフェーズ
- ReAct形式の指示

[動的部分（セッション開始時に生成）]
- リポジトリ情報（ブランチ、構造、CLAUDE.md抜粋）
- 利用可能なWorkerペルソナ一覧
- 既存GitHub Issue/進捗状況（あれば）
```

### Worker完了検出の条件

- Claude CodeはHook Stopを最優先で使用し、失敗時はGUI内蔵ターミナルの複合方式へフォールバックする。
- 複合方式の判定条件は以下のいずれか:
  - プロセス終了（ペイン終了またはPID終了）
  - 出力パターン検出（`GWT_TASK_DONE`）
  - PTY通信による完了確認（CoordinatorがWorkerに状態確認クエリを送信）
- アイドルタイムアウトは廃止する（入力待ちとの区別不可のため）。
- Workerは全自動モードで起動し、入力待ちを最小化する:
  - Claude Code: `--dangerously-skip-permissions`フラグで起動
  - Codex: `--full-auto`フラグで起動
  - Gemini: 利用可能な自動承認フラグで起動
- **Workerのエージェント種別はプロジェクトモード起動時にユーザーが指定する**。指定されたエージェント種別が全Workerに適用される。

### Worker並列実行制御

- 同時実行数の上限はCoordinatorがLLMで判断する。
- 判断基準: タスクの独立性、依存関係、リポジトリの規模、タスクの複雑さ。
- 並列度の変更は実行中にも動的に可能。

### 途中経過報告

- Leadは定期的（2分間隔を目安）にCoordinatorから状態情報を取得し、チャットに進捗を報告する。
- 報告内容: 各タスクの現在の状態（実行中/完了/失敗）、CI結果概要、実行時間、直近の要約。
- 定期報告はLLMコールを伴わない軽量な処理とし、ペイン状態とスクロールバック取得のみで構成する。
- 進捗報告のフォーマットは英語で統一し、1メッセージあたり最大10行とする。

### Lead LLM障害時の挙動

- LeadのLLM APIがダウン（レートリミット、サービス障害等）した場合:
  - 実行中のCoordinator/Workerはそのまま続行する（独立プロセスのため）。
  - ユーザーにチャットでAPI障害を通知する。
- LeadはエクスポネンシャルバックオフでAPIリトライする。
- API復旧後、全層の現在状態を再取得してオーケストレーションを再開する。

### Coordinator障害時の挙動

- Coordinatorがクラッシュした場合:
  - 配下のWorkerは**現在のタスクを独立して続行**する。
  - 新規タスクの割り当てや完了後の次ステップは一時停止する。
  - LeadがCoordinatorの障害を検出し、**自律的にCoordinatorを再起動**する（人間承認不要）。
  - 再起動後、Coordinatorは実行中のWorkerの状態を再取得して管理を再開する。

### 承認フローとドライランモード

- Leadはタスク分割後、計画全体をユーザーに提示する。
- ユーザーが承認（Enterまたは"y"）すると、以降のCoordinator起動・WT作成・Worker起動・テスト検証・PR作成はすべて自律実行される。
- **ドライランモード**: ユーザーが「計画だけ見せて」等の指示をした場合、Leadは仕様策定・計画・タスク生成までを実行し、実行には進まない。
- 承認提示はGitHub Issueの仕様（spec）/計画（plan）/タスク（tasks）をこの順で表示する。
- 承認UIのメッセージは英語で統一する。

### プロジェクトの継続と追加要件

- 実行中のプロジェクトに対して追加の要件や変更がユーザーから伝えられた場合、Leadは以下を判断する:
  - 既存Issueの拡張: 既存のGitHub Issueの仕様を更新し、該当Coordinatorに追加タスクを指示する
  - 新規Issue追加: 新しいGitHub Issue + Coordinatorとしてプロジェクトに追加する
- プロジェクト完了後に別プロジェクトの依頼があった場合は、新しいプロジェクトとして開始する。

### Leadの責務範囲

- Leadは「プロジェクト統括」を担い、要件定義・GitHub Issue仕様管理・全体進捗管理・段階的委譲を行う。
- 技術的な選択肢が存在する場合、Leadは**デフォルト推奨付きでユーザーに質問して確認を取る**。
- ユーザーの回答はCoordinator経由でWorkerのプロンプトに伝達する。

### プロジェクトモードのスコープ

- プロジェクトモードは**プロジェクト全体**を1つのLeadが管理する。
- プロジェクト内の複数の要件（Issue）は、それぞれ独立したCoordinatorで**並列実行**する。
- Leadがプロジェクト全体の要件定義を行い、要件をIssue単位に分割し、各IssueにGitHub Issue/Coordinatorを割り当てる。
- Issue間の依存関係がある場合、Leadが実行順序を制御する。

### セッション永続化と再開

- セッションは`~/.gwt/sessions/`に保存する。
- 保存フォーマットはJSONで、全層の状態（Lead会話・Coordinator状態・Worker状態・タスク一覧・進捗）を含む。
- 保存トリガー: 会話メッセージ追加、タスク状態変更、worktree作成/削除、Worker/Coordinator状態変更。
- GUI再起動時は最新の未完了セッションを自動で復元する。
- 復元時に参照worktreeが消失している場合は該当タスクを`Failed`にする。

### プロジェクトモードUI

#### GUI全体構成

- プロジェクトモードは、**ダッシュボード（左）+ Leadチャット（右）**の2カラム構成を基本とする。
- Worker表示は既存のブランチ/Worktreeタブ（Branch Mode）をそのまま利用する。
- タブ名: `Project Mode`

#### レイアウト

```text
+-----------------------------------------------------------+
|  [Project Mode]  [Branch Mode]  [...]                     |
+--------------+--------------------------------------------+
| Dashboard    | Lead Chat                                  |
|              |                                            |
| Issue #10    | Lead: "I've analyzed the requirements..."  |
|  [Running]   | You: "Proceed with the plan"               |
|  T1: 2 wkrs  | Lead: "Plan approved. Starting 3           |
|  T2: 1 wkr   |        Coordinators..."                    |
|              |                                            |
| Issue #11    | Lead: "Progress update:                    |
|  [CI Fail]   |   #10 Login: 2/3 tasks done               |
|  T3: retry   |   #11 DB: CI failing, retry 1/3           |
|  1/3         |   #12 Auth: pending"                       |
|              |                                            |
| Issue #12    |                                            |
|  [Pending]   | [Input area                           Send]|
+--------------+--------------------------------------------+
```

#### ダッシュボード（左カラム）

- プロジェクト内の全Issue/Task/Coordinatorの状態を**階層表示**する。
- 表示階層: Issue → Task → Worker（折りたたみ可）
- 各Issueに表示する情報:
  - Issue番号 + タイトル
  - ステータスバッジ（Pending / Running / CI Fail / Completed / Failed）
  - 配下タスク数と完了数（例: `2/3 tasks`）
- 各Taskに表示する情報:
  - タスク名
  - 割り当てWorker数
  - ステータス（pending/running/completed/failed）
- Issueをクリックすると展開し、配下のTask/Worker詳細を表示する。
- Taskをクリックすると、Branch Mode側で当該Worker/Worktreeの詳細にジャンプする。
- ダッシュボードは**常時表示**で、プロジェクト全体の俯瞰を常に提供する。

#### Leadチャット（右カラム）

- チャット画面の下部に入力エリア、上部にチャット履歴を表示する。
- チャット履歴は会話形式（バブル）で表示し、ユーザー発言は右寄せ、Leadは左寄せとする。
- Leadは定期的に進捗サマリーをチャット内にインライン表示する。
- 入力はEnter送信、Shift+Enter改行とし、IME変換中のEnterでは送信しない。
- 送信中は送信ボタンにスピナーを表示し、連打を防止する。
- 新規メッセージ追加時はチャット履歴が自動的に最下部へスクロールする。
- Markdownレンダリング（既存 `MarkdownRenderer.svelte` 再利用）
- Thought/Action/Observationの折りたたみ表示（`<details>`）
- 承認フロー用のApprove/Request Changesボタン
- タイピングインジケーター（Lead思考中）
- Tauriイベント経由のインクリメンタル更新
- Lead状態のフェーズ表示（Collecting / Organizing / Waiting for approval 等）

#### ペルソナ設定画面（Settings > Worker Personas）

- ペルソナ一覧（カード形式: 名前、role_label、タグバッジ、エージェントタイプ）
- 追加/編集モーダル:
  - 名前、role_label、説明
  - タグ（フリー入力 + 候補サジェスト）
  - エージェントタイプ選択（Claude/Codex/Gemini）
  - システムプロンプト追加テキスト（テキストエリア）
  - 追加CLI引数
- 削除確認ダイアログ
- 組み込みデフォルトからの複製ボタン
- スコープ表示（Global / Project）

#### Worker表示（Branch Mode連携）

- Workerは既存のBranch Modeタブで**ブランチ/Worktree単位**で表示する。
- `agent/`プレフィックスのブランチがProject Modeの成果物として表示される。
- Branch Modeの既存機能（ターミナルペイン、ファイルツリー等）をそのまま利用できる。
- ダッシュボードのTaskクリック → Branch Modeの該当Worktreeに自動遷移する。

#### Coordinator詳細（ダッシュボード内展開）

- ダッシュボードでIssueを展開すると、Coordinator詳細情報が表示される。
- Coordinator名、ステータス、CI結果、配下Worker一覧。
- `[View Terminal]`リンク: CoordinatorのターミナルペインをBranch Mode側で表示。
- Coordinatorとの直接チャット入力も可能（展開領域内）。

#### コスト可視化

- LeadのLLM APIコール数と推定トークン数をGUI上に表示する。
- Coordinator/WorkerのコストはCoordinator/Worker各自のセッション内で管理される。

### ブランチモードとの連携

- プロジェクトモードが作成した`agent/`ブランチは、ブランチモードのリストに通常ブランチと同じく**完全表示**される。
- ユーザーはブランチモードで`agent/`ブランチを自由に操作（削除、マージ等）できる。
- ブランチモードで`agent/`ブランチが削除された場合、該当タスクのworktree参照が欠落した状態として検出し、Failed/Pausedとする。

### セッション強制中断

- ユーザーは**Escキー**で実行中のセッションを即時中断できる。
- 中断時の処理:
  1. 全Worker + CoordinatorのターミナルペインにSIGTERMを送信する
  2. 停止を確認する（タイムアウト5秒）
  3. セッション状態を「Paused」として永続化する
  4. チャットに中断完了を表示する

### ログ記録

- LeadのLLM全コール（プロンプト + レスポンス）を既存のログシステムに記録する（カテゴリ: `agent.lead.llm`）。
- Coordinator/Workerの起動/完了/失敗イベントも記録する（カテゴリ: `agent.coordinator`, `agent.worker`）。
- ログはJSON Lines形式で`~/.gwt/logs/<cwd>/gwt.jsonl.YYYY-MM-DD`に保存する。

### 実行中の介入（ライブ介入）

- ユーザーが実行中にLeadチャットで新しい要件・変更を伝えた場合、Leadは影響範囲を判定する。
- 影響を受けるタスクのみをCoordinator経由で停止し、影響を受けないタスクは続行する。
- 停止したタスクは、新しい要件を反映した再計画後にリスタートする。

### 成果物検証（テスト実行）

- Worker完了検出後、Coordinatorは同一worktree内でテスト実行をWorkerに指示する。
- テストコマンドはリポジトリのビルドシステムに基づいて自動判定する。
- テストがパスした場合のみPR作成に進む。
- テストが失敗した場合、Coordinatorが修正を指示する（最大3回まで再試行）。
- 3回失敗した場合はタスクを`Failed`とし、Leadに報告 → ユーザーに通知する。

### Worker間コンテキスト共有（Git経由）

- 依存関係のあるタスクにおいて、先行タスクが完了しコミットされた場合、Coordinatorは後続タスクのブランチに先行タスクのコミットをmergeする。
- merge手順:
  1. 先行タスクのworktreeでコミット・プッシュを確認する
  2. 後続タスクのworktreeに移動し、`git merge agent/<先行タスクブランチ>` を実行する
  3. mergeコンフリクトが発生した場合はWorkerに解決を指示する

### PR作成と統合条件

- PR作成の前提条件:
  - worktree内がクリーンであること
  - baseブランチに対して差分が存在すること
  - `gh`が利用可能で認証済みであること
- PRタイトル・本文はCoordinatorがLLMでgit diffを読み取り、品質の高い内容を生成する。

### セッション完了とクリーンアップ

- セッションの「完了」は以下のすべてを満たした時点:
  1. 全タスクがCompleted状態
  2. 全PRがマージ済み（またはPR不要と判断）
  3. CIが全てパス
  4. `agent/`ブランチと対応するWorktreeが自動削除済み
- クリーンアップ手順:
  1. 各worktreeで未コミット・未プッシュの変更がないことを確認
  2. `git worktree remove`でWorktreeを削除
  3. `git branch -d`でローカルの`agent/`ブランチを削除
  4. リモートの`agent/`ブランチも削除（PRマージ後）
- 全Issue完了後、プロジェクト完了とする。

### コンテキスト要約（圧縮）条件

- 直近メッセージと未完了タスク情報は保持し、完了タスクと古い会話を要約対象とする。
- **層ごとに異なる管理方式**:
  - **Worker**: LLMの自動コンテキスト圧縮に任せる（Claude Code等の組み込み機能を利用）。gwt側では制御しない。
  - **Lead/Coordinator**: gwt側でコンテキスト管理を行う。推定トークン数が閾値（コンテキストウィンドウの80%）を超えた場合、完了タスク情報と古い会話を要約圧縮してから次のLLMコールを実行する。

## 要件 *(必須)*

### 機能要件

#### Lead（PM）関連

- **FR-001**: システムはGUIのタブバーでブランチモードとProject Modeを切り替えできなければならない
- **FR-002**: Leadはユーザーと自然言語で対話できなければならない
- **FR-002a**: Leadはタスク計画全体を提示し、ユーザーの一括承認を得てから自律実行を開始しなければならない
- **FR-002b**: Leadは技術的な選択肢が存在する場合、デフォルト推奨付きでユーザーに質問しなければならない
- **FR-002c**: 実行中にユーザーが新しい要件を伝えた場合、Leadは影響範囲を判定し影響タスクのみ停止しなければならない
- **FR-040**: Leadは段階的委譲に基づき、タスク順序/並列度/リトライ/Coordinator再起動を自律的に実行できなければならない
- **FR-041**: Leadは方針変更・タスク計画の大幅変更・新規機能追加時に人間承認を求めなければならない
- **FR-042**: Leadはgwt内蔵AIとして動作し、チャットUIで統一的なUXを提供しなければならない
- **FR-043**: Leadはハイブリッド方式（イベント駆動 + 定期ポーリング）で動作しなければならない
- **FR-044**: Leadは全体進捗（PRマージ状況・CI状況含む）をCoordinatorからの報告で把握し、ユーザーに定期報告しなければならない
- **FR-045**: Leadはプロジェクトのゴールを保持し、要件定義・GitHub Issueへの仕様記録（spec → plan → tasks → tdd）を実行しなければならない

#### Coordinator（Orchestrator）関連

- **FR-050**: CoordinatorはGUI内蔵ターミナルペインで起動しなければならない
- **FR-051**: CoordinatorはLeadから受け取ったGitHub Issue番号に基づき、Issueから仕様・タスクを読み取りWorkerを管理しなければならない
- **FR-052**: CoordinatorはCI結果を監視し、失敗時にWorkerへ修正指示→再プッシュ→CI再実行の自律修正ループを最大3回まで実行しなければならない
- **FR-053**: 1 Issue = 1 Coordinatorで固定し、複数Coordinatorはリソースが許す限り並列起動しなければならない
- **FR-054**: CoordinatorはWorker完了検出後にテスト実行とPR作成を行わなければならない
- **FR-055**: CoordinatorはClaude Code Agent Team等のチームセッションとして動作可能でなければならない

#### Worker関連

- **FR-060**: WorkerはGUI内蔵ターミナルペインで起動しなければならない
- **FR-061**: WorkerはCoordinatorからPTY直接通信でプロンプトを受信しなければならない
- **FR-062**: WorkerはClaude CodeのHook Stop / GWT_TASK_DONEパターン / プロセス終了で完了を通知しなければならない
- **FR-063**: Workerは全自動モードで起動し入力待ちを最小化しなければならない

#### 3層通信関連

- **FR-070**: 層間通信のPTY直接通信（send_keys系）はagent_tools.rsからCodex/Gemini向けローカルSkillとClaude Codeプラグインへ完全移行しなければならない
- **FR-071**: 各層は独立したLLMセッションとして動作し、上位層の障害が下位層に影響してはならない
- **FR-072**: Coordinatorクラッシュ時、Leadは自律的にCoordinatorを再起動しなければならない
- **FR-073**: LeadのLLM API障害時、Coordinator/Workerは独立して続行しなければならない
- **FR-074**: Claude Codeプラグイン（`gwt-integration`）のHook定義に、`UserPromptSubmit`/`PreToolUse`/`PostToolUse`/`Notification`/`Stop`の5イベントを`gwt-tauri hook <Event>`へ転送する設定を含めなければならない
- **FR-075**: gwt起動時に`repair_skill_registration`を実行し、Claude向けは`setup_gwt_plugin`によりマーケットプレイス登録と`enabledPlugins`有効化を自動修復しなければならない（ベストエフォート）
- **FR-076**: Hook設定の導入フローはプラグイン同梱方式を優先し、GUI起動時の手動Hook登録ダイアログ（`check_and_update_hooks`/`register_hooks`）に依存してはならない
- **FR-077**: 後方互換のため`gwt-tauri hook <Event>`のCLI処理は維持し、プラグイン転送と旧settings.json直接呼び出しの両方を受理しなければならない
- **FR-078**: Skill/Plugin登録設定に`default_scope`（`user`/`project`/`local`）を持たなければならない
- **FR-079**: Skill/Plugin登録設定にAgent別上書き（`codex_scope`/`claude_scope`/`gemini_scope`）を持てなければならない
- **FR-079a**: Codex/Geminiの登録先は、`user`=`~/.{codex,gemini}/skills`、`project`=`<repo>/.{codex,gemini}/skills`、`local`=`<repo>/.{codex,gemini}/skills.local`へ解決しなければならない
- **FR-079b**: Claude Codeの登録先は、`user`=`~/.claude/settings.json`、`project`=`<repo>/.claude/settings.json`、`local`=`<repo>/.claude/settings.local.json`へ解決しなければならない
- **FR-079c**: Skill Registrationの`repair`と`status`は、各Agentの有効スコープ（上書き優先）に対してのみ判定・修復を行わなければならない
- **FR-079d**: Settings UIで`default_scope`とAgent別上書きを編集・保存・復元できなければならない

#### Worktree/PR関連

- **FR-004**: システムは`agent/`プレフィックス付きブランチとworktreeを自動作成できなければならない
- **FR-008**: システムはPR経由で複数worktreeの成果物を統合できなければならない
- **FR-009**: システムはコンフリクト発生時にWorkerに解決を指示できなければならない
- **FR-009a**: 依存関係のある後続タスク起動前に、先行タスクのコミットをGit merge経由で統合しなければならない

#### セッション関連

- **FR-010**: システムは全層のセッション状態を`~/.gwt/sessions/`に永続化できなければならない
- **FR-011**: 永続化されたセッションを復元して再開できなければならない
- **FR-012**: コンテキストが大きくなった際に各層で独立して要約圧縮できなければならない
- **FR-014**: プロジェクト内の複数Issueは並列にCoordinatorを起動して実行しなければならない

#### GUI関連

- **FR-080**: プロジェクトモードUIはダッシュボード（左）+ Leadチャット（右）の2カラム構成でなければならない
- **FR-081**: ダッシュボードにIssue/Task/Workerの階層をステータス付きで常時表示しなければならない
- **FR-082**: ダッシュボードのTaskクリックでBranch Modeの該当Worktreeに自動遷移しなければならない
- **FR-083**: ダッシュボードのIssue展開でCoordinator詳細（ターミナル/チャット）にアクセスできなければならない
- **FR-084**: Worker表示は既存のBranch Modeタブをそのまま利用しなければならない
- **FR-085**: Workerのターミナルペイン（Branch Mode側）にユーザーが直接キー入力できなければならない
- **FR-015**: LeadのLLM APIコール数と推定トークン数をGUI上で可視化しなければならない
- **FR-026**: Leadチャット履歴は会話形式（バブル）で表示しなければならない
- **FR-027**: IME変換中のEnter入力では送信しないこと
- **FR-028**: リクエスト送信中は送信ボタンにスピナーを表示すること
- **FR-029**: 新規メッセージ追加時はチャット履歴が自動スクロールすること

#### ペルソナ関連

- **FR-100**: WorkerはPersonaを持ち、ペルソナのsystem_additionがWorker起動時のシステムプロンプトに追加されなければならない
- **FR-101**: ペルソナはグローバル（`~/.gwt/personas/`）、プロジェクト（`<repo>/.gwt/personas/`）、組み込みの3階層で管理されなければならない
- **FR-102**: 同じidのペルソナはプロジェクト > グローバル > 組み込みの優先順位で解決されなければならない
- **FR-103**: Coordinatorはタスクのtagsとペルソナのtagsのマッチ度でWorkerペルソナを選定しなければならない
- **FR-104**: Leadは推奨ペルソナをCoordinatorに指定できなければならない
- **FR-105**: ペルソナ設定画面でペルソナのCRUDが可能でなければならない

#### Lead拡張関連

- **FR-110**: Leadは`read_file`ツールでプロジェクトファイルを読み取り可能でなければならない
- **FR-111**: Leadは`list_directory`ツールでディレクトリ一覧を取得可能でなければならない
- **FR-112**: Leadは`search_code`ツールでコード検索が可能でなければならない
- **FR-113**: Leadは`list_personas`ツールで利用可能なペルソナ一覧を取得可能でなければならない
- **FR-114**: Leadはセッション開始時にリポジトリをスキャンし、プロジェクト構造を把握しなければならない
- **FR-115**: Leadは対話モード（ユーザーの質問にコードベースを調べて回答）と計画モード（収集→整理→仕様化→委譲）の2モードで動作しなければならない
- **FR-116**: LeadStatusは Idle / Collecting / Organizing / WaitingApproval / Specifying / Orchestrating / Thinking / Error を持たなければならない

#### チャットUI関連

- **FR-120**: LeadチャットはMarkdownレンダリングに対応しなければならない
- **FR-121**: Thought/Action/Observationは折りたたみ表示（`<details>`）でなければならない
- **FR-122**: 承認フロー時にApprove/Request Changesボタンを表示しなければならない
- **FR-123**: Lead思考中はタイピングインジケーターを表示しなければならない
- **FR-124**: Lead状態のフェーズ（Collecting / Organizing等）をチャットUI上に表示しなければならない

#### GitHub Issue仕様管理関連

- **FR-017**: gwtはGitHub Issueを仕様・計画・タスクの一元管理先として使用し、issue_specツール群で操作しなければならない
- **FR-018**: issue_specツール群はgwt内蔵（agent_tools.rs）と、Codex/Gemini向けローカルSkill + Claude Code向けgwtプラグインの両方で提供しなければならない
- **FR-019**: issue_specツールはCodex/Gemini向け`~/.codex/skills`/`~/.gemini/skills`とClaude Codeプラグインの双方で公開し、プロジェクトモード・ブランチモード両方から利用可能でなければならない
- **FR-035**: LeadはGitHub Issueに仕様4セクション（spec/plan/tasks/tdd）を記録しなければならない
- **FR-036**: LeadはGitHub Issueに4セクションが揃うまでCoordinatorを起動してはならない

#### コスト関連

- **FR-090**: コストはユーザーが完全制御する（gwt側のコスト上限・自動ダウングレードは設けない）
- **FR-091**: Lead/Coordinator/Workerのモデル選択はユーザーが行う

### テスト要件

- **TR-001**: プロジェクトモードUIのIME送信抑止、送信中スピナー、チャット表示は自動テストで検証する
- **TR-002**: タスク選択時にダッシュボード内でCoordinator/Worker情報が展開表示されることを自動テストで検証する
- **TR-003**: Coordinator起動前にLead側でGitHub Issueの仕様4セクション（spec/plan/tasks/tdd）の存在チェックが行われることを自動テストで検証する
- **TR-004**: 各層の独立動作（上位層障害時に下位層が続行）を自動テストで検証する
- **TR-005**: Leadの段階的委譲（自律範囲と承認要求）を自動テストで検証する
- **TR-006**: ClaudeプラグインのHook定義に5イベント転送（`gwt-tauri hook <Event>`）が含まれることを自動テストまたは静的検証で検証する
- **TR-007**: `repair_skill_registration`経由でClaudeプラグイン自動登録（`setup_gwt_plugin`）が起動時に実行されることを自動テストで検証する
- **TR-008**: GUI起動時に手動Hook登録ダイアログが表示されないこと（manual hook setup非依存）を回帰テストで検証する
- **TR-009**: `default_scope`とAgent別上書きの組み合わせで、Codex/Claude/Geminiの登録先解決が期待どおりになることを自動テストで検証する
- **TR-010**: Scope変更後の`repair`/`status`が旧スコープへ副作用を与えないことを自動テストで検証する
- **TR-011**: Settings UIでScope設定を変更・保存し、再起動後に復元されることを自動テストで検証する
- **TR-012**: ペルソナのCRUD操作（作成/読み取り/更新/削除）が正常に動作することを自動テストで検証する
- **TR-013**: ペルソナ優先順位（プロジェクト > グローバル > 組み込み）が正しく解決されることを自動テストで検証する
- **TR-014**: Coordinatorのペルソナ選定（tagsマッチ）が期待どおりに動作することを自動テストで検証する
- **TR-015**: Lead用ツール（read_file/list_directory/search_code/list_personas）がLLMツールとして正常に動作することを自動テストで検証する
- **TR-016**: LeadStatusの新しい状態遷移（Collecting/Organizing/Specifying等）が正しく動作することを自動テストで検証する
- **TR-017**: ペルソナ設定画面のCRUD操作とスコープ表示を自動テストで検証する

### 主要エンティティ

- **Project**: プロジェクト全体。1つのLeadが管理する。複数のIssueを含む
- **Issue**: GitHub Issueに対応する要件/機能単位。1 Issue = 1 GitHub Issue = 1 Coordinator
- **Task**: Issue内の個別タスク。状態（pending/running/completed/failed）、依存関係を持つ。1 Task = N Worker
- **Coordinator**: Orchestrator層のインスタンス。GUI内蔵ターミナルペイン、管理するWorker一覧、状態を持つ
- **Worker**: Worker層のインスタンス。ペルソナに基づく専門性を持つ。GUI内蔵ターミナルペイン、worktree、状態を持つ。1 Worker = 1 Worktree
- **Persona**: Workerの専門性を定義するプリセット。role_label、tags、system_additionを持つ
- **Conversation**: Leadとユーザーの対話履歴

## 成功基準 *(必須)*

### 測定可能な成果

- **SC-001**: ユーザーはGUIのタブバーで1秒以内にモード切り替えができる
- **SC-002**: Leadは5秒以内に初回応答を返す
- **SC-003**: Worker完了検出は実際の完了から10秒以内に行われる
- **SC-004**: セッション永続化は状態変更から1秒以内に完了する
- **SC-005**: gwtクラッシュ後もセッションの99%が復元可能である
- **SC-006**: IME変換中にEnterを押しても送信が発生しない
- **SC-007**: Coordinatorクラッシュ後、Leadは30秒以内に再起動を完了する
- **SC-008**: CI失敗検出から修正指示発行まで60秒以内に行われる

## 制約と仮定 *(該当する場合)*

### 制約

- GUI内蔵ターミナルが必須（初期化できない場合は使用不可）
- Claude Code以外のWorkerでは完了検出の精度が落ちる可能性がある
- LLM APIコストはユーザー責任で管理（制限機能なし）
- `agent/`プレフィックス以外のブランチは自動作成不可
- 3層構造により全体のLLMコストは増加する（Lead + Coordinator + Worker N体）

### 仮定

- ユーザーは有効なLLM API設定を持っている
- Worker（Claude Code等）が正常にインストールされている
- GUI内蔵ターミナルが利用可能な環境で実行される
- Coordinator用のエージェント（Claude Code Agent Team等）が利用可能

## 範囲外 *(必須)*

次の項目は、この機能の範囲外です：

- tmux以外の端末多重化ツール（screen等）のサポート
- LLM APIコスト管理・制限機能
- `agent/`以外のプレフィックスでのブランチ自動作成
- リリース判断・トリガーの自動化（CI監視のみ対象、リリースは人間が行う）
- リポジトリ知識の蓄積・学習（過去セッションからのパターン学習）

## セキュリティとプライバシーの考慮事項 *(該当する場合)*

- LLM APIキーは既存のAI要約機能と同じ安全な方法で管理される
- セッションファイルには全層の状態が含まれるため、適切なファイルパーミッション（0600/0700）が必要
- 3層間のPTY通信はローカルプロセス間のみで、ネットワーク越しの通信は発生しない

## 依存関係 *(該当する場合)*

- 既存のAI要約機能（SPEC-4b893dae）のAPI設定を共有
- 既存のGUIターミナルベースのエージェント起動機能
- 既存のworktree管理機能
- Claude Code Hook機能（Stop）
- issue_specツール群（`upsert_spec_issue`, `get_spec_issue`等）によるGitHub Issueベースの仕様管理
- Claude Code Agent Team機能（Coordinator実行基盤として）

## 参考資料 *(該当する場合)*

- [既存AI要約機能仕様](../SPEC-4b893dae/spec.md)
- [GUI内蔵ターミナル仕様](../SPEC-1d6dd9fc/spec.md)
