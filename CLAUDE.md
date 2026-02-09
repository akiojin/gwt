# CLAUDE.md

このファイルは、このリポジトリでコードを扱う際のガイダンスを提供します。

## 開発指針

### 🛠️ 技術実装指針

- **設計・実装は複雑にせずに、シンプルさの極限を追求してください**
- **ただし、ユーザビリティと開発者体験の品質は決して妥協しない**
- 実装はシンプルに、開発者体験は最高品質に
- GUI操作の直感性と効率性を技術的複雑さより優先

### 🧩 Tauri GUI ガイドライン

- デスクトップGUI は Tauri v2 + Svelte 5 + xterm.js
- バックエンド: Rust (gwt-core + gwt-tauri)
- フロントエンド: Svelte 5 + TypeScript + Vite (gwt-gui/)
- ターミナルエミュレーション: xterm.js v5
- UIアイコンは ASCII に統一し、全角/絵文字は避ける

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

### 🚨 仕様・TDD優先の絶対ルール

> **重要**: このルールはすべての実装作業に適用される。バグ修正やエッジケース対応であっても例外なく従うこと。

1. **仕様の確認と更新が最優先**
   - 実装に着手する前に、必ず関連する `spec.md` を確認する
   - 新しい動作やエッジケースを追加する場合は、先に `spec.md` のエッジケースセクションまたは受け入れシナリオに追記する
   - 仕様に記載のない動作を実装してはならない

2. **テストの追加が実装より先**
   - 新機能やバグ修正の場合、先にテストを作成する（TDD）
   - テストが失敗することを確認してから実装に着手する
   - テストなしでの実装完了は認めない

3. **変更の順序**
   - ① `spec.md` の更新（仕様の明文化）
   - ② テストの追加（期待動作の定義）
   - ③ 実装（テストを通すコード）
   - ④ コミット＆プッシュ
   - この順序を守らない作業は差し戻しとする

4. **エッジケースやバグ修正の場合も同様**
   - 「急ぎの修正だから仕様は後で」は許可しない
   - 仕様とテストを先に整備することで、同じ問題の再発を防ぐ

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

- このリポジトリのローカル検証・実行は Cargo + Tauri CLI を使用する
- ビルド: `cargo tauri build`
- 開発: `cargo tauri dev`
- テスト: `cargo test`
- Lint: `cargo clippy --all-targets --all-features -- -D warnings`
- フォーマット: `cargo fmt`
- フロントエンドチェック: `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json`

## コミュニケーションガイドライン

- 回答は必ず日本語
- GUIのユーザー向け表示は英語のみ（日本語の文言を表示しない）
- ログ（`~/.gwt/logs/` 等）はこの環境から直接参照できる前提で対応すること
- ログ参照の指示があれば、この環境から直接読み取って調査すること

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

- feature/\* ブランチは develop への PR を作成し、オーナー承認後にマージする。develop で次回リリース候補を蓄積する。
- `/release` コマンドで Release PR を作成:
  - Conventional Commits を解析してバージョン自動判定（feat→minor, fix→patch, !→major）
  - git-cliff で CHANGELOG.md を更新
  - Cargo.toml, package.json のバージョンを更新
  - develop → main への PR を作成（リリースブランチは作成しない）
- Release PR が main にマージされると `.github/workflows/release.yml` が以下を自動実行:
  - タグ・GitHub Release を作成
  - Tauri ビルド（.dmg/.msi/.AppImage）を GitHub Release にアップロード

## パッケージ公開状況

| プラットフォーム | 確認コマンド |
| -------------- | ----------- |
| GitHub Release | `gh release list --repo akiojin/gwt --limit 1` |

## 使用中の技術
- Rust 2021 Edition (stable) + ratatui 0.29, crossterm 0.28, reqwest (blocking), serde_json, chrono (SPEC-4b893dae)
- ファイルシステム（セッションファイル読み取り）、メモリキャッシュ (SPEC-4b893dae)
- Rust 2021 Edition (stable) + ratatui 0.29, crossterm 0.28, tracing, tracing-appender, serde_json, chrono, arboard (SPEC-e66acf66)
- ファイル（gwt.jsonl.YYYY-MM-DD） (SPEC-e66acf66)
- Rust 2021 Edition (stable) + ratatui 0.29, crossterm 0.28, reqwest (blocking), serde_json, chrono, tracing (SPEC-ba3f610c)
- ファイルシステム (`~/.gwt/sessions/` - JSON形式) (SPEC-ba3f610c)
- Rust 2021 Edition (stable) + ratatui 0.29, crossterm 0.28, reqwest (blocking), serde_json, chrono, tracing, tracing-appender, uuid (SPEC-ba3f610c)
- ファイルシステム（`~/.gwt/sessions/` JSON形式、`specs/SPEC-XXXXXXXX/` Spec Kit成果物） (SPEC-ba3f610c)
- Rust 2021 Edition (stable) + ratatui 0.29, crossterm 0.28, serde, serde_json, chrono, directories (SPEC-71f2742d)
- ファイル（~/.gwt/tools.json, .gwt/tools.json） (SPEC-71f2742d)
- ファイルシステム（.gwt/設定、gitメタデータ） (SPEC-a70a1ece)
- メモリキャッシュ（GitViewCache） (SPEC-1ea18899)
- N/A（メモリ内状態のみ） (SPEC-1ad9c07d)

- Rust 2021 Edition (stable) + Tauri v2, portable-pty, serde, tokio
- Svelte 5 + TypeScript + Vite 6
- xterm.js v5 (@xterm/xterm, @xterm/addon-fit, @xterm/addon-web-links)
- ファイル/ローカル Git メタデータ（DB なし）

## プロジェクト構成

```text
├── Cargo.toml          # ワークスペース設定
├── crates/
│   ├── gwt-core/       # コアライブラリ（Git操作・PTY管理・設定）
│   └── gwt-tauri/      # Tauri v2 バックエンド（コマンド・状態管理）
├── gwt-gui/            # Svelte 5 フロントエンド（UI・xterm.js）
│   ├── src/
│   │   ├── lib/components/  # UIコンポーネント
│   │   ├── lib/terminal/    # xterm.jsラッパー
│   │   └── lib/types.ts     # TypeScript型定義
│   └── package.json
└── package.json        # Tauri開発用スクリプト
```

## 最近の変更
- SPEC-1ad9c07d: 追加: Rust 2021 Edition (stable) + ratatui 0.29, crossterm 0.28, reqwest (blocking), serde_json, chrono, tracing
- SPEC-ba3f610c: 追加: Rust 2021 Edition (stable) + ratatui 0.29, crossterm 0.28, reqwest (blocking), serde_json, chrono, tracing, tracing-appender, uuid
- SPEC-ba3f610c: 追加: Rust 2021 Edition (stable) + ratatui 0.29, crossterm 0.28, reqwest (blocking), serde_json, chrono, tracing
