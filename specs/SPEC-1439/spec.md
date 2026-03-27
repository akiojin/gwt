### 概要

gwt が使用する全 GitHub API 操作の一覧と、各操作に必要な最小 PAT スコープを定義する。

### gwt が使用する GitHub 操作と必要スコープ

| カテゴリ | 操作 | スコープ |
|---------|------|---------|
| リポジトリ | `gh repo view` | Contents: Read |
| ブランチ | refs 読み取り / 作成 / 削除 | Contents: Read+Write |
| PR | list / create / edit / merge / review / ready | Pull requests: Read+Write |
| Issue | list / view / create / edit / comment / develop | Issues: Read+Write |
| Release | create / edit | Contents: Write |
| ユーザー | `gh api user` | （認証時に暗黙付与） |

### Fine-grained PAT 最小推奨設定

- **Contents**: Read and Write
- **Pull requests**: Read and Write
- **Issues**: Read and Write
- **Metadata**: Read（暗黙付与）

### 機能要件

| ID | 要件 |
|---|---|
| FR-PAT-001 | README.md / README.ja.md に必要な PAT スコープ一覧を記載 |
| FR-PAT-002 | Fine-grained PAT の推奨設定を記載 |
| FR-PAT-003 | 読み取り専用利用の最小スコープを記載 |

### 受け入れシナリオ

| # | シナリオ | 期待結果 |
|---|---------|---------|
| US1 | README を読んで PAT を設定 | gwt の全機能が利用可能 |
| US2 | 読み取り専用スコープで設定 | ブランチ参照・PR参照が動作 |

### 成功基準

- README.md / README.ja.md に PAT 要件セクションが存在
- markdownlint でエラー・警告なし
