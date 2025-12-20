# リリースガイド（概要）

本ページはメンテナ向けの概要です。詳細な要件や設計は `specs/SPEC-57fde06f/` を参照してください。

## フロー概要

```
feature/* → PR → develop (自動マージ)
                            ↓
               /release (create-release.yml)
                            ↓
               Release PR 作成 → main へ自動マージ
                            ↓ (release.yml)
               タグ・GitHub Release 作成
                            ↓ (publish.yml)
            npm publish(任意) → main → develop へ自動バックマージ
```

## メンテナーチェックリスト（要点）

1. **準備** – `develop` に必要なコミットを揃え、`bun run lint && bun run test && bun run build` を成功させる。
2. **トリガー** – Claude で `/release` を実行するか、ローカルで `gh workflow run create-release.yml --ref develop` を実行（`gh auth login` 済みであること）。
3. **監視** – `create-release.yml` で Release PR が作成されたら `release.yml`、`publish.yml` を Actions で監視する。
4. **確認** – `develop` の `chore(release):` コミット、`vX.Y.Z` タグ、（有効化していれば）npm 公開をチェックする。
5. **リカバリ** – 失敗時は原因を修正しワークフローを再実行、必要に応じて Release PR を閉じて再作成する。詳細手順は spec を参照。

## 追加ドキュメント

- 詳細な要件・エッジケース: `specs/SPEC-57fde06f/spec.md`
- Quickstart / contracts / data model: `specs/SPEC-57fde06f/` 配下の各ファイル
- 利用者向けサマリ: README.ja.md

## 参考ファイル

- `.claude/commands/release.md`
- GitHub Actions: `create-release.yml`, `release.yml`, `publish.yml`
