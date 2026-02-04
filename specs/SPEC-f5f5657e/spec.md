# 機能仕様: Docker環境統合（エージェント自動起動）

**仕様ID**: `SPEC-f5f5657e`
**作成日**: 2026-02-03
**ステータス**: ドラフト
**入力**: ユーザー説明: "worktree選択後、エージェント起動直前にDockerコンテナを起動し、そのコンテナ内でClaude Code/Codex/Gemini等のコーディングエージェントを実行する。Dockerファイルがある場合はデフォルトで有効。"

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - docker-compose.ymlによるコンテナ起動 (優先度: P1)

開発者がdocker-compose.ymlを含むworktreeを選択すると、gwtが自動でDockerコンテナを起動し、選択したエージェントをコンテナ内で実行する。

**この優先度の理由**: docker-compose.ymlは最も一般的なDocker開発環境構成であり、多くのプロジェクトで使用されている。

**独立したテスト**: docker-compose.ymlを含むworktreeでgwtを起動し、エージェント選択後にコンテナが起動してエージェントがコンテナ内で実行されることを確認。

**受け入れシナリオ**:

1. **前提条件** worktreeにdocker-compose.ymlが存在、**操作** TUIでworktreeを選択しエージェントを起動、**期待結果** docker compose upが実行され、コンテナ内でエージェントが起動する
2. **前提条件** docker-compose.ymlにdb, appの2サービスが定義、**操作** エージェント起動、**期待結果** TUIでサービス選択画面が表示され、選択したサービスでエージェントが起動する
3. **前提条件** 既にコンテナが起動済み、**操作** 同じworktreeでエージェント起動、**期待結果** 既存コンテナを再利用してエージェントがexecで起動する
4. **前提条件** docker-compose.ymlが存在、**操作** エージェント終了、**期待結果** docker compose downが実行されコンテナが停止する
5. **前提条件** 複数サービスのcompose、**操作** サービス選択画面でHostOSを選択、**期待結果** コンテナを起動せずホストでエージェントが起動する
6. **前提条件** 複数サービスのcompose、**操作** サービス選択画面を表示、**期待結果** `HostOS` と `Docker:{service}` が同一リストに表示され、カーソルの背景反転は1行全体に適用される
7. **前提条件** Docker起動が必要、**操作** 起動時にビルド確認を行い「No Build」を選択、**期待結果** `docker compose up` は `--no-build` を使用する
8. **前提条件** Docker起動が必要、**操作** 起動時にビルド確認を行い「Build」を選択、**期待結果** `docker compose up` は `--build` を使用する
9. **前提条件** Dockerfile/compose変更なし、**操作** エージェント起動、**期待結果** Build/No Buildの確認は表示されず `--no-build` が使用される
10. **前提条件** Docker起動が必要、**操作** 起動時にRecreate/Reuse確認を行い「Reuse」を選択、**期待結果** `docker compose up` は `--force-recreate` を付与しない
11. **前提条件** Docker起動が必要、**操作** 起動時にRecreate/Reuse確認を行い「Recreate」を選択、**期待結果** `docker compose up` は `--force-recreate` を使用する
12. **前提条件** ホストにCodexのauth.jsonが存在、**操作** Dockerコンテナ起動、**期待結果** コンテナ内のCodex認証情報がホストのauth.jsonに同期される
13. **前提条件** Docker起動が必要、**操作** 起動時にKeep/Stop確認を行い「Keep」を選択、**期待結果** `docker compose down` は実行されない
14. **前提条件** Docker起動が必要、**操作** 起動時にKeep/Stop確認を行い「Stop」を選択、**期待結果** `docker compose down` が実行される
15. **前提条件** Dockerコンテナが存在しない、**操作** エージェント起動、**期待結果** Recreate/Reuseの確認は表示されない
16. **前提条件** 同一ブランチでQuick Start履歴が存在、**操作** Quick Startで「Resume/Start new」を選択、**期待結果** 以前選択したHostOS/Dockerサービス・Recreate/Reuse・Keep/Stopが復元され、Dockerウィザードは表示されない

---

### ユーザーストーリー 2 - Dockerfileのみの場合のコンテナ起動 (優先度: P1)

開発者がDockerfile（composeなし）を含むworktreeを選択すると、gwtが自動でdocker buildとdocker runを実行してエージェントをコンテナ内で実行する。

**この優先度の理由**: Dockerfile単体での開発も一般的であり、compose不要な軽量プロジェクトで使用される。

**独立したテスト**: Dockerfileのみ存在するworktreeでgwtを起動し、イメージがビルドされてコンテナが起動することを確認。

**受け入れシナリオ**:

1. **前提条件** worktreeにDockerfileのみ存在（compose.ymlなし）、**操作** エージェント起動、**期待結果** docker buildが実行され、docker runでエージェントが起動する
2. **前提条件** Dockerfileがあるがイメージが存在しない、**操作** エージェント起動、**期待結果** 自動でdocker buildが実行される
3. **前提条件** Dockerfileが更新された（イメージより新しい）、**操作** エージェント起動、**期待結果** 再ビルドして新しいイメージでコンテナ起動

---

### ユーザーストーリー 3 - .devcontainerの対応 (優先度: P2)

開発者が.devcontainer/devcontainer.jsonを含むworktreeを選択すると、gwtがdevcontainer設定を解析してDockerコンテナを起動する。

**この優先度の理由**: Dev Containerは VS Code/GitHub Codespaces で標準的な開発環境定義だが、CLIでの利用は補助的。

**独立したテスト**: .devcontainer/devcontainer.jsonを含むworktreeでgwtを起動し、設定に基づいてコンテナが起動することを確認。

**受け入れシナリオ**:

1. **前提条件** .devcontainer/devcontainer.jsonが存在、**操作** エージェント起動、**期待結果** devcontainer設定を解析してdocker composeとして扱い起動する
2. **前提条件** devcontainer.jsonにdockerComposeFileが指定、**操作** エージェント起動、**期待結果** 指定されたcompose fileを使用して起動する

---

### ユーザーストーリー 4 - worktree分離とポート競合解決 (優先度: P1)

複数のworktreeで同時にDockerコンテナを起動する場合、各worktreeで独立したコンテナ環境が作成され、ポート競合が自動で解決される。

**この優先度の理由**: gwtの核心機能は複数worktreeの並行開発であり、Docker環境も同様に分離される必要がある。

**独立したテスト**: 2つのworktreeで同時にエージェントを起動し、それぞれ独立したコンテナが作成されることを確認。

**受け入れシナリオ**:

1. **前提条件** worktree-aとworktree-bの両方にdocker-compose.ymlが存在、**操作** 両方でエージェント起動、**期待結果** gwt-worktree-aとgwt-worktree-bという別々のコンテナが起動する
2. **前提条件** 両worktreeのcompose.ymlで同じポート8080を使用、**操作** 両方でエージェント起動、**期待結果** gwtが空きポートを自動検出し、環境変数で渡して競合を回避する
3. **前提条件** worktree-aのコンテナが起動中、**操作** worktree-bでエージェント起動、**期待結果** worktree-aに影響なく独立して起動する

---

### ユーザーストーリー 5 - Dockerファイルなしの場合のフォールバック (優先度: P1)

開発者がDockerファイルを含まないworktreeを選択すると、従来通りホスト環境でエージェントが起動する。

**この優先度の理由**: 全てのプロジェクトがDocker化されているわけではなく、フォールバックは必須。

**独立したテスト**: Dockerファイルなしのworktreeでgwtを起動し、ホスト環境でエージェントが起動することを確認。

**受け入れシナリオ**:

1. **前提条件** worktreeにDockerファイルなし、**操作** エージェント起動、**期待結果** ホスト環境でエージェントが起動する（従来動作）
2. **前提条件** Dockerファイルなし、**操作** TUI表示、**期待結果** Docker関連のUI要素は表示されない

---

### ユーザーストーリー 6 - Dockerデーモン未起動時の対応 (優先度: P2)

Dockerデーモンが起動していない場合、gwtが自動起動を試み、失敗した場合はホスト環境へフォールバックする。

**この優先度の理由**: Docker Desktop利用者は手動でデーモン起動が必要な場合があり、自動対応が望ましい。

**独立したテスト**: Dockerデーモン停止状態でgwtを起動し、自動起動試行後にホストへフォールバックすることを確認。

**受け入れシナリオ**:

1. **前提条件** Dockerデーモン停止中、docker-compose.ymlあり、**操作** エージェント起動、**期待結果** デーモン起動を試行し、成功すればコンテナ起動
2. **前提条件** Dockerデーモン停止中、起動試行失敗、**操作** エージェント起動、**期待結果** エラー表示後、ホスト環境でエージェント起動を提案

---

### ユーザーストーリー 7 - 環境変数とAPIキーの継承 (優先度: P1)

コンテナ内でエージェントを起動する際、ホストの環境変数（APIキーなど）が適切にコンテナに渡される。

**この優先度の理由**: ANTHROPIC_API_KEY等がないとエージェントが動作しないため、必須機能。

**独立したテスト**: ホストにANTHROPIC_API_KEYを設定した状態でコンテナ起動し、エージェントがAPIを利用できることを確認。

**受け入れシナリオ**:

1. **前提条件** ホストにANTHROPIC_API_KEY設定済み、**操作** コンテナでClaude起動、**期待結果** コンテナ内のエージェントがAPIキーを使用できる
2. **前提条件** ホストに複数のAI関連環境変数設定、**操作** コンテナ起動、**期待結果** ANTHROPIC_API_KEY、OPENAI_API_KEY、GEMINI_API_KEY等が全て継承される

---

### エッジケース

- Dockerfileが無効な構文の場合、どうなるか？→ ビルドエラーを表示し、ホストへのフォールバックを提案
- docker compose upが3回連続失敗した場合、どうなるか？→ エラー表示し、ホストでの起動を提案
- コンテナ内でエージェントがクラッシュした場合、どうなるか？→ エラーメッセージを表示してgwtを終了
- docker-compose.ymlに複数のcomposeファイル（override等）がある場合、どうなるか？→ 標準のdocker compose挙動に従い全て読み込む
- ホストにdockerコマンドがない場合、どうなるか？→ Docker機能を無効化し、ホストで起動
- worktree名に特殊文字が含まれる場合、どうなるか？→ コンテナ名は英数字とハイフンのみに正規化（gwt-{sanitized_name}）
- 既存コンテナが古いイメージで起動中の場合、どうなるか？→ Dockerfileの更新を検知して再ビルド・再作成
- 複数サービスでホスト起動を選択した場合、どうなるか？→ docker composeを実行せずホストで起動する
- Dockerコンテナが未作成の状態ではどうなるか？→ Recreate/Reuseの確認を省略する
- Codex認証ファイルがコンテナ側で新しくてもホスト側と内容が異なる場合、どうなるか？→ ホストのauth.jsonを優先して同期する
- Quick StartのDocker設定があるがDockerファイルが見つからない場合はどうなるか？→ Docker設定を無視してホストで起動する
- Quick StartでHostOSを選んだ履歴がある場合はどうなるか？→ Dockerウィザードを表示せずホストで起動する
- Dockerfile/compose変更が検出された場合はどうなるか？→ Build/No Build確認を表示し、Build時はRecreateが推奨選択となる

## 詳細仕様

### Docker検出の優先順位

検出は以下の順序で行い、最初に見つかったものを使用:

1. `docker-compose.yml` または `compose.yml`
2. `.devcontainer/devcontainer.json`
3. `Dockerfile`

### コンテナ命名規則

- パターン: `gwt-{sanitized_worktree_name}`
- worktree名の正規化: 英数字とハイフン以外の文字はハイフンに置換
- 例: `feature/auth-login` → `gwt-feature-auth-login`

### ポート競合解決

gwtは以下の方法でポート競合を回避:

1. docker-compose.yml内の`${PORT:-8080}`形式の環境変数を検出
2. 空きポートを自動検出
3. 環境変数としてコンテナに渡す

設定ファイルは不要。docker-compose.ymlの設定に従う。
gwtのcompose定義では`PORT`（デフォルト3000）を使用し、`GWT_PORT`は使用しない。

### 設定について

**専用の設定ファイル（.gwt/docker.toml）は不要。**

- Dockerファイルが存在すれば自動で有効
- エージェントコマンドは既存の.gwt/tools.tomlを使用
- ポート設定はdocker-compose.ymlに従う
- 環境変数の継承はホストの環境変数をそのまま使用
- worktreeの.gitが参照するgitdirはコンテナ内から参照できる必要がある（HOST_GIT_COMMON_DIRをバインド）
- Docker起動前に「Build/No Build」を選択するUIを表示し、デフォルトはNo Build
- Docker起動前に「Build/No Build」を選択するUIを表示するが、Dockerfile/compose変更が検出された場合のみ表示しデフォルトはNo Build
- Docker起動前に「Recreate/Reuse」を選択するUIを表示し、デフォルトはReuse
- Docker起動前に「Keep/Stop」を選択するUIを表示し、デフォルトはKeep
- コンテナのHOMEはホストから引き継がず、コンテナ側のデフォルトに従う
- Quick Start履歴にDocker選択情報（HostOS/Dockerサービス、Recreate/Reuse、Keep/Stop）がある場合はそれを適用し、Dockerウィザードは表示しない
- Quick Start履歴のDockerサービスが現在のcomposeに存在しない場合はサービス選択にフォールバックする
- Build/No BuildはQuick Startでは保存・復元しない

### TUI進捗表示

Docker起動中はスピナーとステータスメッセージを表示:

```
[/] Starting Docker container...
    Building image: gwt-feature-auth
[/] Starting Docker container...
    Waiting for services to be ready
[*] Docker container ready
    Container: gwt-feature-auth
    Services: app (8080), db (5432)
```

### ログ保存

既存のgwtログ機構に統合（`~/.gwt/logs/`配下）:

- Docker関連のログは既存のgwtログファイルに出力
- tracingクレートを使用してカテゴリ`docker`で出力
- 別途Dockerログファイルは作成しない

### エージェント起動フロー

```
1. worktree選択
2. エージェント選択
3. Dockerファイル検出
   ├─ 存在する場合:
   │  a. 既存コンテナ確認
   │  │  ├─ 起動中 → 再利用
   │  │  └─ なし → 新規起動
   │  b. イメージ更新確認
   │  │  ├─ 更新あり → 再ビルド
   │  │  └─ 更新なし → そのまま
   │  c. 複数サービス → TUIで毎回選択（HostOS/Docker:{service}）
   │  d. 再作成確認 → Recreate/Reuseを選択（デフォルトReuse）
   │  e. ビルド確認 → Build/No Buildを選択（デフォルトNo Build）
   │  f. 終了時処理確認 → Keep/Stopを選択（デフォルトKeep）
   │  g. ポート競合確認 → 動的割り当て
   │  h. docker compose up -d (--build or --no-build, --force-recreate optional)
   │  i. コンテナ内でエージェント起動
   │     docker exec -it {container} {agent_command}
   │  h. エージェント終了 → docker compose down
   │
   └─ 存在しない場合:
      └─ ホストでエージェント起動（従来動作）
```

### リトライロジック

Docker起動失敗時は以下の手順:

1. 1回目失敗 → 2秒待機して再試行
2. 2回目失敗 → 5秒待機して再試行
3. 3回目失敗 → エラー表示、ホストへのフォールバック提案

## 要件 *(必須)*

### 機能要件

- **FR-001**: システムはworktree内のdocker-compose.yml/compose.ymlを検出でき**なければならない**
- **FR-002**: システムはworktree内のDockerfileを検出でき**なければならない**
- **FR-003**: システムは.devcontainer/devcontainer.jsonを検出し、docker-composeとして扱え**なければならない**
- **FR-004**: システムはworktreeごとに独立したコンテナ（gwt-{worktree名}）を作成**しなければならない**
- **FR-005**: システムはポート競合を検出し、空きポートを動的に割り当て**なければならない**
- **FR-006**: システムは既存の起動済みコンテナを再利用でき**なければならない**
- **FR-007**: システムはDockerfileが更新された場合、再ビルドして再作成**しなければならない**
- **FR-008**: システムはホストの環境変数（APIキー等）をコンテナに継承**しなければならない**
- **FR-009**: システムはエージェント終了時にdocker compose downで自動停止**しなければならない**
- **FR-010**: システムはDocker起動失敗時に最大3回リトライ**しなければならない**
- **FR-011**: システムはDockerファイルがない場合、ホスト環境でエージェントを起動**しなければならない**
- **FR-012**: システムはDockerデーモン未起動時に自動起動を試行**しなければならない**
- **FR-013**: システムはdocker compose upの出力を既存のgwtログに統合**しなければならない**
- **FR-014**: システムはTUIでDocker起動の進捗（スピナー+ステータス）を表示**しなければならない**
- **FR-015**: システムは複数サービス定義時にTUIでサービス選択を表示**しなければならない**
- **FR-015a**: サービス選択画面でSkipを選択した場合、Docker起動をスキップしホストでエージェントを起動**しなければならない**
- **FR-015b**: サービス選択画面でEscを選択した場合、起動をキャンセル**しなければならない**
- **FR-016**: システムはDocker Desktop/Docker Engineの両方をサポート**しなければならない**
- **FR-017**: この機能はデフォルトで有効であ**らなければならない**（Dockerファイルがある場合）
- **FR-018**: システムはコンテナ内のworking directoryをマウント先のworktreeパスに設定**しなければならない**

### 主要エンティティ

- **DockerManager**: Docker操作を統括。検出、ビルド、起動、停止を担当
- **ContainerRegistry**: worktreeとコンテナの対応を管理。既存コンテナの検索、状態確認
- **PortAllocator**: ポート競合を解決。空きポート検出、割り当て状態の管理

## 成功基準 *(必須)*

### 測定可能な成果

- **SC-001**: docker-compose.ymlがあるworktreeでコンテナ内エージェントが起動できる
- **SC-002**: 複数worktreeで同時にコンテナが独立して起動できる
- **SC-003**: ポート競合が自動解決され、複数コンテナが同時稼働できる
- **SC-004**: Dockerファイルなしの環境で従来通りホスト起動ができる
- **SC-005**: エージェント終了時にコンテナが自動停止する
- **SC-006**: APIキー等の環境変数がコンテナに正しく継承される

## 制約と仮定 *(該当する場合)*

### 制約

- docker-compose.ymlのvolumes/user/network設定はそのまま使用（gwt側で上書きしない）
- コンテナが作成したファイルの所有権問題はユーザー責任（compose側でuser設定）
- gwtのCLI引数にDocker関連オプションは追加しない（自動検出のみ）

### 仮定

- ホスト環境にdockerコマンドがインストールされている
- Dockerデーモンが起動可能な状態にある（権限等）
- worktreeはマウント可能なパスに存在する

## 範囲外 *(必須)*

次の項目は、この機能の範囲外です：

- gwt CLIへのDockerサブコマンド追加（gwt docker start等）
- Kubernetes/Podman対応
- コンテナのリモートホストでの実行
- Docker Swarmモードでの実行
- コンテナ内でのgwt自体の実行（gwt-in-docker）
- Docker Compose v1（レガシー）の明示的サポート

## セキュリティとプライバシーの考慮事項 *(該当する場合)*

- APIキーはコンテナに環境変数として渡されるため、docker inspectで確認可能
- ログファイルにはAPIキーを含む環境変数を記録しない
- コンテナはホストネットワークを使用しない（compose設定に従う）
- SSH_AUTH_SOCKのフォワードはcompose側で設定

## 依存関係 *(該当する場合)*

- 既存のAgentManager機能（agent/mod.rs）
- 既存のTmuxLauncher機能（tmux/launcher.rs）
- 既存のToolsConfig機能（config/tools.rs）
- dockerコマンド（Docker CLI）
- docker composeコマンド（Docker Compose v2）

## 参考資料 *(該当する場合)*

- [Docker Compose仕様](https://docs.docker.com/compose/compose-file/)
- [Dev Container仕様](https://containers.dev/implementors/json_reference/)
- [既存エージェント仕様](../SPEC-a3f4c9df/spec.md)

## 技術スタック

- Rust 2021 Edition (stable) + ratatui 0.29, crossterm 0.28, serde, serde_json, tracing
- ファイルシステム（~/.gwt/logs/ - 既存ログ機構に統合）
