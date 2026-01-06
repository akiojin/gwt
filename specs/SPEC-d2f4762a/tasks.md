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
- [x] **T104** [US0] `tests/unit/index.entrypoint.test.ts` に相対パス実行でもエントリ判定が通ることを確認するテストを追加
- [x] **T105** [US0] `src/index.ts` のエントリポイント判定を `fileURLToPath` + `path.resolve` で正規化する
- [x] **T106** [P] [共通] `specs/SPEC-d2f4762a/spec.md` にリモート取得の停止時もUIが継続する要件とシナリオを追記
- [x] **T107** [P] [共通] `specs/SPEC-d2f4762a/plan.md` に非ブロッキングフェッチの実装方針を追記
- [x] **T108** [US0] `src/cli/ui/__tests__/hooks/useGitData.nonblocking.test.tsx` にフェッチが解決しなくてもローディングが解除されるテストを追加
- [x] **T109** [US0] `tests/unit/git.fetchAllRemotes.test.ts` に`fetchAllRemotes`がタイムアウト/非対話設定を渡すテストを追加
- [x] **T110** [US0] `src/git.ts` と `src/cli/ui/hooks/useGitData.ts` を更新し、フェッチのタイムアウトと非ブロッキングを実装
- [x] **T111** [US0] `src/cli/ui/__tests__/components/screens/BranchListScreen.test.tsx` に選択中ブランチのフルパス表示（`Branch: ...`）と空表示時の`Branch: (none)`のテストを追加
- [x] **T112** [US0] `src/cli/ui/components/screens/BranchListScreen.tsx` にフッターヘルプ直上のフルパス表示と固定行数調整を実装

## フェーズ3: 統合とポリッシュ

- [ ] **T201** [統合] `package.json` に従い `bun run format:check` / `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore` / `bun run lint` を実行し、失敗があれば修正

## フェーズ4: ユーザーストーリー6 - Worktree作成時のstaleディレクトリ自動回復

**ストーリー**: Worktree作成対象パスがstale状態で残っていても、自動回復して作成を完了する。

**価値**: 手動削除の手戻りをなくし、ブランチ選択から作業開始までを途切れさせない。

### テスト（TDD）

- [ ] **T301** [US6] `tests/integration/branch-creation.test.ts` にstaleディレクトリを検出して削除→再作成できるテストを追加
- [ ] **T302** [US6] `tests/integration/branch-creation.test.ts` にstale判定できない既存ディレクトリは削除せずエラーになるテストを追加

### 実装

- [ ] **T303** [US6] `src/worktree.ts` にstale判定・削除処理を追加し、`createWorktree`の前処理として実行
- [ ] **T304** [US6] `src/worktree.ts` に判定不能な既存ディレクトリ向けの明確なエラーメッセージを追加
