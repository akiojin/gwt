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

## [4.9.1](https://github.com/akiojin/gwt/compare/v4.9.0...v4.9.1) (2026-01-04)


### Bug Fixes

* package.json の description を Coding Agent 対応に修正 ([#471](https://github.com/akiojin/gwt/issues/471)) ([f7de165](https://github.com/akiojin/gwt/commit/f7de165c41ed609b5eb6e2b7e1f01f1df415346f))

## [4.9.0](https://github.com/akiojin/gwt/compare/v4.8.0...v4.9.0) (2025-12-29)


### Features

* AIツールのインストール済み表示をバージョン番号に変更 ([#461](https://github.com/akiojin/gwt/issues/461)) ([2610ef2](https://github.com/akiojin/gwt/commit/2610ef2a8a1b64b8e98b4cde6df1b5afdabcef55))


### Bug Fixes

* claude-worktree後方互換コードを削除 ([#462](https://github.com/akiojin/gwt/issues/462)) ([c8e5fbf](https://github.com/akiojin/gwt/commit/c8e5fbf06f5c18480f2af3f8e78469214282ef28))

## [4.8.0](https://github.com/akiojin/gwt/compare/v4.7.0...v4.8.0) (2025-12-29)


### Features

* Docker構成を最適化しPlaywright noVNCサービスを追加 ([#454](https://github.com/akiojin/gwt/issues/454)) ([6dd0843](https://github.com/akiojin/gwt/commit/6dd08435f9f49061f156d01f5e7ce0a25e15307f))
* Docker構成を最適化しPlaywright noVNCサービスを追加 ([#455](https://github.com/akiojin/gwt/issues/455)) ([e62fef5](https://github.com/akiojin/gwt/commit/e62fef5bc199a26ea0d6f8f36527612ff461b00f))
* ブランチ一覧に最終アクティビティ時間を表示 ([#456](https://github.com/akiojin/gwt/issues/456)) ([7cfab79](https://github.com/akiojin/gwt/commit/7cfab79ebc64393fb1e6666191c57d8baf2c5686))


### Bug Fixes

* execaのshell: trueオプションを削除してbunx起動エラーを修正 ([#458](https://github.com/akiojin/gwt/issues/458)) ([6de849f](https://github.com/akiojin/gwt/commit/6de849f70086dedcb8b79771590985cb6989833a))
* warn then return after dirty worktree ([#453](https://github.com/akiojin/gwt/issues/453)) ([c9cead9](https://github.com/akiojin/gwt/commit/c9cead9841f6522fbe8cf43258eceb1183289c79))
* 自動インストール警告文のタイポ修正 ([#451](https://github.com/akiojin/gwt/issues/451)) ([15917e3](https://github.com/akiojin/gwt/commit/15917e3d28a7ef950278a73dfaa5c298dfd4633f))

## [4.7.0](https://github.com/akiojin/gwt/compare/v4.6.1...v4.7.0) (2025-12-26)


### Features

* ログビューアを追加 ([#442](https://github.com/akiojin/gwt/issues/442)) ([92128c3](https://github.com/akiojin/gwt/commit/92128c37f7453d302025b5d023984543d023adf4))
* ログ表示の通知と選択UIを改善 ([#443](https://github.com/akiojin/gwt/issues/443)) ([cf3d7b3](https://github.com/akiojin/gwt/commit/cf3d7b31e84cdbc762fc6c629b5c670744678cb5))
* 未コミット警告時にEnterキー待機を追加 ([#441](https://github.com/akiojin/gwt/issues/441)) ([d03dac5](https://github.com/akiojin/gwt/commit/d03dac52c12a5cb6ef99b973d7348f4f3ccdeeb6))


### Bug Fixes

* **cli:** AIツール実行時にフルパスを使用 ([#439](https://github.com/akiojin/gwt/issues/439)) ([2fe73e2](https://github.com/akiojin/gwt/commit/2fe73e27ea7f9148415fb67b3bfbe0dce9ac48bb))
* worktree作成時のstale残骸を自動回復 ([#445](https://github.com/akiojin/gwt/issues/445)) ([ce971a6](https://github.com/akiojin/gwt/commit/ce971a6e4cd85db5c8c5295d9a1a6502898e9012))


### Performance Improvements

* ブランチ一覧のgit状態取得をキャッシュ化 ([#446](https://github.com/akiojin/gwt/issues/446)) ([76f30d6](https://github.com/akiojin/gwt/commit/76f30d64f7a5674eea07843c0aafb385f37e8282))

## [4.6.1](https://github.com/akiojin/gwt/compare/v4.6.0...v4.6.1) (2025-12-25)


### Bug Fixes

* **cli:** AIツール実行時にフルパスを使用して非インタラクティブシェルのPATH問題を修正 ([#436](https://github.com/akiojin/gwt/issues/436)) ([ba78cd5](https://github.com/akiojin/gwt/commit/ba78cd52cc95193f894e0aa8635767395567d56b))

## [4.6.0](https://github.com/akiojin/gwt/compare/v4.5.1...v4.6.0) (2025-12-25)


### Features

* Claude Codeプラグイン設定を追加 ([#429](https://github.com/akiojin/gwt/issues/429)) ([06d04db](https://github.com/akiojin/gwt/commit/06d04dbf28e7fa6a65a7349388c4dc48efbb6ae7))
* **cli:** AIツールのインストール状態検出とステータス表示を追加 ([#431](https://github.com/akiojin/gwt/issues/431)) ([79a6995](https://github.com/akiojin/gwt/commit/79a6995349b1dfb966e496c3e50644fcb58c99f6))


### Bug Fixes

* ブランチリスト画面のフリッカーを解消 ([#433](https://github.com/akiojin/gwt/issues/433)) ([3331c5d](https://github.com/akiojin/gwt/commit/3331c5d5f28847a95399a20b23fee97685db858d))
* リモートモードでローカル・リモート両存在ブランチが表示されない問題を修正 ([#430](https://github.com/akiojin/gwt/issues/430)) ([70a5876](https://github.com/akiojin/gwt/commit/70a5876796273c39dafae5223994eccfe826cda7))

## [4.5.1](https://github.com/akiojin/gwt/compare/v4.5.0...v4.5.1) (2025-12-24)


### Bug Fixes

* **ci:** マージ方法をsquashに変更してCHANGELOG重複を防止 ([#425](https://github.com/akiojin/gwt/issues/425)) ([ee6338c](https://github.com/akiojin/gwt/commit/ee6338c726002326707baa8b15b5f70703395502))
* Claude Codeのフォールバックをbunxに統一 ([ac740d9](https://github.com/akiojin/gwt/commit/ac740d9c7bc3ef97a5d3b0903908ecc672c637ce))

## [4.5.0](https://github.com/akiojin/gwt/compare/v4.4.1...v4.5.0) (2025-12-24)


### Features

* requirements-spec-kit スキルを追加 ([01a6644](https://github.com/akiojin/gwt/commit/01a6644ad3ba5e73bdf17f79c278ec6cf311b637))


### Bug Fixes

* gitデータ取得のタイムアウトを延長 ([27b88a0](https://github.com/akiojin/gwt/commit/27b88a07b2785d26e561066147140f5d2da7541d))
* gitデータ取得のタイムアウトを延長 ([5e5bdfe](https://github.com/akiojin/gwt/commit/5e5bdfe980c4636611005dc2858c94e84007c246))

## [4.4.1](https://github.com/akiojin/gwt/compare/v4.4.0...v4.4.1) (2025-12-23)


### Bug Fixes

* ブランチ一覧取得時にrepoRootを使用するよう修正 ([71edc72](https://github.com/akiojin/gwt/commit/71edc7235d93f4f8172ddb38fe2dbf82201cc46f))
* ブランチ一覧取得時にrepoRootを使用するよう修正 ([3d4b370](https://github.com/akiojin/gwt/commit/3d4b37049ef189d6512fabcb0440f11baf00fb24))

## [4.4.0](https://github.com/akiojin/gwt/compare/v4.3.1...v4.4.0) (2025-12-23)


### Features

* ブランチ一覧画面の改善（表示モード切替・スピナー局所化） ([46e42ae](https://github.com/akiojin/gwt/commit/46e42aecb544964fa156c84043b19243a16ee91f))
* ブランチ表示モード切替機能（TABキー）を追加 ([50a1d39](https://github.com/akiojin/gwt/commit/50a1d393024a8e8e7ec3c4bcd98502c15408cabe))


### Bug Fixes

* Git情報取得のタイムアウトを追加 ([bcccdbf](https://github.com/akiojin/gwt/commit/bcccdbf701165d773f8f64032002d02050515507))
* Mode表示を Stats 行の先頭に移動 ([d18ad5c](https://github.com/akiojin/gwt/commit/d18ad5cf2549128cf5faaf6f48f48e2a58ac2f7e))
* WSLの矢印キー誤認を防止 ([cbad8f6](https://github.com/akiojin/gwt/commit/cbad8f600d0bba5c751f1062f54136aaf7235aef))
* リモート取得遅延でもブランチ一覧を表示 ([5cea85a](https://github.com/akiojin/gwt/commit/5cea85a358399aa44633b58d54ef282ef4718b25))
* 相対パス起動のエントリ判定を安定化 ([618093e](https://github.com/akiojin/gwt/commit/618093e61738dbf2799c16b103c015fb27d69913))

## [4.3.1](https://github.com/akiojin/gwt/compare/v4.3.0...v4.3.1) (2025-12-22)


### Bug Fixes

* WSL1検出でChrome統合を無効化する ([085688c](https://github.com/akiojin/gwt/commit/085688cbd1572f0ae79bf2408ca281f74e003f75))
* 未対応環境ではClaude CodeのChrome統合をスキップする ([f76cc0c](https://github.com/akiojin/gwt/commit/f76cc0c8f8ea36c8b270a3132ab63bcdfdfd77ab))
* 未対応環境ではClaude CodeのChrome統合をスキップする ([f744901](https://github.com/akiojin/gwt/commit/f744901062e5c02e35c27947a4beace2359ecaaf))

## [4.3.0](https://github.com/akiojin/gwt/compare/v4.2.0...v4.3.0) (2025-12-21)


### Features

* Claude CodeのTypeScript LSP対応を追加 ([cf2983b](https://github.com/akiojin/gwt/commit/cf2983b1f58cb670dba2e47b645cc8af79d760a0))
* Claude Code起動時にChrome拡張機能統合を有効化 ([b3a5d6d](https://github.com/akiojin/gwt/commit/b3a5d6d7adb85d82577f888c147b3af593bf9160))
* Claude Code起動時にChrome拡張機能統合を有効化 ([f449845](https://github.com/akiojin/gwt/commit/f4498451d1deacbe811126cc34d35e2b73334eb2))
* macOS対応のシステムトレイを実装 ([b2cdfbe](https://github.com/akiojin/gwt/commit/b2cdfbe6dffe391becdbb3799019616e1e45ed74))
* Web UIサーバー全体にログ出力を追加 ([09909f2](https://github.com/akiojin/gwt/commit/09909f2c4b4f7c3a0130ade2536ed5bef55d7512))
* Web UI機能の強化とブランチグラフのリファクタリング ([146f596](https://github.com/akiojin/gwt/commit/146f59609d207b072bfbf10e5150a37f75a4cd77))
* ブランチグラフをReact Flowベースにリファクタリング ([f0deb4a](https://github.com/akiojin/gwt/commit/f0deb4a37c8dc0d31ede97491d34719dd7297999))


### Bug Fixes

* ENABLE_LSP_TOOL環境変数の値を"1"から"true"に修正 ([9bef1eb](https://github.com/akiojin/gwt/commit/9bef1eb809afe3d42cdd190a8ba7b21a71d1a778))
* node-ptyで使用するコマンドのフルパスを解決 ([7d5ab76](https://github.com/akiojin/gwt/commit/7d5ab76d885b0d6363d7dca821312fedf2e746fa))
* SPAルーティング用のフォールバック処理を追加 ([a4a0404](https://github.com/akiojin/gwt/commit/a4a0404a439dc07e70ff646f1ca16d8f05d35fef))
* type-checkでcleanup対象の型エラーを解消 ([a617a2d](https://github.com/akiojin/gwt/commit/a617a2d53575f55985d9d7aa3d3e2fc8717e91fc))
* Web UIからClaude Code起動時にENABLE_LSP_TOOL環境変数を渡す ([49fea84](https://github.com/akiojin/gwt/commit/49fea847f95e3d997313459389ee4a33aa42dade))
* Web UIからClaude Code起動時にENABLE_LSP_TOOL環境変数を渡す ([84fedf3](https://github.com/akiojin/gwt/commit/84fedf3918c30b85cae6bb6cdac8f0488ea079ff))
* Web UIのデフォルトポートを3001に変更 ([597fff3](https://github.com/akiojin/gwt/commit/597fff3310b294d67ec140542d72504ee17a966b))
* WebSocket接続エラーの即時表示を抑制 ([c0ac929](https://github.com/akiojin/gwt/commit/c0ac929a052ceb3700feee1a9964cb85dfd9c052))
* クリーンアップ選択の安全判定を要件どおりに更新 ([6c0a595](https://github.com/akiojin/gwt/commit/6c0a5957a866c4f9475f9dd30cb965331a99bfa8))
* クリーンアップ選択の安全判定を要件どおりに更新 ([23c89c7](https://github.com/akiojin/gwt/commit/23c89c7ee2a5ef843c6d796e013e80c85607e857))

## [4.2.0](https://github.com/akiojin/gwt/compare/v4.1.1...v4.2.0) (2025-12-20)


### Features

* add post-session git prompts ([bc24450](https://github.com/akiojin/gwt/commit/bc244506559fc0676f6740fa3edf7912c0f02af7))
* add post-session push prompt ([89b2839](https://github.com/akiojin/gwt/commit/89b2839fe22177390c98f97bc10063f0602877c0))

## [4.1.1](https://github.com/akiojin/gwt/compare/v4.1.0...v4.1.1) (2025-12-19)


### Bug Fixes

* normalizeModelIdの空文字処理とテスト補強 ([b82230b](https://github.com/akiojin/gwt/commit/b82230b6589e689955cd872fc648655436e7f218))
* Worktree再利用の整合性検証とモデル名正規化 ([69f5fb6](https://github.com/akiojin/gwt/commit/69f5fb6f7419dee017f8ed2946bc0154856ca743))
* Worktree再利用の整合性検証とモデル名正規化 ([255e551](https://github.com/akiojin/gwt/commit/255e551d915ca3af705ce1f64e8d16983001056a))

## [4.1.0](https://github.com/akiojin/gwt/compare/v4.0.1...v4.1.0) (2025-12-19)


### Features

* Codexモデル一覧を4件に整理 ([7a0bac9](https://github.com/akiojin/gwt/commit/7a0bac9459ed22826c95d778924731e8eab207f7))
* gpt-5.2-codex対応 ([14c2cb4](https://github.com/akiojin/gwt/commit/14c2cb4df6275628787e173c7275362bb11c6101))
* gpt-5.2-codex対応 ([865f7c8](https://github.com/akiojin/gwt/commit/865f7c87bcce729ddfa383a1048bc632c8d45486))

## [4.0.1](https://github.com/akiojin/gwt/compare/v4.0.0...v4.0.1) (2025-12-18)


### Bug Fixes

* WSL2とWindowsで矢印キー入力を安定化 ([94539f1](https://github.com/akiojin/gwt/commit/94539f17b1449b49fc93af0e775667e4d1d399c6))
* WSL2とWindowsで矢印キー入力を安定化 ([d96ba58](https://github.com/akiojin/gwt/commit/d96ba58b26f7afc283fd91c952876ff7130175bb))
* デフォルトモデルオプション追加に伴うテスト期待値を修正 ([31a1728](https://github.com/akiojin/gwt/commit/31a17284ba82c27f1dd8aebf6d8bc8b43eca7d5f))

## [4.0.0](https://github.com/akiojin/gwt/compare/v3.1.2...v4.0.0) (2025-12-18)


### ⚠ BREAKING CHANGES

* Qwen CLI (qwen-cli) は起動/選択できません。

### Features

* gemini-3-flash モデルのサポートを追加 ([1561e7f](https://github.com/akiojin/gwt/commit/1561e7fb5b141d350ace7409bad2bfc952c662db))
* gemini-3-flash モデルのサポートを追加 ([161ef2f](https://github.com/akiojin/gwt/commit/161ef2f3444b98e56289f8067e0b1938e84a0864))
* Qwen CLIを未サポート化 ([c007c00](https://github.com/akiojin/gwt/commit/c007c004b854a2552cac7a2f0e2a84266ee59f1d))
* 全てのツールにデフォルト（自動選択）オプションを追加し、Geminiのモデル選択肢を改善 ([5fb143d](https://github.com/akiojin/gwt/commit/5fb143d9b4b18872ea4b83f9e55f366736412951))
* 全てのツールにデフォルトオプションを追加し、Geminiのモデル選択肢を改善 ([ff9554d](https://github.com/akiojin/gwt/commit/ff9554deb34b647a525649e341629872dcae20f5))


### Bug Fixes

* Gemini CLI起動時のTTY描画を維持する ([a27b4ce](https://github.com/akiojin/gwt/commit/a27b4ce679edf5b24f0092ad81d9a8aef5dfde2e))
* Gemini CLI起動時のTTY描画を維持する ([81403da](https://github.com/akiojin/gwt/commit/81403dacda28f87c34eeb8517cf8946299daf76c))
* gemini-3-flash のモデル ID を gemini-3-flash-preview に修正 ([bf17b3a](https://github.com/akiojin/gwt/commit/bf17b3a130d05cdc50484f2de1eb73560c1733e0))
* gemini-3-flash のモデル ID を gemini-3-flash-preview に修正 ([4a5d934](https://github.com/akiojin/gwt/commit/4a5d934b0daef02f2dee9bb34fb7b26a5f783c92))
* Geminiのモデル選択肢を修正（Default追加＋マニュアルリスト復元） ([ebb2c65](https://github.com/akiojin/gwt/commit/ebb2c65645d14e30b67250fd6446994678ef96bd))

## [3.1.2](https://github.com/akiojin/gwt/compare/v3.1.1...v3.1.2) (2025-12-16)


### Bug Fixes

* CodeRabbitレビュー最終修正 ([f39d0a6](https://github.com/akiojin/gwt/commit/f39d0a62b455015296027be2100c55635bd90cea))
* CodeRabbit指摘事項を修正 ([1eebd46](https://github.com/akiojin/gwt/commit/1eebd46a5b8ccafd7b4f7c4849f100a87904f577))
* CodeRabbit追加指摘事項を修正 ([02a137c](https://github.com/akiojin/gwt/commit/02a137c32ce38084563607e1583efc844138dc84))
* matchesCwdにクロスプラットフォームパス正規化を追加 ([93afdcc](https://github.com/akiojin/gwt/commit/93afdcc801e7e6f2401aa6722dab93e56459b507))
* パスプレフィックスマッチングに境界チェックを追加 ([623bc3d](https://github.com/akiojin/gwt/commit/623bc3df12f4923a889b9572395067956b6e5b4c))

## [3.1.1](https://github.com/akiojin/gwt/compare/v3.1.0...v3.1.1) (2025-12-16)


### Bug Fixes

* アクセス不可Worktreeを🔴表示に変更 ([9a4ef35](https://github.com/akiojin/gwt/commit/9a4ef3566a38063a20cb2cf32e4d495d85edb177))
* アクセス不可Worktreeを🔴表示に変更 ([9bbc419](https://github.com/akiojin/gwt/commit/9bbc419e50583ff945e92c4717c75278484ffced))

## [3.1.0](https://github.com/akiojin/gwt/compare/v3.0.0...v3.1.0) (2025-12-16)


### Features

* プロファイル未選択（none）を選択可能にする ([adaaf9c](https://github.com/akiojin/gwt/commit/adaaf9cf2f2d9e5ff3e897a3c39be86693199033))
* プロファイル未選択を選択できるようにする ([d93c3e7](https://github.com/akiojin/gwt/commit/d93c3e7567cbdb3c24d21ee5ff52e3a39b14510e))
* 環境変数プロファイル機能を追加 ([80f3f13](https://github.com/akiojin/gwt/commit/80f3f130815f8bed7128fc99d3831403f51bebaa))
* 環境変数プロファイル機能を追加 ([df01519](https://github.com/akiojin/gwt/commit/df015191a837f413df6235eb00bd23a9fcd0caf0))


### Bug Fixes

* CodeRabbitのレビュー指摘事項を修正 ([45ff1e7](https://github.com/akiojin/gwt/commit/45ff1e75be18b33b9f80fb0f0aafa37261cb3709))
* EnvironmentProfileScreenのキーボード入力を修正 ([fefea29](https://github.com/akiojin/gwt/commit/fefea2958ed44166b7f7840bbd8d2a12b7146cac))
* envキー入力のバリデーションを追加 ([739a951](https://github.com/akiojin/gwt/commit/739a951bd32271e14808da52947bf118280a6913))
* envキー入力バリデーションを調整 ([cdb5bf6](https://github.com/akiojin/gwt/commit/cdb5bf6e44028560ca142ef57e9eff1c8380af92))
* profiles.yaml更新の競合を防止 ([8062807](https://github.com/akiojin/gwt/commit/8062807b812364bb226e3afdd8f78594b1dbb66f))
* profiles.yaml未作成時の作成失敗を修正 ([070853b](https://github.com/akiojin/gwt/commit/070853b917de8e1303abd374c04693f6e61de182))
* Spec Kitスクリプトの安全性改善（eval撤廃/JSON出力） ([434531e](https://github.com/akiojin/gwt/commit/434531ede534c5163e5fcd992cd898d9c8363aba))
* プロファイル保存の一時ファイルとスクロール境界を修正 ([dc53ad2](https://github.com/akiojin/gwt/commit/dc53ad2fa439e7ec34082f8416c49f11359654f4))
* プロファイル名検証と設定パス不整合を修正 ([5c8a422](https://github.com/akiojin/gwt/commit/5c8a422882ed8b17657550618ee1d913ad565578))
* プロファイル変更後にヘッダー表示を更新 ([60883b3](https://github.com/akiojin/gwt/commit/60883b35698ceabc57f70feb2df171b64ba296e1))
* プロファイル画面の入力検証とインデックス境界を修正 ([4354512](https://github.com/akiojin/gwt/commit/43545121cc9c6b2da7bd63304e21b553c087cf6b))

## [3.0.0](https://github.com/akiojin/gwt/compare/v2.14.0...v3.0.0) (2025-12-15)


### ⚠ BREAKING CHANGES

* gwtコマンド起動時にWeb UIサーバーが自動起動しなくなります。 Web UIを使用する場合は `gwt serve` または `npm run start:web` で明示的に起動してください。

### Bug Fixes

* macOS/Linuxでトレイ初期化を無効化してクラッシュを防止 ([d30c320](https://github.com/akiojin/gwt/commit/d30c32098e48858113afdd24a5cbbeb38481ceaf))
* macOS/Linuxでトレイ初期化を無効化してクラッシュを防止 ([e53a0f5](https://github.com/akiojin/gwt/commit/e53a0f568abbb4bd15bd4d8fd97308fc5da9eed4))
* Web UI URL表示削除に伴うテスト修正 ([dbabfcf](https://github.com/akiojin/gwt/commit/dbabfcf92d0c1a7d6be12319f417ac53ba77b3e1))
* トレイ再初期化とテストのplatform注入 ([1b39cf5](https://github.com/akiojin/gwt/commit/1b39cf5491f68ae65d45cb0fba28d0a3566d3243))
* トレイ破棄の二重実行を防止 ([0b7fcd5](https://github.com/akiojin/gwt/commit/0b7fcd5620a76e74c1eef104105a5ca96186bf13))


### Code Refactoring

* CLI起動時のWeb UIサーバー自動起動を廃止 ([32a2a97](https://github.com/akiojin/gwt/commit/32a2a97ab4168fe2d569101076dbb197870620f2))

## [2.14.0](https://github.com/akiojin/gwt/compare/v2.13.0...v2.14.0) (2025-12-13)


### Features

* Web UIトレイ常駐とURL表示 ([bf0d674](https://github.com/akiojin/gwt/commit/bf0d674b4eb49aa5585145fcf19a6dafd6eb2904))
* Web UIトレイ常駐とURL表示 ([51103c3](https://github.com/akiojin/gwt/commit/51103c3250f53c3c4e6d33c64e968f300cb3d39e))
* Web UIをTailwind CSS + shadcn/uiで刷新 ([07d1140](https://github.com/akiojin/gwt/commit/07d114026b74aa39d5748fb5725c9584d89849c7))
* **webui:** CLI起動時にWeb UIサーバーを自動起動 ([519938b](https://github.com/akiojin/gwt/commit/519938baa0ee4f3fd94ff475a14a72b2bfdc2f29))
* **webui:** Tailwind CSS + shadcn/ui基盤を導入 ([b49ede7](https://github.com/akiojin/gwt/commit/b49ede7694c5da0732dcd089af33315ebbb9b09b))
* **webui:** Web UI機能強化とCLI連携 ([982763e](https://github.com/akiojin/gwt/commit/982763e31fdba320c4843a775795efaade35b05a))
* **webui:** 全ページをTailwind + shadcn/uiでリファクタリング ([a5ca94c](https://github.com/akiojin/gwt/commit/a5ca94cf402fdc9e94239f275d153b43ab732a59))
* ポート使用中時のWeb UIサーバー起動スキップ (FR-006) ([d38dc33](https://github.com/akiojin/gwt/commit/d38dc333de868a3dfd0ae111fb075232e72d975a))


### Bug Fixes

* Goodbye後にプロセスが終了しない問題を修正 ([99d6aa1](https://github.com/akiojin/gwt/commit/99d6aa1510e3bf24d38bbd7ae7ed70465868e063))
* Goodbye後にプロセスが終了しない問題を修正 ([a3c4ee1](https://github.com/akiojin/gwt/commit/a3c4ee11df685a72b7536c4600115cdf5d60ade6))
* handle LF enter in Select ([5f7c42f](https://github.com/akiojin/gwt/commit/5f7c42f030e64f74a2d5e9e764290be373309a2e))
* PR [#344](https://github.com/akiojin/gwt/issues/344) CodeRabbitレビュー対応 ([c40cce9](https://github.com/akiojin/gwt/commit/c40cce9a40c763908e26a9d4b476553e12892237))
* Quick Start Enter二度押し問題とテストOOM改善 ([137530a](https://github.com/akiojin/gwt/commit/137530ae796e3af123375847ba52b815304dc12b))
* Quick Start画面の初回表示時にEnterが効かない問題を修正 ([26d8e61](https://github.com/akiojin/gwt/commit/26d8e612eed311c29aa0768c4b2f6824d1199c68))
* Quick Start画面の初回表示時にEnterが効かない問題を修正 ([d5915f7](https://github.com/akiojin/gwt/commit/d5915f7d43d6adb3a23951983925c539061f6d80))
* Resume/ContinueでsessionIdを上書きしない ([f26674e](https://github.com/akiojin/gwt/commit/f26674ef7ad83edde59e789b291a6bd28b075eb5))
* Resume/ContinueでsessionIdを上書きしない ([ec0d682](https://github.com/akiojin/gwt/commit/ec0d682388b1c9bec84b2efd3b411807c033ac62))
* Resumeは各ツールのresume機能に委譲 ([755aaff](https://github.com/akiojin/gwt/commit/755aaff357737bd170a68549be738b34466832f4))
* Resumeは各ツールのresume機能に委譲 ([c8afa5d](https://github.com/akiojin/gwt/commit/c8afa5d4427891cf87b4b0031903e53e40dee4f0))
* **test:** テストモックのAPI形状を修正 ([19c326e](https://github.com/akiojin/gwt/commit/19c326ee88e331f0e486562387b916f3ace806b5))
* Web UIサーバー停止をタイムアウト付きで堅牢化 ([72d366f](https://github.com/akiojin/gwt/commit/72d366fe945d4279e9d5768d3f9892e90d9e43b0))
* Web UIポート解決とトレイ初期化の堅牢化 ([533fbcf](https://github.com/akiojin/gwt/commit/533fbcf5a449d83f74b5582b80abd8ce8e3ddbd2))
* 未使用インポートを削除しESLintエラーを解消 ([7463870](https://github.com/akiojin/gwt/commit/74638704cc7f4f1f0634851d0711db00a68dad64))

## [2.13.0](https://github.com/akiojin/gwt/compare/v2.12.1...v2.13.0) (2025-12-12)


### Features

* Codexにgpt-5.2モデルを追加 ([9c97214](https://github.com/akiojin/gwt/commit/9c97214d365348ecc807e97684ee4233c25c9cfe))
* Codexにgpt-5.2モデルを追加 ([a1ec770](https://github.com/akiojin/gwt/commit/a1ec7704b431a43e15c16e87648e9e831235fd3c))
* Ink.js CLI UIデザインスキル（cli-design）を追加 ([4da6a1c](https://github.com/akiojin/gwt/commit/4da6a1cabe56d683274f0f63e209343b0bf8a1c1))
* Ink.js CLI UIデザインスキル（cli-design）を追加 ([1066498](https://github.com/akiojin/gwt/commit/106649852109478aee692fda4f32d203993a4f16))
* pino構造化ログと7日ローテーションを導入 ([946e42c](https://github.com/akiojin/gwt/commit/946e42c336784cee061af83592b22b08e9d3ace6))
* route logs to ~/.gwt with daily jsonl files ([4a5a5f4](https://github.com/akiojin/gwt/commit/4a5a5f4d8d626c94bc66475c089696a487596229))


### Bug Fixes

* align branch list layout and icon widths ([eee5e00](https://github.com/akiojin/gwt/commit/eee5e00fcaed22b5dd48e12438cf3096fd4604f8))
* divergenceテストにwaitForEnterモックを追加 ([6695f1c](https://github.com/akiojin/gwt/commit/6695f1c9c3d4c5ac860ed037f8c4d786a5f26b37))
* divergenceテストのタイムアウト修正 ([4ba674a](https://github.com/akiojin/gwt/commit/4ba674a3ecf400957a212a5489551a19cbeafb7f))
* Fastify logger型の不整合を修正 ([4396886](https://github.com/akiojin/gwt/commit/439688689e296497077ee3c21976c1ae167d5ef2))
* prompt.jsモックでimportActualを使用 ([8b99f97](https://github.com/akiojin/gwt/commit/8b99f975c7add859a8d9df76668c1cab2a973ee9))
* resolve lint errors on branch list ([30d9b3d](https://github.com/akiojin/gwt/commit/30d9b3ded9a767be5809d6d00e0f6143029352d2))
* share logger date helper and simplify tests ([d73336f](https://github.com/akiojin/gwt/commit/d73336ffc3f57f06cc879955611fbee154648295))

## [2.12.1](https://github.com/akiojin/gwt/compare/v2.12.0...v2.12.1) (2025-12-09)


### Bug Fixes

* ensure divergence prompt waits for input ([248e3eb](https://github.com/akiojin/gwt/commit/248e3ebd7483d18681e02771c411b170cbcd3e2e))
* ensure divergence prompt waits for input ([efca1f5](https://github.com/akiojin/gwt/commit/efca1f5602a6f1264df8b28207c819d8aa4657fe))

## [2.12.0](https://github.com/akiojin/gwt/compare/v2.11.1...v2.12.0) (2025-12-08)


### Features

* add branch quick start reuse last settings ([c0cce6a](https://github.com/akiojin/gwt/commit/c0cce6a8a6ea36a2230980bcc6c4d3cc10fd4433))
* add branch quick start screen ui tests ([e72fd85](https://github.com/akiojin/gwt/commit/e72fd85458570ca1f0f624f9ef3b863290120a01))
* Codex CLIのスキル機能を有効化 ([c406070](https://github.com/akiojin/gwt/commit/c4060703bc3ce30e90d681332f2ba0685a30d8de))
* Codex CLIのスキル機能を有効化 ([abe0fa7](https://github.com/akiojin/gwt/commit/abe0fa749d4b52018853157aff08dfe6f89b1dec))
* fallback resolve continue session id from tool cache ([fe74082](https://github.com/akiojin/gwt/commit/fe7408245d81aa9565f4c492a92d28dbad2203cf))
* persist and surface session ids for continue flow ([fd1053f](https://github.com/akiojin/gwt/commit/fd1053fe41c7d8522b2e598d731b6742bb325970))
* Quick Startをツールカテゴリ別に色分け表示 ([f7464e2](https://github.com/akiojin/gwt/commit/f7464e2ad177ec52b25ecf5a27f29d6a478a2478))
* reuse skip permissions in quick start ([5c75eea](https://github.com/akiojin/gwt/commit/5c75eea4f41ad52826a5a4c1e05a12f54b97eb15))
* skip execution mode when quick-start reusing settings ([ed6bfc3](https://github.com/akiojin/gwt/commit/ed6bfc375a898fc3c16f250b9bd257d9bb927761))
* support gemini session resume ([9cc993d](https://github.com/akiojin/gwt/commit/9cc993df4eb5ba9a1ea1c3be89e64dbca3071929))
* クイックスタートでツール別の直近設定を提示 ([9904992](https://github.com/akiojin/gwt/commit/99049928061105e48fadb103bab423fdb4e7ecf4))
* セッションID再開対応（Codex/Claude/Gemini） ([a2d50ef](https://github.com/akiojin/gwt/commit/a2d50ef9f48e18045b6c5764b38c6f567f6d0d83))
* 全AIツール起動時のパラメーターを表示 ([72d41b4](https://github.com/akiojin/gwt/commit/72d41b4bbf9af82c7ff9a2f90d3d852aa7db8861))
* 全AIツール起動時のパラメーターを表示 ([d3a6827](https://github.com/akiojin/gwt/commit/d3a682790addca7c541a01a37c72efbad75a4b57))


### Bug Fixes

* add execChild helper to handle SIGINT for Codex CLI ([7c3d8a6](https://github.com/akiojin/gwt/commit/7c3d8a693c4b6cffddb2aff61a6413e002d2422c))
* add shell option to Codex execa for proper Ctrl+C handling ([0735646](https://github.com/akiojin/gwt/commit/0735646c02b4e628421358084d8ce45ee4764419))
* add SIGINT/SIGTERM handling to Claude Code launcher ([d34ac76](https://github.com/akiojin/gwt/commit/d34ac76e9ea2bb9ef4883761c4c887a44f7010ea))
* add terminal.exitRawMode() to Codex finally block ([3f058ac](https://github.com/akiojin/gwt/commit/3f058ac6db04c377ec7fb3673eb17606aafee06b))
* always show latest claude session id in quick start ([c8bed86](https://github.com/akiojin/gwt/commit/c8bed868041cdfbed80ce410a299995d477a77a2))
* capture Gemini session ID from exit summary output ([074e071](https://github.com/akiojin/gwt/commit/074e07175be20698fcba6620c8f682d69ca8d247))
* capture session ids and harden quick start filters ([83c1c45](https://github.com/akiojin/gwt/commit/83c1c45461ea2460deb41bcb8f4847b82f421ada))
* Claude CodeでstdoutからsessionIdを確実に捕捉 ([466eeb3](https://github.com/akiojin/gwt/commit/466eeb38853e98b66907ee31abce884b69fc36db))
* Claude/Codexセッションを起動時刻近傍で再解決 ([ad6c5bf](https://github.com/akiojin/gwt/commit/ad6c5bf04d8cdd32ecde24e978b8be597aeaa73c))
* Claude/Geminiのセッション取得を時間帯で厳密化 ([77e1444](https://github.com/akiojin/gwt/commit/77e1444d5add622435b88a4d22359bf6b5d78cc4))
* ClaudeセッションIDを保存時に補完 ([e0a1be1](https://github.com/akiojin/gwt/commit/e0a1be1834cbc49331e265d3d8386b28e2280be6))
* ClaudeセッションIDを起動直後にポーリングして補足 ([81959b4](https://github.com/akiojin/gwt/commit/81959b4c487949fa45130e2b2cd85bf8980ec0e6))
* Claudeセッション検出でdot→dashエンコードを考慮 ([78dac9a](https://github.com/akiojin/gwt/commit/78dac9a630c0bf20eddf359f9c8eba628cb3276a))
* Claudeセッション検出でproject直下のjson/jsonlも探索 ([1c93645](https://github.com/akiojin/gwt/commit/1c93645702062bba005e95f7f79618a0b91be063))
* Claudeセッション検出で最終更新順に有効IDを探索 ([05c674c](https://github.com/akiojin/gwt/commit/05c674c6bcc9e27720cf3f5a616354ead2432098))
* Codex Quick Startで履歴より新しいセッションファイルを優先 ([f228746](https://github.com/akiojin/gwt/commit/f2287468d6a5111f8aca8e4b1156dcd48e950697))
* CodexセッションIDを起動時刻に近いものへ保存 ([0708710](https://github.com/akiojin/gwt/commit/0708710f6264ee1114c695ab6d3e60bc1a96694a))
* CodexセッションIDを起動直後にポーリングして補足 ([226932b](https://github.com/akiojin/gwt/commit/226932b5e911f390678a8e4e8bfe1785de9811a5))
* Codexセッション取得を開始時刻以降の最新ファイルに限定 ([0fbf1b7](https://github.com/akiojin/gwt/commit/0fbf1b7b9cec0044c87342a5e7e401227ec80799))
* CodexのQuick Startで最新セッションIDをファイルから補完 ([15a1665](https://github.com/akiojin/gwt/commit/15a16651ed559a5168ce64e82aa18c8d2f7b9ce6))
* CodexのQuick Startで履歴IDがある場合は上書きしない ([1a6d7f6](https://github.com/akiojin/gwt/commit/1a6d7f6fad321306835af2359bc20df2772c817f))
* Codex保存時に最新セッションIDを再解決 ([0006aa3](https://github.com/akiojin/gwt/commit/0006aa33e812a26f698ff81bccf29ee059c1b073))
* complete stdin reset before/after Claude Code launch ([8fe3504](https://github.com/akiojin/gwt/commit/8fe3504d232416c60c4b456aca028799bb1c1268))
* default skip permissions to no when missing ([491b33e](https://github.com/akiojin/gwt/commit/491b33e464eef332de17c0b19eddf350adfd9b08))
* detect codex session ids in nested dirs ([282fde8](https://github.com/akiojin/gwt/commit/282fde81fc919a57e62aeb80c44728d45ab6ea34))
* extract cwd from nested payload in Codex session files ([3fd1adf](https://github.com/akiojin/gwt/commit/3fd1adfac8139aee7ca088e2a8d6f6e200a45748))
* filter claude quick start entries to existing session files ([fa51163](https://github.com/akiojin/gwt/commit/fa51163d545d1aa97281b6e59d43e3623c7c6fff))
* Gemini resume失敗時に最新セッションへフォールバック ([0db4676](https://github.com/akiojin/gwt/commit/0db4676a383bc3dfeef3e50e53c272b15d326de4))
* Geminiセッションも起動時刻近傍で再解決 ([e5fd21b](https://github.com/akiojin/gwt/commit/e5fd21bf85c174f2c02a1042e1b237046722c9a5))
* Geminiセッション検出をtmp全体のjson/jsonlから抽出 ([c3745b5](https://github.com/akiojin/gwt/commit/c3745b50473485f1a5ba38d73d0e9b531bcd1e2b))
* Gemini起動時にstdoutからsessionIdを確実に捕捉 ([9387c46](https://github.com/akiojin/gwt/commit/9387c46d9c914e33784fce69eeea0fdf10bed202))
* honor CODEX_HOME and CLAUDE_CONFIG_DIR for session lookup ([c8a103d](https://github.com/akiojin/gwt/commit/c8a103d2dd7c60e13f789bc5949489d1401afcd8))
* ignore stdout session ids that lack matching claude session file ([10c6e39](https://github.com/akiojin/gwt/commit/10c6e39328ba1eed14f9bf83cc619df050ea9106))
* improve Codex session cwd matching for worktree paths ([a0e7a9b](https://github.com/akiojin/gwt/commit/a0e7a9bead4bc8663a19238dec71bfe654515999))
* inkの色型エラーを解消 ([60ca0bb](https://github.com/akiojin/gwt/commit/60ca0bb27922d0b779493145c8211979870f3766))
* keep local claude tty to avoid non-interactive launch ([ea6c043](https://github.com/akiojin/gwt/commit/ea6c043417f6ce1211efdb7fef3bebf949df1487))
* limit continue session id to branch history ([daed7dc](https://github.com/akiojin/gwt/commit/daed7dc426e5b1bf4e70f56a318c8573cb26547b))
* localize quick start screen copy ([7d352e6](https://github.com/akiojin/gwt/commit/7d352e6bedf6400ad97b09cc7290ba3ef296e6ea))
* locate Claude sessions under .config fallback ([cdf0322](https://github.com/akiojin/gwt/commit/cdf03226d315a725b39fe407725d8d5302be2291))
* prefer newest claude session file within window ([036cd17](https://github.com/akiojin/gwt/commit/036cd176a9c71a98e4e52b1c2d1dcfd044d9dbda))
* prefer on-disk latest claude session over early probe ([97658ac](https://github.com/akiojin/gwt/commit/97658acf246209f727f03782dd68c8550987fe6a))
* preserve reasoning level and quick start for protected branches ([3e00bdc](https://github.com/akiojin/gwt/commit/3e00bdc14a27b895a8089e05b9cd451577f78899))
* prevent detecting old session IDs on consecutive executions ([4499838](https://github.com/akiojin/gwt/commit/4499838d1ba3c0b5f551551dcce25a63f2b0a202))
* prevent stdin interference in isClaudeCommandAvailable() ([3b96558](https://github.com/akiojin/gwt/commit/3b965585600548c4529f6880a331eba6e6f2db8f))
* prioritize filename UUID over file content for session ID detection ([e5afa1e](https://github.com/akiojin/gwt/commit/e5afa1e5dd31badbcfcb69edceb399e0d5c500d1))
* quick start always resolves latest claude session without time window ([ce656ab](https://github.com/akiojin/gwt/commit/ce656ab488ba25966f4577a31bb21087261b085b))
* quick start uses newest claude session file per worktree ([95c3b99](https://github.com/akiojin/gwt/commit/95c3b999cdee3d50af391ead82e1c2191269dd88))
* Quick StartでClaudeの最新セッションをファイルから優先取得 ([a33fcef](https://github.com/akiojin/gwt/commit/a33fcef2bc7ac9b823ad38b441cc21832dfa08ff))
* Quick StartでEnter二度押し不要に ([fdec5f2](https://github.com/akiojin/gwt/commit/fdec5f2b6a7be241d03a978887c4040f8ee2b73d))
* Quick Startで最新セッションをworktree優先＋カテゴリ表示を簡素化 ([dfbc655](https://github.com/akiojin/gwt/commit/dfbc6550cc1141b33fb54dfb567dd5c5e1f24912))
* Quick Startで初回Enterを受付待ちにバッファ ([6c765ea](https://github.com/akiojin/gwt/commit/6c765ea766bca0af71e378d71b8cf4ff1e3548f8))
* Quick Startの選択でEnterが一度で効くように修正 ([a0e7de3](https://github.com/akiojin/gwt/commit/a0e7de30540c6dc15c0a6c57a727c3a217a76545))
* Quick Startヘッダー初期非表示とレイアウトを改善 ([70ac8ac](https://github.com/akiojin/gwt/commit/70ac8ac689cdbc295f951c294d85bb6aa6bd1994))
* Quick Start表示を短縮しツールごとに見やすく調整 ([5996632](https://github.com/akiojin/gwt/commit/59966322fc5a44261bd2e3f070544d9bb8292965))
* read Claude sessionId from history fallback ([2413d5d](https://github.com/akiojin/gwt/commit/2413d5dc104fdd235b6b6badafc64b2b23820488))
* remove sessionProbe from Codex CLI to prevent Ctrl+C hang ([e59b9f5](https://github.com/akiojin/gwt/commit/e59b9f54ee3680350aceecf18f7c18994f4241f6))
* remove SIGINT catch block from Codex to match Claude Code behavior ([fcf917e](https://github.com/akiojin/gwt/commit/fcf917e0502fd456f2a00c5dfcf2dc0142a9bfb3))
* remove unused imports and variables for ESLint compliance ([1b6ce85](https://github.com/akiojin/gwt/commit/1b6ce850fb8bbb6b9878fa327cfc25a4a9982d57))
* reset stdin state before Ink.js render to prevent hang after Ctrl+C ([7b9b5ff](https://github.com/akiojin/gwt/commit/7b9b5ffa9c528524e01be64e8fd65fe683177524))
* resolve key input lag in Claude Code and Gemini CLI ([cfbc7dd](https://github.com/akiojin/gwt/commit/cfbc7dd4ee0ffa8e56a42b0a050af44e11c78987))
* resume stdin before Claude Code launch to prevent input lag ([34e4353](https://github.com/akiojin/gwt/commit/34e43539589ce415558489a9a465b0eb04080553))
* scope codex/gemini session resolution to worktree ([bf2fafb](https://github.com/akiojin/gwt/commit/bf2fafba7f8eefaf8c8af5787c82cb8d6e55451b))
* show reasoning labels in quick start ([10b99c4](https://github.com/akiojin/gwt/commit/10b99c4de3c61d82d52cdd4997e4483c02e4d4e7))
* show reasoning level in quick start option ([53bfde6](https://github.com/akiojin/gwt/commit/53bfde65b1e66363e0882b81d2ed49d8b2e00974))
* show reasoning level on quick start ([c51a06a](https://github.com/akiojin/gwt/commit/c51a06a8cf0e16c1a53eed6de374b2121032b4b9))
* start new Claude session when no saved ID ([2e7d8b9](https://github.com/akiojin/gwt/commit/2e7d8b95755fede3d38607dce5bcbe33259287cb))
* stop treating arbitrary uuids in claude logs as session ids ([dcfbece](https://github.com/akiojin/gwt/commit/dcfbecee66458465b5d6d15b66a4d0792af38348))
* treat SIGINT as normal exit for AI tool child processes ([b31c912](https://github.com/akiojin/gwt/commit/b31c9125cea7073acaea1c28cbad931200154657))
* update codex test to expect two exitRawMode calls ([e161887](https://github.com/akiojin/gwt/commit/e1618874ef0e6db2a4cdbefb1195ec936b355bba))
* use file-based session detection for Claude/Codex instead of stdout capture ([c1c4211](https://github.com/akiojin/gwt/commit/c1c42114fa84180e6b55671a0660612ab849e220))
* カテゴリ解決をswitchで安全化 ([01942ef](https://github.com/akiojin/gwt/commit/01942ef51c7757bd1cb43d822280db670d179fe9))
* キー入力遅延の解消とGeminiセッションID取得の修正 ([6566c64](https://github.com/akiojin/gwt/commit/6566c64269a85abbb1c4c4cbf2e33b194185c5e7))
* クイックスタートのReasoning/セッションID表示を修正 ([619fa5f](https://github.com/akiojin/gwt/commit/619fa5f16381dc688065cc25bfa7ee27570d6ddb))
* クイックスタートのセッションID表示を修正 ([7a4b5a0](https://github.com/akiojin/gwt/commit/7a4b5a0aa5a9c72a29f59f418d7b6bb0d2e3505c))
* クイックスタート選択時の型チェックを補強 ([95a10e3](https://github.com/akiojin/gwt/commit/95a10e3dd81c2d3151d5f25f598a3a5948a7b3a5))
* セッションファイル探索に時間範囲フィルタを追加 ([5e2aafe](https://github.com/akiojin/gwt/commit/5e2aafea34d13731e962f2af21b9618625cd4424))
* ブランチ/ワークツリー別に最新セッションを抽出 ([a3568c4](https://github.com/akiojin/gwt/commit/a3568c49773049aa6d5fa52b6edd48467b0476cd))
* ブランチ別クイックスタートが最新セッションを誤参照しないように ([b5a789e](https://github.com/akiojin/gwt/commit/b5a789eaf30dd301cda367662a460913183fe7fb))

## [2.11.1](https://github.com/akiojin/gwt/compare/v2.11.0...v2.11.1) (2025-12-05)


### Bug Fixes

* prepare-release.yml を llm-router と同じフローに統一 ([8bace15](https://github.com/akiojin/gwt/commit/8bace15b9532c377e47c275a85fc3277f86f1ffc))
* ブランチ一覧のAIツールラベルからNew/Continue/Resumeを削除 ([45eec22](https://github.com/akiojin/gwt/commit/45eec2208e4955a0b46467b4263e9435725f3633))
* ブランチ一覧のAIツールラベルからNew/Continue/Resumeを削除 ([9d6bf3c](https://github.com/akiojin/gwt/commit/9d6bf3cfbcdef91e0c8301ba8ab52d1397ae7e0f))

## [2.11.0](https://github.com/akiojin/gwt/compare/v2.10.0...v2.11.0) (2025-12-04)


### Features

* ブランチ一覧にLocal/Remote/Sync列を追加 ([7173e59](https://github.com/akiojin/gwt/commit/7173e59a179828ec79942780d6f806bcbd44fa29))
* ブランチ一覧にLocal/Remote/Sync列を追加 ([3459565](https://github.com/akiojin/gwt/commit/3459565ae12a06548ef3215724bed052c9fddc53))
* ブランチ一覧にラベル行を追加 ([b9b23ba](https://github.com/akiojin/gwt/commit/b9b23ba3f68c335bea41de941c0a38f1529fb247))
* ブランチ一覧の表示アイコンを直感的な絵文字に改善 ([cd18b4b](https://github.com/akiojin/gwt/commit/cd18b4b3c5416ecc0f738c84ff0b1c96a1b40631))
* ブランチ一覧の表示アイコンを直感的な絵文字に改善 ([4105f06](https://github.com/akiojin/gwt/commit/4105f0674a2a20cfe1feb3e855799fa0d573c440))


### Bug Fixes

* align branch list headers ([a27b54b](https://github.com/akiojin/gwt/commit/a27b54bc4ee475dfc5e0606a44858b980bb4d622))
* align branch list headers ([6556f46](https://github.com/akiojin/gwt/commit/6556f4695b69addff2533b0b7406b0f470ba6a34))
* ESLint警告103件とPrettier違反12ファイルを修正 ([5b1d61d](https://github.com/akiojin/gwt/commit/5b1d61d2f5128a97556205e3dcfe24b6375f6d9c))
* ESLint警告103件とPrettier違反12ファイルを修正 ([e794b88](https://github.com/akiojin/gwt/commit/e794b88ac5982718f94e6952fcad1ce36cc438f4))
* include upstream base when selecting cleanup targets ([9153de8](https://github.com/akiojin/gwt/commit/9153de8e9ec726bbe88a5ddafc879dd06c19f89b))
* navigation.test.tsxにcollectUpstreamMap/getBranchDivergenceStatusesのモックを追加 ([d4ff9b1](https://github.com/akiojin/gwt/commit/d4ff9b1cd5c785160385bf31a781bad8817db03a))
* origin/developとのマージコンフリクトを解決 ([f0898ed](https://github.com/akiojin/gwt/commit/f0898ed56fc43fcf5080ea25397fd9fea97f617c))
* origin/developとのマージコンフリクトを解決 ([a3e627a](https://github.com/akiojin/gwt/commit/a3e627a51765ca61848967b0f36d03d910720aa4))
* origin/developとのマージコンフリクトを解決 ([e30e192](https://github.com/akiojin/gwt/commit/e30e192cc8fc80159033e4aee679f30d9bf3fac5))
* Remote列の表示を改善（L=ローカルのみ、R=リモートのみ） ([dc8f375](https://github.com/akiojin/gwt/commit/dc8f375a180b0f1eebc273346f997f1f442713aa))
* Sync列の数字をアイコン直後に表示 ([253a529](https://github.com/akiojin/gwt/commit/253a5298d446e24858861a14a074986430973bc1))
* Sync列を固定幅化してブランチ名の位置を揃える ([20d0d67](https://github.com/akiojin/gwt/commit/20d0d6729133e802a9846e0c402acdd84c6de28b))
* レビューコメントへの対応 ([5e79440](https://github.com/akiojin/gwt/commit/5e79440e35fb858a4ee26bb0dd922c166d83de26))
* 自動クリーンアップでリモートブランチを削除しないように修正 ([42f6296](https://github.com/akiojin/gwt/commit/42f6296576b5d81558c5aa5c3823dc0e6c833605))
* 自動クリーンアップでリモートブランチを削除しないように修正 ([674b4ce](https://github.com/akiojin/gwt/commit/674b4ce09b98cbf15359984539d4aa5dfe0e7d16))

## [2.10.0](https://github.com/akiojin/gwt/compare/v2.9.1...v2.10.0) (2025-12-04)


### Features

* Cコマンドでリモートブランチも削除対象に追加 ([632cb9e](https://github.com/akiojin/gwt/commit/632cb9eb6ee325b2959dad5fec44969f09d75764))


### Bug Fixes

* align cleanup reasons with types and dedupe vars ([a3994ed](https://github.com/akiojin/gwt/commit/a3994eda8f122de1ac24da91648a642fdb024ab9))
* expand cleanup candidates for remote-synced branches ([9204c5a](https://github.com/akiojin/gwt/commit/9204c5a9388d0b9cee403ce832ab75f916b2d469))
* stabilize worktree cleanup and ui tests ([61d510b](https://github.com/akiojin/gwt/commit/61d510b97c579cf988342db7a3b1c6d9cf576db1))
* リモートブランチ削除をマージ済みPRのみに限定 ([ba9a653](https://github.com/akiojin/gwt/commit/ba9a653135fa819d992e4d462fdd80766a9c8067))

## [2.9.1](https://github.com/akiojin/gwt/compare/v2.9.0...v2.9.1) (2025-11-27)


### Bug Fixes

* persist last AI tool before launch ([d545f13](https://github.com/akiojin/gwt/commit/d545f133ce93b78158ad1de49b0637fceea7003d))
* persist last AI tool before launch ([5c160b6](https://github.com/akiojin/gwt/commit/5c160b6128ce5c9f399ee98ca23ffebcb317181d))

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
* READMEを更新し、GEMINI.mdを作成 ([4fa1491](https://github.com/akiojin/gwt/commit/4fa14914941593b330efa7486eef3772e387f330))
* remember last model and reasoning selection per tool ([01b5124](https://github.com/akiojin/gwt/commit/01b5124409b13763d8b792493ef72442714cf4f9))

# [2.3.0](https://github.com/akiojin/gwt/compare/v2.2.0...v2.3.0) (2025-11-19)


### Features

* Codex/Geminiの表示名を簡潔化 ([cc8bdb2](https://github.com/akiojin/gwt/commit/cc8bdb25617de58c79dda5b53134fc2a3ac89aa2))
* Gemini CLIをビルトインツールとして追加 ([0e80363](https://github.com/akiojin/gwt/commit/0e80363ae81fdf61c00ae55dbecf8b2cecd677e4))
* ビルトインツールを追加 ([b4f6c94](https://github.com/akiojin/gwt/commit/b4f6c9476f6675bdc4e27006b60868ee14f07dae))

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
