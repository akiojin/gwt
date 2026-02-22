# 機能仕様: Branch Already Exists → Use Existing Branch 確認

**仕様ID**: `SPEC-e1004use`
**作成日**: 2026-02-21
**更新日**: 2026-02-21
**ステータス**: ドラフト
**カテゴリ**: GUI
**依存仕様**: なし

**入力**: ユーザー説明: "Launch Agent で New Branch モードで既存ブランチ名を指定した場合に、既存ブランチとして再起動できるようにする"

## 背景

- Launch Agent で「New Branch」モードで既存ブランチ名を指定すると `[E1004] Branch already exists` エラーが表示されて終了する
- ユーザーは既存ブランチの Worktree を作成して起動したい場合があるが、現状ではエラーで止まるため一度閉じて手動でやり直す必要がある
- UX 改善として、エラー時に「Use Existing Branch」ボタンを表示し、ワンクリックで既存ブランチとして再起動できるようにする

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - E1004 エラー時に既存ブランチで再起動 (優先度: P0)

ユーザーとして、New Branch モードで既存ブランチ名を指定した際に、エラーダイアログから直接そのブランチで再起動したい。

**独立したテスト**: E1004 エラー発生時に「Use Existing Branch」ボタンが表示され、クリックで再起動される

**受け入れシナリオ**:

1. **前提条件** New Branch モードで既存ブランチ名を入力して Launch、**操作** バックエンドが E1004 エラーを返す、**期待結果** LaunchProgressModal に「Use Existing Branch」ボタンが表示される
2. **前提条件** E1004 エラーで「Use Existing Branch」ボタンが表示されている、**操作** ボタンをクリック、**期待結果** モーダルがリセットされ、createBranch を除去した状態で start_launch_job が再呼び出しされる

---

### ユーザーストーリー 2 - 他のエラーでは再起動ボタン非表示 (優先度: P0)

ユーザーとして、E1004 以外のエラーでは「Use Existing Branch」ボタンが表示されないことを期待する。

**独立したテスト**: E1001 等のエラーでは「Use Existing Branch」ボタンが表示されない

**受け入れシナリオ**:

1. **前提条件** Launch 実行中、**操作** バックエンドが E1001 エラーを返す、**期待結果** 「Use Existing Branch」ボタンは表示されず、Close ボタンのみ表示

## エッジケース

- onUseExisting が渡されていない場合、E1004 でもボタンは表示しない
- 再起動後に別のエラーが発生した場合は通常のエラー表示フロー

## 要件 *(必須)*

### 機能要件

- **FR-001**: LaunchProgressModal にオプショナルな `onUseExisting` コールバックプロパティを追加
- **FR-002**: エラーメッセージに `[E1004]` が含まれ、かつ `onUseExisting` が渡されている場合、「Use Existing Branch」ボタンを表示
- **FR-003**: 「Use Existing Branch」ボタンは primary スタイル、「Close」は secondary に変更
- **FR-004**: App.svelte に `handleUseExistingBranch` 関数を追加し、`pendingLaunchRequest` から `createBranch` を除去して再起動

### 非機能要件

- **NFR-001**: バックエンド変更は不要（フロントエンドのみの変更）

## 制約と仮定

- E1004 エラーコードはバックエンドのエラーメッセージ文字列に `[E1004]` として含まれる
- `pendingLaunchRequest` は再起動時に有効な状態で保持されている

## 成功基準 *(必須)*

- **SC-001**: E1004 エラー時に「Use Existing Branch」ボタンが表示され、クリックで既存ブランチとして再起動できる
- **SC-002**: E1004 以外のエラーではボタンが表示されない
- **SC-003**: 既存の LaunchProgressModal テストが引き続きパスする
