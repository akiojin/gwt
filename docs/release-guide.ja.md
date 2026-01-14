# リリースガイド（概要）

本ページはメンテナ向けの概要です。

## フロー概要

```text
feature/* → PR → develop (自動マージ)
                            ↓
               /release (prepare-release.yml)
                            ↓
               develop → main PR → 自動マージ
                            ↓ (release.yml)
               release-please: タグ・GitHub Release・Release PR 作成
                            ↓ (release.yml - リリース作成時)
               crates.io 公開 (Trusted Publishing)
               クロスコンパイル → GitHub Release アップロード
               npm 公開 (provenance 付き)
```

## メンテナーチェックリスト（要点）

1. **準備** – `develop` に必要なコミットを揃え、`cargo test && cargo build --release` を成功させる。
2. **トリガー** – Claude で `/release` を実行するか、ローカルで `gh workflow run prepare-release.yml --ref develop` を実行（`gh auth login` 済みであること）。
3. **監視** – `prepare-release.yml` で Release PR が作成されたら `release.yml` を Actions で監視する。
4. **確認** – `vX.Y.Z` タグ、crates.io 公開、GitHub Release バイナリ、npm パッケージバージョンをチェックする。
5. **リカバリ** – 失敗時は原因を修正しワークフローを再実行、必要に応じて Release PR を閉じて再作成する。

## 追加ドキュメント

- 利用者向けサマリ: README.ja.md

## 参考ファイル

- `.claude/commands/release.md`
- GitHub Actions: `prepare-release.yml`, `release.yml`, `auto-merge.yml`
