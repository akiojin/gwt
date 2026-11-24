# GUI と gwtd の operation パリティ台帳

[Issue #7518](https://github.com/akiojin/gwt/issues/7518) の台帳です。WebSocket の
[`FrontendEvent`](../crates/gwt/src/protocol.rs) に定義されたイベントごとに、
[operation カタログ](../crates/gwt/src/cli/operation_catalog.rs) の canonical 名と対応範囲を記録します。
イベント名は Rust のバリアント名です。

- **対応**は同じ操作結果への経路です。呼び出し元の認証や要求・応答の形式は各経路の契約に従います。
- **一部対応**は対象や作用の範囲が異なります。差分を各行に記録しています。
- **GUI 内部**はブラウザやネイティブ UI の表示、接続、確認フローの都合で必要です。
- **対象外**はこの Issue の優先実装に含めない変更です。境界や理由を各行に記録しています。
- **読み取り**は状態変更を要求しない操作です。

バリアントを追加・削除したときと、対応する operation の名前や作用を変更したときは台帳も更新します。
Rust CI の `gui_envelope_parity_ledger_tracks_frontend_events_and_catalog` がイベントの欠落・重複、
対応の理由の欠落、カタログに存在しない operation 名を検出します。
`catalog_covers_every_dispatched_operation` がカタログと dispatch の一致を確認します。

追加した操作は、ホストの CLI から直接、または対象プロジェクトの登録済み PM セッションから呼び出せます。

- `project.open` は `path` を受け取り、稼働中の gwt がプロジェクトを開いた後に `project_key` と `url_path` を返します。ブラウザは起動しません。
- `pane.layout.place` はカレントプロジェクトのペイン `id`、Kanban `board_id`、`lane_id`、任意の `order` を受け取ります。ID は GUI のワークスペース状態に含まれる `tab_id::window_id` 形式です。レーンには `plan`、`active`、`blocked`、`done` を指定できます。
- `pm.auto_start.set` は `enabled` と任意の `project_root` を受け取ります。保存結果は `pm.status` からも確認できます。設定は次回プロジェクトを開く際に適用されます。

| FrontendEvent | 区分 | canonical operation | 対応範囲・理由 |
| --- | --- | --- | --- |
| `RestartWedgedOwner` | 対象外 | — | 復旧画面の確認と復旧 ID に紐づけて、停止した project owner を再起動する。 |
| `RefreshOrphanDaemons` | 一部対応 | `fleet.orphans` | orphan 情報を取得できる。GUI は Daemon の復旧画面向けに観測を更新する。 |
| `ReapOrphanDaemon` | 一部対応 | `fleet.orphan.terminate` | orphan の停止に対応する。GUI の観測トークンと envelope のプロセス識別情報は別の契約を持つ。 |
| `FrontendReady` | GUI 内部 | — | ブラウザ接続ごとに初期スナップショットの配信を開始する。 |
| `ProjectClientAttention` | GUI 内部 | — | 可視性・フォーカス・通知許可を現在のブラウザ接続に記録する。 |
| `RetryWorkspaceStateLoad` | GUI 内部 | — | ワークスペース表示の初期ロードを再試行する。 |
| `ProjectAggregateAck` | GUI 内部 | — | 集約スナップショットの受領を接続ごとに伝える。 |
| `LoadRecoveryCenter` | GUI 内部 | — | 接続に紐づく復旧画面のスナップショットと、その行を操作するハンドルを更新する。 |
| `OpenRecoveryCenterBoardEntry` | GUI 内部 | — | 接続限定の行ハンドルを Board への GUI ナビゲーションに解決する。 |
| `SetClaudeAccountUsageEnabled` | 対象外 | — | Claude アカウントの使用量収集はホストの設定とプロバイダー認証に依存する。 |
| `RefreshUsage` | 対象外 | — | 現在のホストが持つ認証済みアカウントの使用量を再取得する。 |
| `AgentRoster` | 一部対応 | `agent.registry.status` | 登録状況の診断ができる。メンバーの追加、休止、改名、選出、設定変更は GUI の roster 編集契約に従う。 |
| `AgentSchedule` | 対応 | `issue.monitor.schedule.read`, `issue.monitor.schedule.reassign` | 同じプロジェクトの割当表を取得・変更する。 |
| `AgentSwitch` | 対応 | `issue.monitor.switch.read`, `issue.monitor.switch.request`, `issue.monitor.switch.cancel` | 安全なエージェント切替の状態取得・要求・取消を行う。 |
| `StartupAutoResumeReady` | GUI 内部 | — | GUI 起動タイミングに合わせ、保存済みの再開処理を進める。 |
| `StartupFirstFrame` | GUI 内部 | — | ブラウザの初回描画の計測点を記録する。 |
| `StartupTerminalReady` | GUI 内部 | — | ブラウザ上の端末が入力可能になったことを報告する。 |
| `OpenPmAgent` | 対象外 | — | PM ペインを生成または前面化するホストのランチャー操作。 |
| `SetPmAutoStart` | 対応 | `pm.auto_start.set` | 次回プロジェクトを開く際の PM 自動起動設定を保存する。 |
| `SetPmPaused` | 対応 | `pm.pause`, `pm.resume` | 常駐 PM の自律処理を一時停止または再開する。 |
| `SetPmLoopInterval` | 対象外 | — | PM の起動後の定期ループ間隔は GUI 設定から編集する。 |
| `SetPmLaunchProfile` | 対象外 | — | 次回 PM 起動のモデル等を選択中メンバーと設定 revision に紐づけて変更する。 |
| `RestartPmAgent` | 対象外 | — | 実行中 PM を停止し、設定済みプロファイルの新しい会話で再起動する。 |
| `OpenProjectDialog` | GUI 内部 | — | ネイティブのフォルダー選択ダイアログを開く。 |
| `SelectCloneProjectParent` | GUI 内部 | — | クローン先の親フォルダーをネイティブダイアログで選択する。 |
| `GithubRepositorySearch` | 読み取り | — | クローンダイアログ用に GitHub のリポジトリ候補を検索する。 |
| `CloneProjectStart` | 対象外 | — | 選択済みの親フォルダーへのクローンと、その進捗・重複チェックを GUI が管理する。 |
| `ReopenRecentProject` | 対応 | `project.open` | 指定パスのプロジェクトを稼働中のホストで開く。 |
| `PreviewCloseProject` | GUI 内部 | — | ブラウザ接続ごとに閉じる対象と確認トークンを発行する。 |
| `ConfirmCloseProject` | 一部対応 | `project.execution.choose` | 承認付きでプロジェクト終了時の実行方針を選ぶ。ビュー固有の閉じる処理は GUI の確認トークンとビュー revision に従う。 |
| `CancelCloseProject` | GUI 内部 | — | 現在の接続の確認トークンを破棄する。 |
| `ProjectViewReady` | GUI 内部 | — | ブラウザの project view が読み込み済みになったことを登録する。 |
| `GetProjectExecutionStatus` | 対応 | `project.execution.status` | プロジェクトの実行方針と操作の状態を取得する。 |
| `CreateWindow` | 対象外 | — | GUI のプリセットと表示先からペインを作成し、必要に応じて実行を開始する。 |
| `FocusWindow` | GUI 内部 | — | GUI ワークスペースで選択中のウィンドウを変更する。 |
| `CycleFocus` | GUI 内部 | — | GUI の並び順で次または前のウィンドウへフォーカスを移す。 |
| `ActivateWindowTab` | GUI 内部 | — | GUI ウィンドウの表示タブと最近の選択を更新する。 |
| `PlaceAgentWindowInKanban` | 対応 | `pane.layout.place` | 対象ペインを同じプロジェクトの Kanban ボードの指定レーンに配置する。 |
| `MoveAgentKanbanCard` | 対象外 | — | 配置済みカードの折りたたみ状態を保ったままレーンや並び順を変更する。 |
| `UndockAgentWindow` | 対象外 | — | Kanban や Issue 内の表示から独立した GUI ウィンドウへ戻す。 |
| `DockAgentWindowToIssue` | 対象外 | — | Issue ウィンドウの所属を確認し、その Issue の一覧行へペインを戻す。 |
| `SetAgentKanbanCardCollapsed` | GUI 内部 | — | Kanban カードの表示上の折りたたみ状態を更新する。 |
| `UpdateTerminalGrid` | GUI 内部 | — | ブラウザが測った端末の行・列数を PTY に反映する。 |
| `ListWindows` | 一部対応 | `pane.list` | ペインを列挙できる。GUI は project view のウィンドウ全体を返す。 |
| `ListFleetPanes` | 一部対応 | `fleet.inventory` | 認証済み agent transport から生存ペインを取得する。envelope は他の fleet 情報も併せて返す。 |
| `PaneName` | 対応 | `pane.name` | 認証済みのペインに割り当てられた名前を取得する。 |
| `CloseWindow` | 一部対応 | `pane.close` | ペインの停止と削除に対応する。envelope は認証された呼び出し元の対象に限定される。 |
| `CloseFleetPane` | 対応 | `fleet.disposition` | fleet の正確なペインと session を指定して閉じる。 |
| `RecoverRestoredWindow` | 対応 | `pane.recover` | 復元された対象を特定し、agent transport 上で復旧操作を行う。 |
| `StopWindow` | 対象外 | — | GUI にウィンドウと出力を残したまま実行だけ停止する。 |
| `StopAllWindows` | 対象外 | — | GUI の一括停止は複数のペインに作用し、停止後も表示を保持する。 |
| `RestartWindow` | 対象外 | — | 現在のウィンドウと出力履歴を保って保存済みペインを再開する。 |
| `TerminalInput` | 一部対応 | `pane.send` | 自分のペインに一行を送信できる。GUI の対話的な端末入力はキー入力や改行をそのまま扱う。 |
| `LoadPmConversation` | 読み取り | — | 正規 PM ペインに結び付く会話の履歴を表示用に読む。 |
| `LoadHomeConversation` | 読み取り | — | ホームの会話に出す、開いている各プロジェクトの判断キューを読む。 |
| `HomePmSend` | 対象外 | — | ホームからプロジェクトを指定して、そのプロジェクトの登録済み PM ペインへ文を送る。宛先のセッションはホストが解決する。 |
| `GetFirstRunSetup` | 読み取り | — | 初回セットアップに出す、登録済みのツール、台帳のエージェント、PM の指名、セットアップの要否を読む。`refresh` は導入済みツールを検出し直す。 |
| `SaveFirstRunSetup` | 対象外 | — | この端末の PM の指名（台帳のエージェント、または新しいエージェントの名前）を保存する。初回セットアップの画面からの操作に限る（Issue #7731）。 |
| `LoadPmReports` | 一部対応 | `pm.report.list` | 永続化された PM レポートを取得できる。GUI は現在の PM ペインと表示 revision に応じた差分も返す。 |
| `LoadPmRequestTask` | 対応 | `pm.request_tasks.read` | PM への依頼タスクとその詳細を取得する。 |
| `ReopenPmRequestTask` | 一部対応 | `pm.request_tasks.apply` | 再開するドメイン操作を共有する。GUI は操作者として、envelope は正規 PM として処理する。 |
| `CancelPmRequestTask` | 一部対応 | `pm.request_tasks.apply` | 取消のドメイン操作を共有する。GUI は操作者として、envelope は正規 PM として処理する。 |
| `UpdatePmRequestTask` | 一部対応 | `pm.request_tasks.apply` | 項目更新のドメイン操作を共有する。GUI は操作者として、envelope は正規 PM として処理する。 |
| `CompletePmRequestTask` | 一部対応 | `pm.request_tasks.apply` | 完了のドメイン操作を共有する。GUI は操作者として、envelope は正規 PM として処理する。 |
| `HomeAnswerDecision` | 対応 | `decision.answer` | ホームからプロジェクトを指定して、そのプロジェクトの判断キューの判断へ回答する。PM が動いていなくても記録され、gwtd が待機セッションへ配送する。 |
| `HomeDeferDecision` | 対応 | `decision.defer` | ホームから、判断を判断待ちのままキューの末尾へ回す。件数は変わらない。 |
| `AnswerDecision` | 対応 | `decision.answer` | 判断キューの判断へ回答する。GUI は操作者として、envelope は正規 PM または利用者の直接呼び出しとして記録し、どちらも gwtd が待機セッションへ配送する。 |
| `DeferDecision` | 対応 | `decision.defer` | 判断を判断待ちのままキューの末尾へ回す。件数は変わらない。 |
| `PaneSendInput` | 対応 | `pane.send` | agent transport で呼び出し元自身のペインに入力を送る。 |
| `PmPaneSendInput` | 対応 | `pm.message.send` | PM 権限を確認して同じプロジェクトの対象ペインへ送信する。 |
| `AgentIssueMonitorScanNow` | GUI 内部 | — | agent transport 専用の即時スキャン要求。envelope の再キュー等が内部の通知経路として使う場合がある。 |
| `PasteImage` | GUI 内部 | — | 旧クライアントによる直接ペーストを拒否し、アップロード経由へ誘導する。 |
| `PasteImageUploaded` | 対象外 | — | 接続に紐づくアップロードとペインの対象照合を必要とする。 |
| `AttachmentUploadFailed` | GUI 内部 | — | ブラウザの添付アップロード失敗を操作 ID に対応づける。 |
| `AttachFiles` | 対象外 | — | 接続で受け取った添付メタデータと対象ペインの一致を確認してから入力する。 |
| `LoadBranches` | 読み取り | — | プロジェクト表示用のブランチ一覧を取得する。 |
| `RequestRemoteStartWorkBranches` | 読み取り | — | Start Work の対象として選択可能なリモートブランチを取得する。 |
| `LoadBoard` | 対応 | `board.show` | 現在のプロジェクトの Board を取得する。 |
| `LoadBoardHistory` | 一部対応 | `board.show` | Board 履歴を読める。GUI は表示済みエントリーを起点としたページングを行う。 |
| `LoadProfile` | 読み取り | — | GUI が選択しているプロファイルの設定と環境変数ビューを取得する。 |
| `LoadLogs` | 一部対応 | `errors.list` | エラー台帳を読める。GUI のログ画面はスコープ選択と他のログ種別も扱う。 |
| `CheckKnowledgeRevision` | 読み取り | — | Knowledge の一覧を再読込すべきか判定する revision を読む。 |
| `LoadKnowledgeBridge` | 一部対応 | `issue.view`, `issue.spec.read` | Issue または SPEC の本文を取得できる。GUI は選択状態と一覧の再取得を管理する。 |
| `ReportKnowledgeError` | GUI 内部 | — | GUI の Knowledge 処理エラーを操作段階とともにエラー台帳へ記録する。 |
| `SearchKnowledgeBridge` | 一部対応 | `search` | 検索を実行できる。GUI は Knowledge の選択コンテキストを保持する。 |
| `SearchProjectIndex` | 対応 | `search` | プロジェクトインデックスを指定スコープで検索する。 |
| `RequestWorkAdvisory` | 読み取り | — | Start Work で重複作業を調べるための候補を取得する。 |
| `SelectKnowledgeBridgeEntry` | GUI 内部 | — | Knowledge ウィンドウ内の選択とプレビューを更新する。 |
| `EditIssueSubtask` | 一部対応 | `issue.edit` | 課題本文の子課題を追加・削除し、選択中の課題詳細を更新する。エンベロープでは本文を編集できる。 |
| `UpdateKnowledgeBridgePhase` | 一部対応 | `issue.label` | ラベルを操作できる。GUI は一覧の変更を既存フェーズラベルの置換として扱う。 |
| `SetIssueAutonomousTarget` | 対応 | `issue.label` | 課題を自律駆動の対象に含める／外す。対象の印である `auto-merge` ラベルを付け外しする。 |
| `RunBranchCleanup` | 一部対応 | `branch.prune_merged` | マージ済みブランチの削除に対応する。GUI は候補選択、作業ツリーの削除オプションと進捗も扱う。 |
| `RunWorkspaceCleanup` | 対象外 | — | ブランチに紐づく作業ツリーの除去を、利用中のセッションと削除オプションを確認して実行する。 |
| `SyncBranchCleanup` | GUI 内部 | — | 再接続したブラウザへ実行中のクリーンアップの履歴を再配信する。 |
| `ClearBranchCleanupStatus` | GUI 内部 | — | 確認済みクリーンアップの GUI 進捗履歴を破棄する。 |
| `RebuildIndexCell` | 対応 | `index.rebuild` | インデックスの対象スコープを指定して再構築する。 |
| `RefreshIndexStatus` | 対応 | `index.status` | 現在のインデックスの健全性と進捗を確認する。 |
| `PostBoardEntry` | 対応 | `board.post` | 認証された投稿者と指定した宛先の権限で Board に投稿する。 |
| `OpenBoardOriginAgent` | GUI 内部 | — | Board 投稿の送信元ペインを GUI で開く。 |
| `SelectProfile` | GUI 内部 | — | プロファイル編集ウィンドウ内で表示対象を変更する。 |
| `CreateProfile` | 対象外 | — | ホストに永続化される環境変数プロファイルを新規作成する。 |
| `SetActiveProfile` | 対象外 | — | 将来の起動で評価されるホストのアクティブプロファイルを切り替える。 |
| `SaveProfile` | 対象外 | — | ホストの環境変数プロファイルの名前、説明と設定を更新する。 |
| `DeleteProfile` | 対象外 | — | ホストの環境変数プロファイルを削除する。 |
| `OpenIssueLaunchWizard` | GUI 内部 | — | 指定 Issue を起点に起動ウィザードの入力状態を初期化する。 |
| `OpenStartWorkInAgentKanban` | GUI 内部 | — | Kanban のボードとレーンを起点に Start Work を開く。 |
| `OpenAgentKanbanLaunchWizard` | GUI 内部 | — | 起動ウィザードに作成先の Kanban レーンを設定する。 |
| `ResumeWorkspace` | 一部対応 | `execution.continue` | Work を指定して Execution を継続できる。GUI は表示上の再開元や再開履歴からも対象を選べる。 |
| `ListResumableAgents` | 読み取り | — | ワークスペースの再開候補と過去の会話を列挙する。 |
| `ResumeWorkspaceAgent` | 一部対応 | `execution.continue` | Work の Execution 継続に対応する。個別の Session と会話の指定は GUI の再開経路で扱う。 |
| `ContinueWork` | 対応 | `execution.continue` | Work ID で作業中の Execution を開始または復旧する。 |
| `ResumeBranchLatestAgent` | 対象外 | — | GUI でブランチから最新の再開可能な Session を選び、再開する。 |
| `OpenLaunchWizard` | GUI 内部 | — | 選択中の GUI ウィンドウに起動ウィザードを開く。 |
| `OpenActiveWorkLaunchWizard` | GUI 内部 | — | 現在の Work を再利用するウィザードの起動入力を初期化する。 |
| `LaunchWizardAction` | 対象外 | — | 画面遷移、下書き編集、セットアップ、起動の確定は状態を持つ GUI ウィザードに紐づく。 |
| `SetIssueMonitorEnabled` | 対応 | `issue.monitor.config.set` | Issues モニターの有効状態を切り替える。GUI は起動設定が不足している場合にウィザードを開く。 |
| `SetIssueMonitorAutonomousMode` | 対応 | `issue.monitor.config.set` | プロジェクトの自動実行モードを切り替える。 |
| `GetUpdateSettings` | 対応 | `update.settings.status` | 更新の適用設定と現在の状態を取得する。 |
| `GetHookDoctor` | 対応 | `hook.doctor` | プロジェクトの hook を診断し、明示された場合に修復する。 |
| `UpdateUserHooks` | 一部対応 | `hook.doctor` | 登録済みフックの監査結果は同じ診断で読める。スクリプトの登録と削除は、利用者が画面で明示した操作に限る。 |
| `SetAutoApplyUpdates` | 対応 | `update.settings.set` | アプリ全体の自動更新適用を変更する。envelope は既存の承認契約を使う。 |
| `GetIssueMonitorProfiles` | 対応 | `issue.monitor.profiles` | 保存済みの Issues 起動プロファイルを取得する。 |
| `IssueMonitorProfilesSet` | 対応 | `issue.monitor.profiles.set` | Issues が使う起動プロファイル群を更新する。 |
| `LoadThemes` | 読み取り | — | テーマ画面に出す、プロジェクトのテーマ（sub-issue を持つ Issue）と今日の完了数を読む（Issue #7643）。 |
| `DeleteTheme` | 対象外 | — | テーマを削除する（テーマの GitHub Issue を not planned で閉じる）。テーマ画面の確認ダイアログを経る操作に限る（Issue #7643）。 |
| `GetExecutionEnvironments` | 読み取り | — | 実行環境の定義、アカウント、開いているプロジェクトの優先度リストを取得する。 |
| `GetProjectEnvironmentSet` | 読み取り | — | プロジェクト設定に表示する環境変数セットの候補名と割り当てを取得する。 |
| `SetProjectEnvironmentSet` | 対象外 | — | プロジェクト設定から、次のエージェント起動に使う環境変数セットの割り当てを保存する（Issue #7651）。 |
| `AddExecutionEnvironment` | 対象外 | — | アカウントと経由ツールの組をホストに登録する。実行環境の画面からの操作に限る（Issue #7741）。 |
| `UpdateExecutionEnvironment` | 対象外 | — | 実行環境の名前と実行時指定を変更する。実行環境の画面からの操作に限る（Issue #7741）。 |
| `DeleteExecutionEnvironment` | 対象外 | — | 実行環境を削除し、各プロジェクトのリストから外す。確認画面を経る操作に限る（Issue #7741）。 |
| `SetProjectExecutionEnvironments` | 対象外 | — | プロジェクトの利用可能な実行環境リストを優先度順で保存する。プロジェクト設定からの操作に限る（Issue #7741）。 |
| `SetPmExecutionEnvironments` | 対象外 | — | この端末の PM 専用の実行環境リストを優先度順で保存する。空のリストは保存側で拒否する。アプリ設定からの操作に限る（Issue #7732）。 |
| `SetAgentExecutionEnvironments` | 対象外 | — | エージェント設定から実行環境の優先度を保存する。空のリストはプロジェクト順を使う（Issue #7846）。 |
| `SetIssueMonitorMaxActiveAgents` | 対応 | `issue.monitor.config.set` | Issues の同時稼働エージェント上限を変更する。 |
| `SetIssueMonitorAllowedLabels` | 対応 | `issue.monitor.config.set` | Issues の対象ラベルを変更する。 |
| `GetRepositoryLabels` | 読み取り | — | Issues の設定候補としてリポジトリのラベルを取得する。 |
| `ReorderIssueMonitorIssues` | 対応 | `issue.monitor.priority.set` | Issues の優先順を一括で更新する。 |
| `IssueMonitorQueuePush` | 対応 | `issue.monitor.queue.push` | 明示した Issue をキューに登録する。 |
| `IssueMonitorQueueRemove` | 対応 | `issue.monitor.queue.remove` | 指定 Issue をキューから除外する。 |
| `IssueMonitorQueueMove` | 対応 | `issue.monitor.queue.move` | 指定 Issue をキュー内で移動する。 |
| `SetIssueMonitorAutoRefill` | 対応 | `issue.monitor.queue.auto_refill` | キューの自動補充と上限を変更する。 |
| `SetIssueMonitorTier` | 対応 | `issue.monitor.tier.set` | 設定トークンで現在の revision を確認し、指定 Issue の tier を更新する。 |
| `OpenIssueMonitor` | GUI 内部 | — | Issues 画面へのクライアント側の遷移を記録する。 |
| `ListIssueMonitor` | 対応 | `issue.monitor.status` | Issues の実行状態を取得する。 |
| `QuickRegisterIssue` | 一部対応 | `issue.create` | Issue の作成を行える。GUI には作成後に起動ウィザードへ進む選択がある。 |
| `IssueMonitorLaunchNow` | GUI 内部 | — | Issue の手動起動ウィザードを開く。ウィザードで起動を確定する。 |
| `IssueMonitorRequeue` | 対応 | `issue.monitor.requeue` | 失敗済みで稼働中の実行がない Issue をキューへ戻す。 |
| `IssueMonitorConfigureIssue` | GUI 内部 | — | 個別 Issue の起動設定ウィザードを開く。 |
| `IssueMonitorConfigureProfile` | GUI 内部 | — | Issues の起動プロファイル編集ウィザードを開く。 |
| `CheckForUpdates` | GUI 内部 | — | ヘッダーから明示的な更新確認を開始し、ホストの検出結果を表示へ反映する。 |
| `ApplyUpdate` | 対象外 | — | 旧クライアント向けの更新再起動経路。実際の更新と再起動はホストが担う。 |
| `ApplyUpdateStart` | 対象外 | — | ホストで互換アセットをダウンロードし、GUI で進捗を通知する。 |
| `CancelUpdateDownload` | 対象外 | — | 準備中の更新の取消と保留マニフェスト処理はホストの更新フローに紐づく。 |
| `ApplyUpdateLater` | 一部対応 | `release.update.defer` | 更新を延期できる。GUI はダウンロード済みアセットの保留マニフェストも確認する。 |
| `ApplyUpdateRestartNow` | 対象外 | — | 準備済みバイナリ適用とホストの再起動は GUI の再起動確認後に実施する。 |
| `GetUpdateRestartImpact` | 読み取り | — | 再起動で影響を受けるホストの稼働状況を取得する。 |
| `ApplyUpdateWhenIdle` | 対象外 | — | ダウンロード済み更新の適用をホストの drain に登録する。 |
| `CancelUpdateAutoApply` | 対象外 | — | 現在の適用猶予と drain を解除する。本体の自動適用設定とは別の状態を扱う。 |
| `OpenUpdateLog` | GUI 内部 | — | 更新ログを OS 既定のアプリで開く。 |
| `OpenServerUrl` | GUI 内部 | — | 稼働中サーバーの URL を OS 既定のブラウザで開く。 |
| `ListCustomAgents` | 読み取り | — | 保存済みのカスタムエージェントを取得する。 |
| `ListSupportedAgents` | 読み取り | — | 組み込みエージェントと検出状態を取得する。 |
| `MaintainSupportedAgent` | 対象外 | — | ホストにインストールされたエージェントの更新・修復を行う。 |
| `SetAgentAutoUpdate` | 対象外 | — | ホストにインストールされた各エージェントを管理する自動更新設定を切り替える。 |
| `ListCustomAgentPresets` | 読み取り | — | カスタムエージェントを作るためのプリセットを列挙する。 |
| `AddCustomAgentFromPreset` | 対象外 | — | バックエンド設定を伴うホストのカスタムエージェントを作成する。 |
| `UpdateCustomAgent` | 対象外 | — | ホストのカスタムエージェント定義とバックエンド設定を更新する。 |
| `DeleteCustomAgent` | 対象外 | — | ホストのカスタムエージェント定義を削除する。 |
| `TestBackendConnection` | 対象外 | — | GUI に入力された認証情報で外部バックエンドへ接続する。 |
| `ListAgentBackends` | 読み取り | — | 指定エージェントに保存済みのバックエンド設定を列挙する。 |
| `AddAgentBackend` | 対象外 | — | 組み込みエージェント向けの外部バックエンド設定をホストに追加する。 |
| `UpdateAgentBackend` | 対象外 | — | 組み込みエージェント向けの外部バックエンド設定をホストで編集する。 |
| `DeleteAgentBackend` | 対象外 | — | 指定エージェントの外部バックエンド設定をホストから削除する。 |
| `TestAgentBackendConnection` | 対象外 | — | 選択されたエージェントと認証情報を使い、外部バックエンドへ接続する。 |
| `StartMigration` | 対象外 | — | 対象タブを確認し、ディスク上のプロジェクト配置を移行する。 |
| `SkipMigration` | GUI 内部 | — | 現在のタブを従来のレイアウトで開く。 |
| `QuitMigration` | GUI 内部 | — | 移行ダイアログの判断からネイティブのアプリ終了経路へ進む。 |
| `GetSystemSettings` | 読み取り | — | ホストの設定画面に表示する現在値を取得する。 |
| `GetBoardAuthStatus` | 読み取り | — | Board 用の OAuth 認証状態と接続設定を表示する。 |
| `BoardProviderSignIn` | 対象外 | — | OS ブラウザで利用者の OAuth 同意を取り、ホストへ認証を戻す。 |
| `BoardProviderSignOut` | 対象外 | — | ホストのセキュアストアに保存された Board プロバイダー認証を消去する。 |
| `UpdateBoardProviderConfig` | 対象外 | — | ホスト設定とセキュアストアを使い、Board プロバイダーの接続設定を更新する。 |
| `UpdateBoardOauthPort` | 対象外 | — | 次回起動時にバインドするホストの OAuth コールバックポートを変更する。 |
| `GetProjectBoardConfig` | 対応 | `board.config.show` | プロジェクト個別設定と有効な Board のルーティングを確認する。 |
| `UpdateProjectBoardConfig` | 対応 | `board.config.set` | プロジェクト個別の Board 設定を変更する。envelope は既存の承認契約を使う。 |
| `UpdateSystemSettings` | 対象外 | — | 言語、hook の信頼設定、Board プロバイダー等のホスト設定を変更する。 |
| `GetAutostartStatus` | 読み取り | — | OS のユーザー別自動起動登録の状態を取得する。 |
| `UpdateAutostart` | 対象外 | — | OS のユーザー別自動起動登録を追加または削除する。 |
| `WorkspaceProjectionPrune` | 対応 | `workspace.projection_prune` | 条件を満たす Workspace projection の整理をプレビューまたは適用する。 |
| `SaveUiTrace` | GUI 内部 | — | ブラウザで採取した UI トレースをプロジェクトの診断ファイルに保存する。 |
| `OpenReleaseNotes` | GUI 内部 | — | バンドル済み変更履歴の指定バージョンを GUI ウィンドウに表示する。 |
| `ApplyUpdateToVersion` | 対象外 | — | 指定バージョンのプラットフォームアセットを解決し、ホストの更新フローへ進む。 |
| `CloseWork` | 対応 | `workspace.close` | 指定 Work の終了を記録する。実行中の Work の終了は拒否される。 |
