> **ℹ️ TUI MIGRATION NOTE**: This SPEC describes backend/gwt-core functionality unaffected by the gwt-tui migration (SPEC-1776). No changes required.
> **Canonical Boundary**: `SPEC-1647` は `SPEC-1787` に superseded された履歴 SPEC である。現行の workspace initialization と SPEC-first workflow は `SPEC-1787` を参照する。

### 背景
プロジェクトの開閉・作成・マイグレーション・最近のプロジェクト管理を行う。Studio時代の #1557（プロジェクトライフサイクル管理）と #1558（マルチプロジェクト切替）の機能概念を現行スタックで再定義。

### 境界
- Local Git backend semantics（branch/ref/worktree inventory、cleanup、local Git cache invalidation）は `#1644` が正本
- 本仕様は project open/close/create/switch と recent project lifecycle の orchestration のみを扱う

### ユーザーシナリオとテスト

**S1: プロジェクトを開く**
- Given: ユーザーがプロジェクトディレクトリを選択
- When: Open操作を実行
- Then: プロジェクトが読み込まれ、ターミナルが起動する

**S2: 最近のプロジェクト**
- Given: 過去に開いたプロジェクトがある
- When: 最近のプロジェクト一覧を表示
- Then: 最近開いたプロジェクトが時系列で表示される

**S3: プロジェクト作成**
- Given: 新規プロジェクトを作成したい
- When: 新規作成操作を行う
- Then: ディレクトリが作成され、初期化される

**S4: プロジェクト切替**
- Given: 複数プロジェクトが開かれている
- When: 別プロジェクトに切替
- Then: 対象プロジェクトのウィンドウが前面に来る

### 機能要件

**FR-01: プロジェクト開閉**
- ディレクトリ選択による開始
- セッション保存・復元連携
- local Git backend state の意味論・projection・invalidate は `#1644` を参照する

**FR-02: プロジェクト作成**
- 新規ディレクトリ作成
- Git初期化

**FR-03: 最近のプロジェクト**
- 履歴管理
- クイックアクセス

**FR-04: マイグレーション**
- 古い設定形式からの移行

### 成功基準

1. プロジェクトの開閉が正常に動作する
2. 最近のプロジェクトが正しく管理される
3. プロジェクト切替がスムーズに動作する

---
