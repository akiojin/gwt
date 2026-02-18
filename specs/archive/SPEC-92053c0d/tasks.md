# タスク: commitlint を npm ci 無しで実行可能にする

**入力**: `/specs/SPEC-92053c0d/`
**前提条件**: plan.md、spec.md

## フォーマット: `[ID] [P?] [ストーリー] 説明`

## フェーズ2: ユーザーストーリー1 - node_modules が無くても commitlint が実行できる (優先度: P1)

- [ ] **T101** [P] [US1] `scripts/commitlint-config.test.cjs` にフォールバック読込テストを追加
- [ ] **T102** [US1] `commitlint.config.cjs` にフォールバック設定と安全な parserPreset 解決を追加
- [ ] **T103** [US1] `node scripts/commitlint-config.test.cjs` を実行して成功を確認

## フェーズ3: ユーザーストーリー2 - 既存の commitlint ルールは維持される (優先度: P2)

- [ ] **T201** [US2] `commitlint.config.cjs` のルール上書きが維持されることを確認

## フェーズ5: 統合とポリッシュ

- [ ] **T401** [統合] `bunx commitlint --from HEAD~1 --to HEAD` を実行し npm ci 無しで動作確認
