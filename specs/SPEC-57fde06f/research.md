# Research: releaseブランチ経由の自動リリース＆Auto Mergeフロー

## Decision 1: release ブランチ生成は GitHub Actions (`create-release.yml`) で統一
- **Rationale**: unity-mcp-server と同じく、release/vX.Y.Z の作成と semantic-release dry-run を GitHub Actions に任せればローカル/Claude どちらからでも同じ処理になる。CLI からは `gh workflow run create-release.yml --ref develop` を呼ぶだけで済む。
- **Alternatives considered**: ローカルで release ブランチを直接 push する案 → 実行者の環境に依存し、履歴の再現性が下がるため却下。

## Decision 2: semantic-release は release ブランチ push で実行し main を直接更新
- **Rationale**: release ブランチ上で CHANGELOG/タグ生成まで完結すれば main へ PR を経由せず直接反映できる。`release.yml` が成功した場合のみ main に `chore(release)` コミットを push し、失敗時は main を触らず再実行できる。
- **Alternatives considered**: main push で semantic-release を実行（従来方式） → main への直接 push が必要になり Branch Protection と矛盾。

## Decision 3: publish.yml で npm + develop back-merge を担保
- **Rationale**: release ブランチ削除後に main の結果を develop と npm に反映する責務を 1 つの workflow にまとめることで可視性を上げる。npm publish は任意なので `chore(release):` コミット検知時のみ実行し、常に `main → develop` のバックマージを行う。
- **Alternatives considered**: release.yml 内で back-merge まで済ませる → main push がトリガーされず npm publish のタイミングが曖昧になる。

## Decision 4: Branch Protection で main への直接 push を禁止
- **Rationale**: CI 以外が main を更新できないようにすることで release ブランチ経由フローを強制できる。Required Checks は `release.yml` 内の `lint`, `test`, `semantic-release` job 名と一致させる。
- **Alternatives considered**: ブランチ保護なし（運用ルールのみ） → 手作業ミスで main が汚染される恐れが高い。

## Decision 5: ドキュメント分離（README は概要、詳細は specs/ & docs/）
- **Rationale**: 利用者向け README を簡潔に保ちつつ、開発者向け詳細は `specs/SPEC-57fde06f/` と `docs/release-guide*.md` に集約する。unity-mcp-server と同じ情報密度を確保しつつ、利用者が読む必要のない設計情報を分離。
- **Alternatives considered**: README にすべて記載 → 変更多発時にメンテ難易度が高い。

## Decision 6: ヘルパースクリプトは `scripts/create-release-branch.sh` に一本化
- **Rationale**: 旧 `create-release-pr.sh` は develop→main PR 手順に依存していたため削除。unity 側と同じシンプルなワークフロー起動スクリプトに合わせることで、CLI / Claude どちらからでも同じ挙動を保証する。
- **Alternatives considered**: `/release` コマンドだけに頼る → ローカル作業者がブラウザ無しで実行できず UX が下がる。
