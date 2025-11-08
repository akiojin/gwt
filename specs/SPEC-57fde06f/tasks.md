# タスク: releaseブランチ経由の自動リリース＆Auto Mergeフロー

**入力**: `specs/SPEC-57fde06f/` の設計ドキュメント
**前提条件**: plan.md / spec.md / research.md / data-model.md / quickstart.md / contracts/

## フォーマット: `- [ ] T001 [P?] [US?] 説明 (file)`

## フェーズ1: セットアップ

- [ ] T001 `specs/SPEC-57fde06f/spec.md` と `plan.md` を読み込み、ユーザーストーリーと成功基準を作業ノートに要約する (specs/SPEC-57fde06f/spec.md)
- [ ] T002 `specs/SPEC-57fde06f/research.md` の決定事項（release ブランチ同期方法・Auto Merge 手段）を CLI 実装メモに転記する (specs/SPEC-57fde06f/research.md)
- [ ] T003 release / main ブランチの現在の状態を確認し、release ブランチを最新取得 (`git fetch origin release`) して差分を記録する (.git)

## フェーズ2: 基盤整備

- [ ] T010 `.releaserc.json` の `branches` を `release`（メイン）+ `main`（メンテナンス）構成に更新し、タグ戦略を spec に合わせる (.releaserc.json)
- [ ] T011 [P] release ワークフローのトリガを `push: release` + `workflow_dispatch` に変更し、checkout / git 操作を release ブランチ前提にリライトする (.github/workflows/release.yml)
- [ ] T012 [P] `quickstart.md` を参考に Required チェック名（lint/test/semantic-release）を Actions job 名と一致させるようワークフロー内で `name:` を統一する (.github/workflows/release.yml)

## フェーズ3: ユーザーストーリー1 - release ブランチを `/release` で最新化 (P1)

- [ ] T101 [US1] `release-trigger.yml` の処理を develop→release fast-forward 用に書き換え、main には push しない構成へ変更する (.github/workflows/release-trigger.yml)
- [ ] T102 [P] [US1] 上記ワークフローに release ブランチ push 後の再実行リンクと失敗時の diff 出力を追加し、ログから差分を追えるようにする (.github/workflows/release-trigger.yml)
- [ ] T103 [US1] `/release` コマンドドキュメントを release ブランチベースの手順（gh workflow run → release push 監視）に更新する (.claude/commands/release.md)
- [ ] T104 [US1] README および README.ja のリリース節を develop→release→main フロー図へ差し替える (README.md)
- [ ] T105 [US1] release ブランチへの push が semantic-release / lint / test を起動することを `docs` か `CHANGELOG.md` に記録し、履歴として残す (CHANGELOG.md)

## フェーズ4: ユーザーストーリー2 - release→main PR を Required チェックのみで Auto Merge (P1)

- [ ] T201 [US2] `release-trigger.yml` へ gh CLI ステップを追加し、release→main PR を作成/更新・Auto Merge 有効化・ラベル付与を自動化する (.github/workflows/release-trigger.yml)
- [ ] T202 [US2] PR 本文テンプレートを scripts もしくは workflow 内に実装し semantic-release 結果リンク / Required チェック一覧を含める (.github/workflows/release-trigger.yml)
- [ ] T203 [P] [US2] `.github/workflows/release.yml` に `gh pr checks` 用の summary を追加し、Auto Merge 待機中の開発者向けに URL を表示する (.github/workflows/release.yml)
- [ ] T204 [US2] `.github/workflows/release.yml` のテスト/semantic-release ステップ成功時に job 名が Branch Protection の Required チェックと一致することを確認する (specs/SPEC-57fde06f/contracts/release-automation.md)
- [ ] T205 [US2] main ブランチ保護ルールと Required チェック限定条件を `docs/troubleshooting.md` に追記し、運用チェックリスト化する (docs/troubleshooting.md)

## フェーズ5: ユーザーストーリー3 - ガバナンスとドキュメント更新 (P2)

- [ ] T301 [US3] CLAUDE.md のリリースワークフロー節を develop→release→main / Auto Merge モデルへ書き換える (CLAUDE.md)
- [ ] T302 [US3] README.ja.md の release セクションと図版を新フローへ更新し、main への直接 push 禁止を強調する (README.ja.md)
- [ ] T303 [P] [US3] `.claude/commands/release.md`・`docs/*`・`specs/SPEC-57fde06f/quickstart.md` のリンク関係を再確認し、最新手順のみを残す (specs/SPEC-57fde06f/quickstart.md)
- [ ] T304 [US3] Branch Protection 設定チェックリストを `docs/troubleshooting.md` 末尾に追加し、管理者確認項目を明示する (docs/troubleshooting.md)

## フェーズ6: 統合・検証

- [ ] T401 release-trigger ワークフローを `workflow_dispatch` で手動実行し、release ブランチ push / PR 作成 / Auto Merge までのログを保存する (.github/workflows/release-trigger.yml)
- [ ] T402 release ブランチ push を模擬して release ワークフローが lint/test/semantic-release を完走することを確認 (`gh run watch <id>`) (.github/workflows/release.yml)
- [ ] T403 `bun run lint` `bun run test` `bun run build` を実行しローカルでも回帰が無いことを確認 (package.json)
- [ ] T404 Conventional Commit (`feat: orchestrate release branch auto merge flow`) を作成し差分を最終チェック (git history)

## 依存関係

1. フェーズ1→フェーズ2 は順序必須（文書／設定方針が確定していないと workflow 改修ができない）。
2. フェーズ2で release.yml / releaserc を更新してからフェーズ3の release-trigger 変更を行う必要がある。
3. フェーズ4 はフェーズ3の release PR 作成機構に依存する。フェーズ5（ドキュメント）は技術的変更後であれば並列実行可。
4. フェーズ6 の検証は全ストーリー完了後に実施する。

## 並列実行例

- T011 と T012 は release.yml 内の独立タスクなので別エージェントで進められる。
- T103 (ドキュメント) と T104 (README 更新) はコード変更を待たずに Draft 化でき、フェーズ3後半と並列可能。
- T203 と T205 はワークフロー / ドキュメントに跨るが、フェーズ4内で独立しているため同時作業が可能。

## MVP スコープ

- US1 (T101〜T105) と US2 (T201〜T204) 完了で release ブランチ運用と Auto Merge が実現し、リリースを無人化できる。US3 はガバナンス強化として後追いでもよい。

## 独立テスト基準

- **US1**: `/release` → `release-trigger.yml` 実行で release ブランチが develop と同じ SHA になり、main に push されないログが残る。
- **US2**: release→main PR が Required チェック成功後に自動マージされ、main への直接 push なしにタグが共有される。
- **US3**: CLAUDE.md / README.* / docs が release フローを指示し、手順を辿れば Branch Protection 設定と Auto Merge 条件が理解できる。
