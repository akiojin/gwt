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

## [4.12.2](https://github.com/akiojin/gwt/compare/v4.12.1...v4.12.2) (2026-01-11)


### Bug Fixes

* address review feedback ([5ab4f2c](https://github.com/akiojin/gwt/commit/5ab4f2c4c464d67c3119206b3a126690970ffd98))
* gh-fix-ciとシグナル処理を改善 ([6a1a8ff](https://github.com/akiojin/gwt/commit/6a1a8ffb21e25c18f0726d000814ea22ee17734f))
* gh-fix-ciの検出と解決フローを改善 ([5e35bbd](https://github.com/akiojin/gwt/commit/5e35bbde31ca3144c1cea6f2bf8a8ba40a191ab3))
* map additional signals ([472cd48](https://github.com/akiojin/gwt/commit/472cd489794328ff6f500705e928ee990ff3e98f))
* シグナル正規化と終了判定を改善 ([569b6bb](https://github.com/akiojin/gwt/commit/569b6bbb9c5e4552374f125253c918dfca64c0eb))

## [4.12.1](https://github.com/akiojin/gwt/compare/v4.12.0...v4.12.1) (2026-01-11)


### Bug Fixes

* bunx実行時にBunで再実行する ([#558](https://github.com/akiojin/gwt/issues/558)) ([ed2ad0b](https://github.com/akiojin/gwt/commit/ed2ad0b073fd0672e074cd88914c148cd778908e))
* cache installed versions for wizard ([#555](https://github.com/akiojin/gwt/issues/555)) ([13738bb](https://github.com/akiojin/gwt/commit/13738bb885d50e796767ceba263b247b9827f080))
* Codex skillsフラグをバージョン判定で切替 ([#552](https://github.com/akiojin/gwt/issues/552)) ([6825a81](https://github.com/akiojin/gwt/commit/6825a8119974beb6750a9bf0c8e215737557ba49))
* ブランチリフレッシュ時にリモート追跡参照を更新 & CI/CD最適化 ([#554](https://github.com/akiojin/gwt/issues/554)) ([929a5ce](https://github.com/akiojin/gwt/commit/929a5ceca5e601d96d2a49bff8dee7b7bf3867dd))

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
