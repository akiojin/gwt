# クイックスタート: GitView画面実装

**仕様ID**: `SPEC-1ea18899` | **日付**: 2026-02-02

## セットアップ

### 1. 必要なファイル

```text
crates/gwt-cli/src/tui/
├── app.rs              # Screen/Message enum, Model 修正
├── screens/
│   ├── mod.rs          # git_view モジュール追加
│   ├── git_view.rs     # 【新規】GitView画面実装
│   └── branch_list.rs  # Detailsパネル削除
```

### 2. 依存関係

`Cargo.toml` への追加は不要（既存の依存関係で対応可能）

## 開発ワークフロー

### Step 1: Screen enum に GitView 追加

```rust
// crates/gwt-cli/src/tui/app.rs

pub enum Screen {
    BranchList,
    // ... 既存
    GitView,  // 追加
}
```

### Step 2: Model に状態追加

```rust
// crates/gwt-cli/src/tui/app.rs

pub struct Model {
    // ... 既存フィールド
    pub git_view: GitViewState,
    pub git_view_cache: GitViewCache,
}
```

### Step 3: Message enum 拡張（必要に応じて）

```rust
pub enum Message {
    // ... 既存
    // GitView専用メッセージは不要
    // NavigateTo(Screen::GitView) で画面遷移
    // SelectNext/SelectPrev/Space/Enter は共通利用
}
```

### Step 4: キーハンドラに v キー追加

```rust
// handle_key_event 内
KeyCode::Char('v') => {
    if matches!(self.screen, Screen::BranchList) {
        // GitView画面へ遷移
        if let Some(branch) = self.branch_list.selected_branch() {
            self.git_view = GitViewState::new(branch);
            return Some(Message::NavigateTo(Screen::GitView));
        }
    } else if matches!(self.screen, Screen::GitView) {
        // ブランチ一覧へ戻る
        return Some(Message::NavigateBack);
    }
    None
}
```

### Step 5: render 関数追加

```rust
// crates/gwt-cli/src/tui/screens/git_view.rs

pub fn render_git_view(
    state: &mut GitViewState,
    frame: &mut Frame,
    area: Rect,
) {
    // 垂直レイアウト
    let chunks = Layout::vertical([
        Constraint::Length(3),  // ヘッダー
        Constraint::Min(10),    // Files
        Constraint::Length(8),  // Commits
    ]).split(area);

    render_header(state, frame, chunks[0]);
    render_files(state, frame, chunks[1]);
    render_commits(state, frame, chunks[2]);
}
```

### Step 6: view 関数で GitView を呼び出し

```rust
// crates/gwt-cli/src/tui/app.rs の view 関数内

Screen::GitView => {
    render_git_view(&mut self.git_view, frame, content_area);
}
```

## よくある操作

### ナビゲーション実装

```rust
impl GitViewState {
    pub fn select_next(&mut self) {
        let max_index = self.total_item_count();
        if self.selected_index < max_index - 1 {
            self.selected_index += 1;
        }
    }

    pub fn select_prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn toggle_expand(&mut self) {
        if self.expanded.contains(&self.selected_index) {
            self.expanded.remove(&self.selected_index);
        } else {
            self.expanded.insert(self.selected_index);
        }
    }
}
```

### キャッシュ更新

```rust
impl GitViewCache {
    pub fn get(&self, branch: &str) -> Option<&GitViewData> {
        self.data.get(branch)
    }

    pub fn insert(&mut self, branch: String, data: GitViewData) {
        self.data.insert(branch, data);
    }

    pub fn clear(&mut self) {
        self.data.clear();
    }
}
```

### バックグラウンド取得

```rust
// 既存パターンに従う
fn spawn_git_view_cache_update(&mut self) {
    let branches: Vec<_> = self.branch_list.branches
        .iter()
        .filter(|b| b.has_worktree)
        .cloned()
        .collect();

    let tx = self.git_view_cache_tx.clone();
    let repo_root = self.repo_root.clone();

    thread::spawn(move || {
        for branch in branches {
            if let Some(path) = &branch.worktree_path {
                let data = build_git_view_data(path);
                let _ = tx.send(GitViewCacheUpdate {
                    branch: branch.name.clone(),
                    data,
                });
            }
        }
    });
}
```

## テスト

### ユニットテスト例

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gitview_select_next() {
        let mut state = GitViewState::default();
        state.files = vec![FileEntry::default(); 5];
        state.commits = vec![CommitEntry::default(); 3];

        state.select_next();
        assert_eq!(state.selected_index, 1);
    }

    #[test]
    fn test_gitview_toggle_expand() {
        let mut state = GitViewState::default();
        state.selected_index = 2;

        state.toggle_expand();
        assert!(state.expanded.contains(&2));

        state.toggle_expand();
        assert!(!state.expanded.contains(&2));
    }
}
```

## トラブルシューティング

### 画面が表示されない

1. `Screen::GitView` が `view()` で処理されているか確認
2. `NavigateTo(Screen::GitView)` が正しく発行されているか確認
3. `screen_stack` にプッシュされているか確認

### キャッシュが更新されない

1. `spawn_git_view_cache_update()` が呼ばれているか確認
2. チャネルの送受信が正しいか確認
3. `apply_git_view_cache_update()` が `Tick` で呼ばれているか確認

### マウスクリックが効かない

1. `handle_gitview_mouse()` が実装されているか確認
2. `pr_link_region` が正しく設定されているか確認
3. `open_url()` が呼ばれているか確認
