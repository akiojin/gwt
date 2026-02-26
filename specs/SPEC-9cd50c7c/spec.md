# 機能仕様: AI自動ブランチ命名モード

**仕様ID**: `SPEC-9cd50c7c`
**作成日**: 2026-02-26
**更新日**: 2026-02-26
**ステータス**: clarify済み
**カテゴリ**: GUI
**依存仕様**:

- なし（既存のAIブランチ提案機能を置換する）

**入力**: ユーザー説明: "Launch AgentのブランチAI提案機能を改善する。現在の「Suggestモーダルで3候補から選択」を廃止し、「Direct入力 or AI提案」のセグメンテッドボタン切り替え式にする。AI提案モードではDescription単行入力のみ表示し、Launch後の起動プロセス内（worktreeステップ）でAIが1つだけブランチ名を自動生成する。生成されたブランチ名でそのまま（確認なし）worktreeを作成する。モード選択はlocalStorageで永続化。AI失敗時はDirectモードにフォールバック+エラーバナー表示。AI未設定環境ではAI Suggestセグメントをdisabled。fromIssueタブには適用しない（manualタブのみ）。既存のsuggest_branch_namesコマンドを1つ生成に改修。PrefixもAIに完全委任。"

## 背景

- 現在のAIブランチ名提案は、Suggestモーダルを開く → 説明文を入力 → 3つの候補から選択 という3ステップの操作が必要であり、手間が多い
- ブランチ名は Launch 時に確定すれば十分であり、事前にモーダルで選択する必要がない
- 操作ステップの多さにより、AIブランチ命名機能の利用率が低い
- Direct入力とAI提案の切り替え式にすることで、AIブランチ命名を日常的に利用できるようにする

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - AI自動ブランチ命名でLaunch (優先度: P0)

ユーザーとして、ブランチ名を考えずに説明文だけ入力してLaunchし、AIが適切なブランチ名（prefix含む）を自動生成してworktreeを作成してほしい。

**独立したテスト**: AI Suggestモードで説明文を入力してLaunchすると、AIがブランチ名を1つ生成し、そのブランチ名でworktreeが作成される

**受け入れシナリオ**:

1. **前提条件** AI設定済み・New Branchモード・manualタブ、**操作** セグメンテッドボタンで「AI Suggest」を選択し、Description欄に"Add user login feature"を入力してLaunchをクリック、**期待結果** 起動プロセスのworktreeステップ内でブランチ名が生成され（例: feature/add-user-login）、そのブランチ名でworktreeが作成される
2. **前提条件** AI設定済み・AI Suggestモード、**操作** Description欄が空のままLaunchボタンを確認、**期待結果** Launchボタンがdisabledで押せない
3. **前提条件** AI設定済み・AI Suggestモード、**操作** Description欄に説明文を入力、**期待結果** Launchボタンがenabledになる

---

### ユーザーストーリー 2 - Direct入力でLaunch (優先度: P0)

ユーザーとして、従来通りPrefix+Suffixを手動入力してブランチ名を決めたい。

**独立したテスト**: Directモードでは従来のPrefix選択+Suffix入力UIが表示され、手動入力したブランチ名でworktreeが作成される

**受け入れシナリオ**:

1. **前提条件** New Branchモード・manualタブ、**操作** セグメンテッドボタンで「Direct」を選択、**期待結果** Prefix選択ドロップダウンとSuffix入力フィールドが表示される（Suggest...ボタンは表示されない）
2. **前提条件** Directモード、**操作** Prefix="feature/"、Suffix="my-change"を入力してLaunch、**期待結果** "feature/my-change"でworktreeが作成される

---

### ユーザーストーリー 3 - モード選択の永続化 (優先度: P1)

ユーザーとして、前回選択したモード（Direct or AI Suggest）が次回Launch時に復元されてほしい。

**独立したテスト**: モードを選択してLaunchした後、次回フォームを開くと前回のモードが選択された状態になる

**受け入れシナリオ**:

1. **前提条件** 初回使用かつAI設定済み、**操作** フォームを開く、**期待結果** デフォルトはAI Suggestモード
2. **前提条件** Directモードを選択してLaunch済み、**操作** 再度フォームを開く、**期待結果** Directモードが選択された状態で開く
3. **前提条件** AI未設定、**操作** フォームを開く、**期待結果** AI SuggestセグメントがdisabledでDirectモードが選択される

---

### ユーザーストーリー 4 - AI提案失敗時のフォールバック (優先度: P1)

ユーザーとして、AI提案が失敗しても手動入力でLaunchを続行したい。

**独立したテスト**: AI提案が失敗すると、フォームに戻りDirectモードに切り替わり、警告バナーが表示される

**受け入れシナリオ**:

1. **前提条件** AI Suggestモードで説明入力済み、**操作** Launch後にAI提案がAPIエラーを返す、**期待結果** 起動プロセスが中断し、フォームに戻り、Directモードに自動切替、フォーム上部に"AI suggestion failed. Please enter branch name manually."バナーが表示される
2. **前提条件** AI Suggestモードで説明入力済み、**操作** Launch後にAI提案がタイムアウト、**期待結果** 同上のフォールバック動作

---

### ユーザーストーリー 5 - fromIssueタブとの分離 (優先度: P2)

ユーザーとして、Issueからブランチを作成する場合は従来通りの操作で使いたい。

**独立したテスト**: fromIssueタブではDirect/AI Suggestの切り替えUIが表示されない

**受け入れシナリオ**:

1. **前提条件** New Branchモード、**操作** fromIssueタブを選択、**期待結果** Direct/AI Suggestのセグメンテッドボタンが表示されず、従来のIssue選択+Prefix分類UIが表示される

## エッジケース

- AI未設定環境: AI SuggestセグメントをdisabledにしてDirectモードを強制
- AI応答が空文字やパース不能: フォールバック動作（Directモードに切替+エラーバナー表示）
- 起動プロセスのCancel: 通常のCancel処理（launch全体を中止）
- AIが生成したブランチ名が既に存在: 既存のブランチ重複エラーハンドリングに従う
- ネットワーク障害: AI提案のHTTPエラーとしてフォールバック動作
- 永続化されたモードがAI Suggestだが、AI設定が後から削除された場合: Directモードに自動降格

## 要件 *(必須)*

### 機能要件

- **FR-001**: manualタブ内にDirect / AI Suggestのセグメンテッドボタンを配置し、モードを排他的に切り替えられる
- **FR-002**: AI Suggestモードでは、Prefix選択・Suffix入力・Suggest...ボタンを非表示にし、「Description」ラベル付きの単行入力フィールドのみ表示する。placeholderは具体例（例: "e.g. Add user authentication feature"）とする
- **FR-003**: Directモードでは、従来のPrefix選択+Suffix入力を表示する（Suggest...ボタンは廃止）
- **FR-004**: AI Suggestモードでは、Launch後の起動プロセス内（worktreeステップ）でAIにブランチ名を1つ生成させる
- **FR-005**: AIはPrefix（feature/bugfix/hotfix/release/）を含む完全なブランチ名を1つ生成する
- **FR-006**: 生成されたブランチ名でそのままworktreeを作成する（ユーザー確認なし）
- **FR-007**: AI提案失敗時はフォームに戻り、Directモードに自動切替し、フォーム上部にエラーバナーを表示する。バナーはモード切替時（Direct⇔AI Suggest）に自動消去する
- **FR-008**: 選択したモード（Direct / AI Suggest）を永続化し、次回フォーム起動時に復元する
- **FR-009**: AI未設定環境ではAI Suggestセグメントをdisabledにする。AI設定の有無はフォーム呈示時にバックエンドへ問い合わせて判定する
- **FR-010**: AI Suggestモードで説明が空の場合、Launchボタンをdisabledにする
- **FR-011**: 既存のSuggestモーダル（3候補選択UI）を完全に廃止する
- **FR-012**: 既存のブランチ名提案機能を1つ生成に改修する
- **FR-013**: fromIssueタブにはDirect/AI Suggest切り替えを適用しない
- **FR-014**: Description入力値はモード切替時もクリアせず内部で保持し、AI Suggestモードに戻した際に復元する

### 非機能要件

- **NFR-001**: AI応答待ちはworktreeステップ内で行い、起動プロセスのステップ数を増やさない
- **NFR-002**: AI応答のタイムアウトは既存のAI設定に従う

## 制約と仮定

- AI設定は既存のプロファイル設定機構を使用する
- セグメンテッドボタンは既存UIパターン（manual/fromIssueタブ）と同様のデザインを踏襲する
- fromIssueタブの既存動作（Issue選択→AI Prefix分類→ブランチ名生成）は変更しない
- ブランチ名のPrefix種類は現行の4種（feature/bugfix/hotfix/release/）から変更しない

## 成功基準 *(必須)*

- **SC-001**: AI SuggestモードでDescription入力→Launch→ブランチ名自動生成→worktree作成が1クリックで完了する
- **SC-002**: AI提案失敗時にDirectモードへフォールバックし、ユーザーが手動入力で続行できる
- **SC-003**: モード選択が次回起動時に正しく復元される
- **SC-004**: 既存のSuggestモーダルのコードがすべて削除されている
- **SC-005**: 全テストがパスする
