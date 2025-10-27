# 実装計画: UI移行 - Ink.js（React）ベースのCLIインターフェース

**仕様ID**: `SPEC-4c2ef107` | **日付**: 2025-01-25 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-4c2ef107/spec.md` からの機能仕様

**注**: このテンプレートは `/speckit.plan` コマンドによって記入されます。実行ワークフローについては `.specify/templates/commands/plan.md` を参照してください。

## 概要

claude-worktreeの現在のCLI UIを、inquirer/chalkベースからInk.js（React）ベースに全面移行する。主要目標は以下の通り：

- **保守性向上**: 2000行超のprompts.tsを約70%削減（760行以下）
- **リアルタイム更新**: 統計情報の動的更新機能を追加
- **全画面レイアウト**: ヘッダー固定、スクロール可能コンテンツ、フッター固定
- **TDD対応**: Vitestでコンポーネント単体テスト（80%以上カバレッジ）
- **既存機能維持**: すべての既存機能を後退なく移行

## 技術コンテキスト

**言語/バージョン**: TypeScript（既存プロジェクトに準拠）
**ランタイム**: bun（Node.jsではない - プロジェクト制約）
**主要な依存関係**:

- ink: ^5.0.0（メインUIフレームワーク）
- react: ^18.3.1（Inkの依存関係）
- ink-select-input: ^6.0.0（選択UI） - [要確認: スクロール機能の詳細]
- ink-text-input: ^6.0.0（テキスト入力）
- @types/react: ^18.3.0（TypeScript型定義）
- 既存の依存関係（chalk, execa等）は保持

**ストレージ**: ファイルベース（セッション管理に使用、既存実装を維持）
**テスト**: Vitest（既存のテストフレームワーク） - [要確認: Inkコンポーネントのテスト方法]
**ターゲットプラットフォーム**: bun実行環境（Linux/macOS/Windows CLI）
**プロジェクトタイプ**: 単一プロジェクト（CLIツール）
**パフォーマンス目標**:

- 起動時間: <1秒
- スクロールレスポンス: <50ms
- リサイズ反応: <500ms
- 100+ブランチでもスムーズ動作

**制約**:

- bun実行環境必須（Node.js不可）
- 既存のGit/Worktreeロジックは変更不可（UIレイヤーのみ）
- 段階的移行可能な設計
- Vitest使用（新しいテストツール導入不可）
- Ink UIはTTY制御を一元化し、セッション終了時にraw modeを確実に解除すること（子プロセス起動時の入力欠落を防ぐ）

**スケール/範囲**:

- 画面数: 7画面（BranchList, WorktreeManager, BranchCreator, PRCleanup, AIToolSelector, SessionSelector, ExecutionModeSelector）
- コンポーネント数: 約15-20個（共通コンポーネント含む）
- コード削減目標: 2522行 → 760行以下（70%削減）

## 原則チェック

*ゲート: フェーズ0の調査前に合格する必要があります。フェーズ1の設計後に再チェック。*

### プロジェクト原則（CLAUDE.mdより）

#### ✅ I. シンプルさの追求（譲れない）
**状態**: 合格
**理由**: コード70%削減（2522行 → 760行以下）を目標に設定。Ink.jsのコンポーネントベース設計により、既存の複雑なUIロジックを大幅に簡素化できる。

#### ✅ II. UX/DX品質（譲れない）
**状態**: 合格
**理由**: 全画面レイアウト、スクロール対応、リアルタイム更新により、既存UIより優れた開発者体験を提供。キーボード操作は既存と同じため、学習コストなし。

#### ✅ III. TDD必須（譲れない）
**状態**: 合格 - [要確認: Inkコンポーネントのテスト戦略]
**理由**: Vitestで80%以上のテストカバレッジを目標に設定。ただし、Inkコンポーネントのテスト方法（レンダリング、インタラクション）を調査する必要あり。

#### ✅ IV. bun実行環境（譲れない）
**状態**: 合格 - [要確認: Ink.jsのbun互換性]
**理由**: 技術コンテキストでbunを明記。Ink.jsがbun環境で正常動作することを調査で確認する必要あり。

#### ✅ V. 既存ファイル改修優先
**状態**: 合格
**理由**: 新規ディレクトリ（src/ui/components/）を作成するが、既存のGit/Worktreeロジックは変更せず、UIレイヤーのみ置き換え。段階的移行により既存コードを徐々に削除。

#### ✅ VI. コード品質
**状態**: 合格
**理由**: markdownlint、commitlint対応を継続。TypeScriptの既存lint設定を維持。

### ゲート評価

**Phase 0開始前**: ✅ **合格**
- すべての原則に準拠
- 2つの要確認項目（Inkのbun互換性、テスト戦略）はPhase 0の調査で解決

**Phase 1開始前の再チェック**: Phase 0完了後に実施

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-4c2ef107/
├── spec.md              # 機能仕様書（完成）
├── plan.md              # このファイル（/speckit.plan コマンド出力）
├── research.md          # フェーズ0出力（/speckit.plan コマンド）
├── data-model.md        # フェーズ1出力（/speckit.plan コマンド）
├── quickstart.md        # フェーズ1出力（/speckit.plan コマンド）
├── contracts/           # フェーズ1出力（該当なし - CLIツールのため）
├── tasks.md             # フェーズ2出力（/speckit.tasks コマンド）
└── checklists/
    └── requirements.md  # 仕様書品質チェック（完成）
```

### ソースコード（リポジトリルート）

```text
src/
├── ui/
│   ├── components/           # 新規: Inkコンポーネント
│   │   ├── App.tsx          # メインアプリケーション
│   │   ├── screens/         # 画面コンポーネント
│   │   │   ├── BranchListScreen.tsx
│   │   │   ├── WorktreeManagerScreen.tsx
│   │   │   ├── BranchCreatorScreen.tsx
│   │   │   ├── PRCleanupScreen.tsx
│   │   │   ├── AIToolSelectorScreen.tsx
│   │   │   ├── SessionSelectorScreen.tsx
│   │   │   └── ExecutionModeSelectorScreen.tsx
│   │   ├── parts/           # UI部品
│   │   │   ├── Header.tsx
│   │   │   ├── Footer.tsx
│   │   │   ├── Stats.tsx
│   │   │   └── ScrollableList.tsx
│   │   └── common/          # 共通コンポーネント
│   │       ├── Select.tsx
│   │       ├── Confirm.tsx
│   │       ├── Input.tsx
│   │       └── ErrorBoundary.tsx
│   ├── hooks/               # カスタムフック
│   │   ├── useGitData.ts
│   │   ├── useTerminalSize.ts
│   │   └── useScreenState.ts
│   ├── types.ts             # 既存（型定義）
│   ├── legacy/              # 段階的移行中のみ存在
│   │   ├── display.ts       # 既存をリネーム
│   │   ├── prompts.ts       # 既存をリネーム
│   │   └── table.ts         # 既存をリネーム
│   └── __tests__/           # 新規: UIテスト
│       ├── components/      # コンポーネント単体テスト
│       └── integration/     # 統合テスト
├── git.ts                   # 既存（変更なし）
├── worktree.ts              # 既存（変更なし）
├── services/                # 既存（変更なし）
├── repositories/            # 既存（変更なし）
└── index.ts                 # 既存（Ink起動に変更）

tests/
├── ui/                      # UIテスト（新規）
└── ...                      # 既存テスト（維持）
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 要件に基づいて技術スタックを決定し、既存のコードパターンを理解する

**出力**: `specs/SPEC-4c2ef107/research.md`

### 調査項目

#### 1. Ink.jsのbun互換性検証
**優先度**: P1（ブロッカー）
**タスク**:
- Ink.js v5.0.0がbun環境で正常動作するか検証
- サンプルアプリケーションで動作確認
- 既知の問題やワークアラウンドの調査

**成果物**:
- 互換性レポート（動作確認結果）
- 必要に応じて代替案（Ink v4.xへのダウングレード等）

#### 2. ink-select-inputのスクロール機能調査
**優先度**: P1（コア機能）
**タスク**:
- `limit`プロパティによるスクロール実装の確認
- ターミナルサイズ変更時の動的調整方法
- カスタムレンダリングの可能性
- パフォーマンス特性（100+アイテム時）

**成果物**:
- スクロール実装ガイド
- サンプルコード

#### 3. Ink.jsコンポーネントのテスト戦略
**優先度**: P1（TDD必須）
**タスク**:
- Vitestでのレンダリングテスト方法
- ink-testing-libraryの調査
- インタラクション（キーボード入力）のテスト方法
- スナップショットテストの適用可能性
- 80%カバレッジ達成のための戦略

**成果物**:
- テスト戦略ドキュメント
- サンプルテストコード

#### 4. 既存UIコードのパターン分析
**優先度**: P2（移行戦略）
**タスク**:
- `src/ui/prompts.ts`の機能分解
- `src/ui/display.ts`のメッセージ表示パターン
- `src/ui/table.ts`のブランチ整形ロジック
- 各画面の状態管理パターン
- エラーハンドリングパターン

**成果物**:
- 既存コード機能マップ
- Inkコンポーネントへのマッピング表

#### 5. 段階的移行戦略の設計
**優先度**: P2（リスク軽減）
**タスク**:
- フィーチャーフラグ実装方法
- 既存コードとInk UIの共存方法
- ロールバック戦略
- テスト並行実行方法

**成果物**:
- 移行ステップ計画
- ロールバック手順

#### 6. パフォーマンス最適化のベストプラクティス
**優先度**: P3（最適化）
**タスク**:
- React.memoの効果的な使用
- useMemo/useCallbackの適用ポイント
- 大量データ（100+ブランチ）の処理パターン
- メモリリーク防止策

**成果物**:
- パフォーマンスガイドライン

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前に技術設計を定義する

**出力**:
- `specs/SPEC-4c2ef107/data-model.md`
- `specs/SPEC-4c2ef107/quickstart.md`
- `specs/SPEC-4c2ef107/contracts/` （該当なし - CLIツールのため）

### 1.1 データモデル設計

**ファイル**: `data-model.md`

主要なエンティティとその関係を定義：

#### Screen（画面）
- 種類: BranchList | WorktreeManager | BranchCreator | PRCleanup | AIToolSelector | SessionSelector | ExecutionModeSelector
- 状態: active | hidden
- 遷移ルール

#### BranchItem（ブランチアイテム）
- name: string（ブランチ名）
- type: "local" | "remote"
- branchType: "feature" | "hotfix" | "release" | "main" | "develop" | "other"
- icons: string[]（表示用アイコン配列）
- worktreeStatus?: "active" | "inaccessible"
- hasChanges: boolean

#### Statistics（統計情報）
- localCount: number
- remoteCount: number
- worktreeCount: number
- changesCount: number
- 更新頻度: リアルタイム（P3実装時）

#### Layout（レイアウト）
- headerLines: number（ヘッダーの行数）
- footerLines: number（フッターの行数）
- contentHeight: number（コンテンツ領域の高さ）
- 動的計算: ターミナルサイズ変更時

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`

開発者向けの簡潔なガイド：

1. **セットアップ手順**
   - 依存関係インストール: `bun add ink react ink-select-input ink-text-input @types/react`
   - 開発環境起動: `bun run build && bunx .`

2. **開発ワークフロー**
   - TDD: テスト作成 → 実装 → リファクタリング
   - コンポーネント作成: 共通 → 部品 → 画面
   - テスト実行: `bun test`

3. **よくある操作**
   - 新規コンポーネント作成
   - テスト追加
   - 既存コードの移行

4. **トラブルシューティング**
   - Inkのレンダリング問題
   - bunとの互換性問題
   - テスト失敗時の対処

> ⚠️ Markdown整形ヒント
> - 入れ子のリストは 2 スペースでインデントする（`-` の後に 2 スペース）
> - URL は必ず `[表示名](https://example.com)` の形式で記述し、裸URLは避ける
> - コードブロックやコマンド例には適切な言語ラベルを付ける（例: ```bash）

### 1.3 契約/インターフェース

**該当なし**: CLIツールのため、API契約は不要

代わりに、以下のコンポーネントインターフェースを定義：

#### 共通コンポーネントProps
```typescript
// Select.tsx
interface SelectProps {
  items: Array<{ label: string; value: string }>;
  onSelect: (value: string) => void;
  limit?: number;
}

// Confirm.tsx
interface ConfirmProps {
  message: string;
  onConfirm: (result: boolean) => void;
  defaultValue?: boolean;
}

// Input.tsx
interface InputProps {
  prompt: string;
  onSubmit: (value: string) => void;
  defaultValue?: string;
}
```

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書 + 設計ドキュメント

**出力**: `specs/[SPEC-xxxxxxxx]/tasks.md` - 実装のための実行可能なタスクリスト

## 実装戦略

### 優先順位付け

ユーザーストーリーの優先度に基づいて実装：

1. **P1**: ブランチ一覧表示と選択（最も重要 - MVP）
   - 全画面レイアウトの基盤
   - スクロール機能
   - キーボードナビゲーション
   - **完了条件**: ブランチ選択と起動が動作

2. **P2**: サブ画面のナビゲーション（拡張MVP）
   - 各サブ画面の実装
   - 画面遷移管理
   - 状態管理
   - **完了条件**: すべての既存機能が動作

3. **P3**: リアルタイム統計情報の更新（完全版）
   - 統計情報の自動更新
   - パフォーマンス最適化
   - **完了条件**: 統計が自動更新される

### 独立したデリバリー

各ユーザーストーリーは独立して実装・テスト・デプロイ可能：

- **ストーリー1完了** → デプロイ可能なMVP（ブランチ選択のみ）
- **ストーリー2追加** → 拡張MVP（全機能動作）
- **ストーリー3追加** → 完全な機能（リアルタイム更新）

### 段階的移行アプローチ

1. **Phase A**: 新UIの並行実装（既存UI維持）
2. **Phase B**: フィーチャーフラグでの切り替え
3. **Phase C**: 新UIをデフォルト化
4. **Phase D**: 既存UI削除（レガシーコード削除）

## テスト戦略

**TDD必須**: すべてのコンポーネントはテストファーストで実装

### ユニットテスト（80%カバレッジ目標）

**範囲**:
- すべてのReactコンポーネント（rendering）
- カスタムフック（useGitData, useTerminalSize, useScreenState）
- ユーティリティ関数（アイコン生成、レイアウト計算等）

**アプローチ**:
- Vitest + ink-testing-library
- コンポーネントのレンダリングテスト
- Props変更時の再レンダリング検証
- インタラクション（キーボード入力）のシミュレーション

**例**:
```typescript
describe('BranchListScreen', () => {
  it('should render branch list with scroll', () => {
    const { lastFrame } = render(<BranchListScreen branches={mockBranches} />);
    expect(lastFrame()).toContain('⚡ main');
  });
});
```

### 統合テスト

**範囲**:
- 画面遷移フロー
- 複数コンポーネントの連携
- 状態管理の統合

**アプローチ**:
- 実際のGitリポジトリでのテスト
- ユーザーフローの再現（選択 → 遷移 → 戻る）

### エンドツーエンドテスト

**範囲**:
- 完全なユーザーシナリオ
- 実際のWorktree操作

**アプローチ**:
- テスト用Gitリポジトリを使用
- 既存の統合テストを維持
- 新UIでの動作確認

### パフォーマンステスト

**範囲**:
- 100+ブランチでのレンダリング
- スクロールのレスポンス時間
- メモリ使用量

**基準**:
- レンダリング: <1秒
- スクロール: <50ms
- メモリ: <100MB増加

## リスクと緩和策

### 技術的リスク

1. **Ink.jsがbunで動作しない**
   - **影響**: プロジェクト全体がブロック
   - **可能性**: 低（調査で確認）
   - **緩和策**:
     - Phase 0で早期に検証
     - 代替案: Ink v4.x、または別のUIライブラリ
     - 最悪の場合: 既存inquirerを維持し、部分的な改善に留める

2. **テストカバレッジ80%が達成できない**
   - **影響**: 品質目標未達
   - **可能性**: 中
   - **緩和策**:
     - Phase 0でテスト戦略を確立
     - TDD徹底で初期からカバレッジ確保
     - 複雑な部分を優先的にテスト

3. **コード70%削減が達成できない**
   - **影響**: 保守性向上の効果が薄い
   - **可能性**: 低（React/Inkのコンポーネント化で大幅削減見込み）
   - **緩和策**:
     - 共通コンポーネントの最大活用
     - 重複コードの徹底排除
     - 目標を段階的に調整（60%削減でも価値あり）

4. **既存機能の後退（regression）**
   - **影響**: ユーザー体験の悪化
   - **可能性**: 中
   - **緩和策**:
     - 統合テストで既存機能を網羅
     - フィーチャーフラグで段階的移行
     - ロールバック計画を用意

### 依存関係リスク

1. **ink-select-inputのスクロール機能が期待通りでない**
   - **影響**: コア機能の実装が困難
   - **可能性**: 低
   - **緩和策**:
     - Phase 0で詳細調査
     - カスタムレンダリングで対応
     - 代替ライブラリの検討（ink-spinner等）

2. **Reactのバージョン互換性問題**
   - **影響**: ビルドエラー、実行時エラー
   - **可能性**: 低
   - **緩和策**:
     - package.jsonで明示的なバージョン指定
     - peerDependenciesの確認
     - Phase 0で検証

## 次のステップ

1. ✅ **Phase 0完了**: 調査と技術スタック決定
   - ✅ research.md作成
   - ✅ Ink.jsのbun互換性調査完了（要実地検証）
   - ✅ ink-select-inputのスクロール機能確認
   - ✅ テスト戦略の確立（ink-testing-library + Vitest）
   - ✅ 既存コードパターン分析（70%削減見込み）

2. ✅ **Phase 1完了**: 設計とアーキテクチャ定義
   - ✅ data-model.md作成
   - ✅ quickstart.md作成
   - ✅ コンポーネントインターフェース定義

3. ⏭️ **Phase 2実施**: `/speckit.tasks` を実行してタスクを生成
   - 実装可能なタスクリストの作成
   - 優先度付けと依存関係の明確化

4. ⏭️ **実装開始**: `/speckit.implement` で実装を開始
   - TDD徹底
   - P1 → P2 → P3の順で実装

---

**現在の状態**: ✅ Phase 1完了。次は `/speckit.tasks` でタスク生成。
