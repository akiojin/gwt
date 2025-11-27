## [2.6.1](https://github.com/akiojin/gwt/compare/v2.6.0...v2.6.1) (2025-11-25)


### Bug Fixes

* ensure claude skipPermissions uses sandbox env ([b24b3ea](https://github.com/akiojin/gwt/commit/b24b3ea7e7845ad13cc73ddc81cdde6ad2e445c9))
* アイコン幅計測を補正してブランチ行の日時折り返しを防止 ([7d52a01](https://github.com/akiojin/gwt/commit/7d52a01405a8b1c6845d1043bc06b84b257bb597))
* タイムスタンプ右寄せに安全マージンを設けて改行を防止 ([c8c5b78](https://github.com/akiojin/gwt/commit/c8c5b7846815269dac13ce5c1175b18d4564df95))
* 実幅を過小評価しないよう文字幅計測と整列テストを更新 ([765cdd4](https://github.com/akiojin/gwt/commit/765cdd48a5529c11eaed01b8db19473b60820f8a))
* 実行モード表示をNewに変更 ([9850729](https://github.com/akiojin/gwt/commit/98507299f4c550968711be72ce6fe97f6b55a42a))
* 幅オーバーライドとアイコン計測のずれで発生する改行を再修正 ([3bdbb64](https://github.com/akiojin/gwt/commit/3bdbb647d18864275d5dd06cb15e34b3b11b3193))
* 幅計測ヘルパー欠落による型エラーを解消 ([788413b](https://github.com/akiojin/gwt/commit/788413b82b0fa2f18af3e0759cda03ed30157a6b))

# [2.6.0](https://github.com/akiojin/gwt/compare/v2.5.0...v2.6.0) (2025-11-25)


### Bug Fixes

* prevent false positives in git hook detection ([e5c0f32](https://github.com/akiojin/gwt/commit/e5c0f324f02ba934ef447685eead6aea12b4bdd5))
* renderBranchRowのcursorAdjustロジックを復元してテスト互換性を維持 ([9f4f10e](https://github.com/akiojin/gwt/commit/9f4f10eeaf25b2fe0e773098bc72337f5d10ac08))
* string-width v8対応のためWIDTH_OVERRIDESにVariation Selector付きアイコンを追加 ([c8f5b9b](https://github.com/akiojin/gwt/commit/c8f5b9bac9be76a0351aee06c9e4ebea00fb7cf4))
* 全アイコンの幅オーバーライドを追加してタイムスタンプ折り返しを修正 ([06a7e5d](https://github.com/akiojin/gwt/commit/06a7e5dbf1a62f831db05d9b2ee67a6552945256))
* 全ての幅計算をmeasureDisplayWidthに統一してstring-width v8対応を完了 ([6f7d1ce](https://github.com/akiojin/gwt/commit/6f7d1ceaa97ad99f61e3f0967ccc80dbd9a90d72))


### Features

* set upstream tracking for newly created refs ([7a631a1](https://github.com/akiojin/gwt/commit/7a631a132f7fc99f207dfa1b7d1e10a841fda27f))

# [2.5.0](https://github.com/akiojin/gwt/compare/v2.4.1...v2.5.0) (2025-11-25)


### Bug Fixes

* ensure selected model ID is passed to launcher for Claude Code ([1b2b884](https://github.com/akiojin/gwt/commit/1b2b884835743d9143b0b2a5e4c9234af4d1009a))
* omit --model flag when default Opus 4.5 is selected ([08828e3](https://github.com/akiojin/gwt/commit/08828e38a592e31691ef985b5008dea807e8918c))


### Features

* add Sonnet 4.5 as an explicit model option ([2a52c91](https://github.com/akiojin/gwt/commit/2a52c91faacb8d8c6422c80051bb611757464ddf))
* set Opus 4.5 as default and remove explicit Default option ([86f60fa](https://github.com/akiojin/gwt/commit/86f60fadd8232920bf01f457a370bd2243066f0e))
* update default Claude Code model to Opus 4.5 ([1dd909e](https://github.com/akiojin/gwt/commit/1dd909ed87a55ce3bb796deb0431d0027571dae2))
* update Opus model version to 4.5 ([307faeb](https://github.com/akiojin/gwt/commit/307faeb66ebac0ca89b01c07ee19d213797c4d5c))

## [2.9.0](https://github.com/akiojin/gwt/compare/v2.8.0...v2.9.0) (2025-11-27)


### Features

* preselect last AI tool on selector reopen ([95ec7ce](https://github.com/akiojin/gwt/commit/95ec7ce7aede1ce6ca40599929e0eed6da3d0bb0))
* preselect last AI tool when reopening selector ([f0060a1](https://github.com/akiojin/gwt/commit/f0060a1177920704676524c6cfd7fde073289641))


### Bug Fixes

* save last AI tool immediately on launch ([bf084f2](https://github.com/akiojin/gwt/commit/bf084f2c1c2e5ab40221f0a19daf86b109880b8f))
* save last AI tool immediately on launch ([880cf5c](https://github.com/akiojin/gwt/commit/880cf5c037e6b5ff27815a5400732d043820342f))

## [2.8.0](https://github.com/akiojin/gwt/compare/v2.7.4...v2.8.0) (2025-11-27)


### Features

* show last AI tool usage per branch ([f3662c6](https://github.com/akiojin/gwt/commit/f3662c616232779b8d58f6e4d92064e16301e5ce))


### Bug Fixes

* stabilize worktree flows and branch hook ([0c202db](https://github.com/akiojin/gwt/commit/0c202db077bb1db19106959a8baa3a4849de83ff))
* stabilize worktree support and last ai usage display ([74ce7f3](https://github.com/akiojin/gwt/commit/74ce7f350dffe7a0d70aebea55420084ca3aa9aa))

## [2.7.4](https://github.com/akiojin/gwt/compare/v2.7.3...v2.7.4) (2025-11-26)


### Bug Fixes

* fetchAllRemotes 失敗時にローカルブランチを表示するフォールバックを追加 ([65f0a02](https://github.com/akiojin/gwt/commit/65f0a0239b3dc45e54ba979b67c4bc41934ed12c))
* navigation.test.tsx に fetchAllRemotes のモックを追加 ([b76ea96](https://github.com/akiojin/gwt/commit/b76ea96fd86ae467bc9cb3053806ae7280ae0f57))
* ブランチ一覧表示時にリモートブランチをfetchして最新情報を取得 ([54b610f](https://github.com/akiojin/gwt/commit/54b610feb7c1d5349cdc3f305ffe03a5f11e3bcf))
* ブランチ一覧表示時にリモートブランチをfetchして最新情報を取得 ([14696f1](https://github.com/akiojin/gwt/commit/14696f16e6cb5e5606fbb45e6f57bdfa9a3b4759))

## [2.4.1](https://github.com/akiojin/gwt/compare/v2.4.0...v2.4.1) (2025-11-21)


### Bug Fixes

* Claude Codeのデフォルトモデル指定を標準扱いに修正 ([3acb5c7](https://github.com/akiojin/gwt/commit/3acb5c7b18dc40e3fe7c9723400389fe633a125a))
* **cli:** ターミナル入力がフリーズする問題を修正 ([c6752f3](https://github.com/akiojin/gwt/commit/c6752f3fc0153f3f279b49a77636ebee8154a624))
* フィルターモードでショートカットを無効化 ([96d6f2d](https://github.com/akiojin/gwt/commit/96d6f2d27894cd7f1a46c6861ea4343f34223570))

# [2.4.0](https://github.com/akiojin/gwt/compare/v2.3.0...v2.4.0) (2025-11-20)


### Bug Fixes

* Improve git hook detection for commands with options ([8f4f9a5](https://github.com/akiojin/gwt/commit/8f4f9a5d5a784c171e30efb100a733a6dced9c40))
* use process.platform in claude command availability ([338e779](https://github.com/akiojin/gwt/commit/338e7798825546503c31c77d6dbfb85805c50042))


### Features

* align model selection with provider defaults ([cc8c863](https://github.com/akiojin/gwt/commit/cc8c863e6ce14aa57601deccd6471ca6c0aaa540))
* QwenサポートをREADMEに追加し、GEMINI.mdを作成 ([4fa1491](https://github.com/akiojin/gwt/commit/4fa14914941593b330efa7486eef3772e387f330))
* remember last model and reasoning selection per tool ([01b5124](https://github.com/akiojin/gwt/commit/01b5124409b13763d8b792493ef72442714cf4f9))

# [2.3.0](https://github.com/akiojin/gwt/compare/v2.2.0...v2.3.0) (2025-11-19)


### Features

* Codex/Geminiの表示名を簡潔化 ([cc8bdb2](https://github.com/akiojin/gwt/commit/cc8bdb25617de58c79dda5b53134fc2a3ac89aa2))
* Gemini CLIをビルトインツールとして追加 ([0e80363](https://github.com/akiojin/gwt/commit/0e80363ae81fdf61c00ae55dbecf8b2cecd677e4))
* Qwenをビルトインツールとして追加 ([b4f6c94](https://github.com/akiojin/gwt/commit/b4f6c9476f6675bdc4e27006b60868ee14f07dae))

# [2.2.0](https://github.com/akiojin/gwt/compare/v2.1.1...v2.2.0) (2025-11-18)


### Bug Fixes

* フィルターモード中でもブランチ選択のカーソル移動を可能に ([c00564a](https://github.com/akiojin/gwt/commit/c00564a0ffa4d3fc5fe05a800e8454b821e79421))
* フィルター入力とStatsの間の空行を削除 ([054f092](https://github.com/akiojin/gwt/commit/054f092a18f06a752326d1a38670d2f4323c6e21))
* フィルター入力の表示位置をWorking DirectoryとStatsの間に修正 ([57dd905](https://github.com/akiojin/gwt/commit/57dd9052b7032f8a1173b0d228ec67a186857e0d))
* ブランチ選択モードでのカーソル反転表示を修正 ([fda28a1](https://github.com/akiojin/gwt/commit/fda28a1d0008555c96d6859a031cc4486ad4f94e))


### Features

* fキーでフィルター・検索モードを追加 ([481ab67](https://github.com/akiojin/gwt/commit/481ab678576642dbb01e5bfe57b58a2cc5da011d))
* フィルターモード/ブランチ選択モードの切り替え機能を追加 ([c1e87bc](https://github.com/akiojin/gwt/commit/c1e87bc4557b7e343f72e51635f33a79f10979de))
* フィルターモード中もブランチ選択の反転表示を有効化 ([a3b8eca](https://github.com/akiojin/gwt/commit/a3b8eca8212e481dde5665e34b7e8894155399af))
* フィルター入力中のキーバインド(c/r/m)を無効化＋要件・テスト更新 ([232f66c](https://github.com/akiojin/gwt/commit/232f66cb114ac2fb47b294c71a9aa66afad8c297))

## [2.1.1](https://github.com/akiojin/gwt/compare/v2.1.0...v2.1.1) (2025-11-18)


### Bug Fixes

* publish.ymlでSetup Bunステップの順序を修正 ([81d7c57](https://github.com/akiojin/gwt/commit/81d7c57da150f1fba4cf9440263ba16a05cff3a0))

# [2.1.0](https://github.com/akiojin/gwt/compare/v2.0.4...v2.1.0) (2025-11-18)


### Bug Fixes

* .markdownlintignoreを追加してCHANGELOG.mdを除外 ([d911ee8](https://github.com/akiojin/gwt/commit/d911ee85480a038ae9e2a60c0bc0298566a7c2d3))
* execa互換性問題によるblock-git-branch-ops.test.tsのテスト失敗を修正 ([48f8528](https://github.com/akiojin/gwt/commit/48f8528014c6fc2213256ff221efc52b70e98ee3))
* markdownlintエラーを修正 ([ebf6bc7](https://github.com/akiojin/gwt/commit/ebf6bc7befe9e6e707df31c3e91c82d80e0307c5))
* markdownlintのignore_filesを複数行形式に修正 ([0eda2dd](https://github.com/akiojin/gwt/commit/0eda2ddebebf3aa285d737638378f4d95039fa73))
* semantic-release実行に必要なNode.js setupを追加 ([8d4b8f9](https://github.com/akiojin/gwt/commit/8d4b8f9e3296026dd713e767008526563009f435))


### Features

* bugfixブランチタイプのサポートを追加 ([ca915a0](https://github.com/akiojin/gwt/commit/ca915a0a98206448e9ef1b4b94dbadf10ec58c76))

## [2.0.4](https://github.com/akiojin/gwt/compare/v2.0.3...v2.0.4) (2025-11-18)


### Bug Fixes

* bin/gwt.jsでmain関数を明示的に呼び出すように修正 ([cc8b4b4](https://github.com/akiojin/gwt/commit/cc8b4b4ef8e1c30c6a0e77acd64b96b145beaae9))

## [2.0.3](https://github.com/akiojin/gwt/compare/v2.0.2...v2.0.3) (2025-11-18)


### Bug Fixes

* semantic-release npmプラグインをnpmPublish: falseで有効化 ([6218754](https://github.com/akiojin/gwt/commit/621875478edea7f80b17c866bb3f02504f7d67cd))

## [2.0.2](https://github.com/akiojin/gwt/compare/v2.0.1...v2.0.2) (2025-11-18)


### Bug Fixes

* semantic-releaseからnpm publishを分離してpublish.ymlに移動 ([42e0233](https://github.com/akiojin/gwt/commit/42e0233ec068253ab3efed9d8bda82b8c4b1252c))

## [2.0.1](https://github.com/akiojin/gwt/compare/v2.0.0...v2.0.1) (2025-11-18)


### Bug Fixes

* release.ymlでnpm publish前にビルドを実行 ([4a84359](https://github.com/akiojin/gwt/commit/4a843592cc1ea8e3db743de56ed8ca05cbd76211))

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
