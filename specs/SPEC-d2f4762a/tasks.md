# タスク: ブランチアクション画面のWSL入力互換性改善

**仕様ID**: `SPEC-d2f4762a`
**ポリシー**: CLAUDE.md の TDD ルールに基づき、必ず RED→GREEN→リグレッションチェックの順に進める。

## フェーズ1: セットアップ（共有インフラストラクチャ）

### セットアップタスク

- [x] **T001** [P] [共通] `specs/SPEC-d2f4762a/spec.md` にWSL/Windowsの分割エスケープ対応要件と受け入れシナリオを追記
- [x] **T002** [P] [共通] `specs/SPEC-d2f4762a/plan.md` のハイレベルToDoへ入力互換性の追記

## フェーズ2: ユーザーストーリー0 - ブランチ選択の基本操作（既存機能） (優先度: P0)

**依存関係**: US0 のみ（他ストーリー依存なし）

**ストーリー**: ブランチ選択後のアクション画面でも、矢印キーで選択移動でき、`Esc`は戻る操作として機能する。

**価値**: WSL/Windows環境でも誤って前画面に戻らずにアクション選択ができる。

### テスト（TDD）

- [x] **T101** [US0] `src/cli/ui/screens/__tests__/BranchActionSelectorScreen.test.tsx` に遅延した分割矢印シーケンスでも`Esc`扱いにならないテストを追加

### 実装

- [x] **T102** [US0] `src/cli/ui/hooks/useAppInput.ts` のエスケープシーケンス待機時間を調整し、WSL/Windowsの遅延を許容する
- [x] **T103** [US0] `src/cli/ui/screens/__tests__/BranchActionSelectorScreen.test.tsx` の`Esc`タイムアウト期待値を新しい待機時間に合わせる

## フェーズ3: 統合とポリッシュ

- [ ] **T201** [統合] `package.json` に従い `bun run format:check` / `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore` / `bun run lint` を実行し、失敗があれば修正
