# 機能仕様: semantic-releaseによる自動リリース機能

**仕様ID**: `SPEC-23bb2eed`
**作成日**: 2025-10-25
**更新日**: 2025-10-25
**ステータス**: ドラフト
**入力**: ユーザー説明: "semantic-releaseを使用して、mainブランチへのマージ時にコミットメッセージから自動的にバージョンを決定し、npm registryとGitHub Releasesに公開する。feat:でminor、fix:でpatch、BREAKING CHANGEでmajorバージョンアップを自動実行する。"

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - コミットメッセージベースの自動バージョン決定 (優先度: P1)

開発者として、PRをmainブランチにマージした時に、コミットメッセージの種類（feat、fix、BREAKING CHANGE）に基づいて自動的にバージョンが決定され、リリースされてほしい。

**この優先度の理由**: バージョン管理の自動化は、手動でのバージョン決定ミスを防ぎ、セマンティックバージョニングを一貫して適用できる最も重要な機能。

**独立したテスト**: feat:コミットをmainにマージし、GitHub Actionsでminorバージョンがアップされることを確認することで完全にテスト可能。

**受け入れシナリオ**:

1. **前提条件** 前回リリースがv1.2.0、**操作** `feat: 新機能追加`コミットをmainにマージ、**期待結果** v1.3.0がリリースされ、npmとGitHub Releasesに公開される
2. **前提条件** 前回リリースがv1.2.0、**操作** `fix: バグ修正`コミットをmainにマージ、**期待結果** v1.2.1がリリースされる
3. **前提条件** 前回リリースがv1.2.0、**操作** `feat!: 破壊的変更`またはBREAKING CHANGEを含むコミットをmainにマージ、**期待結果** v2.0.0がリリースされる
4. **前提条件** 前回リリースがv1.2.0、**操作** `docs: ドキュメント更新`のみをmainにマージ、**期待結果** リリースはスキップされ、バージョンは1.2.0のまま
5. **前提条件** 前回リリースがv1.2.0、**操作** `feat: 機能A`と`fix: バグB`をmainにマージ、**期待結果** v1.3.0がリリースされる（minorが優先）

---

### ユーザーストーリー 2 - 自動CHANGELOG生成とパッケージ更新 (優先度: P1)

開発者として、リリース時にCHANGELOG.mdが自動生成され、package.jsonのバージョンが自動更新されてほしい。

**この優先度の理由**: リリースノートとパッケージバージョンの一貫性を保つことは、利用者への情報提供と依存関係管理のために不可欠。

**独立したテスト**: リリース後、CHANGELOG.mdとpackage.jsonが正しく更新されていることを確認することで検証可能。

**受け入れシナリオ**:

1. **前提条件** feat:とfix:コミットがmainにマージ済み、**操作** リリースワークフローが実行される、**期待結果** CHANGELOG.mdにFeaturesとBug Fixesセクションが追加される
2. **前提条件** リリースワークフローが完了、**操作** package.jsonを確認、**期待結果** バージョンが新しいリリースバージョンに更新されている
3. **前提条件** BREAKING CHANGEコミットがマージ済み、**操作** CHANGELOG.mdを確認、**期待結果** BREAKING CHANGESセクションが記載されている
4. **前提条件** リリースが完了、**操作** Gitタグを確認、**期待結果** vX.Y.Z形式のタグが自動作成されている

---

### ユーザーストーリー 3 - npm registryとGitHub Releasesへの自動公開 (優先度: P1)

開発者として、リリース時に自動的にnpm registryとGitHub Releasesに公開されてほしい。

**この優先度の理由**: パッケージの配布を自動化することで、手動公開の手間とミスを削減できる。

**独立したテスト**: リリース後、npm registryとGitHub Releasesに新バージョンが公開されていることを確認することで検証可能。

**受け入れシナリオ**:

1. **前提条件** リリースワークフローが成功、**操作** `npm view @akiojin/claude-worktree version`を実行、**期待結果** 最新バージョンが返される
2. **前提条件** リリースワークフローが成功、**操作** GitHubのReleasesページを確認、**期待結果** 新しいリリースが作成され、CHANGELOGが記載されている
3. **前提条件** リリースワークフローが成功、**操作** `bun add @akiojin/claude-worktree`を実行、**期待結果** 最新バージョンがインストールされる

---

### ユーザーストーリー 4 - リリース対象外コミットの適切な処理 (優先度: P2)

開発者として、docs:やchore:などのリリース対象外コミットのみがmainにマージされた場合、リリースがスキップされてほしい。

**この優先度の理由**: 不要なバージョンアップを防ぎ、セマンティックバージョニングの原則を守るために重要。

**独立したテスト**: docs:コミットのみをmainにマージし、リリースがスキップされることを確認することで検証可能。

**受け入れシナリオ**:

1. **前提条件** 前回リリースがv1.2.0、**操作** `docs: README更新`のみをmainにマージ、**期待結果** リリースワークフローは実行されるが、バージョンアップはスキップされる
2. **前提条件** 前回リリースがv1.2.0、**操作** `chore: 依存関係更新`のみをmainにマージ、**期待結果** npm registryへの公開は行われない
3. **前提条件** リリース対象外コミットのみマージ、**操作** ワークフローログを確認、**期待結果** "No release published"というメッセージが記録されている

---

### エッジケース

- 複数のBREAKING CHANGEコミットがある場合でも、majorバージョンアップは1回のみ実行される
- feat:とfix:が混在する場合、最も大きな変更（feat: → minor）が適用される
- コミットメッセージが規約に従っていない場合、そのコミットはリリース対象外として扱われる
- リリースワークフロー実行中に新しいコミットがmainにプッシュされた場合、次回のmainプッシュ時に再度リリースワークフローが実行される

## 要件 *(必須)*

### 機能要件

- **FR-001**: mainブランチへのプッシュ時に、semantic-releaseが自動的に実行され**なければならない**
- **FR-002**: feat:コミットはminorバージョンアップをトリガーし**なければならない**
- **FR-003**: fix:コミットはpatchバージョンアップをトリガーし**なければならない**
- **FR-004**: BREAKING CHANGEまたはfeat!:コミットはmajorバージョンアップをトリガーし**なければならない**
- **FR-005**: docs:、chore:、test:などのコミットはバージョンアップをトリガーし**てはならない**
- **FR-006**: リリース時にCHANGELOG.mdが自動生成され**なければならない**
- **FR-007**: リリース時にpackage.jsonのバージョンが自動更新され**なければならない**
- **FR-008**: リリース時にnpm registryへの公開が自動実行され**なければならない**
- **FR-009**: リリース時にGitHub Releasesが自動作成され**なければならない**
- **FR-010**: リリース時にvX.Y.Z形式のGitタグが自動作成され**なければならない**

### 非機能要件

- **NFR-001**: リリースワークフローは、CIテストとビルドが成功した後に実行され**なければならない**
- **NFR-002**: リリースワークフローの失敗時、エラーメッセージが明確に記録され**なければならない**
- **NFR-003**: semantic-release関連の依存パッケージがdevDependenciesに含まれ**なければならない**

### 主要エンティティ

- **semantic-release**: コミットメッセージを分析し、バージョン決定、CHANGELOG生成、公開を自動実行するツール
- **コミットメッセージ規約**: Conventional Commits形式（feat:、fix:、BREAKING CHANGE:など）
- **リリースワークフロー**: GitHub Actionsで定義された自動リリースプロセス（`.github/workflows/release.yml`）
- **CHANGELOG.md**: リリースノートを記録するMarkdownファイル
- **package.json**: パッケージのバージョン情報とメタデータを含むファイル

## 成功基準 *(必須)*

### 測定可能な成果

- **SC-001**: feat:コミットをmainにマージした時、自動的にminorバージョンがアップされ、npm registryとGitHub Releasesに公開される
- **SC-002**: fix:コミットをmainにマージした時、自動的にpatchバージョンがアップされる
- **SC-003**: BREAKING CHANGEコミットをmainにマージした時、自動的にmajorバージョンがアップされる
- **SC-004**: docs:のみのコミットをmainにマージした時、リリースがスキップされる
- **SC-005**: リリース後、CHANGELOG.mdとpackage.jsonが自動更新され、Gitにコミットされる
- **SC-006**: リリース成功率が95%以上を維持する

## 制約と仮定 *(該当する場合)*

### 制約

- GitHub Actionsワークフローファイル（`.github/workflows/release.yml`）が存在すること
- NPM_TOKENとGITHUB_TOKENがGitHub Secretsに設定されていること
- mainブランチへのプッシュ時にCIワークフロー（test、lint）が実行されること

### 仮定

- 開発者はConventional Commits形式でコミットメッセージを書いている
- PRマージは主にsquash mergeまたはmerge commitで行われる
- semantic-releaseの.releaserc.jsonファイルが適切に構成されている

## 範囲外 *(必須)*

次の項目は、この機能の範囲外です：

- コミットメッセージの自動検証（commitlintなど）
- プレリリース版（beta、alpha）の自動管理
- 手動でのバージョン指定機能
- 他のパッケージレジストリ（GitHub Packages、Yarnなど）への公開
- リリースノートのカスタムテンプレート

## セキュリティとプライバシーの考慮事項 *(該当する場合)*

- NPM_TOKENはGitHub Secretsで管理され、ログに出力され**てはならない**
- GITHUB_TOKENは最小限の権限（contents: write、issues: write、pull-requests: write）のみを持つべき
- リリースワークフローは、CIテストが成功した場合のみ実行されるべき
- 公開されるパッケージには機密情報（APIキー、トークンなど）が含まれ**てはならない**

## 依存関係 *(該当する場合)*

### 技術的依存関係

- semantic-release（本体）
- @semantic-release/commit-analyzer（コミット分析）
- @semantic-release/release-notes-generator（リリースノート生成）
- @semantic-release/changelog（CHANGELOG生成）
- @semantic-release/npm（npm公開）
- @semantic-release/git（Git操作）
- @semantic-release/github（GitHub Releases作成）

### 環境依存関係

- GitHub Actions（ワークフロー実行環境）
- Bun 1.0+（ビルドとテスト実行環境）
- npm registry（パッケージ公開先）
- GitHub Releases（リリースノート公開先）

## 参考資料 *(該当する場合)*

- semantic-release公式ドキュメント: [https://semantic-release.gitbook.io/](https://semantic-release.gitbook.io/)
- Conventional Commits仕様: [https://www.conventionalcommits.org/](https://www.conventionalcommits.org/)
- GitHub Actions ワークフロー構文: [https://docs.github.com/en/actions/using-workflows/workflow-syntax-for-github-actions](https://docs.github.com/en/actions/using-workflows/workflow-syntax-for-github-actions)
- セマンティックバージョニング: [https://semver.org/](https://semver.org/)
