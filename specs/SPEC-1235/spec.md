# バグ修正仕様: SVN 混在リポジトリで Migration が失敗する問題を解消する

**仕様ID**: `SPEC-1235`  
**作成日**: 2026-02-27  
**更新日**: 2026-02-27  
**ステータス**: ドラフト  
**カテゴリ**: Core / Migration  
**依存仕様**:

- SPEC-a70a1ece（bare リポジトリ対応・強制マイグレーション）

**入力**: ユーザー説明: "Issue #1235を解決して"

## 背景

- Windows 環境で通常リポジトリを Migration する際、`Creating backup` ステップで失敗する。
- 失敗ログは `.gwt-migration-temp/.../.svn/pristine/...` への `Failed to copy` と `os error 5` を示している。
- 現行実装は dirty main repository の退避処理で再帰コピーを行うため、SVN 管理メタデータ配下のアクセス制御に引っかかる。
- 退避処理を move ベースに変更し、rollback で退避物を戻せるようにする必要がある。

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - SVN 作業コピーが混在していても Migration を完了したい (優先度: P0)

Windows ユーザーとして、Git 管理外の SVN 作業コピーがリポジトリ内に存在しても Migration を成功させたい。

**独立したテスト**: `evacuate_main_repo_files` が `.svn` を含むディレクトリを copy ではなく move で退避し、`restore_evacuated_files` で復元できることを単体テストで検証する。

**受け入れシナリオ**:

1. **前提条件** dirty main repository 配下に `.svn/pristine/...` が存在、**操作** Migration の Phase 4 退避を実行、**期待結果** 再帰コピーを行わず退避先へ move される
2. **前提条件** 退避済みデータが存在、**操作** Migration の Phase 8 復元を実行、**期待結果** 退避物が main worktree に move される

---

### ユーザーストーリー 2 - Migration 失敗時に退避データを失いたくない (優先度: P1)

ユーザーとして、Migration が途中失敗しても dirty main repository から退避されたファイルが source root に戻ってほしい。

**独立したテスト**: rollback が `.gwt-migration-temp` から source root へ退避物を move-back できることを単体テストで検証する。

**受け入れシナリオ**:

1. **前提条件** `.gwt-migration-temp` に退避データが残っている、**操作** rollback 実行、**期待結果** source root へ退避データが戻る
2. **前提条件** backup ディレクトリが存在しない、**操作** rollback 実行、**期待結果** 退避データ復旧処理は実行される

## エッジケース

- 退避マニフェストが壊れている場合でも、rollback は temp ディレクトリ走査へフォールバックして復旧を試みる。
- 退避対象に `.git` / `.worktrees` / `.gwt-*` / `*.git` が含まれる場合は従来どおり除外する。

## 要件 *(必須)*

### 機能要件

- **FR-001**: dirty main repository の退避処理はトップレベルエントリを move で `.gwt-migration-temp` に退避しなければならない。
- **FR-002**: 退避処理は `.git` / `.worktrees` / `.gwt-*` / `*.git` を対象外にしなければならない。
- **FR-003**: 退避対象エントリ名を `evacuation-manifest.json` に保存し、復元時に利用しなければならない。
- **FR-004**: rollback は `.gwt-migration-temp` が存在する場合、退避データを source root へ move-back しなければならない。

### 非機能要件

- **NFR-001**: 公開 API（Tauri command / フロントエンドイベント）を変更しない。
- **NFR-002**: 退避・復元・rollback 復旧の単体テストを追加し、既存 migration テスト群を通過させる。

## 制約と仮定

- source root と `.gwt-migration-temp` は同一ボリューム上に作成される前提で、`rename` による move を採用する。
- 問題の根本原因は再帰コピー時のアクセス拒否であり、move 化で解消できると仮定する。

## 成功基準 *(必須)*

- **SC-001**: `.svn` を含む dirty main repository 退避テストが通過する。
- **SC-002**: rollback 退避復旧テストが通過する。
- **SC-003**: migration 関連の既存テストが回帰なしで通過する。
