# 調査レポート: Worktree内でのコマンド実行制限機能

**日付**: 2025-11-09
**仕様ID**: SPEC-eae13040
**調査者**: AI Agent

## 1. 既存のコードベース分析

### 1.1 block-cd-command.sh の実装詳細

**ファイルパス**: `.claude/hooks/block-cd-command.sh`

**主要機能**:
- Worktree外への`cd`コマンドをブロック
- Worktree内への`cd`コマンドは許可
- 相対パス・絶対パス・シンボリックリンクを正しく処理

**実装の詳細**:

1. **Worktreeルート取得** (7-11行目)
   ```bash
   WORKTREE_ROOT=$(git rev-parse --show-toplevel 2>/dev/null)
   if [ -z "$WORKTREE_ROOT" ]; then
       WORKTREE_ROOT=$(pwd)
   fi
   ```
   - `git rev-parse --show-toplevel`でWorktreeルートを取得
   - gitリポジトリでない場合は現在のディレクトリをフォールバック

2. **Worktree境界判定関数** (is_within_worktree, 14-51行目)
   - 空のパスやホームディレクトリ(`~`)はWorktree外と判定
   - 相対パスを絶対パスに変換(29-34行目)
   - シンボリックリンクを`realpath`で解決(37-40行目)
   - Worktreeルートのプレフィックスチェック(43-50行目)

3. **複合コマンド処理** (67-104行目)
   - `&&`、`||`、`;`、`|`、`|&`、`&`で分割
   - 各セグメントを個別にチェック
   - リダイレクトやheredocを除外

**改善点**:
- ✅ シンボリックリンク解決が実装済み
- ✅ 相対パス処理が実装済み
- ✅ 複合コマンド分割が実装済み
- ⚠️ Python shlex.split()は未使用(sed/awkベースの分割のため、複雑なクォートで誤動作の可能性)

### 1.2 block-git-branch-ops.sh の実装詳細

**ファイルパス**: `.claude/hooks/block-git-branch-ops.sh`

**主要機能**:
- `git checkout`、`git switch`、`git worktree`を無条件ブロック
- `git branch`は参照系のみ許可、変更系はブロック

**実装の詳細**:

1. **git branch参照系判定関数** (is_read_only_git_branch, 19-118行目)
   - 引数なし(`git branch`)は参照系と判定(23-25行目)
   - Python shlex.split()でトークン化(28-52行目、Pythonが利用可能な場合)
   - 危険なフラグリスト: `-d`, `-D`, `-m`, `-M`, `-c`, `-C`, `--delete`, `--move`, `--copy`等(54行目)
   - 参照系フラグリスト: `--list`, `-l`, `--contains`, `--merged`, `--no-merged`, `--points-at`, `--format`, `--sort`, `--abbrev`(55行目)
   - ショートオプション展開処理(85-97行目、例: `-ld` → `-l -d`)
   - 値を期待するフラグの処理(63-70行目、103-109行目)

2. **コマンド判定ロジック** (148-170行目)
   - `git branch`の場合のみ`is_read_only_git_branch()`を呼び出し(149-154行目)
   - 参照系なら`continue`で許可、それ以外はブロック処理へ
   - `git checkout`、`git switch`、`git worktree`は無条件ブロック

**問題点**:
- ❌ **148行目の正規表現が広すぎる**: `git (checkout|switch|branch|worktree)`が全てマッチ
- ❌ **条件分岐の不備**: 149行目の`if`内で`continue`しても、156行目のブロック処理に到達する可能性
- ✅ Python shlex.split()が実装済み(堅牢なトークン解析)
- ✅ ショートオプション展開が実装済み

**root cause分析**:
148-169行目のロジックは以下の通り:
```bash
if echo "$trimmed_segment" | grep -qE '^git\s+(checkout|switch|branch|worktree)\b'; then
    if echo "$trimmed_segment" | grep -qE '^git\s+branch\b'; then
        # is_read_only_git_branch()呼び出し
        if is_read_only_git_branch "$branch_args"; then
            continue  # 参照系なら許可
        fi
    fi
    # ここに到達した場合はブロック処理
    cat <<EOF ...
fi
```

このロジックでは、`git branch`が149行目でマッチし、`is_read_only_git_branch()`が`true`を返せば`continue`で許可される。**しかし**、`is_read_only_git_branch()`が`false`を返した場合(変更系と判定)、156行目のブロック処理に到達する。

つまり、**現在のロジックは正しく動作するはず**。もしユーザーが「`git branch --list`がブロックされる」と報告した場合、`is_read_only_git_branch()`の判定ロジックに問題がある可能性。

### 1.3 is_read_only_git_branch() 関数の動作原理と改善点

**動作フロー**:
1. 引数が空なら参照系と判定(23-25行目)
2. Python shlex.split()でトークン化(28-52行目)
3. 各トークンをループ処理(58-115行目)
4. 値を期待中のフラグをスキップ(63-70行目)
5. `--`が出現したらブランチ名と判定し、変更系と判定(72-74行目)
6. ショートオプション展開(`-ld` → `-l -d`)(85-97行目)
7. 危険なフラグが見つかれば変更系と判定(90-92行目、100-102行目)
8. 参照系フラグが見つかれば次のトークンへ(93-96行目、104-109行目)
9. オプションでないトークンが出現したらブランチ名と判定し、変更系と判定(113-114行目)

**改善点**:
- ✅ 実装は堅牢で、ほぼ完璧
- ⚠️ `--`以降のトークンをブランチ名と判定しているが、実際には`git branch -- pattern`のようなケースも存在(ただし稀)
- ⚠️ Python shlex.split()が利用不可の環境向けのフォールバック(51行目)は単純な`read -r -a`のため、クォート処理が不完全

## 2. 技術的決定

### 2.1 ファイル操作コマンドのブロック方法

**決定**: 新規フックスクリプト`block-file-ops.sh`を作成

**理由**:
- `mkdir`、`rm`、`touch`、`cp`、`mv`等のファイル操作コマンドは多岐にわたる
- 既存のフックスクリプトに統合すると複雑化
- 独立したフックとして分離することで保守性向上

**実装方針**:
- 対象コマンドリスト: `mkdir`、`rmdir`、`rm`、`touch`、`cp`、`mv`
- 各コマンドの引数を解析し、Worktree外のパスを検出
- `is_within_worktree()`関数を再利用(block-cd-command.shから共通化)

**代替案と棄却理由**:
- block-cd-command.shに統合: ファイル操作コマンドが多く、スクリプトが肥大化
- 全てのBashコマンドを禁止: 柔軟性が失われ、ユーザビリティが低下

### 2.2 git checkout -- file と git checkout branch の区別方法

**決定**: `--`の有無を明示的にチェック

**理由**:
- `git checkout -- file`はファイル復元(許可)
- `git checkout branch`はブランチ切り替え(禁止)
- `--`の有無で明確に区別可能

**実装方針**:
```bash
if echo "$trimmed_segment" | grep -qE '^git\s+checkout\s+--\s'; then
    continue  # ファイル復元なら許可
fi
if echo "$trimmed_segment" | grep -qE '^git\s+checkout\b'; then
    # ブロック処理(ブランチ切り替えと判定)
fi
```

**代替案と棄却理由**:
- 引数がファイルパスかブランチ名かを判定: 複雑すぎて誤判定のリスク
- `git checkout`を全て禁止: ファイル復元ができなくなり、ユーザビリティが低下

### 2.3 複合コマンド内のコマンド分割ロジックの精度向上

**決定**: Python shlex.split()を優先使用、フォールバックとして現行のsed/awk

**理由**:
- Python shlex.split()はクォート、エスケープ、ヒアドキュメントを正しく処理
- 既にblock-git-branch-ops.shで実装済み
- sedベースの分割は複雑なケースで誤動作の可能性

**実装方針**:
- block-cd-command.shにもPython shlex.split()を導入
- Python3が利用不可の環境向けにsedベースのフォールバックを維持

**代替案と棄却理由**:
- Bashの組み込み機能のみで実装: 複雑すぎて保守が困難
- 複合コマンドを全て禁止: ユーザビリティが大幅に低下

## 3. 制約と依存関係

### 3.1 jqコマンドの利用可能性とバージョン互換性

**調査結果**:
- jq 1.5以上が必要(JSON解析のため)
- 既存のフックスクリプトでjqを使用中
- `.tool_name`、`.tool_input.command`等の基本的な構文のみ使用

**決定**:
- jq 1.5以上を必須要件とする
- 互換性のある基本構文のみ使用

**リスク**:
- jqが利用不可の環境では動作しない
- **緩和策**: README.mdにjqのインストール方法を記載

### 3.2 realpathコマンドのフォールバック実装

**調査結果**:
- macOSではrealpathがデフォルトで利用不可(coreutilsインストールが必要)
- block-cd-command.shでは`command -v realpath`でチェック済み

**決定**:
- `realpath`が利用可能な場合はそれを使用
- 利用不可の場合はPythonやpwdでフォールバック

**実装方針**:
```bash
if command -v realpath >/dev/null 2>&1; then
    resolved_path=$(realpath -m "$abs_path" 2>/dev/null)
else
    # Pythonフォールバック
    resolved_path=$(python3 -c "import os; print(os.path.realpath('$abs_path'))" 2>/dev/null)
fi
```

**代替案と棄却理由**:
- realpathを必須とする: macOSユーザーの利便性が低下
- シンボリックリンク解決を諦める: セキュリティリスク

### 3.3 Python3の利用可能性

**調査結果**:
- block-git-branch-ops.shでPython3を使用中(shlex.split()のため)
- macOS、Linux、WSLではデフォルトで利用可能

**決定**:
- Python3を推奨要件とする(必須ではない)
- Python3が利用不可の場合はフォールバック実装を使用

**リスク**:
- Python3が利用不可の環境では精度が低下
- **緩和策**: README.mdにPython3のインストールを推奨

## 4. ベストプラクティス調査

### 4.1 Bashフックスクリプトのベストプラクティス

**調査結果**:
- ShellCheck互換コードを書く(SC2155、SC2269等の警告を回避)
- エラーハンドリングを明示的に行う(`set -euo pipefail`は使わない、フックの性質上)
- ロギングをstderrに出力
- JSON出力は標準出力に出力

**適用方針**:
- ShellCheckで警告が出ないコードを書く
- エラー時はexit 2でブロックを明示
- 成功時はexit 0で許可を明示

### 4.2 Claude Code PreToolUseフック仕様

**調査結果**:
- JSON入力: `{"tool_name": "Bash", "tool_input": {"command": "..."}}`
- JSON出力(ブロック時): `{"decision": "block", "reason": "...", "stopReason": "..."}`
- 終了コード: 0=許可、2=ブロック

**適用方針**:
- 既存のフックスクリプトと同じフォーマットを踏襲
- エラーメッセージは日本語と英語の併記(既存スクリプトに倣う)

## 5. 結論

### 5.1 主要な決定事項

1. **既存フックの改善**:
   - block-cd-command.sh: Python shlex.split()を導入
   - block-git-branch-ops.sh: `git checkout -- file`の許可を追加

2. **新規フック作成**:
   - block-file-ops.sh: ファイル操作コマンドのWorktree境界チェック

3. **共通化**:
   - `is_within_worktree()`関数を共通ライブラリ化(DRY原則)

4. **依存関係**:
   - jq 1.5以上: 必須
   - Python3: 推奨(フォールバックあり)
   - realpath: 推奨(フォールバックあり)

### 5.2 次のステップ

1. Phase 1: データモデル設計(data-model.md)
2. Phase 1: クイックスタートガイド(quickstart.md)
3. Phase 2: タスク生成(tasks.md)
4. 実装・テスト・デプロイ

### 5.3 リスク

| リスク | 影響 | 緩和策 |
|--------|------|--------|
| jq非互換環境 | 高 | README.mdにインストール方法記載 |
| realpath非互換環境 | 中 | Pythonフォールバック実装 |
| Python3非互換環境 | 低 | sedベースフォールバック維持 |
| 複雑なクォート処理 | 低 | shlex.split()で大部分をカバー |
