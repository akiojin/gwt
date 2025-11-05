# タスク: ヘッダーへの起動ディレクトリ表示

**入力**: `/specs/SPEC-e07f3844/` からの設計ドキュメント
**前提条件**: plan.md、spec.md、research.md、quickstart.md

**テスト**: この機能はUI表示の変更であり、手動テストとビルドテストで検証します。

**構成**: タスクはユーザーストーリーごとにグループ化され、各ストーリーの独立した実装とテストを可能にします。

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（例: US1、US2）
- 説明に正確なファイルパスを含める

## Commitlintルール

- コミットメッセージは件名のみを使用し、空にしてはいけません（`commitlint.config.cjs`の`subject-empty`ルール）。
- 件名は100文字以内に収めてください（`subject-max-length`ルール）。
- タスク生成時は、これらのルールを満たすコミットメッセージが書けるよう変更内容を整理してください。

## Lint最小要件

- `.github/workflows/lint.yml` に対応するため、以下のチェックがローカルで成功することをタスク完了条件に含めてください。
  - `bun run format:check`
  - `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore`
  - `bun run lint`

## フェーズ1: セットアップ（ビルド確認）

**目的**: 実装前の環境確認

### セットアップタスク

- [x] **T001** [共通] ビルドとlintの初期確認: `bun run build && bun run lint && bun run format:check`

## フェーズ2: ユーザーストーリー1 - 現在の作業ディレクトリの即座確認 (優先度: P1)

**ストーリー**: 複数のプロジェクトを並行して開発している開発者が、claude-worktreeを起動した際に、どのプロジェクトディレクトリで作業しているかを即座に確認できる。

**価値**: 誤ったディレクトリでの操作を防ぐために不可欠なコア機能

### UIコンポーネント変更

- [x] **T101** [US1] Header.tsxのHeaderPropsインターフェースに`workingDirectory?: string`プロパティを追加: `src/ui/components/parts/Header.tsx`
- [x] **T102** [US1] Header.tsxのHeader関数コンポーネントのprops分割代入に`workingDirectory`を追加: `src/ui/components/parts/Header.tsx`
- [x] **T103** [US1] T102の後にHeader.tsxのレンダリングロジックでdivider直後にworking directory表示を追加: `src/ui/components/parts/Header.tsx`

### データフロー実装

- [x] **T104** [P] [US1] App.tsxでprocess.cwd()を使用して起動ディレクトリを取得: `src/ui/components/App.tsx`
- [x] **T105** [P] [US1] BranchListScreen.tsxのBranchListScreenPropsインターフェースに`workingDirectory?: string`を追加: `src/ui/components/screens/BranchListScreen.tsx`
- [x] **T106** [US1] BranchListScreen.tsxのBranchListScreen関数コンポーネントのprops分割代入に`workingDirectory`を追加: `src/ui/components/screens/BranchListScreen.tsx`

### 統合

- [x] **T107** [US1] T104とT106の後にApp.tsxからBranchListScreenへworkingDirectoryをpropsで渡す: `src/ui/components/App.tsx`
- [x] **T108** [US1] T103とT107の後にBranchListScreenからHeaderへworkingDirectoryをpropsで渡す: `src/ui/components/screens/BranchListScreen.tsx`

### ビルドとLint確認

- [x] **T109** [US1] T108の後にTypeScriptビルドを実行してエラーがないことを確認: `bun run build`
- [x] **T110** [US1] T109の後にlintとフォーマットチェックを実行: `bun run lint && bun run format:check`

### 手動テスト

- [x] **T111** [US1] T110の後に/home/user/project-aから起動してWorking Directory表示を確認
- [x] **T112** [US1] T110の後に/var/www/project-bから起動してWorking Directory表示を確認
- [x] **T113** [US1] T110の後に深いディレクトリ階層から起動して完全な絶対パス表示を確認

**注記**: 手動テストの代わりに自動テストを追加しました:
- `tests/unit/ui/components/Header.test.tsx`: 14テストケース（すべて成功）
- `tests/integration/ui/BranchListScreen-workingDirectory.test.tsx`: 10テストケース（すべて成功）

**✅ MVP1チェックポイント**: US1完了後、起動ディレクトリが即座に確認可能

## フェーズ3: ユーザーストーリー2 - 視認性の高い配置 (優先度: P2)

**ストーリー**: ユーザーがclaude-worktreeのUI画面を開いた瞬間に、起動ディレクトリ情報が自然に目に入る位置に配置されている。

**価値**: ディレクトリ情報の表示位置がユーザビリティに直結する視覚的改善

### 視覚的確認

- [ ] **T201** [US2] ヘッダー表示順序の確認: タイトル行→区切り線→Working Directory→統計情報
- [ ] **T202** [US2] 80文字幅のターミナルで表示確認（折り返しの有無）

### エッジケース検証

- [ ] **T203** [US2] 100文字超の長いパスでの表示確認（折り返し動作）
- [ ] **T204** [US2] シンボリックリンク経由での起動確認（実パス表示）
- [ ] **T205** [US2] 特殊文字（スペース、日本語）を含むパスでの表示確認

**✅ MVP2チェックポイント**: US2完了後、すべての表示要件が満たされる

## フェーズ4: 統合とポリッシュ

**目的**: 最終確認とドキュメント整備

### 最終確認

- [ ] **T301** [統合] すべてのCIチェックをローカルで実行: `bun run type-check && bun run lint && bun run test && bun run build`
- [ ] **T302** [統合] markdownlintチェックを実行: `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore`
- [ ] **T303** [統合] 複数の異なるディレクトリで動作確認

### コミット＆プッシュ

- [ ] **T304** [統合] T301-T303完了後に変更をコミット（Conventional Commits形式）
- [ ] **T305** [統合] T304の後にfeature/update-uiブランチへプッシュ

## タスク凡例

**優先度**:

- **P1**: 最も重要 - MVP1に必要（即座のディレクトリ確認）
- **P2**: 重要 - MVP2に必要（視認性の高い配置）

**依存関係**:

- **[P]**: 並列実行可能
- **[依存なし]**: 他のタスクの後に実行

**ストーリータグ**:

- **[US1]**: ユーザーストーリー1（現在の作業ディレクトリの即座確認）
- **[US2]**: ユーザーストーリー2（視認性の高い配置）
- **[共通]**: すべてのストーリーで共有
- **[統合]**: 複数ストーリーにまたがる

## 並列実行の機会

### フェーズ2（US1）での並列化

**並列グループ1**: UIコンポーネントとデータソース
```bash
# 並列実行可能
T101-T103: Header.tsx の変更（UIコンポーネント）
T104: App.tsx でのprocess.cwd()取得（データソース）
T105: BranchListScreenProps の型定義追加
```

これらは異なるファイルまたは独立した変更箇所のため、並列実行可能です。

**順次実行が必要**: 統合タスク
```bash
# T107-T108は依存関係があるため順次実行
T107: App.tsx → BranchListScreen のprops渡し
T108: BranchListScreen → Header のprops渡し
```

### フェーズ3（US2）での並列化

**並列グループ2**: 手動テストケース
```bash
# 並列実行可能（異なる環境/ディレクトリ）
T203: 長いパステスト
T204: シンボリックリンクテスト
T205: 特殊文字テスト
```

## 依存関係グラフ

```
T001 (初期ビルド確認)
  ↓
US1: T101 → T102 → T103 (Header.tsx変更)
US1: T104 (App.tsx でcwd取得) [並列]
US1: T105 → T106 (BranchListScreen props) [並列]
  ↓
US1: T107 (App → BranchListScreen propsリレー)
  ↓
US1: T108 (BranchListScreen → Header propsリレー)
  ↓
US1: T109 (ビルド確認) → T110 (lint確認)
  ↓
US1: T111, T112, T113 (手動テスト) [並列可能]
  ↓
US2: T201, T202 (視覚的確認) [並列可能]
US2: T203, T204, T205 (エッジケーステスト) [並列可能]
  ↓
統合: T301 (最終ビルド) → T302 (markdownlint) → T303 (総合確認)
  ↓
統合: T304 (コミット) → T305 (プッシュ)
```

## 実装戦略

### MVP1（ユーザーストーリー1のみ）

**スコープ**: T001-T113

**成果物**:

- Header.tsx: `workingDirectory`プロパティ対応
- BranchListScreen.tsx: `workingDirectory`プロパティリレー
- App.tsx: `process.cwd()`でディレクトリ取得とprops渡し

**価値**: 起動ディレクトリの即座確認が可能（コア機能）

**検証**:

- ビルド成功
- 異なるディレクトリでの起動確認

### MVP2（ユーザーストーリー2追加）

**スコープ**: T201-T205

**追加価値**: 視認性の高い配置とエッジケース対応

**検証**:

- 表示順序の確認
- 長いパス、シンボリックリンク、特殊文字での動作確認

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
- **スキップしたタスク**: 理由と共に文書化

## 注記

- 各タスクは15分から1時間で完了可能
- ファイルパスは正確で、プロジェクト構造と一致
- この機能はUI表示のため、手動テストが主要な検証手段
- TypeScriptの型チェックで多くのエラーを事前防止
- 既存のHeader.tsxのReact.memo最適化を維持

## タスクサマリー

**総タスク数**: 24

**フェーズ別内訳**:

- フェーズ1（セットアップ）: 1タスク
- フェーズ2（US1）: 13タスク
- フェーズ3（US2）: 5タスク
- フェーズ4（統合）: 5タスク

**並列実行機会**: 7タスク（T104, T105, T111-T113, T203-T205）

**MVPスコープ**:

- MVP1: T001-T113（14タスク）
- MVP2: +T201-T205（+5タスク）
