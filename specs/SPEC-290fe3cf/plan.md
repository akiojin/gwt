# 実装計画: Worktree側での.gitignore更新と重複登録防止

**仕様ID**: `SPEC-290fe3cf` | **日付**: 2025-11-07 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-290fe3cf/spec.md` からの機能仕様

## 概要

- `createWorktree` が `.gitignore` をルートブランチ側へ書き込む不具合を解消し、現在のworktreeルートのみを対象にする
- `.gitignore` の CRLF 行末でも `.worktrees/` を重複登録しないための正規化・改行検出ロジックを `ensureGitignoreEntry` に追加
- エラー時の挙動（警告のみ）を維持しつつ、ユニットテストで回帰防止を行う

## 技術コンテキスト

**言語/バージョン**: TypeScript 5.x / Bun 1.x (Node.js 18 相当)
**主要な依存関係**: `execa` (Gitコマンド), `node:fs/promises` (ファイル操作)
**ストレージ**: Gitワーキングツリー
**テスト**: Vitest (`tests/unit/git.test.ts`, `tests/unit/worktree.test.ts`)
**ターゲットプラットフォーム**: CLI (Unix / macOS / Windows)
**プロジェクトタイプ**: 単一CLIプロジェクト (`src/` + `tests/`)
**パフォーマンス目標**: `.gitignore` 更新は < 50ms (ファイルサイズに依存しないO(N)処理)
**制約**: 既存API互換（`ensureGitignoreEntry` のシグネチャ維持）、Worktree作成のメインフローを阻害しない
**スケール/範囲**: 単一ファイル（`src/git.ts`, `src/worktree.ts`）と関連テストの改修

## 原則チェック

- ✅ シンプルさ最優先（ロジックは純粋関数化せず既存関数を拡張）
- ✅ ユーザビリティ重視（ルートブランチを汚さない）
- ✅ Worktree設計思想（既存ブランチを勝手に切らない）
- ✅ Spec Kit順守（仕様→計画→タスク→実装の順）

## プロジェクト構造

### ドキュメント（この機能）

```
specs/SPEC-290fe3cf/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── tasks.md (後述)
└── checklists/requirements.md
```

### ソースコード（抜粋）

```
src/
├── git.ts               # ensureGitignoreEntry, (新規)getWorktreeRoot
└── worktree.ts          # createWorktree 内での gitignore 更新箇所

tests/
└── unit/
    ├── git.test.ts      # ensureGitignoreEntry テスト
    └── worktree.test.ts # createWorktree フロー
```

## フェーズ0: 調査（完了）

- `getRepositoryRoot` が共通 `.git` 親を返すためworktree側を書き換えてしまう
- CRLF の `\r` を考慮していないことが重複原因

## フェーズ1: 設計

1. `src/git.ts`
   - `getWorktreeRoot()` を追加 (`git rev-parse --show-toplevel`)
   - `ensureGitignoreEntry` を以下へ書き換え
     - 改行検出: `const eol = content.includes("\r\n") ? "\r\n" : "\n"`
     - 行比較: `split(/\r?\n/).map(line => line.trim())`
     - 末尾改行不足時の補正
2. `src/worktree.ts`
   - `.gitignore` 更新箇所で `getWorktreeRoot()` を呼び出し、失敗時のみ `config.repoRoot` にフォールバック
   - ログメッセージにどのパスを対象にしたか含める（任意）

## フェーズ2: タスク生成

- `/speckit.tasks` 相当のファイル（tasks.md）に分解済み（後述）

## 実装戦略

1. まず `git.ts` への変更とユニットテストを実施（ロジック変更が局所的なため）
2. 続いて `worktree.ts` を更新し、関連テストを調整
3. 並行してMarkdown/Specファイルは今回のみ（既に作成済み）
4. すべてのテスト・リンターを実行してからコミット

## テスト戦略

- **ユニットテスト**: `ensureGitignoreEntry` で CRLF/末尾改行なし/既存重複/エラーハンドリングをカバー
- **ユニットテスト**: `createWorktree` で `getWorktreeRoot` の呼び出しと `ensureGitignoreEntry` への渡し値をモック検証
- **統合/手動**: 実機で worktree から `claude-worktree` を実行し、ルートブランチの状態を確認

## リスクと緩和策

1. **Gitコマンド失敗**: `getWorktreeRoot` が例外を投げる → try/catch し、従来の `config.repoRoot` へフォールバック + 警告
2. **Windows特有のパス**: `path.join` で問題なしだが、テストでCRLFケースを追加して回避
3. **大規模 `.gitignore`**: 単純な split なのでパフォーマンス懸念は低い。必要ならストリーミングに切替可能だが今回は不要

## 次のステップ

1. ✅ 仕様作成
2. ✅ 計画作成（本ドキュメント）
3. ⏭️ `specs/SPEC-290fe3cf/tasks.md` を確定
4. ⏭️ タスクに従って実装・テスト
