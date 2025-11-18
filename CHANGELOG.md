# [2.0.0](https://github.com/akiojin/gwt/compare/v1.33.0...v2.0.0) (2025-11-18)


* refactor!: パッケージ名を@akiojin/claude-worktreeから@akiojin/gwtに変更 ([91a207e](https://github.com/akiojin/gwt/commit/91a207e680ebc3045dcd057e9bde258bf597baff))


### Bug Fixes

* release.ymlでsemantic-releaseの出力をログに表示するように修正 ([9e932a6](https://github.com/akiojin/gwt/commit/9e932a6156942dc81815cf29d2c416689e3f50dd))
* スコープ付きパッケージをpublicとして公開するよう設定 ([a538301](https://github.com/akiojin/gwt/commit/a53830106a9873e9eb77b683513084e97a96fe25))


### BREAKING CHANGES

* パッケージ名が@akiojin/claude-worktreeから@akiojin/gwtに変更されました。
既存のインストールを更新する必要があります:
- グローバルインストール: npm uninstall -g @akiojin/claude-worktree && npm install -g @akiojin/gwt
- コマンド名: claude-worktree → gwt
- 設定ディレクトリ: ~/.config/claude-worktree → ~/.config/gwt

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>

# [1.33.0](https://github.com/akiojin/claude-worktree/compare/v1.32.2...v1.33.0) (2025-11-17)

## Bug Fixes

* **build:** esbuildバージョン不一致エラーの解決 ([12c247d](https://github.com/akiojin/claude-worktree/commit/12c247d40d4ad77a713aab6f038087e7af464b20))
* CLI英語表示を強制 ([280a22a](https://github.com/akiojin/claude-worktree/commit/280a22a303b02cdcf79e10a2c18e81cf57378d6d))
* **config:** satisfy exact optional types ([c2f26dc](https://github.com/akiojin/claude-worktree/commit/c2f26dc49a0907db8b680d1365522dbeebeba046))
* create-release.ymlのdry-runモードでNPM_TOKENエラーを回避 ([8072622](https://github.com/akiojin/claude-worktree/commit/8072622eb3eacf58458bef415e65ac085c48ec2d))
* **docker:** Docker起動時の強制ビルドを削除し開発環境専用に変更 ([83f1880](https://github.com/akiojin/claude-worktree/commit/83f1880572e534aaeb182f376e3055f1f8d701ae))
* **docker:** Docker起動時の強制ビルドを削除し開発環境専用に変更 ([522302a](https://github.com/akiojin/claude-worktree/commit/522302ae10c35807ebccef829bf976e194e28979))
* **docker:** Web UIアクセス用にポート3000を公開 ([9c22ad6](https://github.com/akiojin/claude-worktree/commit/9c22ad6b6493ba4412ecde4d7bea0fbc40f82407))
* **docs:** specs/feature/webui/spec.mdのbare URL修正 ([2663558](https://github.com/akiojin/claude-worktree/commit/266355883edb86bd91d9c5ff54cb174090a29704))
* **docs:** specsディレクトリのmarkdownlintエラーを修正 ([49d39a1](https://github.com/akiojin/claude-worktree/commit/49d39a17f2446232d444c7971e5796cf07c5ca84))
* **lint:** ESLintエラーを修正（未使用変数の削除） ([8bc6744](https://github.com/akiojin/claude-worktree/commit/8bc67442ca6120e98bdfd84dfae32051d2fdd1d9))
* **lint:** ESLint設定を改善してテストファイルのルールを緩和 ([8e5e972](https://github.com/akiojin/claude-worktree/commit/8e5e972c90497c9da73bebe69952d9ef93ea8a75))
* markdownlint の違反を解消 ([f8af3d5](https://github.com/akiojin/claude-worktree/commit/f8af3d5346749d870a559317ba9d6e05bbbee9e8))
* package-lock.jsonをpackage.jsonと同期 ([461a5a6](https://github.com/akiojin/claude-worktree/commit/461a5a6a5bbf4b4e2fb9efcd7cc1139bf983b290))
* **server:** Docker環境からのアクセス対応とビルドパス修正 ([a6c81dc](https://github.com/akiojin/claude-worktree/commit/a6c81dc558358d02ae6835845cb5a72056949ebb))
* **server:** Web UIサーバーをNode.jsで起動するよう修正 ([12d5688](https://github.com/akiojin/claude-worktree/commit/12d568868cf7ce6e06de05986e73d222bc9f0ab0))
* **server:** 型エラー修正とビルドスクリプト最適化 ([33a35e3](https://github.com/akiojin/claude-worktree/commit/33a35e384cf6148ed18e223144b1e2e03d8177e1))
* **test:** dist-app-bundle.testのファイルパスを修正 ([5c1d306](https://github.com/akiojin/claude-worktree/commit/5c1d306fbcd5ae51a6b56a09cfa4853cc9d25b8d))
* **test:** getSharedEnvironmentモックを追加 ([10efc1d](https://github.com/akiojin/claude-worktree/commit/10efc1dc1e634d5dcc59573c6ced9353a7c2bf0a))
* **test:** importパスを正しい../../../git.jsに戻す ([eaa6c81](https://github.com/akiojin/claude-worktree/commit/eaa6c81100c66ac48d952380e3a5326f0086e579))
* **test:** main error handlingテストとCI環境でのhookテストスキップを修正 ([4e21662](https://github.com/akiojin/claude-worktree/commit/4e2166229424308ce484c6e07cc06cc05d9c813d))
* **test:** vi.mockのパスも修正してテストのimport問題を完全解決 ([bc26be7](https://github.com/akiojin/claude-worktree/commit/bc26be726dd607dd6eb0d7d77be08131147ff19b))
* **test:** vitest.config.tsをESLintの対象に追加し、拡張子解決を改善 ([469747e](https://github.com/akiojin/claude-worktree/commit/469747edce0162958cd24440503e4a3d3d6babad))
* **test:** テストファイルのimportパス修正 ([767224e](https://github.com/akiojin/claude-worktree/commit/767224e302a9676c74cd7bcf563ef79496baeff9))
* **test:** テストファイルのインポートパスとモックを修正 ([b6a6ce0](https://github.com/akiojin/claude-worktree/commit/b6a6ce02c0d6bf74f72989ee7361a13335831308))
* **test:** テストファイルのインポートパスを修正して.ts拡張子に対応 ([5ce4794](https://github.com/akiojin/claude-worktree/commit/5ce4794c9d6a2dc0dee44dfece682a49084c8acb))
* **test:** 通常のimport文も../../../../cli/パスに修正 ([baedfb6](https://github.com/akiojin/claude-worktree/commit/baedfb6a2efee6a27081423f38781bc95b142708))
* xterm パッケージの依存関係問題を解決するため--legacy-peer-depsを追加 ([125ca23](https://github.com/akiojin/claude-worktree/commit/125ca232cb44f7bb813a8c96d67741ca6a99816b))
* 依存インストール失敗時のクラッシュを防止 ([a41e484](https://github.com/akiojin/claude-worktree/commit/a41e4847bcf9ff1373548886069053f91efb337b))
* 依存インストール失敗時も起動を継続 ([4e65457](https://github.com/akiojin/claude-worktree/commit/4e65457536ec95da4ae551be515d7fe8bab4a83c))

## Features

* **client:** ターミナルコンポーネント実装とAI Toolセッション起動機能 ([7f7497a](https://github.com/akiojin/claude-worktree/commit/7f7497a228bb13fe44e1a9c146e87b49747c2cf7))
* **client:** フロントエンド基盤実装 (Vite/React/React Router) ([34103e5](https://github.com/akiojin/claude-worktree/commit/34103e5a7c63c983fc29e3e822f795d78e4a6652))
* **cli:** merge shared environment when launching tools ([299c83e](https://github.com/akiojin/claude-worktree/commit/299c83ed47b34e2d441743a1102d8719b3693995))
* **cli:** src/index.tsにserve分岐ロジックを追加 ([a9c7a68](https://github.com/akiojin/claude-worktree/commit/a9c7a685ad762017efab523d8b3b76df3bf69f59))
* Codex CLI のデフォルトモデルを gpt-5.1 に更新 ([4811fe0](https://github.com/akiojin/claude-worktree/commit/4811fe00b96144bd6cdca7b34259ea2577ae8d71))
* **config:** support shared env persistence ([c096f3c](https://github.com/akiojin/claude-worktree/commit/c096f3c9bfac99da2c038c2cb7e6d3dd49b716e6))
* **server:** expose shared env configuration ([66192fd](https://github.com/akiojin/claude-worktree/commit/66192fd83370a5c1b11a700a713d5b212b4a8d0e))
* **server:** Fastifyベースのバックエンド実装とREST API完成 ([238c218](https://github.com/akiojin/claude-worktree/commit/238c2181673038837cbd465cb4c74a50766b1e3a))
* Web UIのデザイン刷新とテスト追加 ([8c38775](https://github.com/akiojin/claude-worktree/commit/8c3877524f55452a9b592dca4651edd600c6c0c9))
* Web UIのブランチグラフ表示を追加 ([58a781e](https://github.com/akiojin/claude-worktree/commit/58a781e2ee7ae246b7d6e750924eef446df7b2b4))
* **webui:** add shared env management UI ([fe181b1](https://github.com/akiojin/claude-worktree/commit/fe181b13bf06b405dc63d4c72e765659f37598e9))
* **webui:** Web UI からGit同期を実行 ([ea80600](https://github.com/akiojin/claude-worktree/commit/ea8060066aa23cb293d93037b755f87f497dcc24))
* **webui:** ブランチ差分を同期して起動を制御 ([324cf95](https://github.com/akiojin/claude-worktree/commit/324cf95d94640587f99ded674eaf157762ee37df))
* **web:** Web UIディレクトリ構造と共通型定義を作成 ([82a1be1](https://github.com/akiojin/claude-worktree/commit/82a1be10ab4e7bd535f9fa025efdad8c97064b9b))
* **web:** Web UI依存関係追加とCLI UI分離 ([1d480a0](https://github.com/akiojin/claude-worktree/commit/1d480a047bec3183a27975af6618c86748880905))
