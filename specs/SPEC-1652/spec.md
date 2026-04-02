# ビルドと配布パイプライン

> **Canonical Boundary**: 本 SPEC は gwt-tui の build / release / installer pipeline を扱う。Tauri build や auto-update は現行アーキテクチャの対象外とする。

## Background

- gwt は `cargo build -p gwt-tui` を基準にビルドし、GitHub Releases とインストーラスクリプトで配布する。
- 既存の SPEC-1652 は Tauri `.dmg/.msi/.AppImage` と auto-update を前提にしており、現行の TUI 配布方式と一致しない。
- 本 SPEC は Conventional Commits / git-cliff / GitHub Release を使う現行の release pipeline を正本として定義する。

## User Stories

### US-1: develop から release PR を作成する

開発者として、Conventional Commits から次バージョンを判定し、release PR を作りたい。

### US-2: main マージで自動リリースする

開発者として、main への release PR マージ後にタグ・Release・バイナリ生成まで自動化したい。

### US-3: 利用者へインストール導線を提供する

利用者として、GitHub Releases とインストーラスクリプトから TUI バイナリを取得したい。

## Acceptance Scenarios

1. develop から release PR を作成すると version / changelog / manifest が更新される。
2. release PR が main に merge されるとタグと GitHub Release が作成される。
3. README の install 手順から配布物へ到達できる。
4. ビルド、lint、test が release 前の必須ゲートとして扱われる。
5. Tauri 固有の build や auto-update に依存しない。

## Edge Cases

- Conventional Commit 種別が誤っていて version 判定が崩れる。
- release asset の upload に失敗する。
- README の install 手順と実際の asset 名がずれる。

## Functional Requirements

- FR-001: build 正本は `cargo build -p gwt-tui` とする。
- FR-002: release PR で version / changelog を更新する。
- FR-003: main merge 後に GitHub Release と配布 asset を自動生成する。
- FR-004: README と installer script を現行配布フローと同期させる。
- FR-005: Tauri build と auto-update は本 SPEC の対象外とする。

## Success Criteria

- release workflow の正本が TUI 配布方式と一致する。
- README / workflow / versioning が同じ前提で運用できる。
- 旧 Tauri 前提の説明が残らない。
