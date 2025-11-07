# 機能仕様: Worktree側での.gitignore更新と重複登録防止

**仕様ID**: `SPEC-290fe3cf`
**作成日**: 2025-11-07
**ステータス**: ドラフト
**入力**: ユーザー説明: "claude-worktreeを起動するたびに.gitignoreへ.worktreeを登録しているが、既に登録済みでも追記され、しかもルートブランチ側が更新されてしまうので、worktree側でのみ登録してほしい"

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - Worktree配下のみでの.gitignore自動更新 (優先度: P1)

開発者が任意のworktreeディレクトリ内で`claude-worktree`を実行すると、`.gitignore`の`.worktrees/`エントリーは現在のworktreeのルートにのみ追記され、ルートブランチ側の作業ツリーは一切変更されない。これにより、他の作業ツリーが意図せずダーティ状態になることを防ぎ、安全にブランチ固有の差分として管理できる。

**この優先度の理由**: ルートブランチが頻繁に汚染されると開発者体験が大幅に低下するため、最も影響が大きいユーザーストーリー。

**独立したテスト**: worktree A で `claude-worktree` を実行し `.gitignore` の変更が worktree A にのみ生じることを確認すれば完了。

**受け入れシナリオ**:

1. **前提条件** worktreeディレクトリ内でコマンドを実行、**操作** `claude-worktree` から新しいworktreeを追加、**期待結果** `.worktrees/` が現在のworktreeルートの `.gitignore` に存在し、ルートブランチ側には変更が無い
2. **前提条件** ルートブランチとworktreeが同時に存在、**操作** worktree側で `claude-worktree` を実行、**期待結果** `git status` をルートブランチで実行しても `.gitignore` に変更が出ない

---

### ユーザーストーリー 2 - 重複登録の排除 (優先度: P1)

`.gitignore` に既に `.worktrees/` エントリーが含まれている場合、改行コードが `\r\n` であっても再度追加されない。これにより、`.gitignore` が毎回差分を持つ問題を根絶し、余計なコミットやレビュー差分を防止する。

**この優先度の理由**: ルートブランチ側のダーティ状態と同様に、重複行によるノイズは毎回の作業を阻害するためP1。

**独立したテスト**: `\r\n` 改行の `.gitignore` を用意し、連続して `ensureGitignoreEntry` を実行しても1行しか追加されないことを確認する。

**受け入れシナリオ**:

1. **前提条件** `.gitignore` がCRLFで `.worktrees/` を含む、**操作** `ensureGitignoreEntry` を2回実行、**期待結果** `.worktrees/` 行が1つだけ存在
2. **前提条件** `.gitignore` が存在しない、**操作** 機能を実行、**期待結果** `.gitignore` が新規作成され `.worktrees/` 行が末尾に1つだけ追加
3. **前提条件** `.gitignore` がLFで末尾改行無し、**操作** 機能を実行、**期待結果** 既存内容に改行が補われた上で `.worktrees/` 行が追記される

---

### ユーザーストーリー 3 - 失敗時の安全なデグレード (優先度: P2)

`.gitignore` の更新に失敗した場合でも worktree 作成自体は成功し、失敗内容が警告として記録される。ユーザーは最小限のリスクで作業を継続でき、必要に応じて手動で `.gitignore` を調整できる。

**この優先度の理由**: クリティカルではないが、失敗時の影響を限定するために必要。

**独立したテスト**: `.gitignore` の書き込みをモックで失敗させ、worktree 作成がエラーにならないことを確認する。

**受け入れシナリオ**:

1. **前提条件** `.gitignore` が読み取り専用、**操作** `createWorktree` を実行、**期待結果** worktree が作成され、警告ログのみ出力

### エッジケース

- `.gitignore` に `\r` だけで終端する古い形式が存在する
- `.gitignore` が巨大であっても1行検索で過剰なメモリを消費しない
- Git管理下ではないディレクトリで誤って実行された場合は早期に例外扱い
- 実行ユーザーに `.gitignore` への書き込み権限が無い場合は警告のみ

## 要件 *(必須)*

### 機能要件

- **FR-001**: `createWorktree` は現在のworktreeルート（`git rev-parse --show-toplevel`）を取得し、その `.gitignore` にのみ `.worktrees/` を確実に追加**しなければならない**
- **FR-002**: `ensureGitignoreEntry` は `\r\n`/`\n` いずれの改行でも既存行を正規化して重複を検知**しなければならない**
- **FR-003**: `.gitignore` が存在しない場合は新規作成し、既存末尾に改行が無い場合は整形した上で追記**しなければならない**
- **FR-004**: `.gitignore` 更新に失敗した場合でも worktree 作成処理は成功扱いとし、警告ログで原因を明示**しなければならない**
- **FR-005**: すべての変更は既存のテスト (`bun test`) を通過し、新規回帰防止テストを追加**しなければならない**

### 主要エンティティ

- **ActiveWorktreeRoot**: `git rev-parse --show-toplevel` で得られる現在の作業ディレクトリ。`.gitignore` 更新対象となる
- **GitignoreEntry**: `.worktrees/` を指す1行のテキスト。終端改行や空白を無視して比較する

## 成功基準 *(必須)*

### 測定可能な成果

- **SC-001**: worktree内で `claude-worktree` を2回連続実行しても `.gitignore` 差分がゼロである
- **SC-002**: ルートブランチで `git status` を実行しても `.gitignore` に変更が生じない
- **SC-003**: `.gitignore` に `.worktrees/` が存在しない状態でコマンドを実行すると1行だけが追加される
- **SC-004**: `bun test` と `bun run lint` が成功し、既存CI要件を満たす

## 制約と仮定 *(該当する場合)*

### 制約

- Git CLI 2.30+ を前提（`rev-parse --show-toplevel` が利用可能）
- Node.js/Bun のバージョンは既存リポジトリ準拠
- `.gitignore` はUTF-8テキストとして扱う

### 仮定

- `.worktrees/` を複数回登録するユースケースは存在しない
- worktreeルートで `claude-worktree` が実行される
- 権限不足やファイルロック時は警告さえ出ればユーザーが手動で対処できる

## 範囲外 *(必須)*

- `.gitignore` の他エントリーの正規化やソート
- `.worktrees/` 以外の無視パターンの自動追加
- worktree作成以外のコマンドロジック変更
- 既存 `.gitignore` の自動整形やコメント挿入

## セキュリティとプライバシーの考慮事項 *(該当する場合)*

- 書き込み失敗時に機密パス情報をエラーに含めない
- 権限エラーは最小限の情報で通知し、環境情報を漏らさない

## 依存関係 *(該当する場合)*

- `execa`（Gitコマンド実行）
- `node:fs/promises`（`.gitignore` 読み書き）
- 既存 `src/worktree.ts` および `src/git.ts`

## 参考資料 *(該当する場合)*

- [src/worktree.ts](../../src/worktree.ts)
- [src/git.ts](../../src/git.ts)
- [tests/unit/git.test.ts](../../tests/unit/git.test.ts)
