## [4.12.0](https://github.com/akiojin/gwt/compare/v4.11.6...v4.12.0) (2026-01-10)


### Features

* コーディングエージェントバージョンの起動時キャッシュ (FR-028～FR-031) ([#542](https://github.com/akiojin/gwt/issues/542)) ([671b41e](https://github.com/akiojin/gwt/commit/671b41eaa428b158d24aada7a14217888e80f2e1))


### Bug Fixes

* cleanup safety and tool version fallbacks ([#543](https://github.com/akiojin/gwt/issues/543)) ([c6518c0](https://github.com/akiojin/gwt/commit/c6518c032d47ba8f9f62a531b5c67eed34202682))
* Quick Startセッション解決をブランチ基準に修正 ([#547](https://github.com/akiojin/gwt/issues/547)) ([4e92ea9](https://github.com/akiojin/gwt/commit/4e92ea9f62d86c1216efd5e4a4246e1f1cd1e943))
* run UI with bun runtime ([#537](https://github.com/akiojin/gwt/issues/537)) ([384441b](https://github.com/akiojin/gwt/commit/384441b34d8c486ad0b5f9ec8690686766bcec6b))
* unsafe確認ダイアログ反転と凡例のSafe追加 ([#544](https://github.com/akiojin/gwt/issues/544)) ([1b627f7](https://github.com/akiojin/gwt/commit/1b627f7de994c435c3061f6be41a24236f5eccaf))
* カーソル位置をグローバル管理に変更して安全状態更新時のリセットを防止 ([#541](https://github.com/akiojin/gwt/issues/541)) ([dcb9f74](https://github.com/akiojin/gwt/commit/dcb9f74ab506e765f1be8c011a5cd27843435fb2))
* コーディングエージェント起動時の即時終了問題を修正 ([633f0d6](https://github.com/akiojin/gwt/commit/633f0d65aa3724eb38c6084433b6da1cc688d342)), closes [#546](https://github.com/akiojin/gwt/issues/546)
* ログビューア表示と配色の統一 ([#538](https://github.com/akiojin/gwt/issues/538)) ([1067a0c](https://github.com/akiojin/gwt/commit/1067a0c15d5349c8f7e7840649fe2a43967391c5))
* 安全アイコンの安全表示を緑に変更 ([#525](https://github.com/akiojin/gwt/issues/525)) ([f0e7ba9](https://github.com/akiojin/gwt/commit/f0e7ba9b5f01bfc2bd55173afe1d60ca434d2fde))
* 安全状態確認時のカーソルリセット問題を修正 ([#539](https://github.com/akiojin/gwt/issues/539)) ([77db8ea](https://github.com/akiojin/gwt/commit/77db8ea3c629affbb930fc2074efb4a312837895))

## [6.0.0](https://github.com/akiojin/gwt/compare/gwt-v5.0.0...gwt-v6.0.0) (2026-01-14)


### ⚠ BREAKING CHANGES

* TypeScript実装を削除し、Rust実装に完全移行

### Features

* add crates.io, cargo-binstall, and npm release automation ([#566](https://github.com/akiojin/gwt/issues/566)) ([bdcebc0](https://github.com/akiojin/gwt/commit/bdcebc0c255ec11bfc230a7e67cf3bcd9e057b36))
* add post-session git prompts ([bc24450](https://github.com/akiojin/gwt/commit/bc244506559fc0676f6740fa3edf7912c0f02af7))
* add post-session git prompts ([0821c63](https://github.com/akiojin/gwt/commit/0821c63edf7fddd50c20b4239283ddbf6e8593ae))
* AIツールのインストール済み表示をバージョン番号に変更 ([#461](https://github.com/akiojin/gwt/issues/461)) ([2610ef2](https://github.com/akiojin/gwt/commit/2610ef2a8a1b64b8e98b4cde6df1b5afdabcef55))
* AIツールのインストール済み表示をバージョン番号に変更 ([#461](https://github.com/akiojin/gwt/issues/461)) ([4c7ceca](https://github.com/akiojin/gwt/commit/4c7ceca339704ab98a3e013fff3e84df55bd540b))
* Claude CodeのTypeScript LSP対応を追加 ([cf2983b](https://github.com/akiojin/gwt/commit/cf2983b1f58cb670dba2e47b645cc8af79d760a0))
* Claude CodeのTypeScript LSP対応を追加 ([dc74e3a](https://github.com/akiojin/gwt/commit/dc74e3acd8f0c7d8ea20336b7ca42f72ee044ecd))
* Claude Codeプラグイン設定を追加 ([#429](https://github.com/akiojin/gwt/issues/429)) ([06d04db](https://github.com/akiojin/gwt/commit/06d04dbf28e7fa6a65a7349388c4dc48efbb6ae7))
* Claude Codeプラグイン設定を追加 ([#429](https://github.com/akiojin/gwt/issues/429)) ([058c3b4](https://github.com/akiojin/gwt/commit/058c3b48decfe5fa6e0d24156ce63cf182330f98))
* Claude Code起動時にChrome拡張機能統合を有効化 ([b3a5d6d](https://github.com/akiojin/gwt/commit/b3a5d6d7adb85d82577f888c147b3af593bf9160))
* Claude Code起動時にChrome拡張機能統合を有効化 ([8883a4f](https://github.com/akiojin/gwt/commit/8883a4fb5c3b40ce2b80d1a1cafe35a230fa5864))
* Claude Code起動時にChrome拡張機能統合を有効化 ([f449845](https://github.com/akiojin/gwt/commit/f4498451d1deacbe811126cc34d35e2b73334eb2))
* Claude Code起動時にChrome拡張機能統合を有効化 ([d8f1d88](https://github.com/akiojin/gwt/commit/d8f1d88f5e4061124af97b285c032bcad641c99b))
* **cli:** AIツールのインストール状態検出とステータス表示を追加 ([#431](https://github.com/akiojin/gwt/issues/431)) ([79a6995](https://github.com/akiojin/gwt/commit/79a6995349b1dfb966e496c3e50644fcb58c99f6))
* **cli:** AIツールのインストール状態検出とステータス表示を追加 ([#431](https://github.com/akiojin/gwt/issues/431)) ([02a935c](https://github.com/akiojin/gwt/commit/02a935ca2ce354d5597c182bc6a684e3e0ef701a))
* Docker構成を最適化しPlaywright noVNCサービスを追加 ([#454](https://github.com/akiojin/gwt/issues/454)) ([6dd0843](https://github.com/akiojin/gwt/commit/6dd08435f9f49061f156d01f5e7ce0a25e15307f))
* Docker構成を最適化しPlaywright noVNCサービスを追加 ([#454](https://github.com/akiojin/gwt/issues/454)) ([feabdbc](https://github.com/akiojin/gwt/commit/feabdbcc71dc006850becf16c4f19664e54966ec))
* Docker構成を最適化しPlaywright noVNCサービスを追加 ([#455](https://github.com/akiojin/gwt/issues/455)) ([e62fef5](https://github.com/akiojin/gwt/commit/e62fef5bc199a26ea0d6f8f36527612ff461b00f))
* Docker構成を最適化しPlaywright noVNCサービスを追加 ([#455](https://github.com/akiojin/gwt/issues/455)) ([933b0cf](https://github.com/akiojin/gwt/commit/933b0cf5d4f8b7599782d131eb9758dd06771fb2))
* Enterキーでウィザードポップアップを開く機能を実装 ([017234d](https://github.com/akiojin/gwt/commit/017234d0ae6569b4e89db3fa9bd6c8d6dadc0254))
* FR-010/FR-028 ブランチクリーンアップ機能を実装 ([32c2f99](https://github.com/akiojin/gwt/commit/32c2f991ce65913f959ec721d1f04f8dd3882b82))
* FR-029b-e 安全でないブランチ選択時の警告ダイアログを実装 ([67ac545](https://github.com/akiojin/gwt/commit/67ac545f6d2cea2ddbcd4e506255c58ace35ad22))
* FR-038-040 Worktree stale回復機能を実装 ([bb4ef27](https://github.com/akiojin/gwt/commit/bb4ef278f5cfda1d3293523e94df6c00fac4c1ae))
* FR-050 Quick Start機能をウィザードに追加 ([d78f279](https://github.com/akiojin/gwt/commit/d78f279783790644b4ccca4c82cc0e44d429f9e4))
* FR-060-062 ウィザードポップアップのスクロール機能を実装 ([b91bd23](https://github.com/akiojin/gwt/commit/b91bd23ae6bf782fef28b687c5bc86c6298a0b63))
* macOS対応のシステムトレイを実装 ([b2cdfbe](https://github.com/akiojin/gwt/commit/b2cdfbe6dffe391becdbb3799019616e1e45ed74))
* macOS対応のシステムトレイを実装 ([cb9f8f9](https://github.com/akiojin/gwt/commit/cb9f8f9fdf9bcfe7a5ae984a8e1795539e7044da))
* OpenCode コーディングエージェント対応を追加 ([#477](https://github.com/akiojin/gwt/issues/477)) ([c480fb4](https://github.com/akiojin/gwt/commit/c480fb4b16c0f815a205776eb618fbcb875d238b))
* OpenCode コーディングエージェント対応を追加 ([#477](https://github.com/akiojin/gwt/issues/477)) ([7aca557](https://github.com/akiojin/gwt/commit/7aca55726666d01fd529c7d3faf7b435eb5556ee))
* OpenTUI移行 ([#487](https://github.com/akiojin/gwt/issues/487)) ([ed1c872](https://github.com/akiojin/gwt/commit/ed1c872d2e031784c46bbb1ce572866a03de47ef))
* OpenTUI移行 ([#487](https://github.com/akiojin/gwt/issues/487)) ([0d585bf](https://github.com/akiojin/gwt/commit/0d585bfb9eb23a6ab71759e784f5aa22476a39c8))
* requirements-spec-kit スキルを追加 ([01a6644](https://github.com/akiojin/gwt/commit/01a6644ad3ba5e73bdf17f79c278ec6cf311b637))
* requirements-spec-kit スキルを追加 ([6fc629e](https://github.com/akiojin/gwt/commit/6fc629e3890cda28747e4a2bff0111aac4287b1c))
* requirements-spec-kit スキルを追加 ([cbf7596](https://github.com/akiojin/gwt/commit/cbf759680b2dda33c26c9e5ed591f53b15a5d8f8))
* requirements-spec-kit スキルを追加 ([70375b6](https://github.com/akiojin/gwt/commit/70375b6d78c536711c402c1fc2a6a939795f3653))
* Rustコア機能完全実装（Phase 1-4） ([da454a1](https://github.com/akiojin/gwt/commit/da454a1df29167cba1943ff6016009155d68de02))
* Rustワークスペース基盤を作成 ([470e2d8](https://github.com/akiojin/gwt/commit/470e2d8a79a51656c173b946dc555580bb959fe2))
* TUI画面をTypeScript版と完全互換に拡張 ([030b8d5](https://github.com/akiojin/gwt/commit/030b8d59c9adadacc832f5a048d0d1465f681c22))
* TypeScriptからRustへの完全移行 ([2606cbc](https://github.com/akiojin/gwt/commit/2606cbcd29b92298be403b807f1336c0b0fdabb7))
* **ui:** コーディングエージェント名の一貫した色づけを実装 ([#511](https://github.com/akiojin/gwt/issues/511)) ([ac9923f](https://github.com/akiojin/gwt/commit/ac9923f41c01c06aaf3e2fba4856aaa912915fda))
* **ui:** コーディングエージェント名の一貫した色づけを実装 ([#511](https://github.com/akiojin/gwt/issues/511)) ([7fdb2ad](https://github.com/akiojin/gwt/commit/7fdb2ad99f712d9bbbd60bfca5d28f5b8b81e783))
* Web UIサーバー全体にログ出力を追加 ([09909f2](https://github.com/akiojin/gwt/commit/09909f2c4b4f7c3a0130ade2536ed5bef55d7512))
* Web UIサーバー全体にログ出力を追加 ([90ff0ba](https://github.com/akiojin/gwt/commit/90ff0ba16190c3993dc8df7e0e62ef7044860272))
* Web UI機能の強化とブランチグラフのリファクタリング ([146f596](https://github.com/akiojin/gwt/commit/146f59609d207b072bfbf10e5150a37f75a4cd77))
* Web UI機能の強化とブランチグラフのリファクタリング ([193ccf0](https://github.com/akiojin/gwt/commit/193ccf0e57335618ddf0c3dd3f37c04c2268ce17))
* Worktreeパス修復機能を追加 (SPEC-902a89dc) ([#484](https://github.com/akiojin/gwt/issues/484)) ([2c36efa](https://github.com/akiojin/gwt/commit/2c36efaa27d11744a8dc5616c335bc5574c9683d))
* Worktreeパス修復機能を追加 (SPEC-902a89dc) ([#484](https://github.com/akiojin/gwt/issues/484)) ([aa56ce8](https://github.com/akiojin/gwt/commit/aa56ce8f4bde544268a1d81d1f7e2aea8ff1f954))
* xキーでgit worktree repairを実行する機能を実装 ([69cddd9](https://github.com/akiojin/gwt/commit/69cddd9f8d42d1772a1aa4193e10c0d8445fbaff))
* コーディングエージェントのバージョン選択機能を改善 ([#510](https://github.com/akiojin/gwt/issues/510)) ([b3c959a](https://github.com/akiojin/gwt/commit/b3c959a87db822f90832b4c7b3d303249ec03e83))
* コーディングエージェントのバージョン選択機能を改善 ([#510](https://github.com/akiojin/gwt/issues/510)) ([a4bab47](https://github.com/akiojin/gwt/commit/a4bab47a3e0fd13967cc7301d1058477fbf59ab8))
* コーディングエージェントバージョンの起動時キャッシュ (FR-028～FR-031) ([#542](https://github.com/akiojin/gwt/issues/542)) ([671b41e](https://github.com/akiojin/gwt/commit/671b41eaa428b158d24aada7a14217888e80f2e1))
* ショートカット表記を画面内に統合 ([#503](https://github.com/akiojin/gwt/issues/503)) ([d0d9fa2](https://github.com/akiojin/gwt/commit/d0d9fa29ac1e478d61dde0965252e509fe1693f8))
* ショートカット表記を画面内に統合 ([#503](https://github.com/akiojin/gwt/issues/503)) ([5fb839c](https://github.com/akiojin/gwt/commit/5fb839cf355a9d599db68b4722e850eb30a61a5d))
* ブランチグラフをReact Flowベースにリファクタリング ([f0deb4a](https://github.com/akiojin/gwt/commit/f0deb4a37c8dc0d31ede97491d34719dd7297999))
* ブランチグラフをReact Flowベースにリファクタリング ([60d0a0b](https://github.com/akiojin/gwt/commit/60d0a0b2d3b2e16157c6d49b0174022e0ada1e74))
* ブランチ一覧に最終アクティビティ時間を表示 ([#456](https://github.com/akiojin/gwt/issues/456)) ([7cfab79](https://github.com/akiojin/gwt/commit/7cfab79ebc64393fb1e6666191c57d8baf2c5686))
* ブランチ一覧に最終アクティビティ時間を表示 ([#456](https://github.com/akiojin/gwt/issues/456)) ([5f101ca](https://github.com/akiojin/gwt/commit/5f101caa906eca0ab7c06d53453a2c13d7d552d9))
* ブランチ一覧画面の改善（表示モード切替・スピナー局所化） ([46e42ae](https://github.com/akiojin/gwt/commit/46e42aecb544964fa156c84043b19243a16ee91f))
* ブランチ一覧画面の改善（表示モード切替・スピナー局所化） ([26a8dc0](https://github.com/akiojin/gwt/commit/26a8dc0875988313dc59024d6f98804febe010ff))
* ブランチ表示モード切替機能（TABキー）を追加 ([50a1d39](https://github.com/akiojin/gwt/commit/50a1d393024a8e8e7ec3c4bcd98502c15408cabe))
* ブランチ表示モード切替機能（TABキー）を追加 ([f0b8715](https://github.com/akiojin/gwt/commit/f0b87158d5cc9b37fbd5da940795bcb2615214b9))
* ブランチ選択のフルパス表示 ([#486](https://github.com/akiojin/gwt/issues/486)) ([1c6dd54](https://github.com/akiojin/gwt/commit/1c6dd549cdc1a8f3b1b374ed8e4dc330649737ef))
* ブランチ選択のフルパス表示 ([#486](https://github.com/akiojin/gwt/issues/486)) ([1913b4d](https://github.com/akiojin/gwt/commit/1913b4d75e1439ec412329fa46f8bacea2f60e79))
* ログビューアを追加 ([#442](https://github.com/akiojin/gwt/issues/442)) ([92128c3](https://github.com/akiojin/gwt/commit/92128c37f7453d302025b5d023984543d023adf4))
* ログビューアを追加 ([#442](https://github.com/akiojin/gwt/issues/442)) ([095b8c9](https://github.com/akiojin/gwt/commit/095b8c96c920039fe28c44977461d0226d420761))
* ログ表示の通知と選択UIを改善 ([#443](https://github.com/akiojin/gwt/issues/443)) ([cf3d7b3](https://github.com/akiojin/gwt/commit/cf3d7b31e84cdbc762fc6c629b5c670744678cb5))
* ログ表示の通知と選択UIを改善 ([#443](https://github.com/akiojin/gwt/issues/443)) ([58b25d6](https://github.com/akiojin/gwt/commit/58b25d68e38a495272f02340baf2b36420cfb2af))
* 新規ブランチ作成時にブランチタイプ選択とプレフィックス自動付加を追加 ([#494](https://github.com/akiojin/gwt/issues/494)) ([2010f6c](https://github.com/akiojin/gwt/commit/2010f6c37095e98c79dfe20110524df92862ac28))
* 新規ブランチ作成時にブランチタイプ選択とプレフィックス自動付加を追加 ([#494](https://github.com/akiojin/gwt/issues/494)) ([50b8b1c](https://github.com/akiojin/gwt/commit/50b8b1cc8af3b56b00de12ff6b37f8681ea72545))
* 未コミット警告時にEnterキー待機を追加 ([#441](https://github.com/akiojin/gwt/issues/441)) ([d03dac5](https://github.com/akiojin/gwt/commit/d03dac52c12a5cb6ef99b973d7348f4f3ccdeeb6))
* 未コミット警告時にEnterキー待機を追加 ([#441](https://github.com/akiojin/gwt/issues/441)) ([5d3c74b](https://github.com/akiojin/gwt/commit/5d3c74b765cd83475ed99d260448ce6eb5a818ad))


### Bug Fixes

* add gwt-core dependency version to release-please extra-files ([896d57c](https://github.com/akiojin/gwt/commit/896d57cc90e988042cf5fa2810510286940ed90c))
* bunx実行時にBunで再実行する ([#558](https://github.com/akiojin/gwt/issues/558)) ([ed2ad0b](https://github.com/akiojin/gwt/commit/ed2ad0b073fd0672e074cd88914c148cd778908e))
* cache installed versions for wizard ([#555](https://github.com/akiojin/gwt/issues/555)) ([13738bb](https://github.com/akiojin/gwt/commit/13738bb885d50e796767ceba263b247b9827f080))
* **ci:** publishワークフローにテストタイムアウトを追加 ([#530](https://github.com/akiojin/gwt/issues/530)) ([0c73426](https://github.com/akiojin/gwt/commit/0c73426b222c919d5fecb164f6b456bc37d85a46))
* **ci:** publishワークフローにテストタイムアウトを追加 ([#530](https://github.com/akiojin/gwt/issues/530)) ([de12b7f](https://github.com/akiojin/gwt/commit/de12b7f65d174dc44421e4022a302155c9336f45))
* **ci:** マージ方法をsquashに変更してCHANGELOG重複を防止 ([#425](https://github.com/akiojin/gwt/issues/425)) ([ee6338c](https://github.com/akiojin/gwt/commit/ee6338c726002326707baa8b15b5f70703395502))
* **ci:** マージ方法をsquashに変更してCHANGELOG重複を防止 ([#425](https://github.com/akiojin/gwt/issues/425)) ([39f13f0](https://github.com/akiojin/gwt/commit/39f13f0a27a4d1af002cac465eeb89d4318b2940))
* Claude Codeのフォールバックをbunxに統一 ([ac740d9](https://github.com/akiojin/gwt/commit/ac740d9c7bc3ef97a5d3b0903908ecc672c637ce))
* Claude Codeのフォールバックをbunxに統一 ([c68d119](https://github.com/akiojin/gwt/commit/c68d1194e9674f032aa0bc833fe6ba33983ead03))
* claude-worktree後方互換コードを削除 ([#462](https://github.com/akiojin/gwt/issues/462)) ([c8e5fbf](https://github.com/akiojin/gwt/commit/c8e5fbf06f5c18480f2af3f8e78469214282ef28))
* claude-worktree後方互換コードを削除 ([#462](https://github.com/akiojin/gwt/issues/462)) ([2cacaa1](https://github.com/akiojin/gwt/commit/2cacaa168e5faf1747e430c1d68b791417cf2888))
* cleanup safety and tool version fallbacks ([#543](https://github.com/akiojin/gwt/issues/543)) ([c6518c0](https://github.com/akiojin/gwt/commit/c6518c032d47ba8f9f62a531b5c67eed34202682))
* **cli:** AIツール実行時にフルパスを使用 ([#439](https://github.com/akiojin/gwt/issues/439)) ([2fe73e2](https://github.com/akiojin/gwt/commit/2fe73e27ea7f9148415fb67b3bfbe0dce9ac48bb))
* **cli:** AIツール実行時にフルパスを使用 ([#439](https://github.com/akiojin/gwt/issues/439)) ([d50ab07](https://github.com/akiojin/gwt/commit/d50ab07c9e66046091af46112713ec230d87074f))
* **cli:** AIツール実行時にフルパスを使用して非インタラクティブシェルのPATH問題を修正 ([#436](https://github.com/akiojin/gwt/issues/436)) ([ba78cd5](https://github.com/akiojin/gwt/commit/ba78cd52cc95193f894e0aa8635767395567d56b))
* **cli:** AIツール実行時にフルパスを使用して非インタラクティブシェルのPATH問題を修正 ([#436](https://github.com/akiojin/gwt/issues/436)) ([5c2888d](https://github.com/akiojin/gwt/commit/5c2888ddb0794cb4630c0d709d2070f4d4f05542))
* **cli:** keep wizard cursor visible in popup ([#506](https://github.com/akiojin/gwt/issues/506)) ([7cc3257](https://github.com/akiojin/gwt/commit/7cc3257a0923fbc7261cf8e3e1ef7e16f40b647c))
* **cli:** keep wizard cursor visible in popup ([#506](https://github.com/akiojin/gwt/issues/506)) ([3a2744a](https://github.com/akiojin/gwt/commit/3a2744aabfec7800cb47d5cc740c3cb4b6e36335))
* **cli:** keep wizard cursor visible in popup ([#507](https://github.com/akiojin/gwt/issues/507)) ([06bf197](https://github.com/akiojin/gwt/commit/06bf197ecc715e81c49df2b4c7df6de1d7fc9a34))
* **cli:** keep wizard cursor visible in popup ([#507](https://github.com/akiojin/gwt/issues/507)) ([c0f67cc](https://github.com/akiojin/gwt/commit/c0f67cc2c955037ef7ed63bb2a37e39b0cdd4577))
* clippyワーニング解消およびコード品質改善 ([00690f3](https://github.com/akiojin/gwt/commit/00690f3177309509be406147cb1c2f6115421cf9))
* CLI終了時のシグナルハンドリング改善と各種ドキュメント修正 ([#489](https://github.com/akiojin/gwt/issues/489)) ([fe178da](https://github.com/akiojin/gwt/commit/fe178dac9b20c3e391e6f232a40c533407636340))
* CLI終了時のシグナルハンドリング改善と各種ドキュメント修正 ([#489](https://github.com/akiojin/gwt/issues/489)) ([84a1daa](https://github.com/akiojin/gwt/commit/84a1daa288c471ba8c3c4907cc0dab8ec660e290))
* Codex CLIのモデル指定オプションを-mに変更 ([d4546b1](https://github.com/akiojin/gwt/commit/d4546b153e84b6c7bc47db09fd47ea2b03ffb05c))
* Codex skillsフラグをバージョン判定で切替 ([#552](https://github.com/akiojin/gwt/issues/552)) ([6825a81](https://github.com/akiojin/gwt/commit/6825a8119974beb6750a9bf0c8e215737557ba49))
* dependency installer test hang ([52ab3db](https://github.com/akiojin/gwt/commit/52ab3db09226e5355331efb45cc6d845b0fe6cd7))
* divergenceでも起動を継続 ([#483](https://github.com/akiojin/gwt/issues/483)) ([e04d872](https://github.com/akiojin/gwt/commit/e04d872c5ab30e67a9dddab8df9f191c76af2232))
* divergenceでも起動を継続 ([#483](https://github.com/akiojin/gwt/issues/483)) ([b6a0c07](https://github.com/akiojin/gwt/commit/b6a0c0744f889380d0b4530a09952498104006a0))
* ENABLE_LSP_TOOL環境変数の値を"1"から"true"に修正 ([9bef1eb](https://github.com/akiojin/gwt/commit/9bef1eb809afe3d42cdd190a8ba7b21a71d1a778))
* ENABLE_LSP_TOOL環境変数の値を"1"から"true"に修正 ([84d815e](https://github.com/akiojin/gwt/commit/84d815e8ed13db44b667f44ad458c6266f60967f))
* ESCキャンセル後にウィザードが開かない問題を修正 ([#501](https://github.com/akiojin/gwt/issues/501)) ([1a905f3](https://github.com/akiojin/gwt/commit/1a905f3a8807bc1ff9e6e3e13ee5d625b24d2c32))
* ESCキャンセル後にウィザードが開かない問題を修正 ([#501](https://github.com/akiojin/gwt/issues/501)) ([3665cfd](https://github.com/akiojin/gwt/commit/3665cfdd17cc64d5f101e87d87ab5ea9fb52faa7))
* execaのshell: trueオプションを削除してbunx起動エラーを修正 ([#458](https://github.com/akiojin/gwt/issues/458)) ([6de849f](https://github.com/akiojin/gwt/commit/6de849f70086dedcb8b79771590985cb6989833a))
* execaのshell: trueオプションを削除してbunx起動エラーを修正 ([#458](https://github.com/akiojin/gwt/issues/458)) ([65614df](https://github.com/akiojin/gwt/commit/65614dfd5ad6af697e60f54f309391b90ce274c5))
* FR-004準拠のフッターキーバインドヘルプを追加 ([04a4331](https://github.com/akiojin/gwt/commit/04a4331b490f883dae3f291fcac79285f58f89f7))
* FR-063a準拠のinstalled表示形式を修正 ([7eba7dc](https://github.com/akiojin/gwt/commit/7eba7dcd37accd6a6d977ab6d19a5d231beb16f2))
* FR-070準拠のツール表示形式から二重日時表示を削除 ([febfed2](https://github.com/akiojin/gwt/commit/febfed2b7f4c89a8c0b57b0eda76384b93ea87a7))
* FR-070準拠のツール表示形式に日時を追加 ([0002723](https://github.com/akiojin/gwt/commit/0002723a8328de2701fda9467d3f07d0458ad2e1))
* FR-072/FR-073準拠のバージョン表示形式を修正 ([9db149a](https://github.com/akiojin/gwt/commit/9db149ae7c9cf285ee9a49995e4ef0fe5aedf1a7))
* Gemini CLIのnpmパッケージ名を修正 ([5b5a8bb](https://github.com/akiojin/gwt/commit/5b5a8bbf538d74b3e780e757381d26f93d5756b7))
* gitデータ取得のタイムアウトを延長 ([27b88a0](https://github.com/akiojin/gwt/commit/27b88a07b2785d26e561066147140f5d2da7541d))
* gitデータ取得のタイムアウトを延長 ([697f977](https://github.com/akiojin/gwt/commit/697f977a4909447f550821d8e4b99336a713951a))
* gitデータ取得のタイムアウトを延長 ([5e5bdfe](https://github.com/akiojin/gwt/commit/5e5bdfe980c4636611005dc2858c94e84007c246))
* gitデータ取得のタイムアウトを延長 ([669f021](https://github.com/akiojin/gwt/commit/669f021cd5400489eccac98a0f680675ac684452))
* Git情報取得のタイムアウトを追加 ([bcccdbf](https://github.com/akiojin/gwt/commit/bcccdbf701165d773f8f64032002d02050515507))
* Git情報取得のタイムアウトを追加 ([9ec592f](https://github.com/akiojin/gwt/commit/9ec592f2b96f4f35dc1d6f661e29b7817cf70042))
* interactive loop test hang ([0e5643a](https://github.com/akiojin/gwt/commit/0e5643a3223cb4e409fb8758ad128d5bc722de5e))
* Issue 546のログ/ウィザード/モデル選択を改善 ([#551](https://github.com/akiojin/gwt/issues/551)) ([14d84a6](https://github.com/akiojin/gwt/commit/14d84a661bc575ddc8756e3f1de4ea2c509cf416))
* Mode表示を Stats 行の先頭に移動 ([d18ad5c](https://github.com/akiojin/gwt/commit/d18ad5cf2549128cf5faaf6f48f48e2a58ac2f7e))
* Mode表示を Stats 行の先頭に移動 ([0d7de3d](https://github.com/akiojin/gwt/commit/0d7de3dad4a3a5740a45a78c16bf2910311f9767))
* node-ptyで使用するコマンドのフルパスを解決 ([7d5ab76](https://github.com/akiojin/gwt/commit/7d5ab76d885b0d6363d7dca821312fedf2e746fa))
* node-ptyで使用するコマンドのフルパスを解決 ([d153690](https://github.com/akiojin/gwt/commit/d1536908fb3f8f38b394b62464bb33484864e620))
* package.json の description を Coding Agent 対応に修正 ([#471](https://github.com/akiojin/gwt/issues/471)) ([f7de165](https://github.com/akiojin/gwt/commit/f7de165c41ed609b5eb6e2b7e1f01f1df415346f))
* package.json の description を Coding Agent 対応に修正 ([#471](https://github.com/akiojin/gwt/issues/471)) ([5192936](https://github.com/akiojin/gwt/commit/51929364b20d463d42029a2eab85fc9cefceca23))
* post-session checks test hang ([4fa10b4](https://github.com/akiojin/gwt/commit/4fa10b4c477180e1b99beafcf4421b7a2901f3b2))
* Quick Startセッション解決をブランチ基準に修正 ([#547](https://github.com/akiojin/gwt/issues/547)) ([4e92ea9](https://github.com/akiojin/gwt/commit/4e92ea9f62d86c1216efd5e4a4246e1f1cd1e943))
* remove hardcoded release-type from release.yml ([0615caf](https://github.com/akiojin/gwt/commit/0615caf422e71e1b82c6e6299d882a710a3cf9b4))
* Repair機能のクロス環境対応とUI改善 ([#508](https://github.com/akiojin/gwt/issues/508)) ([9467fc0](https://github.com/akiojin/gwt/commit/9467fc0cd31417893e0ee3da808c011a8ecbdb3c))
* Repair機能のクロス環境対応とUI改善 ([#508](https://github.com/akiojin/gwt/issues/508)) ([f02c9f0](https://github.com/akiojin/gwt/commit/f02c9f073d4b197b0d50dddf20c59a96e31acb7c))
* run UI with bun runtime ([#537](https://github.com/akiojin/gwt/issues/537)) ([384441b](https://github.com/akiojin/gwt/commit/384441b34d8c486ad0b5f9ec8690686766bcec6b))
* saveSessionにtoolVersionを追加して履歴に保存 ([#515](https://github.com/akiojin/gwt/issues/515)) ([b2ae183](https://github.com/akiojin/gwt/commit/b2ae18366fbc23bcf331b540ceedf0d9c2402cf8))
* saveSessionにtoolVersionを追加して履歴に保存 ([#515](https://github.com/akiojin/gwt/issues/515)) ([c2066f9](https://github.com/akiojin/gwt/commit/c2066f97c2cf37959a4d529260b18cb7b1d4c2fe))
* show worktree path in branch footer ([#499](https://github.com/akiojin/gwt/issues/499)) ([9d7ceec](https://github.com/akiojin/gwt/commit/9d7ceec012e77fabc4254dbddb342427cba14b15))
* show worktree path in branch footer ([#499](https://github.com/akiojin/gwt/issues/499)) ([db3c194](https://github.com/akiojin/gwt/commit/db3c1947927816c80461f8d68cdc00704a9101d6))
* SPAルーティング用のフォールバック処理を追加 ([a4a0404](https://github.com/akiojin/gwt/commit/a4a0404a439dc07e70ff646f1ca16d8f05d35fef))
* SPAルーティング用のフォールバック処理を追加 ([7c49c2b](https://github.com/akiojin/gwt/commit/7c49c2ba5ecdc6d7abef01bc3bc1d097b04b4d43))
* SPEC-d2f4762a FR要件準拠の修正 ([6b3ac6b](https://github.com/akiojin/gwt/commit/6b3ac6b2d20e345a5321be12689b0157f7f076a7))
* stabilize dependency installer test mocks ([df1181e](https://github.com/akiojin/gwt/commit/df1181e93ae22158bc48d21e60e321f683ee9ac4))
* stabilize dependency installer test mocks ([a8b5b4a](https://github.com/akiojin/gwt/commit/a8b5b4aa182f96111855883343adf770e9131033))
* stabilize OpenTUI solid tests and UI layout ([#490](https://github.com/akiojin/gwt/issues/490)) ([af276b9](https://github.com/akiojin/gwt/commit/af276b9bb06c8140c6d878c86244a5f643e6f1b6))
* stabilize OpenTUI solid tests and UI layout ([#490](https://github.com/akiojin/gwt/issues/490)) ([a1cf460](https://github.com/akiojin/gwt/commit/a1cf46082699a4a76718e812a1642efd7c3337c6))
* **test:** Bun互換性のためのテスト修正 ([#527](https://github.com/akiojin/gwt/issues/527)) ([6630f25](https://github.com/akiojin/gwt/commit/6630f25b8fc77afb1bd31aae4c3ae05196776339))
* **test:** Bun互換性のためのテスト修正 ([#527](https://github.com/akiojin/gwt/issues/527)) ([6d4c3d4](https://github.com/akiojin/gwt/commit/6d4c3d4158bb949d013cd77ad40c91fce93a9178))
* **test:** worktree.test.tsのVitest依存を削除してBun互換に修正 ([#533](https://github.com/akiojin/gwt/issues/533)) ([00e4dc5](https://github.com/akiojin/gwt/commit/00e4dc5958ffe4989ceedad2205c845dd0b65432))
* **test:** worktree.test.tsのVitest依存を削除してBun互換に修正 ([#533](https://github.com/akiojin/gwt/issues/533)) ([d4f9998](https://github.com/akiojin/gwt/commit/d4f99983516f1f56596bded6a23176129d030b5b))
* tools.json の customTools → customCodingAgents マイグレーション対応 ([#476](https://github.com/akiojin/gwt/issues/476)) ([99f1c7f](https://github.com/akiojin/gwt/commit/99f1c7f3b9263e087de0c3c4cd96de8adccbe229))
* tools.json の customTools → customCodingAgents マイグレーション対応 ([#476](https://github.com/akiojin/gwt/issues/476)) ([1f375e8](https://github.com/akiojin/gwt/commit/1f375e8ccdcf41cbeaa687a2792e094be498beaa))
* TUIキーバインドをTypeScript版と一致させる ([055449f](https://github.com/akiojin/gwt/commit/055449ff6386d6e0a77176bc2ed5ee69308a4ea6))
* TUI画面のレイアウト・プロファイル・ログ読み込みを修正 ([a6e0946](https://github.com/akiojin/gwt/commit/a6e0946f011d4b9ddcc5dcc901e09dd102c96049))
* type-checkでcleanup対象の型エラーを解消 ([a617a2d](https://github.com/akiojin/gwt/commit/a617a2d53575f55985d9d7aa3d3e2fc8717e91fc))
* type-checkでcleanup対象の型エラーを解消 ([1abd7e0](https://github.com/akiojin/gwt/commit/1abd7e0adf393165c73e35bca52b3b1790ad906a))
* unsafe確認ダイアログ反転と凡例のSafe追加 ([#544](https://github.com/akiojin/gwt/issues/544)) ([1b627f7](https://github.com/akiojin/gwt/commit/1b627f7de994c435c3061f6be41a24236f5eccaf))
* update workspace version to 5.0.0 for release-please ([7ceb8dc](https://github.com/akiojin/gwt/commit/7ceb8dc95799147f502830d4593b94f30ced73f4))
* use cargo-workspace release type for release-please ([12902f1](https://github.com/akiojin/gwt/commit/12902f16aef86d3f0c9b942f6d7fca229148d7be))
* use explicit versions in crate Cargo.toml for release-please ([ec1c42f](https://github.com/akiojin/gwt/commit/ec1c42fa53f6d2f5a8ae28417c0c4b4d8c92985a))
* use node release type with extra-files for Cargo.toml ([619d1dc](https://github.com/akiojin/gwt/commit/619d1dcb2f7e1df174704b052895e2c1f4680a5f))
* warn then return after dirty worktree ([#453](https://github.com/akiojin/gwt/issues/453)) ([c9cead9](https://github.com/akiojin/gwt/commit/c9cead9841f6522fbe8cf43258eceb1183289c79))
* warn then return after dirty worktree ([#453](https://github.com/akiojin/gwt/issues/453)) ([7a02e23](https://github.com/akiojin/gwt/commit/7a02e2375a8639d4fafc3c7f13710c210797f569))
* Web UIからClaude Code起動時にENABLE_LSP_TOOL環境変数を渡す ([49fea84](https://github.com/akiojin/gwt/commit/49fea847f95e3d997313459389ee4a33aa42dade))
* Web UIからClaude Code起動時にENABLE_LSP_TOOL環境変数を渡す ([204b81c](https://github.com/akiojin/gwt/commit/204b81ce2c36651393fe8f9d1400c86f11215786))
* Web UIからClaude Code起動時にENABLE_LSP_TOOL環境変数を渡す ([84fedf3](https://github.com/akiojin/gwt/commit/84fedf3918c30b85cae6bb6cdac8f0488ea079ff))
* Web UIからClaude Code起動時にENABLE_LSP_TOOL環境変数を渡す ([89f2115](https://github.com/akiojin/gwt/commit/89f2115fe5873649d2f8d47c40e936bf367f8735))
* Web UIのデフォルトポートを3001に変更 ([597fff3](https://github.com/akiojin/gwt/commit/597fff3310b294d67ec140542d72504ee17a966b))
* Web UIのデフォルトポートを3001に変更 ([b33b0a7](https://github.com/akiojin/gwt/commit/b33b0a79acbaadfe2422f13e75270d9b409ff9cf))
* WebSocket接続エラーの即時表示を抑制 ([c0ac929](https://github.com/akiojin/gwt/commit/c0ac929a052ceb3700feee1a9964cb85dfd9c052))
* WebSocket接続エラーの即時表示を抑制 ([848bab6](https://github.com/akiojin/gwt/commit/848bab63f03c1c3ee6155f25b3e1989db407c359))
* Worktree selection and docs updates ([#565](https://github.com/akiojin/gwt/issues/565)) ([4ab30c6](https://github.com/akiojin/gwt/commit/4ab30c61ffc3c09085dcff6c95932409b0620c00))
* worktreeからメインリポジトリルートを解決してセッションファイルを検索 ([d917b00](https://github.com/akiojin/gwt/commit/d917b00142b50b14d8ba17a43d05a67a240e639a))
* worktree作成時のstale残骸を自動回復 ([#445](https://github.com/akiojin/gwt/issues/445)) ([ce971a6](https://github.com/akiojin/gwt/commit/ce971a6e4cd85db5c8c5295d9a1a6502898e9012))
* worktree作成時のstale残骸を自動回復 ([#445](https://github.com/akiojin/gwt/issues/445)) ([1156849](https://github.com/akiojin/gwt/commit/11568491840c985a2c6b68e3d50171c2e6ac2a5a))
* Worktree修復ロジックの統一化とクロス環境対応 ([#509](https://github.com/akiojin/gwt/issues/509)) ([f918328](https://github.com/akiojin/gwt/commit/f918328ed40bc6cccb91835b02c3e40d7c4551e9))
* Worktree修復ロジックの統一化とクロス環境対応 ([#509](https://github.com/akiojin/gwt/issues/509)) ([413213c](https://github.com/akiojin/gwt/commit/413213c8de0b6425ad6f1c24774d647aa6fad4a5))
* Worktree無しブランチの選択を抑止 ([#564](https://github.com/akiojin/gwt/issues/564)) ([3a6073c](https://github.com/akiojin/gwt/commit/3a6073ce48e59557a4b46495a4540703100958da))
* WSL1検出でChrome統合を無効化する ([085688c](https://github.com/akiojin/gwt/commit/085688cbd1572f0ae79bf2408ca281f74e003f75))
* WSL1検出でChrome統合を無効化する ([3e611d8](https://github.com/akiojin/gwt/commit/3e611d8ecee8f44712478f584dae03b00124d359))
* WSLの矢印キー誤認を防止 ([cbad8f6](https://github.com/akiojin/gwt/commit/cbad8f600d0bba5c751f1062f54136aaf7235aef))
* WSLの矢印キー誤認を防止 ([b02d651](https://github.com/akiojin/gwt/commit/b02d6516aa07454f9f4e4c033128a0a97d803d0d))
* ウィザードのfocus型を厳密オプションに合わせる ([de24488](https://github.com/akiojin/gwt/commit/de244887061bc859970d779100839a4ba353de72))
* ウィザードのfocus型を厳密オプションに合わせる ([714f62e](https://github.com/akiojin/gwt/commit/714f62e0c7579ffebae7aef7e777d5ebcad7baa6))
* ウィザードのモデル選択・エージェント色をTypeScript版に合わせて修正 ([5a15d7f](https://github.com/akiojin/gwt/commit/5a15d7f3ca891735aa5653352e6fedcd4cc02555))
* ウィザード内スクロールの上下キー対応を追加 ([80d62c1](https://github.com/akiojin/gwt/commit/80d62c10796cbc1b8ad108a6ba3dae6d43a1f3ec))
* ウィザード内スクロールの上下キー対応を追加 ([205b9a9](https://github.com/akiojin/gwt/commit/205b9a92f7d4f3ec149ace0a2bef749619a4c9d4))
* ウィザード内スクロールの上下キー対応を追加 ([97685df](https://github.com/akiojin/gwt/commit/97685df5c1c71fdf7473778b60aa93cb9586ddaf))
* ウィザード内スクロールの上下キー対応を追加 ([726add9](https://github.com/akiojin/gwt/commit/726add96bb02be0b23497c566f810700796a818c))
* ウィザード表示・スピナー・エージェント色マッピングを修正 ([71564ec](https://github.com/akiojin/gwt/commit/71564ec27203aae796802058539d71f54908c578))
* カーソル位置をグローバル管理に変更して安全状態更新時のリセットを防止 ([#541](https://github.com/akiojin/gwt/issues/541)) ([dcb9f74](https://github.com/akiojin/gwt/commit/dcb9f74ab506e765f1be8c011a5cd27843435fb2))
* クリーンアップ安全表示を候補判定に連動 ([#514](https://github.com/akiojin/gwt/issues/514)) ([ce1af34](https://github.com/akiojin/gwt/commit/ce1af34ada74962a35e7087cbde72366d208184a))
* クリーンアップ安全表示を候補判定に連動 ([#514](https://github.com/akiojin/gwt/issues/514)) ([389a9e6](https://github.com/akiojin/gwt/commit/389a9e6f39a63db31647fee47ec4184862fbed18))
* クリーンアップ選択の安全判定を要件どおりに更新 ([6c0a595](https://github.com/akiojin/gwt/commit/6c0a5957a866c4f9475f9dd30cb965331a99bfa8))
* クリーンアップ選択の安全判定を要件どおりに更新 ([2392aa3](https://github.com/akiojin/gwt/commit/2392aa390748f4d54147570ae6b1e089e39b33bf))
* クリーンアップ選択の安全判定を要件どおりに更新 ([23c89c7](https://github.com/akiojin/gwt/commit/23c89c7ee2a5ef843c6d796e013e80c85607e857))
* クリーンアップ選択の安全判定を要件どおりに更新 ([7e9042f](https://github.com/akiojin/gwt/commit/7e9042f131a1ae573cd3777cc80ebbae0116fe1a))
* コーディングエージェント起動時の即時終了問題を修正 ([633f0d6](https://github.com/akiojin/gwt/commit/633f0d65aa3724eb38c6084433b6da1cc688d342)), closes [#546](https://github.com/akiojin/gwt/issues/546)
* セッションIDの表示と再開を改善 ([#505](https://github.com/akiojin/gwt/issues/505)) ([b8b0b1e](https://github.com/akiojin/gwt/commit/b8b0b1ea501de0336c3f38e8710cd8370aa0d128))
* セッションIDの表示と再開を改善 ([#505](https://github.com/akiojin/gwt/issues/505)) ([0ad5aa5](https://github.com/akiojin/gwt/commit/0ad5aa5394f683e4db847dc12db381eae1f7d2e4))
* フィルターモード中のキーバインド処理を修正 ([74072dc](https://github.com/akiojin/gwt/commit/74072dc52b5387cf22a56708cd182dbca8a07821))
* ブランチリスト画面のフリッカーを解消 ([#433](https://github.com/akiojin/gwt/issues/433)) ([3331c5d](https://github.com/akiojin/gwt/commit/3331c5d5f28847a95399a20b23fee97685db858d))
* ブランチリスト画面のフリッカーを解消 ([#433](https://github.com/akiojin/gwt/issues/433)) ([535fa42](https://github.com/akiojin/gwt/commit/535fa4205ff53cf456bee15b5a36169815c297f8))
* ブランチリフレッシュ時にリモート追跡参照を更新 & CI/CD最適化 ([#554](https://github.com/akiojin/gwt/issues/554)) ([929a5ce](https://github.com/akiojin/gwt/commit/929a5ceca5e601d96d2a49bff8dee7b7bf3867dd))
* ブランチ一覧にセッション履歴を反映 ([#497](https://github.com/akiojin/gwt/issues/497)) ([090a5e7](https://github.com/akiojin/gwt/commit/090a5e739b76699ee6a06fab69dd4d70a1b06090))
* ブランチ一覧にセッション履歴を反映 ([#497](https://github.com/akiojin/gwt/issues/497)) ([177d285](https://github.com/akiojin/gwt/commit/177d285a0351a1d5b01861af241b109831d98ce6))
* ブランチ一覧のASCII表記を調整 ([#500](https://github.com/akiojin/gwt/issues/500)) ([9f3e752](https://github.com/akiojin/gwt/commit/9f3e752132b3313e66b8efac7726679197004bfa))
* ブランチ一覧のASCII表記を調整 ([#500](https://github.com/akiojin/gwt/issues/500)) ([eb21ae5](https://github.com/akiojin/gwt/commit/eb21ae57bbd975a88e26d84a65ad104ea03a8a82))
* ブランチ一覧取得時にrepoRootを使用するよう修正 ([71edc72](https://github.com/akiojin/gwt/commit/71edc7235d93f4f8172ddb38fe2dbf82201cc46f))
* ブランチ一覧取得時にrepoRootを使用するよう修正 ([bcd194b](https://github.com/akiojin/gwt/commit/bcd194b3525789d4c19d552ca2a3bac611eef966))
* ブランチ一覧取得時にrepoRootを使用するよう修正 ([3d4b370](https://github.com/akiojin/gwt/commit/3d4b37049ef189d6512fabcb0440f11baf00fb24))
* ブランチ一覧取得時にrepoRootを使用するよう修正 ([1aec33c](https://github.com/akiojin/gwt/commit/1aec33ccca46afcd1b0542c39c38bf4d8afac1ca))
* ヘッダーフォーマットをTypeScript版に統一 ([19f2a86](https://github.com/akiojin/gwt/commit/19f2a8687d00675087e8dbe8bd70f2b574e813a7))
* マウスキャプチャを無効化してテキスト選択を可能に ([906af41](https://github.com/akiojin/gwt/commit/906af4139bfd567acecc07b5c8f953386bb219ac))
* リモートモードでローカル・リモート両存在ブランチが表示されない問題を修正 ([#430](https://github.com/akiojin/gwt/issues/430)) ([70a5876](https://github.com/akiojin/gwt/commit/70a5876796273c39dafae5223994eccfe826cda7))
* リモートモードでローカル・リモート両存在ブランチが表示されない問題を修正 ([#430](https://github.com/akiojin/gwt/issues/430)) ([163661d](https://github.com/akiojin/gwt/commit/163661d197c5a726c13cf409799328bab8832015))
* リモート取得遅延でもブランチ一覧を表示 ([5cea85a](https://github.com/akiojin/gwt/commit/5cea85a358399aa44633b58d54ef282ef4718b25))
* リモート取得遅延でもブランチ一覧を表示 ([2fb1688](https://github.com/akiojin/gwt/commit/2fb1688112be0a74978de07304004e919be38756))
* ログビューア表示と配色の統一 ([#538](https://github.com/akiojin/gwt/issues/538)) ([1067a0c](https://github.com/akiojin/gwt/commit/1067a0c15d5349c8f7e7840649fe2a43967391c5))
* 依存関係インストール時のスピナー表示を削除 ([#496](https://github.com/akiojin/gwt/issues/496)) ([3c53f51](https://github.com/akiojin/gwt/commit/3c53f5135befea8a71f07695e2e74576d6d31fed))
* 依存関係インストール時のスピナー表示を削除 ([#496](https://github.com/akiojin/gwt/issues/496)) ([28790f3](https://github.com/akiojin/gwt/commit/28790f3d79c2593e2bca21cd59f9d65196d8cdcd))
* 安全アイコンの安全表示を緑oに変更 ([#525](https://github.com/akiojin/gwt/issues/525)) ([f0e7ba9](https://github.com/akiojin/gwt/commit/f0e7ba9b5f01bfc2bd55173afe1d60ca434d2fde))
* 安全アイコン表示のルールを更新 ([#516](https://github.com/akiojin/gwt/issues/516)) ([4a078b5](https://github.com/akiojin/gwt/commit/4a078b53f104c324321ecee339c331578c8a4f22))
* 安全アイコン表示のルールを更新 ([#516](https://github.com/akiojin/gwt/issues/516)) ([70a15f7](https://github.com/akiojin/gwt/commit/70a15f72e0ea5adcc32b7d59bf0c8d13082b675f))
* 安全状態確認時のカーソルリセット問題を修正 ([#539](https://github.com/akiojin/gwt/issues/539)) ([77db8ea](https://github.com/akiojin/gwt/commit/77db8ea3c629affbb930fc2074efb4a312837895))
* 未対応環境ではClaude CodeのChrome統合をスキップする ([f76cc0c](https://github.com/akiojin/gwt/commit/f76cc0c8f8ea36c8b270a3132ab63bcdfdfd77ab))
* 未対応環境ではClaude CodeのChrome統合をスキップする ([e1548a3](https://github.com/akiojin/gwt/commit/e1548a3213fdbfcd6c9df1ab7ace797b6d00b1cd))
* 未対応環境ではClaude CodeのChrome統合をスキップする ([f744901](https://github.com/akiojin/gwt/commit/f744901062e5c02e35c27947a4beace2359ecaaf))
* 未対応環境ではClaude CodeのChrome統合をスキップする ([69018d9](https://github.com/akiojin/gwt/commit/69018d9fb29a5de208db77a55140e855007283bd))
* 相対パス起動のエントリ判定を安定化 ([618093e](https://github.com/akiojin/gwt/commit/618093e61738dbf2799c16b103c015fb27d69913))
* 相対パス起動のエントリ判定を安定化 ([4a9aeaf](https://github.com/akiojin/gwt/commit/4a9aeaf78f0f94bdb172bb1ed674413ce03c2120))
* 自動インストール警告文のタイポ修正 ([#451](https://github.com/akiojin/gwt/issues/451)) ([15917e3](https://github.com/akiojin/gwt/commit/15917e3d28a7ef950278a73dfaa5c298dfd4633f))
* 自動インストール警告文のタイポ修正 ([#451](https://github.com/akiojin/gwt/issues/451)) ([9942141](https://github.com/akiojin/gwt/commit/994214113b86888de097a6409af10ab4d9dc638f))
* 起動ログの出力経路とCodexセッションID検出を改善 ([#495](https://github.com/akiojin/gwt/issues/495)) ([e9da151](https://github.com/akiojin/gwt/commit/e9da1517b1cdedfd0e8a8eff03a6d44654c5fdf3))
* 起動ログの出力経路とCodexセッションID検出を改善 ([#495](https://github.com/akiojin/gwt/issues/495)) ([ccb6391](https://github.com/akiojin/gwt/commit/ccb63912fd2e1265ba77a9b627daaef1ebb873ef))


### Performance Improvements

* ブランチ一覧のgit状態取得をキャッシュ化 ([#446](https://github.com/akiojin/gwt/issues/446)) ([76f30d6](https://github.com/akiojin/gwt/commit/76f30d64f7a5674eea07843c0aafb385f37e8282))
* ブランチ一覧のgit状態取得をキャッシュ化 ([#446](https://github.com/akiojin/gwt/issues/446)) ([13642ac](https://github.com/akiojin/gwt/commit/13642ac7bdfaa465907dbaec8a067c40b9ce6993))

## [4.11.6](https://github.com/akiojin/gwt/compare/v4.11.5...v4.11.6) (2026-01-07)


### Bug Fixes

* ログ読み込みをウィンドウ走査方式に変更しworktree切替時の誤セッション検出を防止 ([#524](https://github.com/akiojin/gwt/issues/524)) ([bb49285](https://github.com/akiojin/gwt/commit/bb49285c33ffa0b2c360f5ce78f8d31f3ef1f85b))

## [4.11.5](https://github.com/akiojin/gwt/compare/v4.11.4...v4.11.5) (2026-01-07)


### Bug Fixes

* ツール選択ステップ状態管理とカーソル可視性を修正 ([#521](https://github.com/akiojin/gwt/issues/521)) ([5a12f79](https://github.com/akiojin/gwt/commit/5a12f7956d4efea93beb98a259afa95e954b78a5))

## [4.11.4](https://github.com/akiojin/gwt/compare/v4.11.3...v4.11.4) (2026-01-07)


### Bug Fixes

* AppSolid のポップアップリスト項目表示を修正 ([#518](https://github.com/akiojin/gwt/issues/518)) ([8daf3b8](https://github.com/akiojin/gwt/commit/8daf3b831b91e0aa54ba1f2dd6f45b4ae3e14e73))

## [4.11.3](https://github.com/akiojin/gwt/compare/v4.11.2...v4.11.3) (2026-01-06)


### Bug Fixes

* CodeRabbit指摘のOpenTUI/SolidJS警告とパフォーマンス問題を修正 ([#513](https://github.com/akiojin/gwt/issues/513)) ([cce6e09](https://github.com/akiojin/gwt/commit/cce6e0930eb5e2a2db42dc1e2eb7ab2c9b6c8e38))

## [4.11.2](https://github.com/akiojin/gwt/compare/v4.11.1...v4.11.2) (2026-01-06)


### Bug Fixes

* ログ読み込みをウィンドウ走査方式に変更しworktree切替時の誤セッション検出を防止 ([#512](https://github.com/akiojin/gwt/issues/512)) ([a2fc839](https://github.com/akiojin/gwt/commit/a2fc8392f61b07e3c7f6a0b18bd39dbc51b30d1d))

## [4.11.1](https://github.com/akiojin/gwt/compare/v4.11.0...v4.11.1) (2026-01-06)


### Bug Fixes

* Claude履歴読み取りとGemini jsonl検出の修正 ([#511](https://github.com/akiojin/gwt/issues/511)) ([71f8c3e](https://github.com/akiojin/gwt/commit/71f8c3e54ca5a1adfb9f8f5214e01a5fbfe3ea77))

## [4.11.0](https://github.com/akiojin/gwt/compare/v4.10.1...v4.11.0) (2026-01-06)


### Features

* Continueモード選択時にセッションID選択画面を表示 ([#508](https://github.com/akiojin/gwt/issues/508)) ([f37d56d](https://github.com/akiojin/gwt/commit/f37d56db6e6e0b24913ea9e0a5f4a07eb1dba79d))

## [4.10.1](https://github.com/akiojin/gwt/compare/v4.10.0...v4.10.1) (2026-01-05)


### Bug Fixes

* バージョン取得とトレイ機能の堅牢化 ([#486](https://github.com/akiojin/gwt/issues/486)) ([9ffb7fb](https://github.com/akiojin/gwt/commit/9ffb7fb16a5af7e4bcbc30aa39e9b039b91611b2))

## [4.10.0](https://github.com/akiojin/gwt/compare/v4.9.1...v4.10.0) (2026-01-05)


### Features

* OpenCode コーディングエージェント対応を追加 ([#477](https://github.com/akiojin/gwt/issues/477)) ([c480fb4](https://github.com/akiojin/gwt/commit/c480fb4b16c0f815a205776eb618fbcb875d238b))


### Bug Fixes

* tools.json の customTools → customCodingAgents マイグレーション対応 ([#476](https://github.com/akiojin/gwt/issues/476)) ([99f1c7f](https://github.com/akiojin/gwt/commit/99f1c7f3b9263e087de0c3c4cd96de8adccbe229))

## [4.9.1](https://github.com/akiojin/gwt/compare/v4.9.0...v4.9.1) (2026-01-05)


### Bug Fixes

* Gemini CLI Resume モードの --last-session 引数追加 ([#474](https://github.com/akiojin/gwt/issues/474)) ([daa2e3a](https://github.com/akiojin/gwt/commit/daa2e3a0f8cda5f1bfb5d9b54a01cb2f6c445af3))

## [4.9.0](https://github.com/akiojin/gwt/compare/v4.8.1...v4.9.0) (2026-01-05)


### Features

* OpenTUI/SolidJS へのUI完全移行（FR-002～FR-006/FR-024） ([#449](https://github.com/akiojin/gwt/issues/449)) ([f26fdb0](https://github.com/akiojin/gwt/commit/f26fdb0115eb6ac2f3de9c9bb9fbe9d37bc9f83d))


### Bug Fixes

* claude resume/continue セッション指定の修正 ([#469](https://github.com/akiojin/gwt/issues/469)) ([b0af9f6](https://github.com/akiojin/gwt/commit/b0af9f6f430b5f2d4e1d99dc5ea21df1eaf4dae1))

## [4.8.1](https://github.com/akiojin/gwt/compare/v4.8.0...v4.8.1) (2026-01-04)


### Bug Fixes

* OpenTUI コンポーネントのマウス操作とカーソル処理を修正 ([#443](https://github.com/akiojin/gwt/issues/443)) ([3ebfbf7](https://github.com/akiojin/gwt/commit/3ebfbf7b31fab429f4cecc6f53ddf4c35f44f740))

## [4.8.0](https://github.com/akiojin/gwt/compare/v4.7.4...v4.8.0) (2026-01-04)


### Features

* Gemini CLI 対応を追加 ([#441](https://github.com/akiojin/gwt/issues/441)) ([5c3abb5](https://github.com/akiojin/gwt/commit/5c3abb508f7a95f3f6cb94e5dca423e7a6a9e7d0))

## [4.7.4](https://github.com/akiojin/gwt/compare/v4.7.3...v4.7.4) (2026-01-03)


### Bug Fixes

* コミットメッセージのmarkdownlintエラーを修正し、空行ルールに準拠 ([#434](https://github.com/akiojin/gwt/issues/434)) ([2ff3c79](https://github.com/akiojin/gwt/commit/2ff3c7992bf9ed1a460a21ab0f14b2f5acfae91f))
* 新規ブランチ作成時のUI選択フローとキー操作を修正 ([#428](https://github.com/akiojin/gwt/issues/428)) ([2a359fa](https://github.com/akiojin/gwt/commit/2a359faa01fd0f35ab0e7c0d6b1edffa3f3d93a6))

## [4.7.3](https://github.com/akiojin/gwt/compare/v4.7.2...v4.7.3) (2026-01-03)


### Bug Fixes

* Codex CLI Continue機能のセッションID検出を修正 ([#426](https://github.com/akiojin/gwt/issues/426)) ([1df6c20](https://github.com/akiojin/gwt/commit/1df6c20e80131dcfd05b37feabf30bdef398d2d5))

## [4.7.2](https://github.com/akiojin/gwt/compare/v4.7.1...v4.7.2) (2026-01-02)


### Bug Fixes

* ブランチ選択でベースブランチ優先とクイックスタートの修正 ([#420](https://github.com/akiojin/gwt/issues/420)) ([d93b14c](https://github.com/akiojin/gwt/commit/d93b14c83f95d1b1a4ad1f1eb6eae94f2068ecb7))

## [4.7.1](https://github.com/akiojin/gwt/compare/v4.7.0...v4.7.1) (2026-01-02)


### Bug Fixes

* ブランチリスト項目の視認性とマウス操作領域を改善 ([#417](https://github.com/akiojin/gwt/issues/417)) ([cddf15a](https://github.com/akiojin/gwt/commit/cddf15a9b55e0cece67e62dfc5d5b39647e27abd))

## [4.7.0](https://github.com/akiojin/gwt/compare/v4.6.2...v4.7.0) (2026-01-02)


### Features

* Codex Continue/Resumeセッション自動検出機能を実装 ([#413](https://github.com/akiojin/gwt/issues/413)) ([ed33953](https://github.com/akiojin/gwt/commit/ed33953e6cbef9b0ac9ff1cbb30d15ace58ff1ff))


### Bug Fixes

* ブランチリストの選択状態をカーソルと別管理に変更 ([#411](https://github.com/akiojin/gwt/issues/411)) ([d4ce7e6](https://github.com/akiojin/gwt/commit/d4ce7e6f4afb1d4ff5f3d8cf59dc90e8f6ca3faa))

## [4.6.2](https://github.com/akiojin/gwt/compare/v4.6.1...v4.6.2) (2026-01-01)


### Bug Fixes

* Quick Start パネルのキー操作を修正 ([#408](https://github.com/akiojin/gwt/issues/408)) ([ba8bdef](https://github.com/akiojin/gwt/commit/ba8bdefd6b1e0f98e159dc6e0d425ef28e81ac6d))

## [4.6.1](https://github.com/akiojin/gwt/compare/v4.6.0...v4.6.1) (2026-01-01)


### Bug Fixes

* Quick Start パネルの挿入行計算エラーを修正し、末尾空行を補正 ([#406](https://github.com/akiojin/gwt/issues/406)) ([25fdc25](https://github.com/akiojin/gwt/commit/25fdc25e2a2bba4d6ff4d1f2bd2f64d7e94afb81))

## [4.6.0](https://github.com/akiojin/gwt/compare/v4.5.0...v4.6.0) (2026-01-01)


### Features

* Quick Start パネルを SolidJS 実装 ([#404](https://github.com/akiojin/gwt/issues/404)) ([84cdd5d](https://github.com/akiojin/gwt/commit/84cdd5d0faf6a3f3f2d4f5bedf93f8bcf1a5e231))

## [4.5.0](https://github.com/akiojin/gwt/compare/v4.4.8...v4.5.0) (2025-12-31)


### Features

* クイックスタート表示切り替えショートカット (Ctrl+Q) ([#399](https://github.com/akiojin/gwt/issues/399)) ([baebdc5](https://github.com/akiojin/gwt/commit/baebdc53fcac1f9d60db22ca7ed7d9005c6e6a8d))


### Bug Fixes

* BranchListScreen のマウス座標設定を修正 ([#396](https://github.com/akiojin/gwt/issues/396)) ([a27a10e](https://github.com/akiojin/gwt/commit/a27a10e2203ed6be5f3f0d1b50b3bf0fff920456))
* クイックスタート選択時のキー入力処理を修正 ([#401](https://github.com/akiojin/gwt/issues/401)) ([d3fc5f2](https://github.com/akiojin/gwt/commit/d3fc5f2f3e4caa54b97ec23d5c7ca75acc4fed1d))

## [4.4.8](https://github.com/akiojin/gwt/compare/v4.4.7...v4.4.8) (2025-12-31)


### Bug Fixes

* BranchListScreen のマウスカーソル位置計算を修正 ([#393](https://github.com/akiojin/gwt/issues/393)) ([43b9bb7](https://github.com/akiojin/gwt/commit/43b9bb73a1a0d9d7f0d930a4ee1a7b69fbf25e6d))

## [4.4.7](https://github.com/akiojin/gwt/compare/v4.4.6...v4.4.7) (2025-12-31)


### Bug Fixes

* ブランチ名切り詰め処理で全角文字幅を正しく計測 ([#390](https://github.com/akiojin/gwt/issues/390)) ([deaffd5](https://github.com/akiojin/gwt/commit/deaffd5f5d1c6bbe6f8f2104f1f0cfdd1ef3f1d9))

## [4.4.6](https://github.com/akiojin/gwt/compare/v4.4.5...v4.4.6) (2025-12-31)


### Bug Fixes

* ブランチ名切り詰め文字数計算の修正 ([#387](https://github.com/akiojin/gwt/issues/387)) ([8c1d1d0](https://github.com/akiojin/gwt/commit/8c1d1d0b5dce6b1f2c474f1a69d07d2d2e4ef66b))

## [4.4.5](https://github.com/akiojin/gwt/compare/v4.4.4...v4.4.5) (2025-12-31)


### Bug Fixes

* ブランチリストの表示幅計算を修正して切り詰め問題を解消 ([#384](https://github.com/akiojin/gwt/issues/384)) ([c44aa9a](https://github.com/akiojin/gwt/commit/c44aa9a56dba5ac61f1f9e430ab50e0f92013bc4))

## [4.4.4](https://github.com/akiojin/gwt/compare/v4.4.3...v4.4.4) (2025-12-30)


### Bug Fixes

* Codex CLI のセッション引継ぎ引数を --resume から --conversation-id に修正 ([#381](https://github.com/akiojin/gwt/issues/381)) ([b11bf4d](https://github.com/akiojin/gwt/commit/b11bf4d57a3f950e0c1418f32e5b6f5f4fc21fa0))

## [4.4.3](https://github.com/akiojin/gwt/compare/v4.4.2...v4.4.3) (2025-12-30)


### Bug Fixes

* ブランチタイプフィルタの選択状態同期とリスト描画の修正 ([#379](https://github.com/akiojin/gwt/issues/379)) ([5e06693](https://github.com/akiojin/gwt/commit/5e06693a63eee4e14410c91f1a2c91f89f95a4b7))

## [4.4.2](https://github.com/akiojin/gwt/compare/v4.4.1...v4.4.2) (2025-12-30)


### Bug Fixes

* BranchFilter 表示問題を修正 ([#376](https://github.com/akiojin/gwt/issues/376)) ([a8b0df2](https://github.com/akiojin/gwt/commit/a8b0df221c0f91dae62f6b72d4f2d0ab399d10a0))

## [4.4.1](https://github.com/akiojin/gwt/compare/v4.4.0...v4.4.1) (2025-12-30)


### Bug Fixes

* スクロールバー表示の条件を修正 ([#373](https://github.com/akiojin/gwt/issues/373)) ([a0e9d0d](https://github.com/akiojin/gwt/commit/a0e9d0da1baca0a26dfbdd1b6dc29b0fc5b4cb79))

## [4.4.0](https://github.com/akiojin/gwt/compare/v4.3.2...v4.4.0) (2025-12-30)


### Features

* OpenTUI 実験的UIバックエンド (GWT_UI=opentui) ([#370](https://github.com/akiojin/gwt/issues/370)) ([1cbb05c](https://github.com/akiojin/gwt/commit/1cbb05c54f87da20f0d5a21e6dca0a2cd54d8c7d))


### Bug Fixes

* OpenTUI ブランチ一覧の表示崩れを修正 ([#371](https://github.com/akiojin/gwt/issues/371)) ([3f84a25](https://github.com/akiojin/gwt/commit/3f84a256e72bd6ad67a4caecc3d5a05bbfaa7d75))

## [4.3.2](https://github.com/akiojin/gwt/compare/v4.3.1...v4.3.2) (2025-12-29)


### Bug Fixes

* TUI 状態管理の null 安全性を強化しハングを防止 ([#358](https://github.com/akiojin/gwt/issues/358)) ([a2e9a97](https://github.com/akiojin/gwt/commit/a2e9a97a52a1c0e30e13ed5a0e0d2b8e01a01a89))

## [4.3.1](https://github.com/akiojin/gwt/compare/v4.3.0...v4.3.1) (2025-12-28)


### Bug Fixes

* TUIレンダリングと入力処理の競合によるハングを修正 ([#354](https://github.com/akiojin/gwt/issues/354)) ([13ec5e8](https://github.com/akiojin/gwt/commit/13ec5e8d14f4195a8eb9b17fe2e66e8ef59f3010))

## [4.3.0](https://github.com/akiojin/gwt/compare/v4.2.0...v4.3.0) (2025-12-28)


### Features

* TUI を React/Ink から Bun/Canvas 実装へ段階的移行開始 (FR-023 先行) ([#351](https://github.com/akiojin/gwt/issues/351)) ([a18a2f1](https://github.com/akiojin/gwt/commit/a18a2f154e68da8d8fa68b45ad9d22cd5ebbd4f9))


### Bug Fixes

* TUI 状態管理と再描画の競合を修正（循環参照除去） ([#349](https://github.com/akiojin/gwt/issues/349)) ([da4cea6](https://github.com/akiojin/gwt/commit/da4cea6ea9de5be9fc93cc0e3f8ecab4b60e7de1))

## [4.2.0](https://github.com/akiojin/gwt/compare/v4.1.1...v4.2.0) (2025-12-27)


### Features

* UI描画を毎フレーム全体クリアに変更し描画残りを解消 ([#341](https://github.com/akiojin/gwt/issues/341)) ([0fc3b9c](https://github.com/akiojin/gwt/commit/0fc3b9c2d7e7e9a8d8bf3d437ac58bad12a2fa09))

## [4.1.1](https://github.com/akiojin/gwt/compare/v4.1.0...v4.1.1) (2025-12-27)


### Bug Fixes

* 再描画時のカーソル位置復元を追加 ([#338](https://github.com/akiojin/gwt/issues/338)) ([00a2dc7](https://github.com/akiojin/gwt/commit/00a2dc798e3b85bd0f85e3c48dc03a3e3a47adf9))

## [4.1.0](https://github.com/akiojin/gwt/compare/v4.0.0...v4.1.0) (2025-12-27)


### Features

* 自作TUIレイヤーへの移行開始（React/Inkを段階的に置換） ([#337](https://github.com/akiojin/gwt/issues/337)) ([e9e3c9e](https://github.com/akiojin/gwt/commit/e9e3c9e5d394f3d5124f29f68a05ada420afc38c))

## [4.0.0](https://github.com/akiojin/gwt/compare/v3.11.0...v4.0.0) (2025-12-26)


### ⚠ BREAKING CHANGES

* Claude Code Version 1.0.25 以降を要求（--chrome フラグ追加）
- **機能追加**: Claude起動時に `--chrome` フラグを追加し、ブラウザ自動認証フローを有効化
- **依存関係更新**: claude-code@1.0.25 以降が前提
- **後方互換性**: 1.0.25未満のClaude Codeでは `--chrome` フラグが認識されず起動失敗の可能性あり

### Features

* Claude起動に --chrome フラグを追加（Claude Code 1.0.25要求） ([#328](https://github.com/akiojin/gwt/issues/328)) ([a5af39e](https://github.com/akiojin/gwt/commit/a5af39e44e36aec6f4dabb1eea7a1ae3ff0f6b20))

## [3.11.0](https://github.com/akiojin/gwt/compare/v3.10.1...v3.11.0) (2025-12-26)


### Features

* ツール選択ステップにモデル選択UIを追加 ([#315](https://github.com/akiojin/gwt/issues/315)) ([06b5daf](https://github.com/akiojin/gwt/commit/06b5dafdb18e5fd8b13fd8e88a28df9e6f1ed78f))


### Bug Fixes

* CodeRabbit指摘のモデルオプション生成ロジック修正 ([#321](https://github.com/akiojin/gwt/issues/321)) ([dff794f](https://github.com/akiojin/gwt/commit/dff794f9cb39a6a58b18f3e2aef5dbbdefc3c5f0))
* Codex CLI実行時の --model/-m と --full-auto 渡しバグを修正 ([#325](https://github.com/akiojin/gwt/issues/325)) ([72d3f66](https://github.com/akiojin/gwt/commit/72d3f66ffb47dcc4914f5d5eb4ad0fb58f4f1eaa))

## [3.10.1](https://github.com/akiojin/gwt/compare/v3.10.0...v3.10.1) (2025-12-24)


### Bug Fixes

* Resume/Continue モード選択の不具合を修正 ([#312](https://github.com/akiojin/gwt/issues/312)) ([9adaa64](https://github.com/akiojin/gwt/commit/9adaa64c1ab1e70fef444e421dd6ba8d5f09b1e7))

## [3.10.0](https://github.com/akiojin/gwt/compare/v3.9.4...v3.10.0) (2025-12-24)


### Features

* ツール選択ウィザードにResume/Continueモード選択を追加 ([#308](https://github.com/akiojin/gwt/issues/308)) ([2cfbf19](https://github.com/akiojin/gwt/commit/2cfbf195f3d5f10b7da30ac8c3ac4fddbc5f8fa9))

## [3.9.4](https://github.com/akiojin/gwt/compare/v3.9.3...v3.9.4) (2025-12-24)


### Bug Fixes

* createBranch のベースブランチ解決を修正し不正なリモート参照を防止 ([#305](https://github.com/akiojin/gwt/issues/305)) ([35c7b3c](https://github.com/akiojin/gwt/commit/35c7b3c19f8ec8bc1fba1bf0de1b3fd8e1f4fdb2))

## [3.9.3](https://github.com/akiojin/gwt/compare/v3.9.2...v3.9.3) (2025-12-23)


### Bug Fixes

* ブランチ一覧表示の即時完了処理を修正 ([#302](https://github.com/akiojin/gwt/issues/302)) ([9f7f96f](https://github.com/akiojin/gwt/commit/9f7f96f24dd4de0c8101f239a6c2f2b04723e6cd))

## [3.9.2](https://github.com/akiojin/gwt/compare/v3.9.1...v3.9.2) (2025-12-23)


### Bug Fixes

* 非同期オーバレイメッセージが空のままになるバグを修正 ([#299](https://github.com/akiojin/gwt/issues/299)) ([d94fc78](https://github.com/akiojin/gwt/commit/d94fc78ea5d5eb2fdbdc5da60ba1edb5b0b6b809))

## [3.9.1](https://github.com/akiojin/gwt/compare/v3.9.0...v3.9.1) (2025-12-23)


### Bug Fixes

* 新規ブランチ作成後の状態初期化とワークツリー生成のバグ修正 ([#296](https://github.com/akiojin/gwt/issues/296)) ([86d1b80](https://github.com/akiojin/gwt/commit/86d1b80ccbd20dbfbae2dc3deb4f9eee1bb6bcf5))

## [3.9.0](https://github.com/akiojin/gwt/compare/v3.8.1...v3.9.0) (2025-12-23)


### Features

* TUI にカスタムコーディングエージェント対応 UI を追加 ([#291](https://github.com/akiojin/gwt/issues/291)) ([0ddfa77](https://github.com/akiojin/gwt/commit/0ddfa77d4b9b2bfae9f1f6ad1fb5f430f429efe4))

## [3.8.1](https://github.com/akiojin/gwt/compare/v3.8.0...v3.8.1) (2025-12-22)


### Bug Fixes

* ワークツリー安全性チェックの並列実行とロック競合を修正 ([#284](https://github.com/akiojin/gwt/issues/284)) ([2d47f2e](https://github.com/akiojin/gwt/commit/2d47f2e0b01a1e53dee35c41ce5ba69d3b24d680))

## [3.8.0](https://github.com/akiojin/gwt/compare/v3.7.1...v3.8.0) (2025-12-21)


### Features

* tools.yaml によるカスタムコーディングエージェント定義機能を追加 ([#280](https://github.com/akiojin/gwt/issues/280)) ([5b5dca6](https://github.com/akiojin/gwt/commit/5b5dca672dc99bc3cb93e40bb39e50fb6db05b17))

## [3.7.1](https://github.com/akiojin/gwt/compare/v3.7.0...v3.7.1) (2025-12-21)


### Bug Fixes

* 分岐状態取得のバグを修正 ([#279](https://github.com/akiojin/gwt/issues/279)) ([23c49f4](https://github.com/akiojin/gwt/commit/23c49f4f925d99e23f94f56dc1106a20f17b2a80))

## [3.7.0](https://github.com/akiojin/gwt/compare/v3.6.2...v3.7.0) (2025-12-21)


### Features

* ブランチ一覧画面での削除安全性チェック機能を追加 ([#270](https://github.com/akiojin/gwt/issues/270)) ([4d5e360](https://github.com/akiojin/gwt/commit/4d5e3604b65f53f56ca2e6b3ef0b84b0e92dcdca))

## [3.6.2](https://github.com/akiojin/gwt/compare/v3.6.1...v3.6.2) (2025-12-20)


### Bug Fixes

* スピナー状態の維持とブランチ一覧更新のバグ修正 ([#265](https://github.com/akiojin/gwt/issues/265)) ([a8047ab](https://github.com/akiojin/gwt/commit/a8047ab23b56f6f0fe3a2a6b4c1b9dee90b40fbb))

## [3.6.1](https://github.com/akiojin/gwt/compare/v3.6.0...v3.6.1) (2025-12-20)


### Bug Fixes

* ブランチ削除と一覧更新のバグを修正 ([#261](https://github.com/akiojin/gwt/issues/261)) ([5e12e77](https://github.com/akiojin/gwt/commit/5e12e77f6ecb9a77bf28aef3f20f9d49b41a5adb))

## [3.6.0](https://github.com/akiojin/gwt/compare/v3.5.1...v3.6.0) (2025-12-20)


### Features

* ブランチ削除前にリモート同期状態を確認するワークフロー追加 ([#258](https://github.com/akiojin/gwt/issues/258)) ([48e0ddf](https://github.com/akiojin/gwt/commit/48e0ddfa0c2b88d72f5cdef6e294bc6d445bacc5))

## [3.5.1](https://github.com/akiojin/gwt/compare/v3.5.0...v3.5.1) (2025-12-19)


### Bug Fixes

* ブランチ削除UIでバッチキャンセル時に残りが削除される問題を修正 ([#255](https://github.com/akiojin/gwt/issues/255)) ([b61fb59](https://github.com/akiojin/gwt/commit/b61fb5926464ae06f08af79ecf3f5f6df5c1a4d7))

## [3.5.0](https://github.com/akiojin/gwt/compare/v3.4.2...v3.5.0) (2025-12-19)


### Features

* ブランチ削除時にリモートブランチも同時削除するオプションを追加 ([#252](https://github.com/akiojin/gwt/issues/252)) ([f2f1c9f](https://github.com/akiojin/gwt/commit/f2f1c9f4bcdfb0f7f1cf4aad2a78a5f9a1fc3dcb))

## [3.4.2](https://github.com/akiojin/gwt/compare/v3.4.1...v3.4.2) (2025-12-19)


### Bug Fixes

* ブランチ削除UIの日本語文字化けとカーソル位置を修正 ([#249](https://github.com/akiojin/gwt/issues/249)) ([dc1eec6](https://github.com/akiojin/gwt/commit/dc1eec6807ae0bc84acbb4e26ea9215e28adc287))

## [3.4.1](https://github.com/akiojin/gwt/compare/v3.4.0...v3.4.1) (2025-12-19)


### Bug Fixes

* 不正なバンドル参照によるビルドエラーを修正 ([#246](https://github.com/akiojin/gwt/issues/246)) ([1cba3bf](https://github.com/akiojin/gwt/commit/1cba3bfc62130bda5467f5aa8e320f99f6fb0b8f))

## [3.4.0](https://github.com/akiojin/gwt/compare/v3.3.1...v3.4.0) (2025-12-18)


### Features

* ブランチ削除機能を実装 ([#242](https://github.com/akiojin/gwt/issues/242)) ([4ca3c3a](https://github.com/akiojin/gwt/commit/4ca3c3a3d9eb71e93d2cfc7cd12a2a34efc69660))

## [3.3.1](https://github.com/akiojin/gwt/compare/v3.3.0...v3.3.1) (2025-12-18)


### Bug Fixes

* TUIレンダリングのバグを修正 ([#240](https://github.com/akiojin/gwt/issues/240)) ([beac47f](https://github.com/akiojin/gwt/commit/beac47f67d44d9d05ab08dfeed08a44f7df23acf))

## [3.3.0](https://github.com/akiojin/gwt/compare/v3.2.0...v3.3.0) (2025-12-18)


### Features

* ブランチグラフ表示機能を追加 ([#232](https://github.com/akiojin/gwt/issues/232)) ([79e2b3a](https://github.com/akiojin/gwt/commit/79e2b3af06a3a2e27f1afe459ce1b09ced1ac27b))


### Bug Fixes

* 新規ブランチ作成フローのメモリリークを修正 ([#237](https://github.com/akiojin/gwt/issues/237)) ([8dd6e80](https://github.com/akiojin/gwt/commit/8dd6e80b72a0be98f3b40a2f0ba8e89a79560a56))

## [3.2.0](https://github.com/akiojin/gwt/compare/v3.1.0...v3.2.0) (2025-12-17)


### Features

* Web UIの基盤実装とブランチ一覧表示 ([#230](https://github.com/akiojin/gwt/issues/230)) ([f38e1bc](https://github.com/akiojin/gwt/commit/f38e1bcd5e16780eb2b6dbabbe79b76d99ec7d0a))

## [3.1.0](https://github.com/akiojin/gwt/compare/v3.0.6...v3.1.0) (2025-12-15)


### Features

* リモートブランチからワークツリー作成機能を追加 ([#227](https://github.com/akiojin/gwt/issues/227)) ([9aa1855](https://github.com/akiojin/gwt/commit/9aa1855e03e69f94b00b7c0a5e3a9af39cd0cdb5))

## [3.0.6](https://github.com/akiojin/gwt/compare/v3.0.5...v3.0.6) (2025-12-14)


### Bug Fixes

* セッション再開の自動検出を修正しログ書き込み単位を最適化 ([#224](https://github.com/akiojin/gwt/issues/224)) ([7ac5af2](https://github.com/akiojin/gwt/commit/7ac5af22f0fabc22cb39f3b70aff6de5a4e54b28))

## [3.0.5](https://github.com/akiojin/gwt/compare/v3.0.4...v3.0.5) (2025-12-14)


### Bug Fixes

* Claude Codeにおけるセッションの再開処理に不具合があったのを修正 ([#223](https://github.com/akiojin/gwt/issues/223)) ([adae8ca](https://github.com/akiojin/gwt/commit/adae8cabee52f0a25f72f7ddf1d1b28f8c29003b))

## [3.0.4](https://github.com/akiojin/gwt/compare/v3.0.3...v3.0.4) (2025-12-14)


### Bug Fixes

* 設定ファイル参照を .gwt から .config/gwt に統一 ([#217](https://github.com/akiojin/gwt/issues/217)) ([0c61e50](https://github.com/akiojin/gwt/commit/0c61e50bebc30f39cba6ed1f94cf18fef70a3bfc))

## [3.0.3](https://github.com/akiojin/gwt/compare/v3.0.2...v3.0.3) (2025-12-14)


### Bug Fixes

* ログ出力のJSON構造崩れとtimestamp漏れを修正 ([#213](https://github.com/akiojin/gwt/issues/213)) ([c3a6fcd](https://github.com/akiojin/gwt/commit/c3a6fcd1f7fce7a09f8f60d9e5ea86fc1c8fa0cc))

## [3.0.2](https://github.com/akiojin/gwt/compare/v3.0.1...v3.0.2) (2025-12-14)


### Bug Fixes

* 改行チェックを追加しログの破損を防止 ([#210](https://github.com/akiojin/gwt/issues/210)) ([41a4dbb](https://github.com/akiojin/gwt/commit/41a4dbbd4ca4b7de7ccd20e4b92a5d0bfb66f0ad))

## [3.0.1](https://github.com/akiojin/gwt/compare/v3.0.0...v3.0.1) (2025-12-14)


### Bug Fixes

* ログ出力の改行位置修正とコーディングエージェント起動ログの先行出力 ([#207](https://github.com/akiojin/gwt/issues/207)) ([0dc76fc](https://github.com/akiojin/gwt/commit/0dc76fc83cc2e5d8f7e1b6e45bc6e34b5a0efb7b))

## [3.0.0](https://github.com/akiojin/gwt/compare/v2.5.3...v3.0.0) (2025-12-14)


### ⚠ BREAKING CHANGES

* 構造化ログ出力を導入（従来のテキストログとの互換性なし）

### Features

* pino導入による構造化ログとログビューア機能 ([#200](https://github.com/akiojin/gwt/issues/200)) ([8af4f2c](https://github.com/akiojin/gwt/commit/8af4f2c1aabc7c8ac2bea3e9f6b36f32cb8f5fc7))

## [2.5.3](https://github.com/akiojin/gwt/compare/v2.5.2...v2.5.3) (2025-12-13)


### Bug Fixes

* 依存関係インストールと準備ステップのバグ修正 ([#191](https://github.com/akiojin/gwt/issues/191)) ([e8d8e9a](https://github.com/akiojin/gwt/commit/e8d8e9aa88aede71d2b3b04a0ff54dc8f9d44cdc))

## [2.5.2](https://github.com/akiojin/gwt/compare/v2.5.1...v2.5.2) (2025-12-12)


### Bug Fixes

* 依存インストールのロックファイル優先順位を修正 ([#188](https://github.com/akiojin/gwt/issues/188)) ([b2b14ac](https://github.com/akiojin/gwt/commit/b2b14ac8011a67d5a6a6e4adeee6d39f6a2f89f1))

## [2.5.1](https://github.com/akiojin/gwt/compare/v2.5.0...v2.5.1) (2025-12-12)


### Bug Fixes

* ワークツリー作成失敗時のユーザーフィードバックを改善 ([#185](https://github.com/akiojin/gwt/issues/185)) ([fc5e2ae](https://github.com/akiojin/gwt/commit/fc5e2ae0d6f63cfe65ff2a5e9e14dd89e1cff8c9))

## [2.5.0](https://github.com/akiojin/gwt/compare/v2.4.3...v2.5.0) (2025-12-11)


### Features

* ワークツリー選択後に lockfile 検出でパッケージインストールを自動実行 ([#182](https://github.com/akiojin/gwt/issues/182)) ([14f4f72](https://github.com/akiojin/gwt/commit/14f4f728f8ac49a2e9e58e2b1d17a0fdef97a8d9))

## [2.4.3](https://github.com/akiojin/gwt/compare/v2.4.2...v2.4.3) (2025-12-11)


### Bug Fixes

* ワークツリー修復時のパス解決とメタデータクリーンアップを修正 ([#176](https://github.com/akiojin/gwt/issues/176)) ([5b5aff3](https://github.com/akiojin/gwt/commit/5b5aff3a09a51a97e7ee5b9cde72ba2b3fe1cd40))

## [2.4.2](https://github.com/akiojin/gwt/compare/v2.4.1...v2.4.2) (2025-12-10)


### Bug Fixes

* ワークツリー不整合検出ロジックを修正し孤立メタデータ処理を追加 ([#172](https://github.com/akiojin/gwt/issues/172)) ([9fbfaa3](https://github.com/akiojin/gwt/commit/9fbfaa31e1a5b399ca2a2f0acd55a3b3a4af06e1))

## [2.4.1](https://github.com/akiojin/gwt/compare/v2.4.0...v2.4.1) (2025-12-10)


### Bug Fixes

* worktreeが見つからない場合にrepairWorktreeで新規作成するよう修正 ([#169](https://github.com/akiojin/gwt/issues/169)) ([dc36d6c](https://github.com/akiojin/gwt/commit/dc36d6ce437e6ae34ea27f30a38b5f31fdaef3f4))

## [2.4.0](https://github.com/akiojin/gwt/compare/v2.3.0...v2.4.0) (2025-12-10)


### Features

* ワークツリー不整合検出時のユーザー通知と自動修復フロー ([#166](https://github.com/akiojin/gwt/issues/166)) ([60ad0aa](https://github.com/akiojin/gwt/commit/60ad0aa2abca3a5b45f6cf4bf2d60e8e469e98e4))

## [2.3.0](https://github.com/akiojin/gwt/compare/v2.2.0...v2.3.0) (2025-12-09)


### Features

* git fetch時に全リモートを更新し、プルのブランチ分岐検出を改善 ([#163](https://github.com/akiojin/gwt/issues/163)) ([e81a79f](https://github.com/akiojin/gwt/commit/e81a79ffeee96bc8c8d19ec0c3d457f8b5105efb))

## [2.2.0](https://github.com/akiojin/gwt/compare/v2.1.3...v2.2.0) (2025-12-09)


### Features

* ワークツリー作成時にプログレスインジケータを追加 ([#160](https://github.com/akiojin/gwt/issues/160)) ([86bcedc](https://github.com/akiojin/gwt/commit/86bcedc9ab4f51b2d21b6d89ffdc8e25df18cf76))

## [2.1.3](https://github.com/akiojin/gwt/compare/v2.1.2...v2.1.3) (2025-12-08)


### Bug Fixes

* 新規ブランチ作成時のworktree生成フローを修正 ([#157](https://github.com/akiojin/gwt/issues/157)) ([b03ffbc](https://github.com/akiojin/gwt/commit/b03ffbc5fae6a7b3ec4c99a2beeb6e5ae35f8d2d))

## [2.1.2](https://github.com/akiojin/gwt/compare/v2.1.1...v2.1.2) (2025-12-08)


### Bug Fixes

* ベースブランチ選択UIの表示を修正しテストを追加 ([#154](https://github.com/akiojin/gwt/issues/154)) ([3a16d5d](https://github.com/akiojin/gwt/commit/3a16d5d5e4fc6d0c92dd74dc4a24f4dbfc53f2f9))

## [2.1.1](https://github.com/akiojin/gwt/compare/v2.1.0...v2.1.1) (2025-12-07)


### Bug Fixes

* Codex CLIをbypassApprovals対応に更新 ([#151](https://github.com/akiojin/gwt/issues/151)) ([c7b2bd2](https://github.com/akiojin/gwt/commit/c7b2bd231efe1c97ae8a1ae3fdd9de96197e8c2d))

## [2.1.0](https://github.com/akiojin/gwt/compare/v2.0.1...v2.1.0) (2025-12-07)


### Features

* AIツール選択UIを多階層ウィザード形式に変更 ([#148](https://github.com/akiojin/gwt/issues/148)) ([5f75a1a](https://github.com/akiojin/gwt/commit/5f75a1ae85e24b8b0ed0a731bcd3bfbde94953d7))

## [2.0.1](https://github.com/akiojin/gwt/compare/v2.0.0...v2.0.1) (2025-12-07)


### Bug Fixes

* Codex CLI非対話モードでセッション再開引数を生成するよう修正 ([#145](https://github.com/akiojin/gwt/issues/145)) ([a4dbe1c](https://github.com/akiojin/gwt/commit/a4dbe1ca6f73a9f8cb58e1f9ddf4c40fded7fbc7))

## [2.0.0](https://github.com/akiojin/gwt/compare/v1.11.0...v2.0.0) (2025-12-05)


### ⚠ BREAKING CHANGES

* Ink/React TUI への移行により、一部の Ctrl-C/Q 終了挙動や画面描画タイミングが変更されます。

### Features

* Ink/React TUI へ移行しコンポーネント化 ([#140](https://github.com/akiojin/gwt/issues/140)) ([0e4a2e3](https://github.com/akiojin/gwt/commit/0e4a2e36cbc6fbba6ac25d96ea27db3e6c54bad7))

## [1.11.0](https://github.com/akiojin/gwt/compare/v1.10.0...v1.11.0) (2025-12-05)


### Features

* Codex CLI 対話モードを追加 ([#137](https://github.com/akiojin/gwt/issues/137)) ([1e59bf0](https://github.com/akiojin/gwt/commit/1e59bf019b59bec89a51b4a5a94b15b1f0fb5d27))

## [1.10.0](https://github.com/akiojin/gwt/compare/v1.9.2...v1.10.0) (2025-12-04)


### Features

* Codex CLI セッション再開機能を追加 ([#132](https://github.com/akiojin/gwt/issues/132)) ([a059a2c](https://github.com/akiojin/gwt/commit/a059a2c39b2aad3f2da4e3acb2f92c8faa3e2c1a))

## [1.9.2](https://github.com/akiojin/gwt/compare/v1.9.1...v1.9.2) (2025-12-04)


### Bug Fixes

* シグナルハンドラとフォアグラウンドプロセスの終了処理を修正 ([#126](https://github.com/akiojin/gwt/issues/126)) ([44c6557](https://github.com/akiojin/gwt/commit/44c6557e1ef7aea2ea95f6f8ee89f6ae95f1fbf7))

## [1.9.1](https://github.com/akiojin/gwt/compare/v1.9.0...v1.9.1) (2025-12-04)


### Bug Fixes

* プロファイル切替時の Claude Code 再起動処理を修正 ([#123](https://github.com/akiojin/gwt/issues/123)) ([e3ac5ce](https://github.com/akiojin/gwt/commit/e3ac5cef6eb3f3467e43cfb4abfaad3b5fbf9d60))

## [1.9.0](https://github.com/akiojin/gwt/compare/v1.8.0...v1.9.0) (2025-12-03)


### Features

* プロファイル管理UIを追加 ([#120](https://github.com/akiojin/gwt/issues/120)) ([8b25f0f](https://github.com/akiojin/gwt/commit/8b25f0fe1eafdbee02bd8bcfd4f70ded4f8d9a6a))

## [1.8.0](https://github.com/akiojin/gwt/compare/v1.7.2...v1.8.0) (2025-12-03)


### Features

* Codex CLI に自動 bypass approvals を追加 ([#115](https://github.com/akiojin/gwt/issues/115)) ([3209ce7](https://github.com/akiojin/gwt/commit/3209ce7ebb3b2953a5f3bde6be5db6f8e79e9fa0))

## [1.7.2](https://github.com/akiojin/gwt/compare/v1.7.1...v1.7.2) (2025-12-02)


### Bug Fixes

* MCP設定のビルトインサーバー認識と空配列処理を修正 ([#112](https://github.com/akiojin/gwt/issues/112)) ([2c32f8e](https://github.com/akiojin/gwt/commit/2c32f8ed3e9b3f9d1f5f05faed4e01e7a00a7dbe))

## [1.7.1](https://github.com/akiojin/gwt/compare/v1.7.0...v1.7.1) (2025-12-02)


### Bug Fixes

* profiles.yaml空時とMCP設定継承のバグを修正 ([#109](https://github.com/akiojin/gwt/issues/109)) ([61b60a4](https://github.com/akiojin/gwt/commit/61b60a4dcb23e2412e2dab94df54c2fbbf2b83a3))

## [1.7.0](https://github.com/akiojin/gwt/compare/v1.6.0...v1.7.0) (2025-12-02)


### Features

* MCP設定管理機能を追加 ([#106](https://github.com/akiojin/gwt/issues/106)) ([10e1a94](https://github.com/akiojin/gwt/commit/10e1a94cd8cee76e1d8fe19fa4a9f40424b2f5f6))

## [1.6.0](https://github.com/akiojin/gwt/compare/v1.5.0...v1.6.0) (2025-12-01)


### Features

* プロファイル起動機能を追加 ([#103](https://github.com/akiojin/gwt/issues/103)) ([e81ab25](https://github.com/akiojin/gwt/commit/e81ab25e7f34fc0f90a03e18d1e393d3a8ba5c2a))

## [1.5.0](https://github.com/akiojin/gwt/compare/v1.4.0...v1.5.0) (2025-12-01)


### Features

* 環境変数スナップショット管理機能を追加 ([#99](https://github.com/akiojin/gwt/issues/99)) ([f464e86](https://github.com/akiojin/gwt/commit/f464e863b93ecdc9cbf70d6e3b53af59e3f68f6b))

## [1.4.0](https://github.com/akiojin/gwt/compare/v1.3.2...v1.4.0) (2025-11-30)


### Features

* Codex CLI対応を追加 ([#92](https://github.com/akiojin/gwt/issues/92)) ([2c2bb1e](https://github.com/akiojin/gwt/commit/2c2bb1e3f4a374aa397ad1af41bf4b6e0e600f57))

## [1.3.2](https://github.com/akiojin/gwt/compare/v1.3.1...v1.3.2) (2025-11-29)


### Bug Fixes

* Claude Codeセッション取得の競合と冗長待機を修正 ([#88](https://github.com/akiojin/gwt/issues/88)) ([7fe85f8](https://github.com/akiojin/gwt/commit/7fe85f8ef33f2dfa6a69f41f0b429c8853fc4d0b))

## [1.3.1](https://github.com/akiojin/gwt/compare/v1.3.0...v1.3.1) (2025-11-28)


### Bug Fixes

* 入力ブロック時の文字残留とセッションIDコールバック競合を修正 ([#84](https://github.com/akiojin/gwt/issues/84)) ([3b0fce5](https://github.com/akiojin/gwt/commit/3b0fce5f1d20fd69f7b7a7ddafa2f422dc79ccee))

## [1.3.0](https://github.com/akiojin/gwt/compare/v1.2.2...v1.3.0) (2025-11-28)


### Features

* クイックスタート機能を追加 ([#82](https://github.com/akiojin/gwt/issues/82)) ([f00c89e](https://github.com/akiojin/gwt/commit/f00c89e8e58b72515a20e3c0f2e22b16e0a78e5f))

## [1.2.2](https://github.com/akiojin/gwt/compare/v1.2.1...v1.2.2) (2025-11-27)


### Bug Fixes

* セッション再開時のセッションID更新処理を修正 ([#79](https://github.com/akiojin/gwt/issues/79)) ([44f1847](https://github.com/akiojin/gwt/commit/44f18478ed3c4cde91dd5f5e45adf49bc55f4f4e))

## [1.2.1](https://github.com/akiojin/gwt/compare/v1.2.0...v1.2.1) (2025-11-26)


### Bug Fixes

* セッションID取得とメニュー表示のバグを修正 ([#76](https://github.com/akiojin/gwt/issues/76)) ([1d7ee19](https://github.com/akiojin/gwt/commit/1d7ee1956f5cb91de4f9ff01e9f5013dcb52dc00))

## [1.2.0](https://github.com/akiojin/gwt/compare/v1.1.0...v1.2.0) (2025-11-26)


### Features

* セッション再開機能を追加 ([#73](https://github.com/akiojin/gwt/issues/73)) ([2cac3d6](https://github.com/akiojin/gwt/commit/2cac3d6e039aff059ffbed90da4a3fd15de6fa32))

## [1.1.0](https://github.com/akiojin/gwt/compare/v1.0.1...v1.1.0) (2025-11-25)


### Features

* protectedブランチ用ワークツリー無効化とリポジトリルートでの作業サポート ([#70](https://github.com/akiojin/gwt/issues/70)) ([d3ece5a](https://github.com/akiojin/gwt/commit/d3ece5ab3d58d0cd6e2ebed0e1f0b39fa7aae8dd))

## [1.0.1](https://github.com/akiojin/gwt/compare/v1.0.0...v1.0.1) (2025-11-24)


### Bug Fixes

* リモートブランチからローカルブランチ作成時にworktreeのブランチを切り替え ([#66](https://github.com/akiojin/gwt/issues/66)) ([7f84e16](https://github.com/akiojin/gwt/commit/7f84e168c07c6dbe9e75da8f9aceb3bf59b94eca))

## [1.0.0](https://github.com/akiojin/gwt/compare/v0.4.0...v1.0.0) (2025-11-24)


### ⚠ BREAKING CHANGES

* リモートブランチから自動ブランチ作成と統合フローを導入

### Features

* リモートブランチ選択からのローカルブランチ自動作成機能 ([#63](https://github.com/akiojin/gwt/issues/63)) ([0d6a1fd](https://github.com/akiojin/gwt/commit/0d6a1fd95ae36a73f1ef93a49dbceae30bf3b8f7))

## [0.4.0](https://github.com/akiojin/gwt/compare/v0.3.0...v0.4.0) (2025-11-23)


### Features

* 新規ブランチ作成UIとベースブランチ選択機能を追加 ([#60](https://github.com/akiojin/gwt/issues/60)) ([9e1ed73](https://github.com/akiojin/gwt/commit/9e1ed7347dd8249ff6e259b3d6abd94a5b84dc28))

## [0.3.0](https://github.com/akiojin/gwt/compare/v0.2.1...v0.3.0) (2025-11-23)


### Features

* ブランチ一覧から新規ブランチ名を入力して作成する機能を追加 ([#57](https://github.com/akiojin/gwt/issues/57)) ([e1a2e2b](https://github.com/akiojin/gwt/commit/e1a2e2b0b5cbb53f37e0e91f1ed7a1b34f6e08f4))

## [0.2.1](https://github.com/akiojin/gwt/compare/v0.2.0...v0.2.1) (2025-11-22)


### Bug Fixes

* package.json の files フィールドに bin/ を追加 ([#53](https://github.com/akiojin/gwt/issues/53)) ([4f88b6f](https://github.com/akiojin/gwt/commit/4f88b6f1fa00fc5bfb87d87e8ea455f5d5a65c96))

## [0.2.0](https://github.com/akiojin/gwt/compare/v0.1.3...v0.2.0) (2025-11-22)


### Features

* AIツール選択メニュー追加とリモートブランチ表示改善 ([#51](https://github.com/akiojin/gwt/issues/51)) ([7f92bc3](https://github.com/akiojin/gwt/commit/7f92bc33bda3f1d439f6e4f88e42e24b5ff41e73))

## [0.1.3](https://github.com/akiojin/gwt/compare/v0.1.2...v0.1.3) (2025-11-21)


### Bug Fixes

* ワークツリー準備中のスピナー表示とカーソル制御を改善 ([#48](https://github.com/akiojin/gwt/issues/48)) ([1fe3af5](https://github.com/akiojin/gwt/commit/1fe3af5feb02fc0f0488df5b61f1efcb4ee78aa7))

## [0.1.2](https://github.com/akiojin/gwt/compare/v0.1.1...v0.1.2) (2025-11-21)


### Bug Fixes

* 終了処理のシグナルハンドリングを統一し、クリーンアップを保証 ([#46](https://github.com/akiojin/gwt/issues/46)) ([7d33a9f](https://github.com/akiojin/gwt/commit/7d33a9f1a4b2c1fa5d8cd6a1d1b2b1b0dc98c91c))

## [0.1.1](https://github.com/akiojin/gwt/compare/v0.1.0...v0.1.1) (2025-11-20)


### Bug Fixes

* CI/CDパイプライン修正とテストカバレッジ改善 ([#38](https://github.com/akiojin/gwt/issues/38)) ([f3cdf7a](https://github.com/akiojin/gwt/commit/f3cdf7a5f84e3fd7b3b2fea9f8c01b3f5f3bfdcf))
* ルートディレクトリでのworktree一覧取得を修正 ([#41](https://github.com/akiojin/gwt/issues/41)) ([d1df9d7](https://github.com/akiojin/gwt/commit/d1df9d70d9e7bb9c9ec8b9c9d3d3b2d3cfbd1e1f))

## 0.1.0 (2025-11-18)


### Features

* 初回リリース - Git worktree管理CLIツール ([e2d7d5c](https://github.com/akiojin/gwt/commit/e2d7d5c1d1b1c1e1f1a1b1c1d1e1f1a1b1c1d1e1))
