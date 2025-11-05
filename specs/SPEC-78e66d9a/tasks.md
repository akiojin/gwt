# タスク: semantic-release リリースワークフローの安定化

**入力**: `/specs/SPEC-78e66d9a/` の設計ドキュメント  
**フォーマット**: `- [ ] T001 [P] [US?] 説明 (ファイルパス)`

## フェーズ1: セットアップ

- [x] T001 失敗した GitHub Actions 実行ログをダウンロードし根本原因を整理する (`ci/logs/release-19107220150.md`)
- [x] T002 ローカル環境で LoadingIndicator テストを複数回実行し再現状況を記録する (`src/ui/__tests__/components/common/LoadingIndicator.test.tsx`)

## フェーズ2: 基盤整備

- [x] T101 現行 LoadingIndicator 実装のタイマー処理を確認しコメント化された仮説を整理する (`src/ui/components/common/LoadingIndicator.tsx`)
- [x] T102 [P] テスト用ユーティリティで DOM 取得方法と `data-testid` 使用を確認する (`src/ui/__tests__/components/common/LoadingIndicator.test.tsx`)

## フェーズ3: ユーザーストーリー1 (P1) - リリース失敗の再発防止

- [x] T201 [US1] テストで `vi.useFakeTimers()` を導入し delay/interval を制御する (`src/ui/__tests__/components/common/LoadingIndicator.test.tsx`)
- [x] T202 [P] [US1] `advanceTimersByTime` ヘルパーを作成し DOM 更新を `act` と同期させる (`src/ui/__tests__/components/common/LoadingIndicator.test.tsx`)
- [x] T203 [US1] LoadingIndicator が delay=0 でも確実に可視化されるよう必要に応じて実装を調整する (`src/ui/components/common/LoadingIndicator.tsx`)
- [x] T204 [P] [US1] ターゲットテストと全体テスト (`bun run test`) を実行し安定性を確認する (プロジェクトルート)

## フェーズ4: ユーザーストーリー2 (P2) - スピナー挙動の検証可能性向上

- [x] T301 [US2] フレーム循環を追加アサーションで検証し単一要素・複数要素両方をカバーする (`src/ui/__tests__/components/common/LoadingIndicator.test.tsx`)
- [x] T302 [P] [US2] `quickstart.md` に再現手順と解決手順を反映する (`specs/SPEC-78e66d9a/quickstart.md`)

## フェーズ5: 最終確認

- [ ] T401 Markdownlint とフォーマット検証を実施しドキュメント品質を確認する (`specs/SPEC-78e66d9a/*.md`)
- [ ] T402 CI 成功確認: release ワークフローを再実行し結果を記録する (`.github/workflows/release.yml`)
