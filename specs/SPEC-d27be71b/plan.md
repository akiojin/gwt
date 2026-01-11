# 実装計画: Ink.js から OpenTUI への移行

**仕様ID**: `SPEC-d27be71b` | **日付**: 2026-01-10 | **仕様書**: [specs/SPEC-d27be71b/spec.md](specs/SPEC-d27be71b/spec.md)
**入力**: `/specs/SPEC-d27be71b/spec.md` からの機能仕様

## 概要

- CLI UI を Ink.js + React から OpenTUI + SolidJS に移行する。
- 移行は段階的に進めるが、リリース時には Ink.js 依存を残さない。
- OpenTUI v1.0 前でも移行を進めるため、品質ゲート（自動テスト維持 + 性能ベンチ合格 + Windows ネイティブ動作確認）を必須とする。
- Zig ビルドを CI に組み込み、配布物はネイティブバイナリ同梱でユーザーに Zig を要求しない。

## 技術コンテキスト

**言語/バージョン**: TypeScript 5.8 + Bun >= 1.0
**主要な依存関係**: Ink.js (現行), React 19, @opentui/core, @opentui/solid, solid-js
**ストレージ**: ファイル/ローカル Git メタデータ（DB なし）
**テスト**: Vitest, ink-testing-library, Playwright
**ターゲットプラットフォーム**:

- macOS / Linux: 全ターミナル
- Windows: Windows Terminal, PowerShell 7+ のみ（**cmd.exe は対象外**）

**プロジェクトタイプ**: 単一プロジェクト（CLI + Web）、**両方を SolidJS に統一**
**パフォーマンス目標**:

- **5000+ ブランチ**でスムーズなスクロール（**60fps 以上**）
- 入力反映レイテンシ: **16ms 以下**
- 起動時間: 調査後に設定

**制約**:

- Ink.js をリリースに残さない（移行完了と同時に**即座削除**）
- Zig ビルド必須
- ログ/画面出力分離（specs/SPEC-b9f5c4a1 参照）
- **Zig コアはブラックボックス**として扱い、TypeScript レイヤーのみメンテ
- **テストカバレッジ 100% 維持必須**
- OpenTUI の内蔵コンソールは無効化し、UI オーバーレイを表示しない

**スケール/範囲**: CLI UI（15 スクリーン + 10 以上のコンポーネント）と関連テスト 307+ 件

## 原則チェック

*ゲート: フェーズ0の調査前に合格する必要があります。フェーズ1の設計後に再チェック。*

- `.specify/memory/constitution.md` はテンプレート状態のため追加原則なし。
- CLAUDE.md の規約（Spec Kit/TDD/シンプルさ/ドキュメント規約）を適用。
- 現時点で矛盾なし。ゲート合格。

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-d27be71b/
├── plan.md              # このファイル（/speckit.plan コマンド出力）
├── research.md          # フェーズ0出力（/speckit.plan コマンド）
├── data-model.md        # フェーズ1出力（/speckit.plan コマンド）
├── quickstart.md        # フェーズ1出力（/speckit.plan コマンド）
├── contracts/           # フェーズ1出力（/speckit.plan コマンド）
└── tasks.md             # フェーズ2出力（/speckit.tasks コマンド - /speckit.planでは作成されません）
```

### ソースコード（リポジトリルート）

```text
src/
├── cli/
│   └── ui/
│       ├── components/
│       │   ├── common/
│       │   ├── parts/
│       │   └── screens/
│       ├── screens/
│       ├── hooks/
│       ├── utils/
│       └── __tests__/
├── web/
└── ...

tests/
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 要件に基づいて技術スタックを決定し、既存のコードパターンを理解する

**出力**: `specs/SPEC-d27be71b/research.md`

### 調査項目

1. **既存のコードベース分析**
   - `src/cli/ui` の画面/部品/フック構成
   - Ink.js 依存箇所と置換対象の洗い出し
   - 既存の UI テストと性能テストの所在

2. **技術的決定**
   - OpenTUI + SolidJS の採用
   - Zig ビルドを CI へ導入
   - テスト/ベンチマークを維持する移行方針

3. **追加調査（インタビュー結果）**
   - **仮想スクロール**: OpenTUI が 5000+ ブランチの仮想スクロールをサポートしているか確認
   - **テストフレームワーク**: OpenTUI/OpenCode のテスト方法を調査し、solid-testing-library の評価
   - **npm 配布方法**: OpenCode の具体的な配布方法（optionalDependencies vs postinstall）を調査
   - **性能ベースライン**: 現行 Ink.js での 5000 ブランチスクロール fps と入力レイテンシを計測

4. **制約と依存関係**
   - Windows Terminal / PowerShell 7+ での動作保証（cmd.exe は対象外）
   - 配布物に Zig バイナリを同梱
   - Ink.js 依存の完全撤去（移行完了と同時に即座削除）

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前に技術設計を定義する

**出力**:
- `specs/SPEC-d27be71b/data-model.md`
- `specs/SPEC-d27be71b/quickstart.md`
- `specs/SPEC-d27be71b/contracts/` （外部契約は無し）

### 1.1 データモデル設計

**ファイル**: `data-model.md`

- UI スクリーン遷移/選択状態/フィルタ等の状態管理
- 既存の型定義（`src/cli/ui/types.ts`）に沿ったモデル整理

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`

- Zig セットアップ手順
- Bun を使ったビルド/テスト/実行
- Windows 向けの注意点

### 1.3 契約/インターフェース（該当する場合）

**ディレクトリ**: `contracts/`

- 外部 API 契約は無し
- UI と CLI コアのインターフェース境界のみ整理

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書 + 設計ドキュメント

**出力**: `specs/SPEC-d27be71b/tasks.md` - 実装のための実行可能なタスクリスト

## 実装戦略

### 優先順位付け

1. **P1（最優先）**: **BranchListScreen** - 最も使用頻度が高いメイン画面
2. **P2**: 共通コンポーネント（Header, Footer, ScrollableList, Stats）
3. **P3**: 単純なスクリーン（LoadingIndicator, Confirm, Input）
4. **P4**: 残りのスクリーン（Log 系、Selector 系、Environment/Profile 等）
5. **P5**: **ヘルプオーバーレイ** - OpenTUI のレイヤー描画機能を活用した最初の新機能

### 独立したデリバリー

- スクリーン単位で移行し、機能単位でリリース可能な状態を維持
- 移行期間中のユーザー影響は許容可能（不安定さを伝えても OK）
- ただし最終リリース時は Ink.js 依存を残さない

### 完了基準

1. **全スクリーン移行完了**: 15 スクリーン + 共通コンポーネントすべて
2. **テスト全パス**: 307+ テストすべてが SolidJS 版でパス
3. **性能ベンチマーク合格**: 5000+ ブランチで 60fps、入力 16ms 以下
4. **ユーザーテスト完了**: 実際のユーザーによる動作確認
5. **Ink.js 依存削除**: React/Ink.js 関連の依存をすべて削除

### Go/No-Go 判定

- **各 Phase 完了時**に Go/No-Go を判定
- Phase 2 中間（7-8 スクリーン完了時）にも中間判定を実施

## テスト戦略

- **テストフレームワーク**: OpenTUI/OpenCode のテスト方法を**調査してから決定**
  - Vitest 継続が有力だが、solid-testing-library の評価が必要
- **ユニットテスト**: 既存の 307+ UI テストを OpenTUI 向けに移植し**100% 維持**
- **ユニットテスト**: OpenTUI の内蔵コンソール無効化設定が適用されることを検証
- **統合テスト**: 画面遷移/入力操作の統合テストを維持
- **エンドツーエンドテスト**: Playwright（Web）への影響は維持
- **パフォーマンステスト**:
  - 5000+ ブランチでのスクロール fps 計測
  - 入力レイテンシ計測（16ms 以下）
  - Ink.js ベースライン比で悪化しないことを確認
- **Windows 動作確認**: Windows Terminal / PowerShell 7+ での動作確認をゲート化

## リスクと緩和策

### 技術的リスク

1. **OpenTUI の成熟度**: 仕様変更や API 変更リスク
   - **緩和策**: 各 Phase 完了時に Go/No-Go を判定

2. **Zig 依存**: CI/配布でのビルド失敗
   - **緩和策**: 早期に Zig ビルドを CI へ導入し、配布物同梱を検証

3. **テスト移行コスト**: 307+ テストの置換
   - **緩和策**: 100% 維持必須。既存テストを優先的に移植

4. **仮想スクロール未対応の場合**
   - **緩和策**: 自前実装を検討

5. **Zig コアのバグ**
   - **緩和策**: 上流への Issue 報告、TypeScript レイヤーでの回避策

### 依存関係リスク

1. **Windows ネイティブ互換性**
   - **緩和策**: Windows Terminal / PowerShell 7+ での動作確認を移行判定に含める

2. **OpenTUI プロジェクト放棄**
   - **緩和策**: **OpenTUI をフォーク**して自前メンテナンス（決定済み）
   - Zig コアはブラックボックスとして扱い、TypeScript レイヤーのみメンテ

## 次のステップ

1. ⏳ フェーズ0: 調査（research.md を作成）
   - [ ] OpenTUI の仮想スクロールサポートを確認
   - [ ] OpenTUI/OpenCode のテスト方法を調査
   - [ ] Ink.js の性能ベースラインを計測（5000 ブランチ）
   - [ ] npm 配布方法を調査（OpenCode の具体的な実装）
2. ⏭️ フェーズ1: 設計（data-model.md, quickstart.md を作成）
3. ⏭️ `/speckit.tasks` を実行してタスクを生成
4. ⏭️ `/speckit.implement` で実装を開始

## 追加作業（移行完了後）

- [ ] **Solid DevTools 導入**: 開発時のデバッグ体験向上
- [ ] **pino ログ強化**: 構造化ログの改善
- [ ] **Web UI の SolidJS 統一**: CLI 完了後に実施
- [ ] **ドキュメント更新**: README, CLAUDE.md の更新
