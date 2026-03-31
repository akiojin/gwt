> **ℹ️ TUI MIGRATION NOTE**: This SPEC describes backend/gwt-core functionality unaffected by the gwt-tui migration (SPEC-1776). No changes required.

### 背景
GitHub Issue/Spec 探索、PR 管理、Release/Tag 表示、VersionHistoryPanel、gh CLI 連携を包含する GitHub 連携機能。Studio 時代の #1544（GitHub連携）の機能概念を現行スタックで再定義する。

`#1684` で分離されていた GitHub Issue 検索責務は、本 Issue を canonical spec として再統合する。現行実装に独立した PR index は存在せず、ユーザー導線として必要なのは Git 側からの Issue/Spec 探索である。Project Index の files/index/recovery は `#1520` が正本とし、Issue タイトル解決・local cache・GitHub linkage は `#1714` が正本とする。

Issue/Spec の detail rendering contract は `#1354` が正本であり、本 Issue は discovery/search 導線と検索基盤のみを扱う。

### 境界
- local Git backend（git CLI wrapper、branch/ref/worktree inventory、local projection/cache invalidation）は `#1644` が正本
- Issue title lookup / exact cache / GitHub linkage は `#1714` が正本
- 本仕様は GitHub API を使う discovery/search/version history/release-tag 導線だけを扱う

### ユーザーシナリオとテスト

**S1: Issue/Spec 一覧表示**
- Given: GitHub リポジトリが接続済み
- When: Git 側の Issues/Specs 画面を開く
- Then: Issue または Spec の一覧を閲覧し、適切な detail view へ遷移できる

**S2: unified search**
- Given: Issue と Spec が存在する
- When: `#1684` / `SPEC 1684` / タイトル / ラベル / 自然言語で検索する
- Then: 1つの検索欄から Issue/Spec を探せる

**S3: semantic spec search**
- Given: spec index が更新済み
- When: 概念語で検索する
- Then: gwt-spec を中心とした semantic match が返る

**S4: PR 作成・管理**
- Given: 変更がブランチにコミット済み
- When: PR 作成操作を行う
- Then: gh CLI で PR が作成・管理できる

**S5: リリース・タグ表示（バージョン履歴）**
- Given: リポジトリにリリースが存在する
- When: VersionHistoryPanel を表示する
- Then: GitHub Releases/Tags が時系列で表示される

### 機能要件

**FR-01: Issue/Spec discovery**
- Issue/Spec 一覧表示と detail view への遷移導線
- 1つの検索欄で `#1234` / `SPEC 1234` / title / label / semantic search を扱う
- Files index の UI/責務は `#1520` を参照し、本 spec は持たない

**FR-02: semantic spec index**
- gwt-spec の index 更新操作を Git 側から実行できる
- semantic 検索結果は unified search に統合して表示する
- index 元データと更新ポリシーは `#1714` の local issue cache を参照する

**FR-03: Issue tab integration**
- Issue タブ画面の detail/list flow は `#1354` を参照する
- 本 spec は検索基盤・発見導線のみを正本として扱う

**FR-04: PR 管理基盤**
- PR 一覧/ステータス表示
- PR 管理 SPEC と連携

**FR-05: Release/Tag 管理**
- GitHub Releases 表示
- Tags 表示
- CHANGELOG 生成連携

**FR-06: バージョン履歴**
- VersionHistoryPanel
- リリースノート表示

**FR-07: gh CLI 連携**
- gh コマンド統合
- 認証管理

### 成功基準

1. Issue/Spec 探索が Git 側の単一導線で完結する
2. Files index の正本が `#1520` に固定され、責務が混ざらない
3. semantic spec search と通常 Issue 検索が同じ検索欄から使える
4. Issue タブ詳細は `#1354`、Issue title lookup/cache は `#1714` を参照する関係が明確である
5. PR / Release / Version History 既存導線に回帰がない
