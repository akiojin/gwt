---
description: "環境変数プロファイル機能（Ratatui）のタスク"
---

# タスク: 環境変数プロファイル機能（Ratatui）

**入力**: `/specs/SPEC-dafff079/` からの仕様書・計画書
**前提条件**: `specs/SPEC-dafff079/spec.md`, `specs/SPEC-dafff079/plan.md`

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能
- **[ストーリー]**: US1..US8
- 説明に正確なファイルパスを含める

## フェーズ1: 統合表示/操作 (US4/US6)

- [x] **T001** [US4] `crates/gwt-cli/src/tui/screens/environment.rs` にOS/プロファイル統合表示と色分けロジックを追加
- [x] **T002** [US4] `crates/gwt-cli/src/tui/app.rs` でOS環境変数取得を環境変数編集画面に統合
- [x] **T003** [US6] `crates/gwt-cli/src/tui/app.rs` に Enter 編集・OSのみ削除警告・スクロールキー統合を追加
- [x] **T005** [US3] `crates/gwt-cli/src/tui/app.rs` に Enter で環境変数編集へ遷移する処理を追加
- [x] **T004** [US4] `crates/gwt-cli/src/tui/screens/mod.rs` からOS専用画面の公開を削除
- [x] **T006** [US4] `crates/gwt-cli/src/tui/app.rs` に `r` リセット操作を追加
- [x] **T007** [US4] `crates/gwt-core/src/config/profile.rs` にOS無効化リストを追加
- [x] **T008** [US4] `crates/gwt-cli/src/tui/screens/environment.rs` に赤色取り消し線の無効化表示を追加
- [x] **T009** [US4] `crates/gwt-cli/src/tui/app.rs` で無効化状態の保存とエージェント起動への反映を追加
- [x] **T010** [US4] `crates/gwt-cli/src/main.rs` で `env_remove` を反映

## フェーズ2: テスト（TDD）

- [x] **T101** [US4] `crates/gwt-cli/src/tui/screens/environment.rs` に統合表示の分類テストを追加
- [x] **T102** [US4] `crates/gwt-cli/src/tui/screens/environment.rs` にスクロールオフセット更新テストを追加
- [x] **T103** [US6] `crates/gwt-cli/src/tui/screens/environment.rs` の hidden 表示マスクテストを更新
- [x] **T104** [US6] `crates/gwt-cli/src/tui/app.rs` の編集確定処理テストを維持
- [x] **T105** [US3] `crates/gwt-cli/src/tui/screens/profiles.rs` にアクション表示の Enter 表記テストを追加
- [x] **T106** [US4] `crates/gwt-cli/src/tui/screens/environment.rs` に選択種別ヘルパーテストを追加
- [x] **T107** [US4] `crates/gwt-cli/src/tui/screens/environment.rs` に無効化表示の分類テストを更新

## フェーズ3: 統合とチェック

- [x] **T201** [統合] `crates/gwt-cli/Cargo.toml` に紐づく `cargo test -p gwt-cli` を実行
