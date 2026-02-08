# 機能仕様: エージェント起動ウィザード統合

**仕様ID**: `SPEC-1ad9c07d`
**作成日**: 2026-02-08
**ステータス**: ドラフト
**入力**: ユーザー説明: "エージェント起動ウィザードの散在する仕様を統合し、AIブランチ名提案機能を追加する"

## 概要

gwtのエージェント起動ウィザード（WizardPopup）の全ステップフローを統合的に定義する。ブランチ選択からエージェント起動確定までのウィザード内の各ステップの遷移ロジック、条件分岐、UI仕様を一元管理する。

本仕様は以下の既存仕様のウィザード関連部分を統合し、新規のAIブランチ名提案ステップを追加する：

- SPEC-3b0ed29b（コーディングエージェント対応）のウィザードステップ部分
- SPEC-e4798383（GitHub Issue連携）のIssueSelectステップ
- SPEC-f47db390（セッションID永続化）のQuickStartフロー
- SPEC-fdebd681（Codex collaboration_modes）のCollaborationModesステップ
- SPEC-71f2742d（カスタムエージェント登録）のウィザード表示部分

> **注**: 各既存SPECの非ウィザード部分（起動実行、セッション管理、進捗モーダル、設定画面等）は各SPECに残存する。

## ウィザードステップフロー全体図

```text
[既存ブランチ選択時]
  QuickStart ──(履歴あり)──> 即時起動
     │                          or
     └──(設定を選び直す)──> BranchAction
                                │
                    ┌───────────┴───────────┐
                    │                       │
              Use selected           Create new from selected
                    │                       │
                    v                       v
              AgentSelect          BranchTypeSelect
                                        │
                                        v
                                   IssueSelect ─(gh CLI無し/0件)─> skip
                                        │
                                        v
                                  AIBranchSuggest ─(AI無効)─> skip  [NEW]
                                        │
                                        v
                                  BranchNameInput
                                        │
                                        v
                                   AgentSelect
                                        │
                                        v
                                   ModelSelect ─(モデル無し)─> skip
                                        │
                                        v
                                  ReasoningLevel ─(Codex以外)─> skip
                                        │
                                        v
                                  VersionSelect
                                        │
                                        v
                                CollaborationModes ─(自動判定)─> skip
                                        │
                                        v
                                  ExecutionMode
                                        │
                                        v
                                 SkipPermissions
                                        │
                                        v
                                    [起動開始]

[新規ブランチ作成時]
  BranchTypeSelect ──> IssueSelect ──> AIBranchSuggest ──> BranchNameInput ──> AgentSelect ──> ...
                  └─(gh CLI無し/0件)──────────┘
```

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - QuickStartで前回設定から素早く再開 (優先度: P1)

開発者がブランチを選択すると、前回そのブランチで使ったAIツール・モデル・セッションIDに基づいて「前回設定で続きから」「前回設定で新規」「設定を選び直す」を選べる。

**この優先度の理由**: 毎回ツールとモデルを選択する手間を削減し、高速に再開できる。

**独立したテスト**: ブランチに紐づく履歴を残した状態でブランチを選択し、QuickStartが表示されることを確認。

**受け入れシナリオ**:

1. **前提条件** 対象ブランチの履歴にtoolId/model/sessionIdがある、**操作** ブランチ選択→「前回設定で続きから」を選択、**期待結果** ツール/モデル/スキップ設定が前回値で適用され、中間ステップを表示せず即時起動される
2. **前提条件** 対象ブランチに履歴が無い、**操作** ブランチ選択、**期待結果** QuickStartをスキップし従来のツール選択画面に遷移
3. **前提条件** QuickStart画面で「Choose different」を選択、**操作** Enterで確定、**期待結果** BranchActionステップに遷移
4. **前提条件** 同一ブランチで複数ツールを使用済み、**操作** QuickStartを表示、**期待結果** 各ツールの直近設定が並列に提示される

> 詳細: [SPEC-f47db390](../SPEC-f47db390/spec.md) US5

---

### ユーザーストーリー 2 - ブランチアクションの選択 (優先度: P1)

既存ブランチを選択した場合、そのブランチを直接使うか、そのブランチから新規ブランチを作成するかを選択できる。

**この優先度の理由**: 既存ブランチでの作業と新規ブランチ分岐の両方に対応する基本機能。

**独立したテスト**: ブランチ選択後にBranchActionで「Create new from selected」を選び、BranchTypeSelectに進むことを確認。

**受け入れシナリオ**:

1. **前提条件** 既存ブランチが選択されている、**操作** 「Use selected branch」を選択、**期待結果** AgentSelectステップに直接遷移
2. **前提条件** 既存ブランチが選択されている、**操作** 「Create new branch from this」を選択、**期待結果** BranchTypeSelectステップに遷移し、選択ブランチがベースブランチとして設定される

---

### ユーザーストーリー 3 - ブランチタイプの選択 (優先度: P1)

新規ブランチ作成時に、ブランチタイプ（feature/bugfix/hotfix/release）を選択し、対応するプレフィックスがブランチ名に適用される。

**この優先度の理由**: ブランチ命名規則の一貫性を保証する基本機能。

**独立したテスト**: 各ブランチタイプを選択し、対応するプレフィックスが設定されることを確認。

**受け入れシナリオ**:

1. **前提条件** BranchTypeSelectステップが表示されている、**操作** featureを選択、**期待結果** プレフィックス「feature/」が設定される
2. **前提条件** BranchTypeSelectステップが表示されている、**操作** bugfixを選択、**期待結果** プレフィックス「bugfix/」が設定される

> 参考: [SPEC-1defd8fd](../SPEC-1defd8fd/spec.md)

---

### ユーザーストーリー 4 - GitHub Issueを選択してブランチ名を自動生成 (優先度: P1)

ブランチタイプ選択後、GitHub Issueを選択してIssue番号を含むブランチ名を自動生成できる。スキップも可能。

**この優先度の理由**: Issue駆動開発での命名ミスを防止する重要機能。

**独立したテスト**: IssueSelectでIssueを選択し、ブランチ名が自動入力されることを確認。

**受け入れシナリオ**:

1. **前提条件** BranchTypeSelectでfeatureを選択済み、gh CLIがインストール済み、**操作** IssueSelectステップに進む、**期待結果** リポジトリのopen Issue一覧が表示される
2. **前提条件** Issue一覧が表示されている、**操作** Issue #42を選択してEnter、**期待結果** ブランチ名入力欄に「issue-42」が自動入力される
3. **前提条件** Issue一覧が表示されている、**操作** Skipを選択してEnter、**期待結果** ブランチ名入力欄は空のまま次に進む
4. **前提条件** gh CLIが未インストール、**操作** BranchTypeSelectからNextに進む、**期待結果** IssueSelectをスキップし、次のステップ（AIBranchSuggestまたはBranchNameInput）に進む
5. **前提条件** Issue一覧が0件、**操作** ロード完了、**期待結果** IssueSelectを自動スキップし、次のステップ（AIBranchSuggestまたはBranchNameInput）に進む
6. **前提条件** Issue一覧が表示されている、**操作** キーワード入力でインクリメンタル検索、**期待結果** タイトルに一致するIssueのみ表示される

> 詳細: [SPEC-e4798383](../SPEC-e4798383/spec.md)

---

### ユーザーストーリー 5 - AIによるブランチ名候補の生成と選択 [NEW] (優先度: P1)

新規ブランチ作成ウィザードで、AI設定が有効なユーザーがブランチの目的を自然言語で入力すると、AIが3つのブランチ名候補をプレフィックス込みで提案する。ユーザーは候補から1つを選択し、選択された名前がブランチ名入力欄に事前入力される。

**この優先度の理由**: ブランチ命名規則に沿った一貫性のある名前を簡単に生成でき、ユーザーの命名負担を削減する中核機能。

**独立したテスト**: AI設定を有効にした状態で新規ブランチ作成ウィザードを開始し、目的を入力してAI提案候補を選択することで完全にテストできる。

**受け入れシナリオ**:

1. **前提条件** AI設定が有効（endpoint・modelが設定済み）で新規ブランチ作成ウィザードを開始し、BranchTypeSelectでfeatureを選択済み、**操作** IssueSelectステップの次に進む、**期待結果** AIBranchSuggestステップが表示され、テキスト入力欄と「What is this branch for?」ラベルが表示される
2. **前提条件** AIBranchSuggestステップのテキスト入力欄が表示されている、**操作** 「Add OAuth login to the login page」と入力してEnterを押す、**期待結果** ローディング表示後、3つのブランチ名候補がプレフィックス込み（例: `feature/add-oauth-login`）でリスト表示される
3. **前提条件** 3つのブランチ名候補がリスト表示されている、**操作** 上下キーで候補を選択しEnterを押す、**期待結果** BranchNameInputステップに遷移し、選択した候補のプレフィックスを除いた部分がnew_branch_nameに事前入力され、branch_typeが候補のプレフィックスに対応するタイプに設定される
4. **前提条件** BranchNameInputステップに事前入力された状態、**操作** ユーザーがブランチ名を自由に編集、**期待結果** 事前入力された名前を編集・変更して次のステップに進める
5. **前提条件** gh CLIが未インストール、AI設定が有効でBranchTypeSelectでfeatureを選択済み、**操作** BranchTypeSelectからNextに進む、**期待結果** IssueSelectをスキップしAIBranchSuggestステップが表示される

---

### ユーザーストーリー 6 - AI無効時のAIBranchSuggestスキップ [NEW] (優先度: P1)

AI設定が無効（未設定または無効化）のユーザーは、従来通りAIBranchSuggestステップなしでブランチ作成ウィザードを進める。

**この優先度の理由**: AI設定が無効な場合の後方互換性を維持し、既存ユーザーの体験を損なわない。

**独立したテスト**: AI設定を無効にした状態で新規ブランチ作成ウィザードを進め、AIBranchSuggestステップが表示されないことを確認。

**受け入れシナリオ**:

1. **前提条件** AI設定が無効（endpointまたはmodelが未設定）で新規ブランチ作成ウィザードを開始、**操作** IssueSelectステップから次に進む、**期待結果** AIBranchSuggestステップをスキップし、BranchNameInputステップに直接遷移する

---

### ユーザーストーリー 7 - AIBranchSuggestのスキップとフォールバック [NEW] (優先度: P2)

AI設定が有効でもAIBranchSuggestステップでEscを押すことで手動入力にフォールバックでき、APIエラー時もブランチ作成フローが中断されない。

**この優先度の理由**: AIに依存したくない場合や障害時の代替手段を提供する。

**独立したテスト**: AIBranchSuggestステップでEscを押して手動入力画面に遷移すること、APIエラー時にフォールバックすることを確認。

**受け入れシナリオ**:

1. **前提条件** AIBranchSuggestステップのテキスト入力フェーズが表示されている、**操作** Escを押す、**期待結果** BranchNameInputステップに遷移し、new_branch_nameは現在値を保持する（未設定なら空のまま）
2. **前提条件** AIBranchSuggestステップの候補選択フェーズが表示されている、**操作** Escを押す、**期待結果** テキスト入力フェーズに戻る
3. **前提条件** AIBranchSuggestステップでユーザーが目的を入力済み、**操作** AI APIリクエストがエラーを返す、**期待結果** エラーメッセージが表示され、Enterを押すとBranchNameInputステップに遷移する（new_branch_nameは現在値を保持）
4. **前提条件** 空のテキストでEnterを押す、**操作** 送信を試みる、**期待結果** リクエストは送信されず入力を促すエラーが表示される
5. **前提条件** ローディング中、**操作** Escを押す、**期待結果** リクエスト結果を無視してテキスト入力フェーズに戻る（HTTPリクエスト自体は継続する場合がある）

---

### ユーザーストーリー 8 - ブランチ名の手動入力 (優先度: P1)

ブランチ名入力ステップで、ユーザーがブランチ名を手動で入力または編集できる。Issue選択やAI提案による事前入力がある場合はその値を編集可能。

**この優先度の理由**: 最終的なブランチ名をユーザーが確認・編集できる必要がある。

**独立したテスト**: BranchNameInputで名前を入力し、次のステップに進めることを確認。

**受け入れシナリオ**:

1. **前提条件** BranchNameInputが表示され、入力欄が空、**操作** 「my-feature」と入力してEnter、**期待結果** branch_type.prefix() + "my-feature"がフルブランチ名として設定される
2. **前提条件** Issue選択やAI提案で事前入力済み、**操作** 値を編集してEnter、**期待結果** 編集後の値が使用される
3. **前提条件** BranchNameInputが表示されている、**操作** 空のままEnter、**期待結果** 次のステップに進める（バリデーションは後続で実施）

---

### ユーザーストーリー 9 - エージェント選択 (優先度: P1)

ユーザーがコーディングエージェント（Claude Code、Codex、Gemini、OpenCode、カスタム）を選択できる。カスタムエージェントはビルトインの後にセパレータを挟んで表示される。

**この優先度の理由**: エージェント選択はウィザードの中核機能。

**独立したテスト**: AgentSelectで各エージェントを選択し、次のステップに進めることを確認。

**受け入れシナリオ**:

1. **前提条件** AgentSelectが表示されている、**操作** Claude Codeを選択、**期待結果** agent=ClaudeCodeが設定され、ModelSelectに遷移
2. **前提条件** カスタムエージェントが登録済み、**操作** AgentSelectを表示、**期待結果** ビルトイン4種の後にセパレータ + カスタムエージェントが表示される
3. **前提条件** カスタムエージェントのコマンドが未インストール、**操作** AgentSelectを表示、**期待結果** グレーアウト表示で"Not installed"ラベルが付く

> 詳細: [SPEC-3b0ed29b](../SPEC-3b0ed29b/spec.md) US1, [SPEC-71f2742d](../SPEC-71f2742d/spec.md) US1

---

### ユーザーストーリー 10 - モデル・バージョン・実行モードの選択 (優先度: P1)

エージェント選択後、モデル、推論レベル（Codexのみ）、バージョン、CollaborationModes（Codex v0.91.0+のみ）、実行モード、権限スキップを順に選択する。各ステップは条件に応じてスキップされる。

**この優先度の理由**: エージェント起動に必要な設定を段階的に収集する基本フロー。

**独立したテスト**: 各ステップを順に進め、条件付きスキップが正しく動作することを確認。

**受け入れシナリオ**:

1. **前提条件** Claude Codeを選択、**操作** ModelSelectに進む、**期待結果** Claude Code用モデル一覧が表示される
2. **前提条件** Codexを選択、**操作** ModelSelect後に進む、**期待結果** ReasoningLevelステップが表示される
3. **前提条件** Claude Codeを選択、**操作** ModelSelect後に進む、**期待結果** ReasoningLevelをスキップしVersionSelectに遷移
4. **前提条件** Codex v0.91.0+を選択、**操作** VersionSelect完了、**期待結果** CollaborationModesが自動有効化されスキップ
5. **前提条件** Claude Codeを選択、**操作** VersionSelect完了、**期待結果** CollaborationModesをスキップしExecutionModeに遷移
6. **前提条件** SkipPermissionsでEnter、**操作** 確定、**期待結果** ウィザード完了、起動処理開始

> 詳細: [SPEC-3b0ed29b](../SPEC-3b0ed29b/spec.md) US2-US5, [SPEC-fdebd681](../SPEC-fdebd681/spec.md)

---

### エッジケース

#### ウィザード共通

- ウィザード表示中にEscを押した場合、前のステップに戻る（最初のステップの場合はウィザードを閉じる）。ただしAIBranchSuggestのInputフェーズではEscは「スキップ（BranchNameInputへ）」として扱う
- ウィザード表示中にマウスクリックでリスト項目を直接選択できる

#### AIBranchSuggest [NEW]

- 空のテキストでEnterを押した場合、リクエストは送信されず入力を促す
- AIレスポンスが不正なフォーマットの場合、エラーとして扱いフォールバックする
- AIレスポンスに含まれるブランチ名が既存ブランチと重複する場合、候補リストにはそのまま表示する（BranchNameInputで編集可能）
- 提案されたブランチ名に許可されたプレフィックス（feature/bugfix/hotfix/release）が含まれない場合、無効候補として扱いパースエラーにする
- ローディング中にEscを押した場合、リクエスト結果を無視してテキスト入力フェーズに戻る（HTTPリクエスト自体は継続する場合がある）
- AIBranchSuggestから手動入力にフォールバックする場合、new_branch_nameは現在値を保持する（IssueSelectで自動入力済みの場合はそれを維持）

#### IssueSelect

- gh CLIがインストールされていない場合、自動スキップ
- Issue一覧が0件の場合、自動スキップ
- 同一Issue番号のブランチが既存の場合、選択をブロック

#### QuickStart

- 履歴が空の場合、QuickStartをスキップ
- エージェントが既に実行中の場合、QuickStartをスキップしBranchActionに遷移

#### 条件付きステップ

- カスタムエージェントでmodels未定義の場合、ModelSelectをスキップ
- カスタムエージェントでpermissionSkipArgs未定義の場合、SkipPermissionsを非表示
- Codex以外ではReasoningLevelをスキップ
- Codex v0.90.x以下ではCollaborationModesをスキップ

## 要件 *(必須)*

### 機能要件

#### ウィザードフロー制御

- **FR-001**: ウィザードは以下のステップを順序通り管理**しなければならない**: QuickStart → BranchAction → BranchTypeSelect → IssueSelect（条件によりスキップ） → AIBranchSuggest（条件によりスキップ） → BranchNameInput → AgentSelect → ModelSelect → ReasoningLevel → VersionSelect → CollaborationModes → ExecutionMode → SkipPermissions
- **FR-002**: 各ステップはEnterで次に進み、Escで前に戻る操作を提供**しなければならない**（ただしAIBranchSuggestのInputフェーズではEscはスキップとしてBranchNameInputへ進む）
- **FR-003**: ステップ間のナビゲーションで上下キーによるリスト選択とマウスクリック選択を提供**しなければならない**
- **FR-004**: ウィザードは中央ポップアップとして表示し、背景のブランチリストが見える状態を維持**しなければならない**

#### QuickStart (SPEC-f47db390統合)

- **FR-010**: ブランチ選択時、同ブランチの最新履歴が存在する場合はQuickStartを表示**しなければならない**
- **FR-011**: QuickStartは各ツールの直近設定（toolId/model/sessionId/skipPermissions）を提示**しなければならない**
- **FR-012**: QuickStartで「Resume/Start new with previous settings」を選択した場合、中間ステップを表示せず即時起動**しなければならない**
- **FR-013**: QuickStartはReasoning設定を持たないエージェント（Claude/Gemini/OpenCode）ではReasoning表示を出してはならない

#### BranchAction

- **FR-020**: 既存ブランチ選択時、「Use selected branch」と「Create new branch from this」の2択を表示**しなければならない**
- **FR-021**: 「Create new branch from this」選択時、選択ブランチをbase_branch_overrideとして設定**しなければならない**

#### BranchTypeSelect

- **FR-030**: ブランチタイプとしてfeature/bugfix/hotfix/releaseの4種を表示**しなければならない**
- **FR-031**: 選択されたタイプに応じたプレフィックス（feature/、bugfix/、hotfix/、release/）を設定**しなければならない**

#### IssueSelect (SPEC-e4798383統合)

- **FR-040**: gh CLIが利用可能な場合にIssueSelectステップを表示**しなければならない**
- **FR-041**: gh CLIが未インストールの場合、IssueSelectを自動スキップ**しなければならない**
- **FR-042**: Issue一覧が0件の場合、IssueSelectを自動スキップ**しなければならない**
- **FR-043**: Issue一覧はopen状態のIssueを更新日時降順で最大50件表示**しなければならない**
- **FR-044**: 各Issueは「#番号: タイトル」形式で表示**しなければならない**
- **FR-045**: タイトルによるインクリメンタル検索を提供**しなければならない**
- **FR-046**: Issue選択時、ブランチ名を「issue-{number}」形式で自動生成**しなければならない**
- **FR-047**: 空Enter（Skipオプション）でスキップ可能**でなければならない**
- **FR-048**: 同一Issue番号のブランチが既存の場合、選択をブロック**しなければならない**

#### AIBranchSuggest [NEW]

- **FR-050**: ウィザードにAIBranchSuggestステップを追加し、IssueSelectとBranchNameInputの間に配置**しなければならない**
- **FR-051**: AI設定が有効（endpointとmodelが設定済み）な場合にのみAIBranchSuggestステップを表示**しなければならない**
- **FR-051a**: gh CLI無し/Issueが0件でIssueSelectがスキップされる場合でも、AI設定が有効ならAIBranchSuggestを表示**しなければならない**
- **FR-052**: AI設定が無効な場合、AIBranchSuggestステップをスキップし従来のフローを維持**しなければならない**
- **FR-053**: AIBranchSuggestステップは4つのサブフェーズ（入力、ローディング、選択、エラー）で構成**されなければならない**
- **FR-054**: 入力フェーズでユーザーがブランチの目的を自然言語で入力できる**ようにしなければならない**
- **FR-055**: Enter押下時にAI APIへ非同期でリクエストを送信し、UIをブロックしない**ようにしなければならない**
- **FR-056**: AIは3つのブランチ名候補をプレフィックス込み（例: `feature/add-login-page`）で生成**しなければならない**
- **FR-057**: 選択フェーズで上下キーによる候補ナビゲーションとEnterによる確定を提供**しなければならない**
- **FR-058**: 選択されたブランチ名のプレフィックス部分をbranch_typeに反映し、残りをnew_branch_nameに設定**しなければならない**
- **FR-059**: AIBranchSuggestステップの入力フェーズでEscを押すとBranchNameInputステップにスキップ**しなければならない**
- **FR-060**: 候補選択フェーズでEscを押すとテキスト入力フェーズに戻**らなければならない**
- **FR-061**: AI APIエラー時にエラーメッセージを表示し、Enterで手動入力にフォールバック**しなければならない**
- **FR-061a**: 手動入力にフォールバックする場合、new_branch_nameは現在値を保持**しなければならない**（IssueSelectで自動入力済みの場合はそれを維持）
- **FR-062**: 空のテキストでEnterを押した場合、リクエストを送信せずエラー表示する**必要がある**
- **FR-063**: ローディング中にEscを押した場合、リクエスト結果を無視してテキスト入力フェーズに戻**らなければならない**
- **FR-064**: ブランチ名候補は正規化（小文字化、特殊文字除去、ハイフン区切り、64文字上限）される**必要がある**

#### BranchNameInput

- **FR-070**: テキスト入力でブランチ名を入力・編集できる**ようにしなければならない**
- **FR-071**: Issue選択やAI提案による事前入力値を表示し、ユーザーが編集可能**にしなければならない**
- **FR-072**: ブランチ名はbranch_type.prefix()と結合してフルブランチ名を構成**しなければならない**

#### AgentSelect (SPEC-3b0ed29b, SPEC-71f2742d統合)

- **FR-080**: ビルトインエージェント（Claude Code、Codex、Gemini、OpenCode）を表示**しなければならない**
- **FR-081**: カスタムエージェントが登録済みの場合、ビルトインの後にセパレータを挟んで表示**しなければならない**
- **FR-082**: エージェント名はエージェント固有の色で表示**しなければならない**（Claude=黄、Codex=シアン、Gemini=マゼンタ、OpenCode=緑、カスタム=自動割り当て）
- **FR-083**: 未インストールのカスタムエージェントはグレーアウト表示で選択不可に**しなければならない**

#### ModelSelect

- **FR-090**: 選択されたエージェントに応じたモデル一覧を表示**しなければならない**
- **FR-091**: カスタムエージェントでmodelsが未定義の場合、ModelSelectをスキップ**しなければならない**
- **FR-092**: OpenCodeのモデル選択肢が空の場合でも「Default (Auto)」を提示**しなければならない**

#### ReasoningLevel

- **FR-100**: Codex選択時のみReasoningLevelステップを表示**しなければならない**
- **FR-101**: Codex以外のエージェントではReasoningLevelをスキップ**しなければならない**

#### VersionSelect

- **FR-110**: バージョン一覧は起動時にキャッシュした情報を使用**しなければならない**
- **FR-111**: キャッシュが空の場合、「latest」のみを表示**しなければならない**

#### CollaborationModes (SPEC-fdebd681統合)

- **FR-120**: Codex v0.91.0+では自動的にcollaboration_modesを有効化しステップをスキップ**しなければならない**
- **FR-121**: Codex v0.90.x以下ではCollaborationModesをスキップ**しなければならない**
- **FR-122**: Codex以外のエージェントではCollaborationModesをスキップ**しなければならない**
- **FR-123**: collaboration_modes有効時、起動引数に`--enable collaboration_modes`を追加**しなければならない**

#### ExecutionMode

- **FR-130**: Normal/Continue/Resume/Convertの4モードを表示**しなければならない**
- **FR-131**: カスタムエージェントでmodeArgsに定義されていないモードは非表示**にしなければならない**

#### SkipPermissions

- **FR-140**: 権限スキップのYes/No選択を提供**しなければならない**
- **FR-141**: カスタムエージェントでpermissionSkipArgsが未定義の場合、SkipPermissionsを非表示**にしなければならない**
- **FR-142**: SkipPermissions確定でウィザード完了、起動処理を開始**しなければならない**

### 主要エンティティ

- **WizardStep**: ウィザードのステップを表すenum。QuickStart、BranchAction、BranchTypeSelect、IssueSelect、AIBranchSuggest、BranchNameInput、AgentSelect、ModelSelect、ReasoningLevel、VersionSelect、CollaborationModes、ExecutionMode、ConvertAgentSelect、ConvertSessionSelect、SkipPermissions
- **WizardState**: ウィザード全体の状態。表示フラグ、各ステップの選択値、入力値、カーソル位置等を保持
- **AIBranchSuggestフェーズ**: 入力（Input）、ローディング（Loading）、選択（Select）、エラー（Error）の4状態
- **ブランチ名候補**: AIが生成するプレフィックス込みのブランチ名。最大3件

## 成功基準 *(必須)*

### 測定可能な成果

- **SC-001**: ウィザードの全ステップが定義された順序で遷移し、条件付きスキップが正しく動作する
- **SC-002**: QuickStartで前回設定を選択した場合、中間ステップを経ずに即時起動される
- **SC-003**: AI設定有効時にAIBranchSuggestステップが表示され、3件のブランチ名候補が生成・表示される
- **SC-004**: AI設定無効時にAIBranchSuggestステップがスキップされ、従来フローが維持される
- **SC-005**: APIエラー時にフォールバックが正常に機能し、ブランチ作成フローが中断されない
- **SC-006**: Issue選択時にブランチ名が正しく自動生成される
- **SC-007**: カスタムエージェントがビルトインと並んで正しく表示・選択・起動される

## 制約と仮定 *(該当する場合)*

### 制約

- ウィザードのステップ遷移は既存のnext_step/prev_stepパターンに準拠する
- 非同期処理はstd::thread::spawn + mpsc::channelパターンに準拠する
- CLIのユーザー向け出力は英語のみ
- AIBranchSuggestは既存のAIClient（OpenAI互換API）を使用する

### 仮定

- ユーザーはコーディングエージェントの基本的な使い方を理解している
- AI設定が有効なユーザーはAPIエンドポイントとモデルが正しく設定されている
- AIモデルはブランチ命名規則を理解し、適切な候補を生成できる

## 範囲外 *(必須)*

次の項目は、この機能の範囲外です：

- エージェント起動実行（PTYラッパー、進捗モーダル等） → [SPEC-3b0ed29b](../SPEC-3b0ed29b/spec.md)
- セッションID永続化・管理 → [SPEC-f47db390](../SPEC-f47db390/spec.md)
- カスタムエージェント登録・TUI設定画面 → [SPEC-71f2742d](../SPEC-71f2742d/spec.md)
- エージェントモード（マスターエージェント） → [SPEC-ba3f610c](../SPEC-ba3f610c/spec.md)
- ブランチ名候補のキャッシュ・履歴管理
- AIモデルのファインチューニングや特別な学習
- `Screen::WorktreeCreate`画面でのAI提案（現在未使用のため対象外）

## セキュリティとプライバシーの考慮事項 *(該当する場合)*

- ユーザーが入力するブランチ目的テキストはAI APIに送信されるため、機密情報を含まないようユーザーの判断に委ねる
- AI APIへの通信は既存のAIClient設定（HTTPS）に従う
- APIキーの管理は既存のAI設定機能に準拠する

## 依存関係 *(該当する場合)*

- AI設定機能（AISettings、AIClient） - 既に実装済み
- ウィザードフロー（wizard.rs） - 既に実装済み
- ブランチ名正規化（sanitize_branch_name） - 既に実装済み
- GitHub CLI（gh）- Issue選択に使用
- カスタムエージェント定義（tools.json） - エージェント選択に使用

## 参考資料 *(該当する場合)*

- [コーディングエージェント対応仕様](../SPEC-3b0ed29b/spec.md) - 起動実行・エージェント固有要件
- [GitHub Issue連携仕様](../SPEC-e4798383/spec.md) - Issue選択の詳細要件
- [セッションID永続化仕様](../SPEC-f47db390/spec.md) - QuickStartの詳細要件
- [Codex collaboration_modes仕様](../SPEC-fdebd681/spec.md) - CollaborationModesの詳細要件
- [カスタムエージェント登録仕様](../SPEC-71f2742d/spec.md) - カスタムエージェントの詳細要件
