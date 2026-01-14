# リリースガイド（概要）

本ページはメンテナ向けの概要です。

## フロー概要

```text
feature/* → PR → develop (自動マージ)
                           ↓
              /release (prepare-release.yml)
                           ↓
              Conventional Commits解析 → バージョン自動判定
              git-cliff → CHANGELOG.md 更新
              Cargo.toml, package.json → バージョン更新
                           ↓
              release/YYYYMMDD-HHMMSS → main PR
                           ↓ (release.yml - マージ時)
              タグ・GitHub Release 作成
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

- [akiojin/create-release-pr](https://github.com/akiojin/create-release-pr) – 再利用可能なリリースPR作成Action
- `.github/workflows/prepare-release.yml` – リリーストリガー用ワークフロー
- `.github/workflows/release.yml` – 公開用ワークフロー
- `cliff.toml` – git-cliff設定（CHANGELOG生成）
