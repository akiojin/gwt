# SPEC-62c129ca: タスク一覧

## タスク

### T-1: TDDテスト作成

- [x] T-1.1: `test_mouse_single_click_selects_branch` - シングルクリックでブランチ選択
- [x] T-1.3: `test_mouse_double_click_opens_wizard` - ダブルクリックでウィザード起動
- [x] T-1.7: `test_mouse_click_different_branch_resets_double_click` - 異なるブランチクリックでダブルクリックリセット

### T-2: 実装

- [x] T-2.1: `handle_branch_list_mouse` 関数を修正してシングルクリックでフォーカス移動
- [x] T-2.2: URLクリック動作をダブルクリックに変更
- [x] T-2.3: 既存テスト `test_mouse_double_click_selects_branch_and_opens_wizard` の分割・修正

### T-3: 検証

- [x] T-3.1: 全テスト通過確認 (152 passed)
- [x] T-3.2: clippy/fmt チェック
- [x] T-3.3: コミット＆プッシュ