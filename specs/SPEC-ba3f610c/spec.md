# 機能仕様: プロジェクトチーム（Project Team）

**仕様ID**: `SPEC-ba3f610c`
**作成日**: 2026-01-22
**更新日**: 2026-02-19
**ステータス**: 更新済み
**カテゴリ**: GUI
**入力**: 3層エージェントアーキテクチャ（Lead / Coordinator / Developer）によるプロジェクト統括。Lead（PM相当）がプロジェクトのゴールを保持し、要件定義・全Spec Kitワークフロー・GitHub Issue/Project管理を担う。Coordinator（オーケストレーター）がDeveloper管理・CI監視・修正ループを行い、Developer（ワーカー）がWorktree内で実際の実装を行う。ユーザーはプロジェクト概要または具体的な機能要求のどちらでもLeadに伝えられる。

## アーキテクチャ概要

### 3層エージェント構造

| 層 | 内部名 | UI表示名 | 役割 | 実行場所 |
|---|---|---|---|---|
| PM | PM | Lead | プロジェクト統括・要件定義・Spec Kit実行・進捗管理・段階的委譲 | gwt内蔵AI |
| Orchestrator | Orchestrator | Coordinator | Issue単位のタスク管理・CI監視・修正ループ・Developer管理 | GUI内蔵ターミナルペイン |
| Worker | Worker | Developer | 実際のコード実装・テスト・PR作成 | GUI内蔵ターミナルペイン |

### 各層の責務

**Lead（PM）**:

- **プロジェクトのゴールを保持**し、全体像を理解する
- ユーザーからのプロジェクト概要または具体的な機能要求を受け取る
- **要件定義**: ユーザーに質問しながら要件を明確化する（clarify）
- **全Spec Kitワークフロー**: specify → plan → tasks → tdd を実行
- GitHub Issue作成・GitHub Project登録・ステータス管理
- ユーザーへの計画提示と承認取得
- Coordinatorの起動・停止・再起動の判断
- 全体進捗の把握（PRマージ状況・CI状況含む）とユーザーへの報告
- 段階的委譲に基づく自律的判断
- Worktreeは持たない（コードを書かないため）

**Coordinator（Orchestrator）**:

- Leadから受け取った仕様・タスクに基づくDeveloper管理
- Worktree作成とDeveloperへのタスク割り当て
- Developer間の依存関係管理（Git merge等）
- 成果物検証（テスト実行）とPR作成
- **CI結果の監視と修正ループ**（CI失敗 → Developer修正指示 → 再プッシュ → CI再実行）
- 失敗時のリトライ・代替アプローチ判断
- Claude Code Agent Team等のチームセッションとして動作可能
- Worktreeは持たない（コードを書かないため）

**Developer（Worker）**:

- **Worktree内**での個別タスクの実装（コーディング・テスト・コミット）
- Worktree単位で管理・表示される
- 完了報告（Hook / GWT_TASK_DONE / プロセス終了）

### エンティティ関係

| 関係 | カーディナリティ | 説明 |
|---|---|---|
| Project : Lead | 1 : 1 | 1プロジェクトに1つのLeadが常駐 |
| Project : Issue | 1 : N | Leadが要件定義からIssueを生成 |
| Issue : Coordinator | 1 : 1 | 各IssueにCoordinatorを1つ起動 |
| Issue : Spec | 1 : 1 | 各Issueに`specs/SPEC-xxx/`が対応 |
| Issue : Task | 1 : N | Spec内のtasks.mdからタスクを生成 |
| Task : Developer | 1 : N | 1タスクを複数Developerが並列実装可 |
| Developer : Worktree | 1 : 1 | 各Developerが専用Worktreeで作業 |

### プロセス分離

- Lead、Coordinator、Developerはそれぞれ**独立したLLMセッション**として動作する。
- 各層は独立したプロセスであり、上位層の障害は下位層に影響しない。
- 層間の通信はPTY直接通信（send_keys系）を基盤とし、スキルとして公開する。
- 複数のCoordinatorが**並列に起動可能**（リソースが許す限り）。

### エンドツーエンドフロー

1. ユーザーがプロジェクト概要**または**具体的な機能要求をLeadに伝える
2. Leadがプロジェクトのゴールを理解し、ユーザーに**質問**して要件を明確化する（clarify）
3. Leadが要件をIssue単位に分割し、各Issueについてspec.mdを生成する（specify）
4. Leadが各Issueについて実装計画を策定しplan.md / tasks.md / tdd.mdを生成する
5. LeadがGitHub Issueを作成し、GitHub Projectに登録する
6. Leadがユーザーに**プロジェクト全体の計画**を提示し承認を得る
7. 承認後、Leadが各Issueに対してCoordinatorを内蔵ターミナルペインで起動し、**Spec付きタスクのファイルパス**を渡す（複数Coordinator並列起動可）
8. 各CoordinatorがWorktreeを作成し、Developerを起動して実装を実行する（1 Task = N Developer = N Worktree可）
9. CoordinatorがDeveloper完了検出 → テスト検証 → PR作成を行う
10. CoordinatorがCI結果を監視し、失敗時はDeveloperに修正を指示する（自律修正ループ）
11. Leadが全Coordinatorの進捗を把握し、ユーザーに定期報告する
12. 各Issue完了（全タスク完了・CI全パス）後、LeadがGitHub Issueをクローズし、Projectステータスを更新する
13. 全Issue完了後、プロジェクト完了とする

### プロジェクト管理の統一先

- プロジェクトとして対応すべき全てのアイテムは**GitHub Issue**に統一する。
- Leadが作成したIssueは**GitHub Project**に登録し、フェーズ管理を行う。
- Issue ↔ Spec ↔ PR の紐付けにより、要件から成果物までのトレーサビリティを確保する。
- Project上のステータス遷移: draft → ready → planned → ready-for-dev → in-progress → done

### gwtの提供価値

gwtは以下を統合的に提供するプラットフォームとして機能する：

- 複数のAgent Teamセッション（Coordinator）の統括管理
- 各セッションへのWorktree自動割り当て
- 成果物のPR経由統合
- Spec Kit統合による仕様駆動開発
- CI監視と自律的修正ループ
- プロジェクト全体の可視化

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - モード切り替えと基本対話 (優先度: P1)

開発者がGUIのタブバーで`Project Team`タブを選ぶと、Leadとの対話画面が表示される。ユーザーは自由形式でタスクを入力し、Leadがタスクを分析・Coordinatorの起動計画を応答する。

**この優先度の理由**: プロジェクトチームへの入口であり、Leadとの対話がすべての機能の基盤となる。

**独立したテスト**: `Project Team`タブを開き、タスクを入力して応答を受け取ることで検証できる。

**受け入れシナリオ**:

1. **前提条件** ブランチモードが表示されている、**操作** `Project Team`タブを選択、**期待結果** プロジェクトチーム画面に切り替わり、Leadチャット画面が表示される
2. **前提条件** プロジェクトチーム画面が表示されている、**操作** `Settings`や`Version History`など別タブを選択、**期待結果** ブランチモードに戻る
3. **前提条件** AI設定が無効（エンドポイントまたはモデル未設定）、**操作** プロジェクトチームに切り替え、**期待結果** 設定促進メッセージが表示される
4. **前提条件** プロジェクトチーム画面が表示されている、**操作** 「認証機能を実装して」と入力、**期待結果** Leadがタスク分析とCoordinator起動計画を応答
5. **前提条件** プロジェクトチームでタスクが実行中、**操作** `Tab`でブランチモードに切り替え、**期待結果** タスクはバックグラウンドで継続、ブランチモードに切り替わる

---

### ユーザーストーリー 2 - Spec Kitワークフローと計画承認 (優先度: P1)

Leadがユーザーの入力を受けて要件定義を行い、全Spec Kitワークフロー（clarify → specify → plan → tasks → tdd）を実行する。完成した計画をユーザーに提示し、承認後にCoordinatorを起動してタスクを渡す。

**この優先度の理由**: Spec Kitワークフローによる仕様策定がプロジェクトチームの核心機能であり、Coordinator/Developer実行の前提条件となる。

**独立したテスト**: タスクを入力し、Leadがspec.md/plan.md/tasks.md/tdd.mdを生成し、承認後にCoordinatorが起動されることを確認する。

**承認方式**: 計画一括承認。Leadがタスク計画全体を提示し、ユーザーが1回承認すれば、以降のCoordinator起動・WT作成・Developer起動・テスト検証・PR作成はすべてLead/Coordinatorが自律実行する。ユーザーは対話画面でいつでも介入・中止が可能。

**受け入れシナリオ**:

1. **前提条件** ユーザーがプロジェクト概要または機能要求を入力、**操作** Leadが要件を明確化する質問を行う、**期待結果** ユーザーとの対話で要件が確定する
2. **前提条件** 要件が確定、**操作** Leadが全Spec Kitワークフローを実行、**期待結果** spec.md/plan.md/tasks.md/tdd.mdが生成される
3. **前提条件** Spec Kit成果物4点が揃う、**操作** Leadが計画全体をユーザーに提示、**期待結果** ユーザーが承認/拒否できる
4. **前提条件** ユーザーが計画を承認、**操作** LeadがCoordinatorを起動、**期待結果** Coordinatorが内蔵ターミナルペインで起動され、完成したSpec付きタスクを受け取る
5. **前提条件** ユーザーが計画を拒否、**操作** 拒否理由を入力、**期待結果** Leadが計画を再策定する

---

### ユーザーストーリー 3 - Developer起動と実装 (優先度: P1)

Coordinatorがworktreeを作成し、Developer（Claude Code等）をGUI内蔵ターミナルペインで起動する。Developerはタスクを実装し、完了を報告する。

**この優先度の理由**: Developerへの指示がタスク実行の実体であり、プロジェクトチームの価値を実現する核心機能。

**独立したテスト**: タスク実行を開始し、DeveloperがGUI内蔵ターミナルペインで起動してプロンプトを受け取ることを確認する。

**受け入れシナリオ**:

1. **前提条件** worktreeが作成済み、**操作** タスク実行開始、**期待結果** GUI内蔵ターミナルペインでDeveloper（Claude Code等）が起動
2. **前提条件** Developerが起動、**操作** Coordinatorがプロンプト送信、**期待結果** PTY直接通信でプロンプトが入力される
3. **前提条件** プロンプト送信、**操作** プロンプト内容を確認、**期待結果** タスク指示と完了指示が含まれる
4. **前提条件** 複数Developerを起動、**操作** 並列実行、**期待結果** 各Developerが別々のGUI内蔵ターミナルペインで動作

---

### ユーザーストーリー 4 - Developer完了検出 (優先度: P1)

CoordinatorがDeveloperの完了を検出する。Claude CodeはHookのStop経由、他のエージェントはGUI内蔵ターミナルの複合方式（プロセス終了監視 + スクロールバック出力パターン監視 + アクティビティ監視）で検出する。

**この優先度の理由**: 完了検出なしにはタスクの進行管理と次のステップへの移行ができない。

**独立したテスト**: Developerがタスクを完了し、Coordinatorが完了を検出して次のアクションに移行することを確認する。

**受け入れシナリオ**:

1. **前提条件** Claude Codeが動作中、**操作** タスク完了でqを押す、**期待結果** Hook経由で完了が検出される
2. **前提条件** Claude Code以外のDeveloperが動作中、**操作** プロセスが終了、**期待結果** ペイン終了検出で完了が検出される
3. **前提条件** スクロールバック出力パターン監視、**操作** 特定の完了パターンが出力、**期待結果** `capture_scrollback_tail`で完了が検出される
4. **前提条件** 複合方式での監視、**操作** いずれかの条件を満たす、**期待結果** 完了が検出され、Coordinatorに通知される

---

### ユーザーストーリー 5 - 成果物検証と統合（PR経由） (優先度: P2)

Developer完了後、Coordinatorがテスト実行による自動検証を行い、パスした場合にPRを作成する。CoordinatorがCI結果を監視し、失敗時はDeveloperに修正を指示する自律修正ループを実行する。

**この優先度の理由**: 成果物の品質検証と統合はフルサイクルに必須だが、基本的なタスク実行が動作してからの改善項目。

**独立したテスト**: 複数タスク完了後、テスト検証を経てPRが作成され、CIがパスすることを確認する。

**受け入れシナリオ**:

1. **前提条件** Developerがタスク完了、**操作** Coordinatorがテスト実行を指示、**期待結果** Developerが当該worktreeでテスト（`cargo test`等）を実行する
2. **前提条件** テストがパス、**操作** 成果物統合フェーズ開始、**期待結果** worktreeからPRが作成される
3. **前提条件** テストが失敗、**操作** Coordinatorが検出、**期待結果** Developerに修正を指示し再テスト（最大3回まで）
4. **前提条件** PRが作成された、**操作** CoordinatorがCI結果を監視、**期待結果** CI失敗時にDeveloperへ修正指示を自律的に発行し再プッシュ→CI再実行する（最大3回）
5. **前提条件** マージ時にコンフリクト発生、**操作** Coordinatorが検出、**期待結果** Developerにコンフリクト解決を指示

---

### ユーザーストーリー 6 - 障害ハンドリングと層間独立性 (優先度: P2)

各層が独立して動作し、上位層の障害が下位層に影響しない。Developerの失敗はCoordinatorが、Coordinatorの失敗はLeadが対処する。

**この優先度の理由**: 堅牢性に必要だが、基本フローが動作してからの改善項目。

**独立したテスト**: 各層の障害シナリオを再現し、他層が独立して動作し続けることを確認する。

**受け入れシナリオ**:

1. **前提条件** Developerがエラー終了、**操作** Coordinatorが検出、**期待結果** Coordinatorがリトライ/代替/相談を判断（Lead承認不要）
2. **前提条件** Coordinatorがクラッシュ、**操作** Leadが検出、**期待結果** Leadが自律的にCoordinatorを再起動（人間承認不要）。実行中のDeveloperは現タスクを続行
3. **前提条件** LeadのLLM API障害、**操作** API応答なし、**期待結果** Coordinator/Developerは独立して続行。Lead復旧後に状態を再取得
4. **前提条件** 複数回の失敗、**操作** Coordinatorが判断、**期待結果** ユーザーに相談または代替アプローチを提案

---

### ユーザーストーリー 7 - セッション永続化と再開 (優先度: P2)

プロジェクトチームのセッション状態（全層の状態、タスク一覧、進捗、会話履歴）を完全永続化する。gwtを再起動しても途中のセッションを再開できる。

**この優先度の理由**: 長時間タスクの中断・再開に必須だが、基本フローが動作してからの改善項目。

**受け入れシナリオ**:

1. **前提条件** タスク実行中、**操作** gwtを終了、**期待結果** 全層のセッション状態が`~/.gwt/sessions/`に保存される
2. **前提条件** セッションが保存されている、**操作** gwtを再起動、**期待結果** 前回のセッションを再開できる
3. **前提条件** セッション再開、**操作** 継続実行、**期待結果** 中断前の状態から継続できる

---

### ユーザーストーリー 8 - 直接アクセスと層間対話 (優先度: P2)

ユーザーはLeadとの対話を基本としつつ、Coordinator/Developerにも直接アクセスできる。Developerにはターミナル直接操作、Coordinator/Leadにはチャットで対話する。

**この優先度の理由**: 上級ユーザーの柔軟な操作に必要だが、Lead単一窓口で基本機能は動作する。

**受け入れシナリオ**:

1. **前提条件** Developerが動作中、**操作** Developerのターミナルペインを選択してキー入力、**期待結果** Developer端末に直接入力される
2. **前提条件** Coordinatorが動作中、**操作** Coordinatorのチャットに「タスクXを優先して」と入力、**期待結果** Coordinatorが指示を受けて対応する
3. **前提条件** Lead画面表示中、**操作** 下部パネルでCoordinator/Developer切替、**期待結果** 選択した層の詳細が表示される

---

### ユーザーストーリー 9 - コンテキスト管理（要約圧縮） (優先度: P3)

タスクが大規模になりLLMのコンテキストウィンドウを超える可能性がある場合、完了タスクの情報を要約圧縮してコンテキストを管理する。

**受け入れシナリオ**:

1. **前提条件** 対話が長くなりコンテキストが大きくなる、**操作** 継続して対話、**期待結果** 完了タスクの情報が要約圧縮される

---

### エッジケース

- Coordinator起動中にGUIウィンドウが閉じられた場合、どう復旧するか？
- 同一ファイルを複数Developerが同時に編集しようとした場合、コンフリクト検出のタイミングは？（→ 依存関係のあるタスクはGit merge時に検出、独立タスクはPR統合時に検出）
- LeadのLLM API呼び出しがタイムアウトした場合、Coordinator/Developerはどうなるか？（→ 独立続行）
- セッション復元時に参照していたworktreeが削除されていた場合、どう対処するか？
- テスト検証で3回連続失敗した場合のタスク状態遷移（→ Failed + ユーザー通知）
- Coordinatorがクラッシュした場合、配下のDeveloperはどうなるか？（→ 現タスク続行、新規指示なし）
- 大規模プロジェクトでCoordinatorが多数並列起動した場合のリソース管理は？

## 詳細仕様 *(必須)*

### 3層通信プロトコル

#### PTY直接通信（スキル化）

- 層間通信の基盤は既存のPTY直接通信（`send_keys_to_pane`, `send_keys_broadcast`, `capture_scrollback_tail`）を使用する。
- これらのツールは`agent_tools.rs`から**完全移行**し、Claude Codeプラグインのスキルとして一本化する。
- Lead/CoordinatorはスキルとしてPTY通信を呼び出す。

#### 通信方向と手段

| 方向 | 手段 | 即時性 |
|---|---|---|
| Lead → Coordinator | PTY直接通信（send_keys） | 即時 |
| Coordinator → Developer | PTY直接通信（send_keys） | 即時 |
| Developer → Coordinator | Hook Stop（Claude Code）/ 出力パターン / プロセス終了 | ハイブリッド |
| Coordinator → Lead | PTY出力 + scrollback読み取り / Tauriイベント | ハイブリッド |
| Lead → ユーザー | GUIチャット | 即時 |
| ユーザー → Lead/Coordinator/Developer | チャット / ターミナル直接操作 | 即時 |

#### 完了検出の階層

- **Developer → Coordinator**: Claude CodeはHook Stop、他はGWT_TASK_DONEパターン / プロセス終了
- **Coordinator → Lead**: Coordinator完了時はTauriイベント（`agent-status-changed`）+ scrollback確認

### Lead（PM）の段階的委譲

Leadは以下の範囲で人間承認なしに自律的に判断・実行できる：

**自律実行可能（承認不要）**:

- タスクの実行順序変更
- 並列度の調整
- 失敗タスクのリトライ指示
- Coordinatorの再起動
- Developerの差し替え
- CI失敗報告受信時のCoordinatorへの修正方針指示

**人間承認が必要**:

- 実装方針の変更
- タスク計画の大幅な変更
- 新規機能の追加
- リリース判断・トリガー

### Lead（PM）の常駐性

- **ハイブリッド方式**: 基本はイベント駆動、アクティブセッション中は定期ポーリングも併用。
- イベント駆動のトリガー:
  - Developer完了検出（Coordinator経由）
  - Coordinator完了/失敗検出
  - ユーザーからのチャット入力
  - CI結果の変更（GitHub Actionsステータス変更）
  - セッション開始（初回タスク入力）
- アクティブセッション中の定期ポーリング:
  - 2分間隔でCoordinator/Developerの状態をチェック
  - CI結果の定期確認
- イベント間はLead LLMコール不要（アイドル状態）。

### Lead実行基盤

- Leadは**gwt内蔵AI**として動作する（現在のMaster Agent相当を拡張）。
- gwt自身がLLMを呼び出し、Spec Kitワークフロー・対話ループ・Coordinator管理を実行する。
- チャットUIで統一的なUXを提供する。

### Coordinator起動と管理

- CoordinatorはGUI内蔵ターミナルペインで起動する（ユーザーが出力を見られる）。
- **1 Issue = 1 Coordinator**で固定。各Issueに専用のCoordinatorを起動する。
- 複数のCoordinatorは**並列に起動可能**（リソースが許す限り）。
- CoordinatorはClaude Code Agent Team等のチームセッションとして動作可能。
- Coordinatorの起動コマンドと引数はgwtが生成する。
- Coordinatorには`specs/SPEC-xxx/`のファイルパスを渡し、Coordinator自身がファイルを読んでタスクを把握する。

### CI監視と自律修正ループ

- **CoordinatorがPR作成後のGitHub Actionsの結果を監視する**（Leadではなく、Coordinatorの責務）。
- CI失敗時の自律修正フロー:
  1. CoordinatorがCI失敗を検出（`gh pr checks`等で確認）
  2. CoordinatorがDeveloperに修正タスクを指示
  3. Developerが修正を実行する
  4. Developer修正完了 → コミット・プッシュ
  5. CI再実行 → Coordinatorが結果を監視
  6. 成功するまで繰り返し（最大3回まで）
- 3回連続CI失敗の場合はCoordinatorがLeadに報告し、Leadがユーザーに通知して人間判断を仰ぐ。
- LeadはCoordinatorから報告されるCI結果を全体進捗として把握する（直接監視はしない）。
- リリース判断・トリガーはLeadの責務外（人間が行う）。

### AI設定未構成時の扱い

- AI設定が有効とみなされる条件は「endpointとmodelが設定済みで、AIClientの初期化に成功すること」。
- AI設定が無効の場合でもプロジェクトチーム画面は表示するが、送信入力は無効化する。
- 画面内に英語のエラーメッセージ（例: "AI settings are required"）と、既存のAI設定ウィザードへ遷移する導線を表示する。

### Spec Kit内蔵化

- gwt自体にSpec Kitの機能を**LLMプロンプトテンプレート**として組み込む。
- 組み込む機能: specify（仕様策定）、plan（計画策定）、tasks（タスク生成）、clarify（曖昧さ解消）、analyze（整合性分析）。
- 各機能はLLMプロンプトテンプレートとしてRustの`include_str!`マクロでバイナリにコンパイル時埋め込みする。
- **LeadがLLM経由で各テンプレートを実行する**（Coordinatorではなく、Leadの責務）。
- 成果物（spec.md, plan.md, tasks.md, tdd.md）は既存と同じ`specs/SPEC-XXXXXXXX/`ディレクトリに保存する。
- Spec Kit機能は**モード横断**で利用可能とする:
  - プロジェクトチーム: Leadが自動的にワークフローを実行
  - ブランチモード: ショートカットキーでSpec Kitワークフローを起動可能

### Spec Kit連携ワークフロー（プロジェクトチーム）

- **LeadがSpec Kitワークフロー全体を実行する**（Coordinatorは完成済みSpec付きタスクを受け取る）:
  1. **要件収集フェーズ**: ユーザーの入力（プロジェクト概要 or 機能要求）+ リポジトリディープスキャンをもとに、Leadがユーザーに質問して要件を明確化する（clarify相当）
  2. **仕様策定**: Leadがspec.mdを自動生成する（specify相当）
  3. **計画策定**: 仕様に基づいてLeadがplan.mdを自動生成する（plan相当）
  4. **タスク生成**: Leadがtasks.mdを自動生成する（tasks相当）
  5. **TDD策定**: tasks.mdを入力として、Leadがテスト戦略とテストケースを定義した`tdd.md`を自動生成する
  6. **一括承認**: Leadが仕様概要 + 計画 + タスク一覧 + TDD要約をチャットでユーザーに提示し、一括承認を得る
  7. **Coordinator起動**: 承認後、LeadがCoordinatorを起動し、完成したSpec付きタスクを渡す
  8. **実行**: Coordinatorが受け取ったタスクを各Developerに割り当てて自律実行する
- Leadは`spec.md`/`plan.md`/`tasks.md`/`tdd.md`が揃うまでCoordinatorを起動してはならない。

### タスク分割の入出力仕様

- タスク生成はSpec Kitの`/speckit.tasks`の出力形式に準拠する（Leadが生成）。
- Coordinatorは受け取ったtasks.mdの各タスクにDeveloperを割り当てる。
- **1 Task = N Developer = N Worktree**: 1つのタスクを複数Developerで並列実装できる。
- 割り当て時の判断:
  - 大きなタスクはさらに分割し、複数Developer+Worktreeで並列実行する
  - 独立したタスク間は別々のWorktree+Developerに割り当てる
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
- ブランチの起点は「プロジェクトチーム開始時点の現在ブランチ」とする。

### Developer起動プロンプト規約

- プロンプトには必ず完了指示を含める（Claude Code: 「完了したらqで終了」、他: 「GWT_TASK_DONEを出力」）。
- プロンプトの豊かさはCoordinatorがLLMで**アダプティブに判断**する。
- すべてのDeveloperプロンプトには、CLAUDE.mdから抽出したコーディング規約を含める。

### Developer完了検出の条件

- Claude CodeはHook Stopを最優先で使用し、失敗時はGUI内蔵ターミナルの複合方式へフォールバックする。
- 複合方式の判定条件は以下のいずれか:
  - プロセス終了（ペイン終了またはPID終了）
  - 出力パターン検出（`GWT_TASK_DONE`）
  - PTY通信による完了確認（CoordinatorがDeveloperに状態確認クエリを送信）
- アイドルタイムアウトは廃止する（入力待ちとの区別不可のため）。
- Developerは全自動モードで起動し、入力待ちを最小化する:
  - Claude Code: `--dangerously-skip-permissions`フラグで起動
  - Codex: `--full-auto`フラグで起動
  - Gemini: 利用可能な自動承認フラグで起動

### Developer並列実行制御

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
  - 実行中のCoordinator/Developerはそのまま続行する（独立プロセスのため）。
  - ユーザーにチャットでAPI障害を通知する。
- LeadはエクスポネンシャルバックオフでAPIリトライする。
- API復旧後、全層の現在状態を再取得してオーケストレーションを再開する。

### Coordinator障害時の挙動

- Coordinatorがクラッシュした場合:
  - 配下のDeveloperは**現在のタスクを独立して続行**する。
  - 新規タスクの割り当てや完了後の次ステップは一時停止する。
  - LeadがCoordinatorの障害を検出し、**自律的にCoordinatorを再起動**する（人間承認不要）。
  - 再起動後、Coordinatorは実行中のDeveloperの状態を再取得して管理を再開する。

### 承認フローとドライランモード

- Leadはタスク分割後、計画全体をユーザーに提示する。
- ユーザーが承認（Enterまたは"y"）すると、以降のCoordinator起動・WT作成・Developer起動・テスト検証・PR作成はすべて自律実行される。
- **ドライランモード**: ユーザーが「計画だけ見せて」等の指示をした場合、Leadは仕様策定・計画・タスク生成までを実行し、実行には進まない。
- 承認提示は`spec.md`/`plan.md`/`tasks.md`をこの順で表示する。
- 承認UIのメッセージは英語で統一する。

### プロジェクトの継続と追加要件

- 実行中のプロジェクトに対して追加の要件や変更がユーザーから伝えられた場合、Leadは以下を判断する:
  - 既存Issueの拡張: 既存のSpecを拡張し、該当Coordinatorに追加タスクを指示する
  - 新規Issue追加: 新しいIssue + Spec + Coordinatorとしてプロジェクトに追加する
- プロジェクト完了後に別プロジェクトの依頼があった場合は、新しいプロジェクトとして開始する。

### Leadの責務範囲

- Leadは「プロジェクト統括」を担い、要件定義・Spec Kitワークフロー実行・全体進捗管理・段階的委譲を行う。
- 技術的な選択肢が存在する場合、Leadは**デフォルト推奨付きでユーザーに質問して確認を取る**。
- ユーザーの回答はCoordinator経由でDeveloperのプロンプトに伝達する。

### プロジェクトチームのスコープ

- プロジェクトチームは**プロジェクト全体**を1つのLeadが管理する。
- プロジェクト内の複数の要件（Issue）は、それぞれ独立したCoordinatorで**並列実行**する。
- Leadがプロジェクト全体の要件定義を行い、要件をIssue単位に分割し、各IssueにSpec/Coordinatorを割り当てる。
- Issue間の依存関係がある場合、Leadが実行順序を制御する。

### セッション永続化と再開

- セッションは`~/.gwt/sessions/`に保存する。
- 保存フォーマットはJSONで、全層の状態（Lead会話・Coordinator状態・Developer状態・タスク一覧・進捗）を含む。
- 保存トリガー: 会話メッセージ追加、タスク状態変更、worktree作成/削除、Developer/Coordinator状態変更。
- GUI再起動時は最新の未完了セッションを自動で復元する。
- 復元時に参照worktreeが消失している場合は該当タスクを`Failed`にする。

### プロジェクトチームUI

#### GUI全体構成

- プロジェクトチームは、**Leadチャット（上部メイン）** + **下部切替パネル（Chat / Kanban / Coordinator）**で構成する。
- タブ名: `Project Team`

#### レイアウト

```text
+-----------------------------------------------------------+
|  [Project Team]  [Branch Mode]  [...]                     |
+-----------------------------------------------------------+
|              Lead Chat (Main conversation)                 |
|                                                           |
|  Lead: "I've analyzed the requirements..."                |
|  You: "Proceed with the plan"                             |
|  Lead: "Plan approved. Starting 3 Coordinators..."        |
|                                                           |
|  [Input area                                         Send]|
+-----------------------------------------------------------+
|  [Chat]  [Kanban]  [Coordinator]                          |
+-----------------------------------------------------------+
| Filter: [All Issues ▼]                                    |
| Pending    | Running     | Completed  | Failed            |
|            |             |            |                    |
| [ T003-1 ] | [ T001-1 ]  | [ T004-1 ] |                   |
| #12 Auth   | #10 Login   | #11 DB     |                   |
| api-auth   | login-ui    | db-schema  |                   |
| agent/t003 | agent/t001  | agent/t004 |                   |
|            |             |            |                    |
|            | [ T001-2 ]  |            |                    |
|            | #10 Login   |            |                    |
|            | oauth-flow  |            |                    |
|            | agent/t001b |            |                    |
+-----------------------------------------------------------+
```

#### パネル構成

メイン画面は上部の**Leadチャット**と、下部の**切替パネル**で構成する。下部パネルは3つのビューをタブで切り替える:

- **Chat**: Leadチャット（デフォルト。上部と統合表示）
- **Kanban**: Developer/タスクのKanbanボード
- **Coordinator**: Coordinator詳細パネル

#### Leadチャット

- チャット画面の下部に入力エリア、上部にチャット履歴を表示する。
- チャット履歴は会話形式（バブル）で表示し、ユーザー発言は右寄せ、Leadは左寄せとする。
- 入力はEnter送信、Shift+Enter改行とし、IME変換中のEnterでは送信しない。
- 送信中は送信ボタンにスピナーを表示し、連打を防止する。
- 新規メッセージ追加時はチャット履歴が自動的に最下部へスクロールする。

#### Kanbanボード（Developer/タスク表示）

- Developer（Worker）をブランチ/Worktree単位でタスクカードとして表示する。
- 4つのカラム: **Pending** / **Running** / **Completed** / **Failed**
- 上部にIssue（Coordinator）単位のフィルタ/グルーピングを提供する。
- 各カードに表示する情報:
  - 所属Issue名（Coordinator名）
  - タスクID + タスク名
  - 割り当てブランチ名（`agent/`プレフィックス付き）
  - worktree相対パス（ホバーで絶対パス表示）
- 1つのタスクに複数Developerが割り当てられている場合、それぞれ別カードとして表示する。
- カードをクリックすると、当該DeveloperのターミナルペインまたはCoordinatorの詳細に切り替わる。
- カードの表示順はカラム内で作成順（ID昇順）。

#### Coordinatorパネル

- 選択したCoordinatorの詳細状態を表示する。
- Coordinator名、ステータス、配下のDeveloper数を表示する。
- `[View Terminal]`ボタン: CoordinatorまたはDeveloperのターミナルペインを表示
- `[Chat]`ボタン: Coordinatorとの直接チャットを表示
- Developer一覧にはエージェント名・状態・割り当てworktreeパスを表示する。
- worktree表示はリポジトリ相対パスをデフォルトとし、ホバーで絶対パスを確認できる。

#### コスト可視化

- LeadのLLM APIコール数と推定トークン数をGUI上に表示する。
- Coordinator/DeveloperのコストはCoordinator/Developer各自のセッション内で管理される。

### ブランチモードとの連携

- プロジェクトチームが作成した`agent/`ブランチは、ブランチモードのリストに通常ブランチと同じく**完全表示**される。
- ユーザーはブランチモードで`agent/`ブランチを自由に操作（削除、マージ等）できる。
- ブランチモードで`agent/`ブランチが削除された場合、該当タスクのworktree参照が欠落した状態として検出し、Failed/Pausedとする。

### セッション強制中断

- ユーザーは**Escキー**で実行中のセッションを即時中断できる。
- 中断時の処理:
  1. 全Developer + CoordinatorのターミナルペインにSIGTERMを送信する
  2. 停止を確認する（タイムアウト5秒）
  3. セッション状態を「Paused」として永続化する
  4. チャットに中断完了を表示する

### ログ記録

- LeadのLLM全コール（プロンプト + レスポンス）を既存のログシステムに記録する（カテゴリ: `agent.lead.llm`）。
- Coordinator/Developerの起動/完了/失敗イベントも記録する（カテゴリ: `agent.coordinator`, `agent.developer`）。
- ログはJSON Lines形式で`~/.gwt/logs/<cwd>/gwt.jsonl.YYYY-MM-DD`に保存する。

### 実行中の介入（ライブ介入）

- ユーザーが実行中にLeadチャットで新しい要件・変更を伝えた場合、Leadは影響範囲を判定する。
- 影響を受けるタスクのみをCoordinator経由で停止し、影響を受けないタスクは続行する。
- 停止したタスクは、新しい要件を反映した再計画後にリスタートする。

### 成果物検証（テスト実行）

- Developer完了検出後、Coordinatorは同一worktree内でテスト実行をDeveloperに指示する。
- テストコマンドはリポジトリのビルドシステムに基づいて自動判定する。
- テストがパスした場合のみPR作成に進む。
- テストが失敗した場合、Coordinatorが修正を指示する（最大3回まで再試行）。
- 3回失敗した場合はタスクを`Failed`とし、Leadに報告 → ユーザーに通知する。

### Developer間コンテキスト共有（Git経由）

- 依存関係のあるタスクにおいて、先行タスクが完了しコミットされた場合、Coordinatorは後続タスクのブランチに先行タスクのコミットをmergeする。
- merge手順:
  1. 先行タスクのworktreeでコミット・プッシュを確認する
  2. 後続タスクのworktreeに移動し、`git merge agent/<先行タスクブランチ>` を実行する
  3. mergeコンフリクトが発生した場合はDeveloperに解決を指示する

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
- 各層（Lead/Coordinator）で独立してコンテキスト管理を行う。

## 要件 *(必須)*

### 機能要件

#### Lead（PM）関連

- **FR-001**: システムはGUIのタブバーでブランチモードとProject Teamを切り替えできなければならない
- **FR-002**: Leadはユーザーと自然言語で対話できなければならない
- **FR-002a**: Leadはタスク計画全体を提示し、ユーザーの一括承認を得てから自律実行を開始しなければならない
- **FR-002b**: Leadは技術的な選択肢が存在する場合、デフォルト推奨付きでユーザーに質問しなければならない
- **FR-002c**: 実行中にユーザーが新しい要件を伝えた場合、Leadは影響範囲を判定し影響タスクのみ停止しなければならない
- **FR-040**: Leadは段階的委譲に基づき、タスク順序/並列度/リトライ/Coordinator再起動を自律的に実行できなければならない
- **FR-041**: Leadは方針変更・タスク計画の大幅変更・新規機能追加時に人間承認を求めなければならない
- **FR-042**: Leadはgwt内蔵AIとして動作し、チャットUIで統一的なUXを提供しなければならない
- **FR-043**: Leadはハイブリッド方式（イベント駆動 + 定期ポーリング）で動作しなければならない
- **FR-044**: Leadは全体進捗（PRマージ状況・CI状況含む）をCoordinatorからの報告で把握し、ユーザーに定期報告しなければならない
- **FR-045**: Leadはプロジェクトのゴールを保持し、要件定義・全Spec Kitワークフロー（clarify → specify → plan → tasks → tdd）を実行しなければならない

#### Coordinator（Orchestrator）関連

- **FR-050**: CoordinatorはGUI内蔵ターミナルペインで起動しなければならない
- **FR-051**: CoordinatorはLeadから受け取った完成済みSpec付きタスクに基づきDeveloperを管理しなければならない
- **FR-052**: CoordinatorはCI結果を監視し、失敗時にDeveloperへ修正指示→再プッシュ→CI再実行の自律修正ループを最大3回まで実行しなければならない
- **FR-053**: 1 Issue = 1 Coordinatorで固定し、複数Coordinatorはリソースが許す限り並列起動しなければならない
- **FR-054**: CoordinatorはDeveloper完了検出後にテスト実行とPR作成を行わなければならない
- **FR-055**: CoordinatorはClaude Code Agent Team等のチームセッションとして動作可能でなければならない

#### Developer（Worker）関連

- **FR-060**: DeveloperはGUI内蔵ターミナルペインで起動しなければならない
- **FR-061**: DeveloperはCoordinatorからPTY直接通信でプロンプトを受信しなければならない
- **FR-062**: DeveloperはClaude CodeのHook Stop / GWT_TASK_DONEパターン / プロセス終了で完了を通知しなければならない
- **FR-063**: Developerは全自動モードで起動し入力待ちを最小化しなければならない

#### 3層通信関連

- **FR-070**: 層間通信のPTY直接通信（send_keys系）はagent_tools.rsからClaude Codeプラグインスキルに完全移行しなければならない
- **FR-071**: 各層は独立したLLMセッションとして動作し、上位層の障害が下位層に影響してはならない
- **FR-072**: Coordinatorクラッシュ時、Leadは自律的にCoordinatorを再起動しなければならない
- **FR-073**: LeadのLLM API障害時、Coordinator/Developerは独立して続行しなければならない

#### Worktree/PR関連

- **FR-004**: システムは`agent/`プレフィックス付きブランチとworktreeを自動作成できなければならない
- **FR-008**: システムはPR経由で複数worktreeの成果物を統合できなければならない
- **FR-009**: システムはコンフリクト発生時にDeveloperに解決を指示できなければならない
- **FR-009a**: 依存関係のある後続タスク起動前に、先行タスクのコミットをGit merge経由で統合しなければならない

#### セッション関連

- **FR-010**: システムは全層のセッション状態を`~/.gwt/sessions/`に永続化できなければならない
- **FR-011**: 永続化されたセッションを復元して再開できなければならない
- **FR-012**: コンテキストが大きくなった際に各層で独立して要約圧縮できなければならない
- **FR-014**: プロジェクト内の複数Issueは並列にCoordinatorを起動して実行しなければならない

#### GUI関連

- **FR-080**: プロジェクトチームUIは上部Leadチャット + 下部切替パネル（Chat/Kanban/Coordinator）のレイアウトでなければならない
- **FR-081**: KanbanボードでDeveloper/タスクをPending/Running/Completed/Failedの4カラムで表示しなければならない
- **FR-082**: Kanbanカードにタスク名・ブランチ名・worktree相対パスを表示しなければならない
- **FR-083**: CoordinatorパネルからView TerminalでCoordinator/Developerのターミナルペインに切り替えられなければならない
- **FR-084**: CoordinatorパネルからChatでCoordinatorとの直接チャットができなければならない
- **FR-085**: Developerのターミナルペインにユーザーが直接キー入力できなければならない
- **FR-015**: LeadのLLM APIコール数と推定トークン数をGUI上で可視化しなければならない
- **FR-026**: Leadチャット履歴は会話形式（バブル）で表示しなければならない
- **FR-027**: IME変換中のEnter入力では送信しないこと
- **FR-028**: リクエスト送信中は送信ボタンにスピナーを表示すること
- **FR-029**: 新規メッセージ追加時はチャット履歴が自動スクロールすること

#### Spec Kit関連

- **FR-017**: gwt自体にSpec Kit機能をLLMプロンプトテンプレートとして内蔵しなければならない
- **FR-018**: Spec Kit成果物は既存の`specs/SPEC-XXXXXXXX/`ディレクトリに保存しなければならない
- **FR-019**: Spec Kit機能はプロジェクトチームとブランチモードの両方から利用可能でなければならない
- **FR-035**: Leadは`spec.md`/`plan.md`/`tasks.md`/`tdd.md`を生成しなければならない
- **FR-036**: Leadは4点が揃うまでCoordinatorを起動してはならない

#### コスト関連

- **FR-090**: コストはユーザーが完全制御する（gwt側のコスト上限・自動ダウングレードは設けない）
- **FR-091**: Lead/Coordinator/Developerのモデル選択はユーザーが行う

### テスト要件

- **TR-001**: プロジェクトチームUIのIME送信抑止、送信中スピナー、チャット表示は自動テストで検証する
- **TR-002**: タスク選択時に対応するCoordinator/Developer情報が下部パネルに表示されることを自動テストで検証する
- **TR-003**: Coordinator起動前にLead側で成果物4点の存在チェックが行われることを自動テストで検証する
- **TR-004**: 各層の独立動作（上位層障害時に下位層が続行）を自動テストで検証する
- **TR-005**: Leadの段階的委譲（自律範囲と承認要求）を自動テストで検証する

### 主要エンティティ

- **Project**: プロジェクト全体。1つのLeadが管理する。複数のIssueを含む
- **Issue**: GitHub Issueに対応する要件/機能単位。1 Issue = 1 Spec = 1 Coordinator
- **Task**: Issue内の個別タスク。状態（pending/running/completed/failed）、依存関係を持つ。1 Task = N Developer
- **Coordinator**: Orchestrator層のインスタンス。GUI内蔵ターミナルペイン、管理するDeveloper一覧、状態を持つ
- **Developer**: Worker層のインスタンス。GUI内蔵ターミナルペイン、worktree、状態を持つ。1 Developer = 1 Worktree
- **Conversation**: Leadとユーザーの対話履歴

## 成功基準 *(必須)*

### 測定可能な成果

- **SC-001**: ユーザーはGUIのタブバーで1秒以内にモード切り替えができる
- **SC-002**: Leadは5秒以内に初回応答を返す
- **SC-003**: Developer完了検出は実際の完了から10秒以内に行われる
- **SC-004**: セッション永続化は状態変更から1秒以内に完了する
- **SC-005**: gwtクラッシュ後もセッションの99%が復元可能である
- **SC-006**: IME変換中にEnterを押しても送信が発生しない
- **SC-007**: Coordinatorクラッシュ後、Leadは30秒以内に再起動を完了する
- **SC-008**: CI失敗検出から修正指示発行まで60秒以内に行われる

## 制約と仮定 *(該当する場合)*

### 制約

- GUI内蔵ターミナルが必須（初期化できない場合は使用不可）
- Claude Code以外のDeveloperでは完了検出の精度が落ちる可能性がある
- LLM APIコストはユーザー責任で管理（制限機能なし）
- `agent/`プレフィックス以外のブランチは自動作成不可
- 3層構造により全体のLLMコストは増加する（Lead + Coordinator + Developer N体）

### 仮定

- ユーザーは有効なLLM API設定を持っている
- Developer（Claude Code等）が正常にインストールされている
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
- Spec Kit（`/speckit.specify`, `/speckit.plan`, `/speckit.tasks`）による仕様策定・計画・タスク生成
- Claude Code Agent Team機能（Coordinator実行基盤として）

## 参考資料 *(該当する場合)*

- [既存AI要約機能仕様](../SPEC-4b893dae/spec.md)
- [GUI内蔵ターミナル仕様](../SPEC-1d6dd9fc/spec.md)
