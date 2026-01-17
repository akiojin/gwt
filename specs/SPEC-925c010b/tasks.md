---
description: "Docker Compose の Playwright noVNC を arm64 で起動可能にするためのタスクリスト"
---

# タスク: Docker Compose の Playwright noVNC を arm64 で起動可能にする

**入力**: `/specs/SPEC-925c010b/` からの設計ドキュメント
**前提条件**: plan.md、spec.md、research.md、data-model.md、quickstart.md

## フォーマット: `[ID] [P?] [ストーリー] 説明`

## フェーズ1: ユーザーストーリー1 - arm64 でも docker compose が完走する (優先度: P1)

- [ ] **T101** [P] [US1] `docker-compose.yml` の playwright-novnc に platform の環境変数上書き設定を追加する
- [ ] **T102** [US1] `docs/docker-usage.md` に arm64 向けの manifest エラー回避手順と `PLAYWRIGHT_NOVNC_PLATFORM` の説明を追記する

## フェーズ2: ユーザーストーリー2 - 既存の amd64 利用は影響を受けない (優先度: P2)

- [ ] **T201** [US2] `docker-compose.yml` の変更が gwt サービスに影響しないことを確認する

## フェーズ3: ユーザーストーリー3 - arm64 向けの手順が分かる (優先度: P3)

- [ ] **T301** [P] [US3] `docs/docker-usage.md` にエミュレーション要件の注意点を追記する

## フェーズ4: 統合と検証

- [ ] **T401** [統合] `docker-compose.yml` を対象に docker compose config を実行し platform 設定が反映されることを確認する
- [ ] **T402** [統合] `docs/docker-usage.md` と `specs/SPEC-925c010b/*.md` に対して markdownlint を実行する
- [ ] **T403** [統合] `commitlint.config.cjs` に従ってコミットメッセージを検証する

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
- **スキップしたタスク**: 理由と共に文書化
