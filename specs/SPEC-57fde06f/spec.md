# 機能仕様: releaseブランチ経由の自動リリース＆Auto Mergeフロー

**仕様ID**: `SPEC-57fde06f`
**作成日**: 2025-11-07
**ステータス**: ドラフト
**入力**: ユーザー説明: "自動配信を develop→release→main に統制し、main への直接 push を禁止。semantic-release は release ブランチで実行し、release→main の PR を Required チェック完了後に自動マージさせたい。"

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - releaseコマンドで release ブランチを最新化しリリース開始 (優先度: P1)

リリース担当者が `/release` コマンドを実行すると、develop の最新コミットが release ブランチに反映され、release ブランチに対して semantic-release が走り、必要なバージョンタグやリリースノートが生成される。

**この優先度の理由**: release ブランチに統一したエントリーポイントがないとフロー全体が成立しないため。

**独立したテスト**: テスト用リポジトリでダミーコミットを develop に積み、`/release` を実行して release ブランチが fast-forward され、semantic-release の dry-run またはCI結果が release ブランチで成功することを確認する。

**受け入れシナリオ**:

1. **前提条件** develop に新しいリリース候補コミットが存在する、**操作** `/release` を実行、**期待結果** release ブランチが develop と同じコミットを指し、release ブランチ向けの semantic-release ワークフローが起動する。
2. **前提条件** release ブランチに既存の PR が main に向けてオープン済み、**操作** `/release` を再実行、**期待結果** 同じ PR が更新され、重複 PR は作られない。

---

### ユーザーストーリー 2 - release→main PR を Required チェック完了後に自動マージ (優先度: P1)

リリース担当者は release ブランチから main への PR が作成され Auto Merge が設定されていることを確認するだけで、Required チェック（テスト、semantic-release 完了など）が通れば人手を介さず main に取り込まれる。

**この優先度の理由**: main への直接 push を禁止しつつ手動作業を増やさないため。

**独立したテスト**: release→main PR で必要チェックが成功したときに自動的にマージされたこと、チェックが失敗するとマージが保留されることを確認する。

**受け入れシナリオ**:

1. **前提条件** release→main PR が存在し Required チェックが pending、**操作** チェックが成功するまで待機、**期待結果** 人手操作なしで PR が main にマージされタグが main に反映される。
2. **前提条件** Required チェックの一つが失敗、**操作** 失敗したチェックを再実行、**期待結果** Auto Merge が解除されず、全チェック成功後にのみマージが実行される。

---

### ユーザーストーリー 3 - ブランチ保護とドキュメントで新フローを周知 (優先度: P2)

開発者・レビュアが main への push を試みるとブロックされ、新しいリリースフロー（develop→release→main/Auto Merge）が CLAUDE.md や `.claude/commands/release.md` に明記されているため迷わず運用できる。

**この優先度の理由**: 誤操作防止とオンボーディング短縮によりフローの定着を支援するため。

**独立したテスト**: main ブランチに直接 push して拒否されること、関連ドキュメントに手順と制約が追記されていることを確認する。

**受け入れシナリオ**:

1. **前提条件** 開発者が main に直接 push する権限を持たない、**操作** main へ push を試行、**期待結果** ブランチ保護で拒否され release フローを案内するメッセージが表示される。
2. **前提条件** 新規メンバーが README/CLAUDE.md を参照、**操作** リリース手順を確認、**期待結果** release ブランチ経由フローと Auto Merge 条件が明示されている。

### エッジケース

- release ブランチに semantic-release が失敗した場合、Auto Merge も保留になり再実行手順が案内される必要がある。
- develop が release より進んでいない状態で `/release` を実行した場合は何も更新されず既存 PR がそのまま維持される。
- main で緊急 hotfix が必要になった場合は既存手順（例: hotfix ブランチ）を用い、release ブランチへ逆マージするガードレールが必要。

## 要件 *(必須)*

### 機能要件

- **FR-001**: `/release` 実行時に develop の HEAD を release ブランチへ fast-forward し、タグやメタ情報が重複しないようにする。
- **FR-002**: release ブランチへの push をトリガーに semantic-release（npm publish、GitHub Release 作成を含む）が実行されるよう CI 設定を変更する。
- **FR-003**: release→main の PR を作成または更新し、PR タイトル・本文に今回のリリースノート要約と semantic-release 実行結果を記載する。
- **FR-004**: release→main PR に GitHub Auto Merge（squash もしくは merge queue）を自動で有効化し、Required チェックのみをマージ条件として設定する。
- **FR-005**: main ブランチへの直接 push を Branch Protection で禁止し、release ブランチのみ main への PR 作成を許可する。
- **FR-006**: Required チェックの一覧（例: lint、test、semantic-release 完了）を定義し PR 作成時に自動で適用する。
- **FR-007**: 新フローと制約を CLAUDE.md、`.claude/commands/release.md`、関連 README に追記し、開発者が参照できるようにする。
- **FR-008**: 失敗時のリカバリー手順（再実行、Auto Merge 再設定、release→main PR の手動更新方法）をドキュメント化する。

### 主要エンティティ *(機能がデータを含む場合は含める)*

- **release ブランチ**: develop からの最新リリース候補を集約し、semantic-release が実行される唯一のブランチ。
- **release→main PR**: release ブランチを main に取り込むための唯一の経路。Auto Merge が設定され、Required チェックが通れば自動マージされる。
- **Required チェック**: semantic-release job、テスト、lint など main 反映前に必ず成功させる CI ジョブ群。

## 成功基準 *(必須)*

### 測定可能な成果

- **SC-001**: 100% のリリースが release ブランチ経由で実行され、main への直接 push が 0 件になる。
- **SC-002**: release→main PR は Required チェック完了後 10 分以内に自動マージされる。
- **SC-003**: リリース手順に関する問い合わせ件数が 1 リリースあたり 0 件（もしくは現状比 50% 減）になる。
- **SC-004**: release ブランチで semantic-release が連続 3 回成功し、タグ付与と GitHub Release が欠落しない。

## 制約と仮定 *(該当する場合)*

### 制約

- GitHub Branch Protection と semantic-release 設定のみで実現し、追加の外部サービスは導入しない。
- `/release` コマンドおよび CI は bun / pnpm など既存ツールチェーンを用いる。

### 仮定

- `/release` コマンドはリリース担当者のみが実行し、CI トークンは release ブランチへの push 権限を持つ。
- main ブランチで hotfix が発生した場合は別途合意済みの手順で対処し、本仕様のフローを乱さない。

## 範囲外 *(必須)*

次の項目は、この機能の範囲外です：

- develop 以前（feature/* → develop）のフロー変更。
- 他リポジトリや monorepo 全体のリリースプロセス刷新。
- semantic-release 以外のリリースツール導入。

## セキュリティとプライバシーの考慮事項 *(該当する場合)*

- main ブランチへの push 権限を最小化し、Branch Protection の `Restrict who can push` を有効にする。
- release→main PR に含まれるリリースノートやログに機密情報を記載しないようガイドラインを設ける。

## 依存関係 *(該当する場合)*

- GitHub の Auto Merge / Branch Protection 機能。
- semantic-release の `branches` 設定と GitHub Actions ワークフロー。
- `.claude/commands/release.md` および `release-trigger.yml` の既存実装。

## 参考資料 *(該当する場合)*

- [既存リリース手順 (`CLAUDE.md`)](../../CLAUDE.md)
- [release コマンドドキュメント (`.claude/commands/release.md`)](../../.claude/commands/release.md)
- [release-trigger ワークフロー (`.github/workflows/release-trigger.yml`)](../../.github/workflows/release-trigger.yml)
