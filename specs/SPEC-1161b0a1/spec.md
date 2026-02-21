# 機能仕様: Windows 移行プロジェクトの Docker 起動でポート競合を回避する

**仕様ID**: `SPEC-1161b0a1`
**作成日**: 2026-02-20
**更新日**: 2026-02-20
**ステータス**: ドラフト
**カテゴリ**: GUI
**依存仕様**:

- specs/archive/SPEC-f5f5657e/spec.md
- specs/SPEC-4e2f1028/spec.md

**入力**: ユーザー説明: "Issue #1161: Windows 環境から docker で起動が失敗する（Bind for 0.0.0.0:5432 failed）"

## 背景

- Docker 起動時、compose の公開ポートが既存プロセス（例: 5432）と衝突すると `docker compose up` が失敗する。
- `gwt-core` 側には compose の `${PORT:-default}` を空きポートへ寄せる処理があるが、`gwt-tauri` 側の compose 環境変数マージで再び衝突ポートに上書きされる経路がある。

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - Docker 起動を失敗させず継続したい (優先度: P0)

Windows ユーザーとして、移行済みプロジェクトを Docker で起動する際、`5432` が使用中でも自動解決済みポートを維持して起動したい。

**独立したテスト**: compose 環境変数マージ時に、既存値が空きポートで incoming 値が使用中ポートの場合、既存値が保持されることを単体テストで確認する。

**受け入れシナリオ**:

1. **前提条件** base env に `KNOWLEDGE_DB_PORT=15432`、incoming env に `KNOWLEDGE_DB_PORT=5432`、5432 が使用中、**操作** compose env マージ、**期待結果** `KNOWLEDGE_DB_PORT=15432` が保持される。
2. **前提条件** base env に compose キーが未設定、incoming env に `API_TOKEN=xxx`、**操作** compose env マージ、**期待結果** `API_TOKEN=xxx` が追加される。

---

### ユーザーストーリー 2 - 既存の env マージ互換を維持したい (優先度: P1)

開発者として、ポート競合回避を入れても、非ポート値の compose env マージ挙動は従来どおり維持したい。

**独立したテスト**: 非ポート値の compose env が従来どおり上書きされることを単体テストで確認する。

**受け入れシナリオ**:

1. **前提条件** base env に `GITHUB_TOKEN=old`、incoming env に `GITHUB_TOKEN=new`、**操作** compose env マージ、**期待結果** `GITHUB_TOKEN=new` になる。

---

## エッジケース

- incoming 値が数値でも、そのポートが未使用なら従来どおり上書きを許可する。
- base 値が非数値文字列の場合は、従来挙動（incoming 上書き）を維持する。

## 要件 *(必須)*

### 機能要件

- **FR-001**: システムは compose env マージ時、既存値・incoming 値がともにポート番号で、incoming 側ポートが使用中の場合、既存値を保持しなければならない。
- **FR-002**: システムは compose ファイルに存在する env キーが base env に未定義の場合、incoming 値を追加しなければならない。
- **FR-003**: システムは非ポート値の compose env について、従来どおり incoming 値で上書きしなければならない。

### 非機能要件

- **NFR-001**: 変更は `crates/gwt-tauri/src/commands/terminal.rs` のユニットテストで再現ケースをカバーする。

## 制約と仮定

- Docker compose 定義そのもの（ユーザープロジェクトの `docker-compose.yml`）は編集しない。
- ポート競合判定はローカルホストでの bind 可否に基づく。

## 成功基準 *(必須)*

- **SC-001**: Issue #1161 再現系（5432 競合）で `docker compose up` 前の env マージ結果が衝突ポートへ巻き戻らない。
- **SC-002**: 追加したユニットテストが通過し、既存 compose env テストが回帰しない。
