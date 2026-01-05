# タスク: Ink.js から OpenTUI への移行

**入力**: `/specs/SPEC-d27be71b/` からの設計ドキュメント
**前提条件**: plan.md（必須）、spec.md（ユーザーストーリー用に必須）

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（例: US1、US2、US3）
- 説明に正確なファイルパスを含める

## 依存関係マップ

```text
Phase 1 (セットアップ) → Phase 2 (基盤)
                              ↓
Phase 3 (US1: BranchListScreen) ← 最優先
                              ↓
Phase 4 (US2: 共通コンポーネント)
                              ↓
Phase 5 (US3: 単純スクリーン)
                              ↓
Phase 6 (US4: 残りスクリーン)
                              ↓
Phase 7 (US5: ヘルプオーバーレイ)
                              ↓
Phase 8 (統合・仕上げ)
```

## フェーズ1: セットアップ（共有インフラストラクチャ）

**目的**: OpenTUI + SolidJS 環境の初期化とビルド基盤構築

### セットアップタスク

- [x] **T001** [P] [共通] Zig コンパイラのインストール手順を `specs/SPEC-d27be71b/quickstart.md` に文書化
- [x] **T002** [P] [共通] CI/CD に Zig セットアップを追加 `.github/workflows/test.yml`
- [x] **T003** [共通] `package.json` に OpenTUI 依存を追加（@opentui/core, @opentui/solid, solid-js）
- [x] **T004** [共通] T003の後に `bun install` で依存インストールを検証
- [x] **T005** [P] [共通] `tsconfig.solid.json` を作成（SolidJS 用の TypeScript 設定）
- [ ] **T006** [P] [共通] `vite.config.solid.ts` を作成（SolidJS 用の Vite 設定）※ 既存 `src/web/client/vite.config.ts` を Solid 化する方針のため保留
- [x] **T007** [共通] T005,T006の後に OpenTUI の最小サンプルでビルド検証（`@opentui/solid/bun-plugin` で Bun.build 実行済み）
- [x] **T008** [P] [共通] Windows Terminal / PowerShell 7+ での動作確認手順を文書化

## フェーズ2: 基盤（全ストーリー共通）

**目的**: UI フレームワーク非依存のロジック分離と共通インフラ

### 抽象化レイヤー

- [x] **T101** [P] [基盤] `src/cli/ui/core/types.ts` に UI 状態の型定義を整理（フレームワーク非依存）
- [x] **T102** [P] [基盤] `src/cli/ui/core/theme.ts` にテーマシステムを集約（色・アイコン定義）
- [x] **T103** [P] [基盤] `src/cli/ui/core/keybindings.ts` にキーバインド定義を集約
- [x] **T104** [基盤] T101の後に `src/cli/ui/stores/` に SolidJS ストア構造を設計
- [x] **T105** [P] [基盤] 現行 Ink.js の性能ベースラインを計測（5000ブランチ + 入力レイテンシ簡易測定）
- [x] **T106** [基盤] T105の後に `specs/SPEC-d27be71b/research.md` にベースライン結果を記録

**✅ 基盤チェックポイント**: ビルド成功 + ベースライン計測完了で Go/No-Go 判定

## フェーズ3: US1 - BranchListScreen 移行 (優先度: P1)

**ストーリー**: 最も使用頻度が高いメイン画面を OpenTUI に移行

**価値**: コアユースケースの移行により OpenTUI の実用性を検証

### テスト（TDD 必須）

- [x] **T201** [US1] `src/cli/ui/__tests__/solid/BranchListScreen.test.tsx` にテスト骨格を作成
- [x] **T202** [US1] T201の後に BranchListScreen のユニットテストを実装（選択、スクロール、フィルター）

### 実装

- [x] **T203** [US1] T202の後に `src/cli/ui/screens/solid/BranchListScreen.tsx` を作成
- [x] **T204** [US1] T203の後に 5000ブランチでの性能テストを実施（60fps / 16ms 目標）
- [x] **T205** [US1] T204の後に 既存 `src/cli/ui/screens/BranchListScreen.tsx` との機能パリティ確認（結果: 色/cleanupスピナー/フィルターカーソル/DEBUGスタックまで対応済み）
- [x] **T206** [US1] T205の後に Go/No-Go 判定（性能目標達成確認: Go）

**✅ MVP1チェックポイント**: BranchListScreen が OpenTUI で動作し性能目標達成

## フェーズ4: US2 - 共通コンポーネント移行 (優先度: P2)

**ストーリー**: Header, Footer, ScrollableList 等の共通コンポーネントを移行

**価値**: 他スクリーンの移行を加速する共通基盤

### テスト（TDD 必須）

- [x] **T301** [P] [US2] `src/cli/ui/__tests__/solid/components/Header.test.tsx` にテスト作成
- [x] **T302** [P] [US2] `src/cli/ui/__tests__/solid/components/Footer.test.tsx` にテスト作成
- [x] **T303** [P] [US2] `src/cli/ui/__tests__/solid/components/ScrollableList.test.tsx` にテスト作成
- [x] **T304** [P] [US2] `src/cli/ui/__tests__/solid/components/Stats.test.tsx` にテスト作成
- [x] **T305** [P] [US2] `src/cli/ui/__tests__/solid/components/SearchInput.test.tsx` にテスト作成
- [x] **T306** [P] [US2] `src/cli/ui/__tests__/solid/components/SelectInput.test.tsx` にテスト作成
- [x] **T307** [P] [US2] `src/cli/ui/__tests__/solid/components/TextInput.test.tsx` にテスト作成

### 実装

- [x] **T308** [US2] T301の後に `src/cli/ui/components/solid/Header.tsx` を作成
- [x] **T309** [US2] T302の後に `src/cli/ui/components/solid/Footer.tsx` を作成
- [x] **T310** [US2] T303の後に `src/cli/ui/components/solid/ScrollableList.tsx` を作成
- [x] **T311** [US2] T304の後に `src/cli/ui/components/solid/Stats.tsx` を作成
- [x] **T312** [US2] T305の後に `src/cli/ui/components/solid/SearchInput.tsx` を作成
- [x] **T313** [US2] T306の後に `src/cli/ui/components/solid/SelectInput.tsx` を作成
- [x] **T314** [US2] T307の後に `src/cli/ui/components/solid/TextInput.tsx` を作成

### カスタムフック移行

- [x] **T315** [P] [US2] `src/cli/ui/hooks/solid/useKeyHandler.ts` を作成
- [x] **T316** [P] [US2] `src/cli/ui/hooks/solid/useScrollableList.ts` を作成
- [x] **T317** [P] [US2] `src/cli/ui/hooks/solid/useFilter.ts` を作成
- [x] **T318** [P] [US2] `src/cli/ui/hooks/solid/useSelection.ts` を作成
- [ ] **T319** [P] [US2] `src/cli/ui/hooks/solid/useTerminalSize.ts` を作成
- [ ] **T320** [P] [US2] `src/cli/ui/hooks/solid/useAsyncOperation.ts` を作成
- [ ] **T321** [P] [US2] `src/cli/ui/hooks/solid/useGitOperations.ts` を作成

### 統合

- [ ] **T322** [US2] T308-T321の後に 共通コンポーネントの統合テスト実施

**✅ MVP2チェックポイント**: 共通コンポーネントがすべて移行完了

## フェーズ5: US3 - 単純スクリーン移行 (優先度: P3)

**ストーリー**: LoadingIndicator, Confirm, Input 等の単純スクリーンを移行

**価値**: 基本的な UI フローの完成

### テスト（TDD 必須）

- [ ] **T401** [P] [US3] `src/cli/ui/__tests__/solid/screens/LoadingIndicator.test.tsx` にテスト作成
- [ ] **T402** [P] [US3] `src/cli/ui/__tests__/solid/screens/ConfirmScreen.test.tsx` にテスト作成
- [ ] **T403** [P] [US3] `src/cli/ui/__tests__/solid/screens/InputScreen.test.tsx` にテスト作成
- [ ] **T404** [P] [US3] `src/cli/ui/__tests__/solid/screens/ErrorScreen.test.tsx` にテスト作成

### 実装

- [ ] **T405** [US3] T401の後に `src/cli/ui/screens/solid/LoadingIndicator.tsx` を作成
- [ ] **T406** [US3] T402の後に `src/cli/ui/screens/solid/ConfirmScreen.tsx` を作成
- [ ] **T407** [US3] T403の後に `src/cli/ui/screens/solid/InputScreen.tsx` を作成
- [ ] **T408** [US3] T404の後に `src/cli/ui/screens/solid/ErrorScreen.tsx` を作成

### 統合

- [ ] **T409** [US3] T405-T408の後に 単純スクリーンの統合テスト実施
- [ ] **T410** [US3] T409の後に Go/No-Go 中間判定（7-8 スクリーン完了時点）

**✅ MVP3チェックポイント**: 基本 UI フローが OpenTUI で動作

## フェーズ6: US4 - 残りスクリーン移行 (優先度: P4)

**ストーリー**: Log 系、Selector 系、Environment/Profile 等の残りスクリーンを移行

**価値**: 全機能の完全移行

### テスト（TDD 必須）

- [ ] **T501** [P] [US4] `src/cli/ui/__tests__/solid/screens/LogScreen.test.tsx` にテスト作成
- [ ] **T502** [P] [US4] `src/cli/ui/__tests__/solid/screens/LogDetailScreen.test.tsx` にテスト作成
- [ ] **T503** [P] [US4] `src/cli/ui/__tests__/solid/screens/SelectorScreen.test.tsx` にテスト作成
- [ ] **T504** [P] [US4] `src/cli/ui/__tests__/solid/screens/EnvironmentScreen.test.tsx` にテスト作成
- [ ] **T505** [P] [US4] `src/cli/ui/__tests__/solid/screens/ProfileScreen.test.tsx` にテスト作成
- [ ] **T506** [P] [US4] `src/cli/ui/__tests__/solid/screens/SettingsScreen.test.tsx` にテスト作成
- [ ] **T507** [P] [US4] `src/cli/ui/__tests__/solid/screens/WorktreeCreateScreen.test.tsx` にテスト作成
- [ ] **T508** [P] [US4] `src/cli/ui/__tests__/solid/screens/WorktreeDeleteScreen.test.tsx` にテスト作成

### 実装

- [ ] **T509** [US4] T501の後に `src/cli/ui/screens/solid/LogScreen.tsx` を作成
- [ ] **T510** [US4] T502の後に `src/cli/ui/screens/solid/LogDetailScreen.tsx` を作成
- [ ] **T511** [US4] T503の後に `src/cli/ui/screens/solid/SelectorScreen.tsx` を作成
- [ ] **T512** [US4] T504の後に `src/cli/ui/screens/solid/EnvironmentScreen.tsx` を作成
- [ ] **T513** [US4] T505の後に `src/cli/ui/screens/solid/ProfileScreen.tsx` を作成
- [ ] **T514** [US4] T506の後に `src/cli/ui/screens/solid/SettingsScreen.tsx` を作成
- [ ] **T515** [US4] T507の後に `src/cli/ui/screens/solid/WorktreeCreateScreen.tsx` を作成
- [ ] **T516** [US4] T508の後に `src/cli/ui/screens/solid/WorktreeDeleteScreen.tsx` を作成

### App.tsx 移行

- [ ] **T517** [US4] T509-T516の後に `src/cli/ui/App.solid.tsx` を作成（ルーティング/状態管理）
- [ ] **T518** [US4] T517の後に `src/cli/ui/index.solid.ts` を作成（エントリーポイント）

### 統合

- [ ] **T519** [US4] T518の後に 全スクリーンの統合テスト実施
- [ ] **T520** [US4] T519の後に 5000ブランチでの最終性能テスト

**✅ MVP4チェックポイント**: 全スクリーンが OpenTUI で動作

## フェーズ7: US5 - ヘルプオーバーレイ (優先度: P5)

**ストーリー**: OpenTUI のレイヤー描画機能を活用した最初の新機能

**価値**: Ink.js では不可能だったオーバーレイ UI の実現

### テスト（TDD 必須）

- [ ] **T601** [US5] `src/cli/ui/__tests__/solid/components/HelpOverlay.test.tsx` にテスト作成

### 実装

- [ ] **T602** [US5] T601の後に `src/cli/ui/components/solid/HelpOverlay.tsx` を作成
- [ ] **T603** [US5] T602の後に BranchListScreen にヘルプオーバーレイを統合
- [ ] **T604** [US5] T603の後に 他スクリーンにもヘルプオーバーレイを展開

**✅ 新機能チェックポイント**: ヘルプオーバーレイが動作

## フェーズ8: 統合とポリッシュ

**目的**: すべてのストーリーを統合し、プロダクション準備を整える

### Ink.js 依存削除

- [ ] **T701** [統合] 全 OpenTUI スクリーンの動作確認完了後、Ink.js 版コードを削除
- [ ] **T702** [統合] T701の後に `package.json` から ink, ink-select-input, ink-text-input, ink-testing-library を削除
- [ ] **T703** [統合] T702の後に `package.json` から react, react-dom を削除（Web UI 移行まで保持する場合は除く）
- [ ] **T704** [統合] T703の後に 不要になった React 関連の型定義を削除

### テスト移行完了

- [ ] **T705** [統合] 既存 307+ テストの SolidJS 版への移植状況を確認
- [ ] **T706** [統合] T705の後に テストカバレッジ 100% 達成を確認
- [ ] **T707** [統合] T706の後に CI でのテスト実行を確認

### 性能検証

- [ ] **T708** [統合] 5000ブランチでの最終性能ベンチマーク実施
- [ ] **T709** [統合] T708の後に 60fps スクロール目標達成を確認
- [ ] **T710** [統合] T709の後に 16ms 入力レイテンシ目標達成を確認

### Windows 動作確認

- [ ] **T711** [統合] Windows Terminal での動作確認
- [ ] **T712** [統合] PowerShell 7+ での動作確認

### Lint/品質チェック

- [ ] **T713** [統合] `bun run format:check` 成功を確認
- [ ] **T714** [統合] `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore` 成功を確認
- [ ] **T715** [統合] `bun run lint` 成功を確認
- [ ] **T716** [統合] `bun run type-check` 成功を確認
- [ ] **T717** [統合] `bun run build` 成功を確認

### ドキュメント

- [ ] **T718** [P] [ドキュメント] `README.md` に OpenTUI 移行完了を反映
- [ ] **T719** [P] [ドキュメント] `CLAUDE.md` に SolidJS 開発ガイドラインを追加

**✅ 完了基準**: 全テストパス + 性能目標達成 + Ink.js 依存完全削除

## タスク凡例

**優先度**:

- **P1**: 最重要 - BranchListScreen（MVP1）
- **P2**: 重要 - 共通コンポーネント（MVP2）
- **P3**: 標準 - 単純スクリーン（MVP3）
- **P4**: 標準 - 残りスクリーン（MVP4）
- **P5**: 追加 - ヘルプオーバーレイ（新機能）

**依存関係**:

- **[P]**: 並列実行可能
- **[依存なし]**: 他のタスクの後に実行

**ストーリータグ**:

- **[共通]**: セットアップ/インフラ
- **[基盤]**: 全ストーリー共通の基盤
- **[US1]**: BranchListScreen 移行
- **[US2]**: 共通コンポーネント移行
- **[US3]**: 単純スクリーン移行
- **[US4]**: 残りスクリーン移行
- **[US5]**: ヘルプオーバーレイ
- **[統合]**: 複数ストーリーにまたがる
- **[ドキュメント]**: ドキュメント専用

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
- **スキップしたタスク**: 理由と共に文書化

## 要約

| フェーズ | タスク数 | 並列可能 | 依存 |
| ---- | ---- | ---- | ---- |
| Phase 1: セットアップ | 8 | 5 | 3 |
| Phase 2: 基盤 | 6 | 4 | 2 |
| Phase 3: US1 BranchListScreen | 6 | 0 | 6 |
| Phase 4: US2 共通コンポーネント | 22 | 14 | 8 |
| Phase 5: US3 単純スクリーン | 10 | 4 | 6 |
| Phase 6: US4 残りスクリーン | 20 | 8 | 12 |
| Phase 7: US5 ヘルプオーバーレイ | 4 | 0 | 4 |
| Phase 8: 統合 | 19 | 2 | 17 |
| **合計** | **95** | **37** | **58** |

## 推奨 MVP 範囲

- **MVP1**: Phase 1-3 完了（BranchListScreen が OpenTUI で動作）
- **MVP2**: Phase 4 完了（共通コンポーネント移行完了）
- **MVP3**: Phase 5 完了（単純スクリーン移行完了）
- **MVP4**: Phase 6 完了（全スクリーン移行完了）
- **完全版**: Phase 7-8 完了（新機能 + Ink.js 依存削除）

## Go/No-Go 判定ポイント

1. **Phase 2 完了時**: ビルド成功 + ベースライン計測完了
2. **Phase 3 完了時**: BranchListScreen 性能目標達成
3. **Phase 5 完了時**: 7-8 スクリーン完了の中間判定
4. **Phase 8 完了時**: 全テストパス + 性能目標達成 + ユーザーテスト完了
