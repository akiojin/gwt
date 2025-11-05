# タスク: アプリケーションバージョン表示機能

**入力**: `/specs/SPEC-207ccae7/` からの設計ドキュメント
**前提条件**: plan.md（✅）、spec.md（✅）、research.md（✅）、data-model.md（✅）、contracts/（✅）

**テスト**: CLAUDE.mdの指針に従い、TDD/SDD手法を適用するため、テストタスクを含めます。

**構成**: タスクはユーザーストーリーごとにグループ化され、各ストーリーの独立した実装とテストを可能にします。

## フォーマット: `- [ ] [ID] [P?] [ストーリー] 説明`

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

## 実装戦略

### MVP配信

- **MVP1（P1完了後）**: CLIフラグでのバージョン表示機能
  - `claude-worktree --version` でバージョンを即座に確認可能
  - トラブルシューティングやバグレポートで最も重要な基本機能

- **MVP2（P2完了後）**: UIヘッダーでのバージョン表示機能
  - 全画面でバージョンを視覚的に確認可能
  - ユーザビリティの向上

### 独立したデリバリー

各ユーザーストーリーは独立してテスト・デプロイ可能：
- US1完了 → CLIフラグ機能が動作（デプロイ可能）
- US2完了 → UIヘッダー機能が追加（完全な機能）

## フェーズ1: 基礎タスク

**目的**: 両方のユーザーストーリーで使用される共通の前提条件

### 前提条件確認

- [x] **T001** [P] [共通] 既存の`getPackageVersion()`関数の動作確認（`src/utils.ts:48-60`）
- [x] **T002** [P] [共通] package.jsonの存在とversionフィールドの確認

## フェーズ2: ユーザーストーリー1 - CLIフラグでバージョン確認 (優先度: P1)

**ストーリー**: CLIユーザーとして、`--version`または`-v`フラグを使用してアプリケーションのバージョンを素早く確認したい。

**価値**: トラブルシューティングやバグレポートで最も頻繁に使用される基本的なCLI標準機能。

**独立したテスト**: `claude-worktree --version`コマンドを実行することで完全にテストでき、即座にバージョン情報を取得できる。

### TDD: テストファースト

- [x] **T101** [P] [US1] `src/utils.test.ts`に`getPackageVersion()`のユニットテストを作成
  - 正常系: package.jsonが存在し、versionフィールドがある
  - 異常系: package.jsonが存在しない
  - 異常系: versionフィールドが存在しない
- [x] **T102** [P] [US1] `src/index.test.ts`に`showVersion()`関数のユニットテストを作成
  - バージョン取得成功時の標準出力確認
  - バージョン取得失敗時のエラーメッセージと終了コード確認

### 実装: CLIフラグ機能

- [x] **T103** [US1] T102の後に`src/index.ts`に`showVersion()`関数を実装
  - `getPackageVersion()`を呼び出し
  - 成功時: バージョンをstdoutに出力
  - 失敗時: エラーメッセージをstderrに出力 + `process.exit(1)`
- [x] **T104** [US1] T103の後に`src/index.ts`の`main()`関数のCLI引数パース処理を修正
  - `--version`と`-v`フラグの検出を追加
  - `showVersion()`を呼び出して早期リターン
  - `--help`よりも優先度を高く配置
- [x] **T105** [P] [US1] `src/index.ts`の`showHelp()`関数を更新
  - ヘルプメッセージに`-v, --version    Show version information`を追加

### TDD: テスト実行と修正

- [x] **T106** [US1] T105の後にT101とT102のテストを実行し、すべてパスすることを確認
  - 失敗したテストがあれば実装を修正
- [x] **T107** [US1] T106の後に統合テストを実行
  - `bun run build && bunx . --version` の実行
  - `bun run build && bunx . -v` の実行
  - 出力が期待されるバージョン番号と一致することを確認

### 動作確認

- [x] **T108** [US1] T107の後にローカルで動作確認
  - `bunx . --version` でバージョンが表示される
  - `bunx . -v` でバージョンが表示される
  - `bunx . --version --help` で`--version`のみ処理される（優先度確認）
  - エラーケース: package.jsonをリネームして実行 → エラーメッセージ表示

**✅ MVP1チェックポイント**: US1完了後、CLIフラグでのバージョン確認機能が独立して動作し、デプロイ可能

## フェーズ3: ユーザーストーリー2 - UIヘッダーでバージョン確認 (優先度: P2)

**ストーリー**: UIユーザーとして、アプリケーション使用中にメイン画面のヘッダーで現在のバージョンを確認したい。

**価値**: サポート問い合わせ時やバグレポート時に正確なバージョンを伝えることができる。UIでのユーザビリティ向上。

**独立したテスト**: アプリケーションのメインUIを起動し、ヘッダー部分を視覚的に確認することでテストできる。

### TDD: テストファースト

- [ ] **T201** [P] [US2] `src/ui/components/parts/Header.test.tsx`にHeaderコンポーネントのユニットテストを作成
  - `version`プロップありの場合のレンダリング確認（`"Title v1.12.3"`形式）
  - `version`プロップなしの場合のレンダリング確認（タイトルのみ）
  - `version={null}`の場合のレンダリング確認（タイトルのみ）

### 実装: Headerコンポーネント拡張

- [ ] **T202** [US2] T201の後に`src/ui/components/parts/Header.tsx`のHeaderPropsインターフェースを拡張
  - `version?: string | null`プロップを追加
  - JSDocコメントを追加
- [ ] **T203** [US2] T202の後に`src/ui/components/parts/Header.tsx`のレンダリングロジックを修正
  - `version ? \`\${title} v\${version}\` : title` の条件分岐を実装
  - React.memoの動作を維持

### 実装: App.tsx バージョン取得

- [ ] **T204** [US2] T203の後に`src/ui/components/App.tsx`にバージョン取得ロジックを追加
  - `useState<string | null>(null)`でversion状態を管理
  - `useEffect`内で`getPackageVersion()`を呼び出し
  - 取得したバージョンをstateに保存

### 実装: 各画面コンポーネントへのversionプロップ追加

- [ ] **T205** [P] [US2] `src/ui/components/screens/BranchListScreen.tsx`のHeaderに`version`プロップを追加
- [ ] **T206** [P] [US2] `src/ui/components/screens/BranchCreatorScreen.tsx`のHeaderに`version`プロップを追加
- [ ] **T207** [P] [US2] `src/ui/components/screens/WorktreeManagerScreen.tsx`のHeaderに`version`プロップを追加
- [ ] **T208** [P] [US2] `src/ui/components/screens/SessionSelectorScreen.tsx`のHeaderに`version`プロップを追加
- [ ] **T209** [P] [US2] `src/ui/components/screens/PRCleanupScreen.tsx`のHeaderに`version`プロップを追加
- [ ] **T210** [P] [US2] `src/ui/components/screens/ExecutionModeSelectorScreen.tsx`のHeaderに`version`プロップを追加
- [ ] **T211** [P] [US2] `src/ui/components/screens/AIToolSelectorScreen.tsx`のHeaderに`version`プロップを追加

### TDD: テスト実行と修正

- [ ] **T212** [US2] T211の後にT201のテストを実行し、すべてパスすることを確認
  - 失敗したテストがあれば実装を修正

### 動作確認

- [ ] **T213** [US2] T212の後にローカルで動作確認
  - `bun run build && bunx .` でメインUIを起動
  - ブランチ一覧画面のヘッダーに`"Claude Worktree v1.12.3"`と表示される
  - 各画面（7画面）のヘッダーにバージョンが表示される
  - エラーケース: package.jsonをリネームして起動 → ヘッダーにバージョンなし（タイトルのみ）、アプリは正常動作

**✅ MVP2チェックポイント**: US2完了後、UIヘッダーでのバージョン確認機能が追加され、完全な機能が実現

## フェーズ4: 統合とポリッシュ

**目的**: すべてのユーザーストーリーを統合し、プロダクション準備を整える

### 統合テスト

- [ ] **T301** [統合] エンドツーエンドの統合テストを実行
  - US1とUS2の両方の機能が正常に動作することを確認
  - `bunx . --version` → バージョン表示 → 終了
  - `bunx .` → UIヘッダーにバージョン表示 → 正常動作
- [ ] **T302** [統合] エッジケースと境界条件のテスト
  - package.json不在時の動作（US1: エラーメッセージ、US2: バージョンなしで動作）
  - versionフィールド不在時の動作
  - 不正なJSON形式のpackage.json
  - プレリリースバージョン（例: `"2.0.0-beta.1"`）の表示

### 品質チェック

- [ ] **T303** [統合] `.github/workflows/test.yml`に合わせてローカルで品質チェックを実行
  - `bun run type-check` - TypeScriptの型チェック
  - `bun run lint` - ESLintチェック
  - `bun run test` - すべてのテストを実行
  - `bun run test:coverage` - テストカバレッジ確認
  - `bun run build` - ビルド成功確認
  - 失敗時は修正してすべてパスするまで繰り返す
- [ ] **T304** [統合] `.github/workflows/lint.yml`に合わせてローカルでフォーマットチェックを実行
  - `bun run format:check` - コードフォーマットチェック
  - `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore` - Markdownリントチェック
  - 失敗時は修正してすべてパスするまで繰り返す

### ドキュメント更新

- [ ] **T305** [P] [ドキュメント] `README.md`または`README.ja.md`にバージョン確認方法を追加
  - CLIフラグの使用方法（`claude-worktree --version`）
  - UIヘッダーでの確認方法
- [ ] **T306** [P] [ドキュメント] `CHANGELOG.md`に機能追加を記録（存在する場合）
  - バージョン表示機能の追加を記載

### コミット準備

- [ ] **T307** [統合] すべての変更をステージング
  - `git add src/index.ts src/utils.ts src/utils.test.ts src/index.test.ts`
  - `git add src/ui/components/parts/Header.tsx src/ui/components/parts/Header.test.tsx`
  - `git add src/ui/components/App.tsx src/ui/components/screens/*.tsx`
  - `git add README.md`（または該当するドキュメント）
- [ ] **T308** [統合] コミットメッセージを作成（commitlintルール準拠）
  - 件名: `feat: add version display in CLI flag and UI header`
  - 本文: US1とUS2の実装内容を簡潔に説明
  - フッター: `SPEC-207ccae7` を参照

## タスク凡例

**優先度**:
- **P1**: 最も重要 - MVP1に必要（CLIフラグ）
- **P2**: 重要 - MVP2に必要（UIヘッダー）

**並列実行**:
- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **依存関係あり**: "TxxxのあとにYYY" と明記

**ストーリータグ**:
- **[US1]**: ユーザーストーリー1（CLIフラグ）
- **[US2]**: ユーザーストーリー2（UIヘッダー）
- **[共通]**: すべてのストーリーで共有
- **[統合]**: 複数ストーリーにまたがる
- **[ドキュメント]**: ドキュメント専用

## 依存関係グラフ

### ユーザーストーリーの依存関係

```
[共通] → [US1] → [US2] → [統合]
          ↓              ↓
        MVP1           MVP2
```

- **共通**: 両方のストーリーで使用される前提条件
- **US1**: 独立して実装・テスト・デプロイ可能
- **US2**: US1とは独立（並行開発可能）
- **統合**: すべてのストーリー完了後

### タスクの並列実行例

**US1フェーズ内の並列実行**:
```
T101（utils.test.ts）   T102（index.test.ts）
     ↓                        ↓
T103, T104, T105（並列実行可能 - 異なるセクション）
     ↓
T106, T107, T108（順次実行）
```

**US2フェーズ内の並列実行**:
```
T201（Header.test.tsx）
     ↓
T202, T203（Header.tsx）
     ↓
T204（App.tsx）
     ↓
T205-T211（7画面を並列実行可能）
     ↓
T212, T213（順次実行）
```

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
- **スキップしたタスク**: 理由と共に文書化

## タスクサマリー

- **総タスク数**: 36タスク
- **US1タスク数**: 10タスク（T001-T002共通 + T101-T108）
- **US2タスク数**: 15タスク（T201-T213、画面修正7タスク含む）
- **統合タスク数**: 8タスク（T301-T308）
- **並列実行可能タスク**: 15タスク（[P]マーク付き）
- **推奨MVPスコープ**: US1のみ（T001-T108）

## 実装の注意事項

### TypeScript型定義

- すべての新規関数・インターフェースに適切な型定義を追加
- `string | null`型を使用してエラー状態を明示
- オプショナルプロップは`?`で明示

### エラーハンドリング

- CLIフラグ: エラー時は`process.exit(1)`で終了
- UIヘッダー: エラー時もアプリケーションは継続動作

### コードスタイル

- 既存のコードスタイルに従う（Prettier、ESLint）
- Chalkを使用した色付け（既存パターンに倣う）
- React.memoの最適化を維持

### テスト戦略

- TDDアプローチ: テストファースト、Red-Green-Refactorサイクル
- ユニットテスト: 各関数・コンポーネント単位
- 統合テスト: CLIとUIの実際の動作確認
- エンドツーエンドテスト: ユーザーシナリオに基づく

## 次のステップ

1. ✅ タスクリスト生成完了
2. ⏭️ `/speckit.analyze` で品質分析を実行
3. ⏭️ `/speckit.implement` で実装を開始
4. ⏭️ 各タスクを順次実行し、チェックボックスを更新
5. ⏭️ すべてのタスク完了後、プルリクエストを作成

---

**最終更新**: 2025-10-31
**ステータス**: タスクリスト準備完了
**推奨開始点**: T001（共通の前提条件確認）
