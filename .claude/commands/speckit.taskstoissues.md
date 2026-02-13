---
description: tasks.md のタスクを依存順に GitHub Issue 化します（GitHub リポジトリのみ）。
---

## ユーザー入力

```text
$ARGUMENTS
```

追加の指示があれば必ず反映します。

## 手順概要

1. リポジトリルートで次を実行し、JSON をパースして `FEATURE_DIR` / `AVAILABLE_DOCS` を取得します。
   - `.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks`
2. `tasks.md` のパスを特定します。
3. Git のリモート URL を取得します。

```bash
git config --get remote.origin.url
```

> [!CAUTION]
> **GitHub URL の場合のみ** 次へ進みます（それ以外は中断）。

4. tasks.md の各タスクを GitHub Issue として作成します。

> [!CAUTION]
> **リモート URL と一致しないリポジトリには Issue を作成しないこと。**
