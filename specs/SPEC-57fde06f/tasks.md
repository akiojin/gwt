# タスク: releaseブランチ経由の自動リリース＆Auto Mergeフロー

**入力**: `specs/SPEC-57fde06f/` の各ドキュメント

## フォーマット: `- [ ] T000 [P?] [US?] 説明 (file)`

## フェーズ1: セットアップ

- [ ] T001 spec/plan/data-model を読み、release ブランチ方式の要件と成功基準をノート化する (specs/SPEC-57fde06f/spec.md)
- [ ] T002 `.releaserc.json` と `.github/workflows/create-release.yml` の現状を確認し、必要な差分を洗い出す (
.releaserc.json)

## フェーズ2: インフラ整備

- [ ] T010 `.releaserc.json` を `branches: [{ name: "release/*" }]` に固定し、unity と同じプラグイン構成を反映する (.releaserc.json)
- [ ] T011 `.github/workflows/create-release.yml` を release ブランチ作成専用として整理し、Summary/ログを最新内容に更新する (.github/workflows/create-release.yml)
- [ ] T012 `.github/workflows/release.yml` を release/** push 前提に見直し、semantic-release 成功時のみ main へ merge → branch delete を実施する (.github/workflows/release.yml)
- [ ] T013 `.github/workflows/publish.yml` を npm publish + develop back-merge ワークフローとして整理し、`chore(release):` コミット検出と back-merge ログを標準化する (.github/workflows/publish.yml)

## フェーズ3: ユーザーストーリー1（release ブランチを `/release` で最新化）

- [ ] T101 [US1] `/release` コマンドと `scripts/create-release-branch.sh` を同一実装で `create-release.yml` dispatch に統一する (.claude/commands/release.md)
- [ ] T102 [US1] `scripts/create-release-branch.sh` にワークフロー監視コマンドと失敗時の案内を追加する (scripts/create-release-branch.sh)
- [ ] T103 [US1] CLAUDE.md / README.* / docs の release 節を release/vX.Y.Z フロー説明で揃える (CLAUDE.md)

## フェーズ4: ユーザーストーリー2（release ブランチの CI 完了後に main を更新）

- [ ] T201 [US2] `release.yml` の `semantic-release` ステップに新しいログ解析と失敗時の手動リカバリ手順を追加し、Summary で run URL / version を出力する (.github/workflows/release.yml)
- [ ] T202 [US2] `publish.yml` に develop back-merge の結果を明示するログと失敗時の再実行コマンドを記載する (.github/workflows/publish.yml)
- [ ] T203 [US2] Branch Protection 設定手順を specs/quickstart と docs ガイドに追加し、Required Checks リストを同期させる (specs/SPEC-57fde06f/quickstart.md)

## フェーズ5: ユーザーストーリー3（ドキュメントとガバナンス）

- [ ] T301 [US3] specs/ 以下の plan/research/data-model/quickstart/contracts を release ブランチ方式で最新化し、設計は specs 側に集約する (specs/SPEC-57fde06f/plan.md)
- [ ] T302 [US3] docs/release-guide*.md に概要のみを記載し、詳細設計は specs へのリンクに統一する (docs/release-guide.md)
- [ ] T303 [US3] README.* から詳細設計情報を削減し、利用者向けの簡潔な案内 + docs/specs へのリンクに限定する (README.md)

## フェーズ6: 検証

- [ ] T401 `create-release.yml` を `workflow_dispatch` で実行して release/vX.Y.Z 作成～削除までのログを記録する (.github/workflows/create-release.yml)
- [ ] T402 `release.yml` / `publish.yml` の Actions 実行結果を保存し、semantic-release の `Published release` ログと develop back-mergeを確認する (.github/workflows/release.yml)
- [ ] T403 `bun run lint` `bun run test` `bun run build` を実行してローカル回帰を確認する (package.json)
- [ ] T404 Conventional Commit (`feat: align release workflow with unity flow`) を作成し、差分を最終チェックする (git)

## 依存関係

1. フェーズ2で `.releaserc.json` / workflow の下地を整えてからフェーズ3/4の改修を行う。
2. ドキュメント更新（フェーズ5）は実装内容が固まってから着手。
3. 検証（フェーズ6）はすべてのユーザーストーリー完了後に実施する。

## 独立テスト基準

- **US1**: `/release` 実行で release/vX.Y.Z が develop と同じ SHA を指し、`release.yml` が自動実行される。
- **US2**: `release.yml` が成功したときのみ main に `chore(release)` が現れ、失敗時は main が無変更で残る。
- **US3**: README/CLAUDE/docs/specs を参照すれば release ブランチ方式と復旧手順を理解できる。
