---
description: "環境変数プロファイル機能（OpenTUI）のタスク"
---

# タスク: 環境変数プロファイル機能（OpenTUI）

**入力**: `/specs/SPEC-dafff079/` からの仕様書・計画書
**前提条件**: `specs/SPEC-dafff079/spec.md`, `specs/SPEC-dafff079/plan.md`

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能
- **[ストーリー]**: US1..US6
- 説明に正確なファイルパスを含める

## フェーズ1: 既存プロファイル状態の整理（US1/US2）

- [ ] **T001** [US1] `src/cli/ui/App.solid.tsx` に `refreshProfiles()` を追加し、`profiles.yaml` 読み込み結果とアクティブ表示を一元化

## フェーズ2: プロファイルエディター（US3）

- [ ] **T101** [P] [US3] `src/cli/ui/screens/solid/ProfileScreen.tsx` に `e/n/d` 操作を追加し、選択中プロファイルを通知できるようにする
- [ ] **T102** [US3] `src/cli/ui/App.solid.tsx` にプロファイル編集・作成・削除画面への遷移を追加する

## フェーズ3: OS環境変数の表示（US4）

- [ ] **T201** [P] [US4] `src/cli/ui/screens/solid/EnvironmentScreen.tsx` に上書きキーの強調表示を追加する
- [ ] **T202** [US4] `src/cli/ui/App.solid.tsx` で OS 環境変数一覧を生成し、閲覧画面へ遷移できるようにする

## フェーズ4: 作成・削除（US5）

- [ ] **T301** [P] [US5] `src/cli/ui/screens/solid/InputScreen.tsx` に入力幅指定を追加し、プロファイル作成入力に適用する
- [ ] **T302** [US5] `src/cli/ui/App.solid.tsx` でプロファイル作成フロー（入力→保存→一覧更新）を実装する
- [ ] **T303** [US5] `src/cli/ui/App.solid.tsx` でプロファイル削除フロー（確認→削除→一覧更新）を実装する

## フェーズ5: 環境変数の編集（US6）

- [ ] **T401** [P] [US6] `src/cli/ui/screens/solid/ProfileEnvScreen.tsx` を新規追加し、環境変数一覧と `a/e/d` 操作を実装する
- [ ] **T402** [US6] `src/cli/ui/App.solid.tsx` で環境変数の追加/編集/削除フローを実装する

## フェーズ6: 統合とチェック

- [ ] **T501** [統合] `bun run build` を実行し、OpenTUIのビルドが成功することを確認する
- [ ] **T502** [統合] `bun run format:check` / `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore` / `bun run lint` を実行し、lint要件を満たす

## 追加作業: 入力モードのショートカット無効化 (2026-01-14)

### テスト（TDD）

- [x] **T701** [US7] `crates/gwt-cli/src/tui/app.rs` にプロファイル作成入力中のショートカット抑止テストを追加
- [x] **T702** [US7] `crates/gwt-cli/src/tui/app.rs` に環境変数入力中のショートカット抑止テストを追加
- [x] **T703** [US8] `crates/gwt-cli/src/tui/screens/environment.rs` に空値プレースホルダーが保存値へ混入しないテストを追加

### 実装

- [x] **T704** [US7] `crates/gwt-cli/src/tui/app.rs` に入力モード時のキー処理優先（ショートカット無効化）を実装
