# フェーズ0 調査: Worktree側での.gitignore更新と重複登録防止

## 目的

- Worktree内で `claude-worktree` を実行した際に `.gitignore` がルート作業ツリーへ書き込まれてしまう原因を特定する
- `.worktrees/` エントリーの重複追記が発生する条件を洗い出し、正規化戦略を決める

## 現状の挙動

1. `src/worktree.ts#createWorktree` は `config.repoRoot` を `git rev-parse --git-common-dir` 起点で取得し、常にルート作業ツリーの `.gitignore` を更新している
2. `ensureGitignoreEntry` は `content.split("\n")` で行を抽出しているため、CRLF (`\r\n`) の場合に末尾 `\r` が残り、`lines.includes(entry)` が `false` となり重複が発生する
3. ルート作業ツリーとworktreeは別ディレクトリを共有しないため、ルート側の `.gitignore` が毎回ダーティ状態になる

## 調査結果

- `git rev-parse --show-toplevel` は現在の作業ツリーのルートを返すため、worktree側で実行すればworktree内の `.gitignore` を特定できる
- 改行コード検出は既存ファイル内に `\r\n` が含まれるかで判断でき、書き込み時に同じ改行を使用すれば差分が最小化される
- `.gitignore` が存在しない場合でも `fs.writeFile` で新規作成するだけで十分で、特別なテンプレートは要らない

## 技術的決定

1. **ルート決定**: `getRepositoryRoot` とは別に "現在のworktreeルート" を返す `getWorktreeRoot()` を `src/git.ts` に追加し、`createWorktree` ではこちらを使用する
2. **重複検出**: `ensureGitignoreEntry` で `content.split(/\r?\n/)` + `line.trim()` を用いて行を比較し、改行コードを保存する
3. **書き込みポリシー**: ファイルが改行で終わっていない場合は既存スタイルで改行を追加してから `.worktrees/` を追記する
4. **エラー処理**: `.gitignore` 更新失敗時は現行と同様に警告ログのみ（仕様US3）

## リスク

- スクリプト実行場所によって `git rev-parse --show-toplevel` が失敗する可能性 → 例外をキャッチし、フォールバックとして従来の `config.repoRoot` を使用
- `.gitignore` が非常に大きい場合のパフォーマンス → 1回読み込み/書き込みなので問題なしだが、split時にメモリ確保が必要な点だけ把握
