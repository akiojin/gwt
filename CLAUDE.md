# CLAUDE.md

このファイルは、このリポジトリでコードを扱う際のガイダンスを提供します。

## 開発指針

### 🛠️ 技術実装指針

- **設計・実装は複雑にせずに、シンプルさの極限を追求してください**
- **ただし、ユーザビリティと開発者体験の品質は決して妥協しない**
- 実装はシンプルに、開発者体験は最高品質に
- CLI操作の直感性と効率性を技術的複雑さより優先

### 🧩 Ratatui ガイドライン

- CLI TUI は `ratatui` を利用
- 端末描画で使用するアイコンは ASCII に統一し、全角/絵文字は避ける

### 📝 設計ガイドライン

- 設計に関するドキュメントには、ソースコードを書かないこと

## 開発品質

### 完了条件

- エラーが発生している状態で完了としないこと。必ずエラーが解消された時点で完了とする。

## 開発ワークフロー

### 基本ルール

- **指示を受けた場合、まず既存要件（spec.md）に追記可能かを調べ、次に要件化（Spec Kit による仕様策定）とTDD化を優先的に実行する。実装は要件とテストが確定した後に着手する。**
- 作業（タスク）を完了したら、変更点を日本語でコミットログに追加して、コミット＆プッシュを必ず行う
- 作業（タスク）は、最大限の並列化をして進める
- 作業（タスク）は、最大限の細分化をしてPLANS.mdにやるべきことを出力する
- `git rebase -i origin/main` はLLMでの失敗率が高いため禁止（必要な場合は人間が手動で整形すること）
- Spec Kitを用いたSDD/TDDの絶対遵守を義務付ける。Spec Kit承認前およびSpec準拠のTDD完了前の実装着手を全面禁止し、違反タスクは即時差し戻す。
- 作業（タスク）は、忖度なしで進める
- **エージェントはユーザーからの明示的な指示なく新規ブランチの作成・削除を行ってはならない。Worktreeは起動ブランチで作業を完結する設計。**

### コミットメッセージポリシー

> 🚨 **コミットログはリリースワークフローがバージョン判定に使用する唯一の真実であり、ここに齟齬があるとリリースバージョン・CHANGELOG 生成が即座に破綻します。commitlint を素通りさせることは絶対に許されません。**

- バージョン判定とリリースノート生成を Conventional Commits から自動化しているため、コミットメッセージは例外なく Conventional Commits 形式（`feat:`/`fix:`/`docs:`/`chore:` ...）で記述する。
- コミットを作成する前に、変更内容と Conventional Commits の種別（`feat`/`fix`/`docs` など）が 1 対 1 で一致しているかを厳格に突き合わせる。バージョン種別（major/minor/patch）がこの判定で決まるため、嘘の種類を付けた瞬間にバージョン管理が壊れる。
- ローカルでは `bunx commitlint --from HEAD~1 --to HEAD` などで必ず自己検証し、CI の commitlint に丸投げしない。エラーが出た状態で push しない。
- `feat:` はマイナーバージョン、`fix:` はパッチ、`type!:` もしくは本文の `BREAKING CHANGE:` はメジャー扱いになる。 breaking change を含む場合は例外なく `!` か `BREAKING CHANGE:` を記載し、破壊的変更を認識させる。
- 1コミットで複数タスクを抱き合わせない。変更内容とコミットメッセージの対応関係を明確に保ち、解析精度を担保する。
- `chore:` や `docs:` などリリース対象外のタイプでも必ずプレフィックスを付け、曖昧な自然文だけのコミットメッセージを禁止する。
- コミット前に commitlint ルール（subject 空欄禁止・100文字以内など）を自己確認し、CI での差し戻しを防止する。

### ローカル検証/実行ルール（Rust）

- このリポジトリのローカル検証・実行は Cargo を使用する
- ビルド: `cargo build --release`
- テスト: `cargo test`
- Lint: `cargo clippy --all-targets --all-features -- -D warnings`
- フォーマット: `cargo fmt`
- 実行: `./target/release/gwt` または `cargo run`
- npm配布: `bunx @akiojin/gwt` または `npm install -g @akiojin/gwt`

## コミュニケーションガイドライン

- 回答は必ず日本語
- CLIのユーザー向け出力は英語のみ（日本語の文言を表示しない）

## ドキュメント管理

- ドキュメントはREADME.md/README.ja.mdに集約する
- 仕様ファイルは必ず `specs/SPEC-????????/` （UUID8桁）配下に配置する。`specs/feature/*` など別階層への配置は禁止。

## コードクオリティガイドライン

- マークダウンファイルはmarkdownlintでエラー及び警告がない状態にする
- コミットログはcommitlintに対応する

## 開発ガイドライン

- Spec Kitを用いたSDD/TDDの絶対遵守を組織ルールとする。Spec Kit外での設計・テスト着手、およびSpec未承認・TDD未完了での実装開始を検知次第即時差し戻す。
- 既存のファイルのメンテナンスを無視して、新規ファイルばかり作成するのは禁止。既存ファイルを改修することを優先する。

## ドキュメント作成ガイドライン

- README.mdには設計などは書いてはいけない。プロジェクトの説明やディレクトリ構成などの説明のみに徹底する。設計などは、適切なファイルへのリンクを書く。

## リリースワークフロー

- feature/\* ブランチは develop へ Auto Merge し、develop で次回リリース候補を蓄積する。
- `/release` コマンド（または `gh workflow run prepare-release.yml --ref develop`）で Release PR を作成:
  - Conventional Commits を解析してバージョン自動判定（feat→minor, fix→patch, !→major）
  - git-cliff で CHANGELOG.md を更新
  - Cargo.toml, package.json のバージョンを更新
  - release/YYYYMMDD-HHMMSS ブランチから main への PR を作成
- Release PR が main にマージされると `.github/workflows/release.yml` が以下を自動実行:
  - タグ・GitHub Release を作成
  - クロスコンパイル済みバイナリを GitHub Release にアップロード
  - npm へ公開（provenance 付き）

## パッケージ公開状況

> **重要**: 各プラットフォームのバージョンは独立して管理されており、一度公開したバージョンは再利用不可。リリース前に必ず各プラットフォームの最新バージョンを確認すること。

| プラットフォーム | パッケージ名 | 確認コマンド |
| -------------- | ----------- | ----------- |
| npmjs | `@akiojin/gwt` | `npm view @akiojin/gwt version` |
| GitHub Release | - | `gh release list --repo akiojin/gwt --limit 1` |

### 次回リリース時の注意

- 各プラットフォームのバージョンは一度公開すると再利用不可
- リリース前に上記の確認コマンドで最新バージョンをチェックすること
- npm の `latest` タグが古いバージョンを指している場合は手動で修正が必要:
  `npm dist-tag add @akiojin/gwt@<version> latest`

## 使用中の技術

- Rust (Stable) + Ratatui TUI フレームワーク
- ファイル/ローカル Git メタデータ（DB なし）

## プロジェクト構成

```text
├── Cargo.toml          # ワークスペース設定
├── crates/
│   ├── gwt-cli/        # CLIエントリポイント・TUI
│   ├── gwt-core/       # コアライブラリ（worktree管理）
│   ├── gwt-web/        # Webサーバー（将来）
│   └── gwt-frontend/   # Webフロントエンド（将来）
├── package.json        # npm配布用ラッパー
├── bin/gwt.js          # バイナリラッパースクリプト
└── scripts/postinstall.js  # バイナリダウンロードスクリプト
```

## 最近の変更

- TypeScript/Bun から Rust への完全移行
- ブランチ一覧は枠線表示・統計非表示に整理し、クリーンアップはWorktreeのあるブランチのみ対象
