---
description: "npm postinstall ダウンロード安定化の実装タスク"
---

# タスク: npm postinstall ダウンロード安定化

**入力**: `/specs/SPEC-f59c553d/`
**前提条件**: plan.md / spec.md / research.md / data-model.md / quickstart.md

## フォーマット: `[ID] [P?] [ストーリー] 説明`

## フェーズ1: ユーザーストーリー1 - 初回実行での安定ダウンロード (優先度: P1)

**ストーリー**: 初回のpostinstallで確実にバイナリを取得できる

**価値**: 再実行が不要になり、初回体験が安定する

### 実装

- [ ] **T101** [US1] `scripts/postinstall.js` にバージョンタグURL生成関数とGitHub APIフォールバック関数を追加
- [ ] **T102** [US1] `scripts/postinstall.js` にHTTP 404/403/5xx + network errorのリトライ（最大5回、初回0.5秒、倍率2、上限5秒）を追加

### テスト

- [ ] **T103** [P] [US1] `scripts/postinstall.test.js` にURL生成・リトライ判定のユニットテストを追加

**✅ MVP1チェックポイント**: US1完了で初回取得の成功率が改善される

## フェーズ2: ユーザーストーリー2 - 失敗時の英語ガイダンス (優先度: P2)

**ストーリー**: 失敗時に英語の復旧ガイドを表示する

**価値**: 再試行で解決しない場合の自己復旧が可能になる

### 実装

- [ ] **T201** [US2] `scripts/postinstall.js` のエラーメッセージをバージョン付きURL案内に更新（`latest` へはフォールバックしない）

### テスト

- [ ] **T202** [P] [US2] `scripts/postinstall.test.js` に失敗時メッセージ検証を追加

## フェーズ3: 統合とポリッシュ

### 統合

- [ ] **T401** [統合] `node --test scripts/postinstall.test.js` を実行し結果を記録
- [ ] **T402** [統合] `bun run format:check` / `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore` / `bun run lint` を実行し失敗時は修正
