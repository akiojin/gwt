# 機能仕様: Git Branch Operations Block Hook

**仕様ID**: `SPEC-hook-block-ops`
**作成日**: 2025-11-09
**ステータス**: 実装済み
**入力**: ユーザー説明: "Block dangerous git operations (branch switching, interactive rebase) in Worktree environment via PreToolUse hook"

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - Interactive Rebase のブロック (優先度: P1)

LLMエージェントとして、`git rebase -i origin/main` を実行しようとしたときに、Worktree環境で失敗率が高いため事前にブロックされ、セッションが中断されることを防ぎたい。

**この優先度の理由**: LLMによるインタラクティブrebaseは成功率が極めて低く、作業セッションの中断を引き起こすため最優先で防止する必要がある。

**独立したテスト**: `bun test tests/unit/hooks/block-git-branch-ops.test.ts -t "Interactive rebase blocking"` を実行し、`git rebase -i origin/main` が確実にブロックされることを検証できる。

**受け入れシナリオ**:

1. **前提条件** Worktree環境でLLMエージェントが動作している、**操作** `git rebase -i origin/main` を実行、**期待結果** exit code 2 で即座にブロックされ、"Interactive rebase against origin/main is not allowed" というメッセージが表示される。
2. **前提条件** Worktree環境でLLMエージェントが動作している、**操作** `git rebase --interactive origin/main` を実行、**期待結果** `-i` と同様にブロックされる。
3. **前提条件** Worktree環境でLLMエージェントが動作している、**操作** `git rebase origin/main` (非インタラクティブ) を実行、**期待結果** 正常に許可され実行される (exit code 0)。
4. **前提条件** Worktree環境でLLMエージェントが動作している、**操作** `git rebase -i develop` を実行、**期待結果** origin/main以外の場合は許可され実行される。

---

### ユーザーストーリー 2 - ブランチ切り替えのブロック (優先度: P1)

開発者として、Worktree環境では起動したブランチで作業を完結させる設計のため、`git checkout`、`git switch` によるブランチ切り替えをブロックしたい。

**この優先度の理由**: Worktreeの設計思想の根幹であり、ブランチ切り替えを許可するとWorktreeの利点が失われるため。

**独立したテスト**: `bun test tests/unit/hooks/block-git-branch-ops.test.ts -t "Branch switching blocking"` を実行し、ブランチ切り替えコマンドが確実にブロックされることを検証できる。

**受け入れシナリオ**:

1. **前提条件** Worktree環境で作業中、**操作** `git checkout main` を実行、**期待結果** exit code 2 でブロックされ、"Branch switching, creation, and worktree commands are not allowed" というメッセージが表示される。
2. **前提条件** Worktree環境で作業中、**操作** `git switch develop` を実行、**期待結果** 同様にブロックされる。
3. **前提条件** Worktree環境で作業中、**操作** `git checkout -b new-branch` を実行、**期待結果** 新規ブランチ作成も含めてブロックされる。

---

### ユーザーストーリー 3 - ブランチ操作のブロック (優先度: P2)

開発者として、Worktree環境ではブランチの削除・リネーム・作成などの破壊的操作をブロックし、誤操作を防ぎたい。

**この優先度の理由**: ブランチの削除やリネームは取り消しが困難な操作であり、Worktree環境での実行は意図しないデータ損失を招く可能性がある。

**独立したテスト**: `bun test tests/unit/hooks/block-git-branch-ops.test.ts -t "Branch operations blocking"` を実行し、破壊的ブランチ操作がブロックされ、読み取り専用操作が許可されることを検証できる。

**受け入れシナリオ**:

1. **前提条件** Worktree環境で作業中、**操作** `git branch -d test-branch` を実行、**期待結果** exit code 2 でブロックされる。
2. **前提条件** Worktree環境で作業中、**操作** `git branch -D test-branch` を実行、**期待結果** 強制削除もブロックされる。
3. **前提条件** Worktree環境で作業中、**操作** `git branch -m old new` を実行、**期待結果** リネームもブロックされる。
4. **前提条件** Worktree環境で作業中、**操作** `git branch` (引数なし) を実行、**期待結果** exit code 0 で許可され、ブランチ一覧が表示される。
5. **前提条件** Worktree環境で作業中、**操作** `git branch --list` を実行、**期待結果** 読み取り専用オプションのため許可される。
6. **前提条件** Worktree環境で作業中、**操作** `git branch -a` を実行、**期待結果** すべてのブランチ表示のため許可される。
7. **前提条件** Worktree環境で作業中、**操作** `git branch --merged` を実行、**期待結果** マージ済みブランチ表示のため許可される。

---

### ユーザーストーリー 4 - Worktree操作のブロック (優先度: P2)

開発者として、既存のWorktree環境内から新しいWorktreeを作成したり削除したりする操作をブロックし、Worktree階層の混乱を防ぎたい。

**この優先度の理由**: Worktree内でWorktreeを操作すると状態管理が複雑になり、意図しない動作を引き起こす可能性がある。

**独立したテスト**: `bun test tests/unit/hooks/block-git-branch-ops.test.ts -t "Worktree operations blocking"` を実行し、worktreeコマンドがブロックされることを検証できる。

**受け入れシナリオ**:

1. **前提条件** Worktree環境で作業中、**操作** `git worktree add /tmp/test main` を実行、**期待結果** exit code 2 でブロックされる。
2. **前提条件** Worktree環境で作業中、**操作** `git worktree remove test` を実行、**期待結果** 削除操作もブロックされる。

---

### ユーザーストーリー 5 - 安全なGitコマンドの許可 (優先度: P1)

開発者として、`git status`、`git add`、`git commit`、`git push` などの通常の開発フローで必要なコマンドは制限なく使用できるようにしたい。

**この優先度の理由**: 開発作業の基本的なコマンドが使えないとWorktreeの利便性が失われるため、必ず許可する必要がある。

**独立したテスト**: `bun test tests/unit/hooks/block-git-branch-ops.test.ts -t "Safe git commands"` を実行し、安全なgitコマンドが全て許可されることを検証できる。

**受け入れシナリオ**:

1. **前提条件** Worktree環境で作業中、**操作** `git status` を実行、**期待結果** exit code 0 で正常に実行される。
2. **前提条件** Worktree環境で作業中、**操作** `git add .` を実行、**期待結果** 正常に実行される。
3. **前提条件** Worktree環境で作業中、**操作** `git commit -m "message"` を実行、**期待結果** 正常に実行される。
4. **前提条件** Worktree環境で作業中、**操作** `git push` を実行、**期待結果** 正常に実行される。
5. **前提条件** Worktree環境で作業中、**操作** `git log` を実行、**期待結果** 正常に実行される。
6. **前提条件** Worktree環境で作業中、**操作** `git diff` を実行、**期待結果** 正常に実行される。

---

### ユーザーストーリー 6 - 複合コマンドの処理 (優先度: P2)

開発者として、`&&` や `||` で連結された複合コマンドの中に危険なコマンドが含まれている場合は確実にブロックし、安全なコマンドチェーンは許可したい。

**この優先度の理由**: 実際の開発ではコマンドを連結して使用することが多く、部分的なブロックができないと利便性が損なわれる。

**独立したテスト**: `bun test tests/unit/hooks/block-git-branch-ops.test.ts -t "Compound commands"` を実行し、複合コマンドが正しく解析・ブロックされることを検証できる。

**受け入れシナリオ**:

1. **前提条件** Worktree環境で作業中、**操作** `git add . && git checkout main` を実行、**期待結果** 危険なコマンドが含まれているためexit code 2 でブロックされる。
2. **前提条件** Worktree環境で作業中、**操作** `git add . && git commit -m 'test' && git push` を実行、**期待結果** すべて安全なコマンドのためexit code 0 で許可される。

---

## 技術仕様 *(必須)*

### コンポーネント設計

**フックスクリプト**: `.claude/hooks/block-git-branch-ops.sh`

- **責務**: Claude Code の PreToolUse フックとして、Bash ツールの実行前にコマンドを検査し、危険な操作をブロックする
- **入力**: JSON 形式の tool_name および tool_input (stdin から受け取る)
- **出力**:
  - 許可の場合: exit code 0
  - ブロックの場合: exit code 2 + JSON形式のエラーメッセージ (stdout) + エラーメッセージ (stderr)
- **処理フロー**:
  1. stdin から JSON を読み取り
  2. tool_name が "Bash" でない場合は即座に許可 (exit 0)
  3. tool_input.command を取得
  4. コマンドを `&&`, `||`, `;`, `|` で分割して個別に解析
  5. 各セグメントに対して以下をチェック:
     - `git rebase -i origin/main` または `git rebase --interactive origin/main` の場合はブロック
     - `git checkout`, `git switch` の場合はブロック
     - `git branch` の場合は引数を解析し、読み取り専用オプションのみ許可
     - `git worktree` の場合はブロック
  6. すべてのセグメントが安全な場合は許可 (exit 0)

**読み取り専用 git branch 判定関数**: `is_read_only_git_branch()`

- **責務**: `git branch` コマンドの引数を解析し、読み取り専用操作かどうかを判定
- **入力**: git branch の引数文字列
- **出力**:
  - 読み取り専用の場合: return 0 (true)
  - 破壊的操作の場合: return 1 (false)
- **判定ロジック**:
  - 引数なし → 読み取り専用
  - 危険なフラグ (`-d`, `-D`, `-m`, `-M`, `-c`, `-C`, `--delete`, `--move`, `--copy`, `--force`, etc.) が含まれる → 破壊的
  - 読み取り専用フラグのみ (`--list`, `-a`, `-r`, `--merged`, `--no-merged`, `--contains`, etc.) → 読み取り専用
  - ブランチ名が指定されている (オプション以外のトークン) → 破壊的 (新規作成の可能性)

### データフロー

```
Claude Code Bash ツール呼び出し
  ↓
PreToolUse Hook (.claude/hooks/block-git-branch-ops.sh) 起動
  ↓
JSON 入力読み取り (tool_name, tool_input.command)
  ↓
tool_name が "Bash" か確認
  ↓ Yes
コマンド文字列を演算子で分割
  ↓
各セグメントを解析
  ↓
危険な操作を検出?
  ↓ Yes              ↓ No
exit 2 + エラー     exit 0 (許可)
  ↓
Claude Code がブロックメッセージを表示
```

### エラーハンドリング

- **ブロック時のJSON出力形式**:
```json
{
  "decision": "block",
  "reason": "<簡潔なエラーメッセージ>",
  "stopReason": "<詳細な説明とブロックされたコマンド>"
}
```

- **stderr出力**: ユーザーが直接確認できるエラーメッセージも stderr に出力

### セキュリティ考慮事項

- **コマンドインジェクション対策**:
  - コマンド文字列の解析には sed, grep, xargs などの標準的なツールのみ使用
  - Python が利用可能な場合は shlex.split() でより正確なトークン分割を実行
  - すべての変数は適切にクォートして評価

- **バイパス防止**:
  - `&&`, `||`, `;`, `|`, `|&`, `&`, 改行など、すべての主要なコマンド連結演算子に対応
  - リダイレクトや heredoc 内のコマンドは解析対象外

### パフォーマンス考慮事項

- **高速化**:
  - tool_name が "Bash" でない場合は即座に exit 0 (大部分のツール呼び出しはBash以外)
  - 危険なコマンドを検出した時点で即座に exit 2 (すべてのセグメントを解析しない)

---

## 非機能要件 *(任意)*

### テストカバレッジ

- **ユニットテスト**: `tests/unit/hooks/block-git-branch-ops.test.ts`
  - Interactive rebase blocking: 4 テストケース
  - Branch switching blocking: 3 テストケース
  - Branch operations blocking: 9 テストケース
  - Worktree operations blocking: 2 テストケース
  - Non-Bash tools: 1 テストケース
  - Safe git commands: 6 テストケース
  - Compound commands: 2 テストケース
  - **合計: 27 テストケース**

### ローカライゼーション

- すべてのエラーメッセージは英語で統一
- 日本語メッセージは削除済み (以前は二言語併記だったが、英語のみに変更)

### ドキュメント

- CLAUDE.md に Worktree の基本ルールとして記載:
  > **エージェントはユーザーからの明示的な指示なく新規ブランチの作成・削除を行ってはならない。Worktreeは起動ブランチで作業を完結する設計。**
  >
  > `git rebase -i origin/main` はLLMでの失敗率が高いため禁止（必要な場合は人間が手動で整形すること）

---

## 既知の制限事項 *(任意)*

1. **Python未インストール環境での引数解析精度**: Python が利用できない環境では bash の `read -a` によるフォールバック処理を使用するため、クォートされた引数の解析精度が低下する可能性がある。
2. **複雑なシェルスクリプトの解析**: 関数定義やサブシェル内のコマンドは解析対象外。
3. **エイリアス展開**: git コマンドのエイリアスは展開されないため、エイリアスを使用した場合は検出できない可能性がある。

---

## 依存関係 *(任意)*

### 必須

- `bash` (シェルスクリプト実行環境)
- `jq` (JSON 解析)
- `grep`, `sed`, `xargs` (テキスト処理)

### オプション

- `python3` (より正確な引数トークン分割のため)
