## [1.29.1](https://github.com/akiojin/claude-worktree/compare/v1.29.0...v1.29.1) (2025-11-08)


### Bug Fixes

* npm publish時の認証設定を修正 ([3e4208b](https://github.com/akiojin/claude-worktree/commit/3e4208bb25c3fbe609cfec45e8ede3a17e2858c1))
* remove redundant terminal.exitRawMode() call in error path ([60ae16d](https://github.com/akiojin/claude-worktree/commit/60ae16d13feacb33b3494f0730c65253d0b84b6e))

# [1.29.0](https://github.com/akiojin/claude-worktree/compare/v1.28.2...v1.29.0) (2025-11-08)


### Bug Fixes

* Codex起動時のJSON構文エラー修正とエラー時のCLI継続 ([ef40e6d](https://github.com/akiojin/claude-worktree/commit/ef40e6dd09a695231f0adb264fea630d76c8240d))
* execaのshell: trueオプションを削除してCodex CLI起動エラーを修正 ([08cc1fd](https://github.com/akiojin/claude-worktree/commit/08cc1fd046a3484db0621f808fbaff4889f63859))
* npm publish時の認証設定を修正 ([#203](https://github.com/akiojin/claude-worktree/issues/203)) ([99f78b4](https://github.com/akiojin/claude-worktree/commit/99f78b42202e12026796c2ac67a49433839ed532))


### Features

* npm公開機能を有効化 ([20df08a](https://github.com/akiojin/claude-worktree/commit/20df08ab66a64525009c71574e459e53540c598e))

## [1.28.2](https://github.com/akiojin/claude-worktree/compare/v1.28.1...v1.28.2) (2025-11-08)


### Bug Fixes

* publish.ymlへのバックマージ処理の移行 ([3a12077](https://github.com/akiojin/claude-worktree/commit/3a12077ad597cc7ebdfe035e8ce81fb762165c7e))

## [1.28.1](https://github.com/akiojin/claude-worktree/compare/v1.28.0...v1.28.1) (2025-11-08)


### Bug Fixes

* 3回目のパッチバージョンテスト修正追加 ([e96ebe8](https://github.com/akiojin/claude-worktree/commit/e96ebe81cd95c7dfc9162ebc540bdce47e303f05))

# [1.28.0](https://github.com/akiojin/claude-worktree/compare/v1.27.1...v1.28.0) (2025-11-08)


### Features

* 3回目のマイナーバージョンテスト機能追加 ([cb2a689](https://github.com/akiojin/claude-worktree/commit/cb2a6896d00474a9a3a922bfb466b5e9da59d9ce))

## [1.27.1](https://github.com/akiojin/claude-worktree/compare/v1.27.0...v1.27.1) (2025-11-08)


### Bug Fixes

* パッチバージョンリリーステスト用修正追加 ([766f97d](https://github.com/akiojin/claude-worktree/commit/766f97d1016e1aa0cae42d4183a2ec80fe4dda50))

# [1.27.0](https://github.com/akiojin/claude-worktree/compare/v1.26.1...v1.27.0) (2025-11-08)


### Features

* マイナーバージョンリリーステスト機能追加 ([c8afaf0](https://github.com/akiojin/claude-worktree/commit/c8afaf0563cea104bb620c879a0dd4df3f432f1d))

## [1.26.1](https://github.com/akiojin/claude-worktree/compare/v1.26.0...v1.26.1) (2025-11-08)


### Bug Fixes

* format entry workflow tests ([32853c5](https://github.com/akiojin/claude-worktree/commit/32853c5eaa60cf9e2f8b0bca9bed86338b7e956d))
* WorktreeOrchestratorモックをクラスベースに修正 ([0f058a1](https://github.com/akiojin/claude-worktree/commit/0f058a10fbb2652e790df198b5ed816583210da2))
* カバレッジレポート生成失敗を許容 ([a34fcf6](https://github.com/akiojin/claude-worktree/commit/a34fcf62e1496c45a09eec5fd860af344da6a4df))
* パッチバージョンリリーステスト用ファイル追加 ([be7e853](https://github.com/akiojin/claude-worktree/commit/be7e8533f9c85fb8ede07f94866e6651d2c9c8f5))

# [1.26.0](https://github.com/akiojin/claude-worktree/compare/v1.25.1...v1.26.0) (2025-11-08)


### Bug Fixes

* add test file for patch version release ([3e1ecc3](https://github.com/akiojin/claude-worktree/commit/3e1ecc39877eac1dc5bb6653f2a26870230bbad0))
* auto-mergeをpull_request_targetに変更 ([79f5798](https://github.com/akiojin/claude-worktree/commit/79f5798f63c0d1f55b46c05af2fe143d6cad3507))
* keep cli running on git failures ([80bd4f6](https://github.com/akiojin/claude-worktree/commit/80bd4f68a619c50fa80f52f50709596232e5b91e))
* semantic-releaseのドライラン実行時にGITHUB_TOKENを設定 ([e840130](https://github.com/akiojin/claude-worktree/commit/e8401306bf60538ca828b60f6d57dc57e2710bcf))
* unity-mcp-serverとの完全統一（残り20%の修正） ([3172f4b](https://github.com/akiojin/claude-worktree/commit/3172f4bd5dd30518c1e12f3559d1b8c6e901f9bc))
* unity-mcp-serverとの差分を修正 ([4036b06](https://github.com/akiojin/claude-worktree/commit/4036b06c30e78babaf3ba25b9c6db973adabcfcb))


### Features

* unity-mcp-server型自動リリースフロー完全導入 ([e146bed](https://github.com/akiojin/claude-worktree/commit/e146bedc4fa2abd59e2618426d7f8617cd661f5d))

## [1.25.1](https://github.com/akiojin/claude-worktree/compare/v1.25.0...v1.25.1) (2025-11-07)


### Bug Fixes

* make release sync safe for develop ([a8b1059](https://github.com/akiojin/claude-worktree/commit/a8b1059df4ab93a37f4791f62fb9d82caa7cdc02))

# [1.25.0](https://github.com/akiojin/claude-worktree/compare/v1.24.2...v1.25.0) (2025-11-07)


### Bug Fixes

* disable husky in release workflow ([a5e1548](https://github.com/akiojin/claude-worktree/commit/a5e1548852b72007faec1735983ea52edacd7205))
* Docker環境でのpnpmセットアップとプロジェクトビルドを修正 ([a5b40f3](https://github.com/akiojin/claude-worktree/commit/a5b40f35d4b3f43efa475e4e91905908a7bce9e9))
* update Dockerfile to use npm for global tool installation ([25b0410](https://github.com/akiojin/claude-worktree/commit/25b04108cda872c10c07f91fa8211b08c99aa1c7))
* use node 22 for release workflow ([cb05bea](https://github.com/akiojin/claude-worktree/commit/cb05bea7eaacc807587efb8652b8612865e19e24))
* use PAT for release pushes ([ef8027b](https://github.com/akiojin/claude-worktree/commit/ef8027be707e505b99464959118f1ada73e0b9f9))


### Features

* orchestrate release branch auto merge flow ([82e19aa](https://github.com/akiojin/claude-worktree/commit/82e19aa2a344f8d1022bd21b58639bf942e6cd50))

## [1.24.2](https://github.com/akiojin/claude-worktree/compare/v1.24.1...v1.24.2) (2025-11-07)

### Bug Fixes

- Codexエラー時でもCLIを継続 ([6b5d537](https://github.com/akiojin/claude-worktree/commit/6b5d537192d2a1b48c8cffd89b488aabb2b1dca2))

## [1.24.1](https://github.com/akiojin/claude-worktree/compare/v1.24.0...v1.24.1) (2025-11-07)

### Bug Fixes

- scope gitignore updates to active worktree ([e176d5b](https://github.com/akiojin/claude-worktree/commit/e176d5b4c82752e849a4b41142e28ae6418b4802))

# [1.24.0](https://github.com/akiojin/claude-worktree/compare/v1.23.0...v1.24.0) (2025-11-07)

### Bug Fixes

- allow protected branches to launch ai tools ([9b32f21](https://github.com/akiojin/claude-worktree/commit/9b32f2124a395d5a1d916fb7899e803952d08935))
- correct protected branch type handling ([1444863](https://github.com/akiojin/claude-worktree/commit/14448634827c5742faa0920e42be129d9cf50bea))
- git branch参照コマンドのブロックを解除 ([23e8777](https://github.com/akiojin/claude-worktree/commit/23e87775553cfe8bd8797b89f6775c8dfcaaca94))
- move mock functions inside vi.mock factory ([f52a2a8](https://github.com/akiojin/claude-worktree/commit/f52a2a8772772ef33beb09abf3b21dead4505442))
- replace vi.hoisted() with direct mock definitions ([4c61bbe](https://github.com/akiojin/claude-worktree/commit/4c61bbedc2ffa4d612ff9bd8bb5e316425e19cc7))
- reuse repository root for protected branches ([2464b98](https://github.com/akiojin/claude-worktree/commit/2464b983837ff1efcaff4c7f03f8cb6a418c76b6))
- stabilize release test suites ([9c5177c](https://github.com/akiojin/claude-worktree/commit/9c5177c9418097cd22fd2e586a1962204dd86e85))
- 保護ブランチ選択時のルート切替とUIを整備 ([8798074](https://github.com/akiojin/claude-worktree/commit/8798074434ccea00afdd83cc9350d57d9602b54c))

### Features

- clarify protected branch workflow in ui ([a191845](https://github.com/akiojin/claude-worktree/commit/a191845468b9cc91418d274cca091ea19d22153e))
- guard protected branches from worktree creation ([eeb9c45](https://github.com/akiojin/claude-worktree/commit/eeb9c45d6c58d961dec7f36ee0d3dbf537ed5c3b))
- PRベースブランチ検証とブランチ戦略の明確化 ([daf57d2](https://github.com/akiojin/claude-worktree/commit/daf57d237f0f7edb3411e5b1ba480f7c1a81a2e4))

# [1.23.0](https://github.com/akiojin/claude-worktree/compare/v1.22.0...v1.23.0) (2025-11-06)

### Bug Fixes

- AIツール起動失敗時もCLIを継続 ([ffa9398](https://github.com/akiojin/claude-worktree/commit/ffa9398e9f3e891661c08a0b9f6d0ecab89cdae8))
- worktree作成時の進捗表示を改善 ([4bf26b7](https://github.com/akiojin/claude-worktree/commit/4bf26b797d8f60c14fa6e217e6a42ea59dfbbd6d))

### Features

- worktree作成中にスピナーを表示 ([d65f506](https://github.com/akiojin/claude-worktree/commit/d65f506fbc726991fecb71a1ce6730742a49a148))

# [1.22.0](https://github.com/akiojin/claude-worktree/compare/v1.21.3...v1.22.0) (2025-11-06)

### Features

- develop-to-main手動リリースフローの実装 ([8f0efec](https://github.com/akiojin/claude-worktree/commit/8f0efec9b6f52500f097d2f1b53d892d43c8b6ac))

## [1.21.3](https://github.com/akiojin/claude-worktree/compare/v1.21.2...v1.21.3) (2025-11-06)

### Bug Fixes

- ensure worktree directory exists before creation ([ea78df2](https://github.com/akiojin/claude-worktree/commit/ea78df27421bddb7cb3148fc2c9bebad81ad74aa))

## [1.21.2](https://github.com/akiojin/claude-worktree/compare/v1.21.1...v1.21.2) (2025-11-06)

### Bug Fixes

- エラー発生時の入力待機処理を追加 ([5eeb28e](https://github.com/akiojin/claude-worktree/commit/5eeb28e04a6448f62790cea9feba5bfd9793f6fc))

## [1.21.1](https://github.com/akiojin/claude-worktree/compare/v1.21.0...v1.21.1) (2025-11-05)

### Bug Fixes

- show pending state during branch creation ([a4dbc3f](https://github.com/akiojin/claude-worktree/commit/a4dbc3fb0a63a78b76e47994319f95759f7cac66))

# [1.21.0](https://github.com/akiojin/claude-worktree/compare/v1.20.2...v1.21.0) (2025-11-05)

### Bug Fixes

- align timestamp column for branch list ([5d49b8c](https://github.com/akiojin/claude-worktree/commit/5d49b8c0e91cc599b7fe4066616756ae43c984bf))

### Features

- ブランチ行の最終更新表示を整形し右寄せを改善 ([5f97b67](https://github.com/akiojin/claude-worktree/commit/5f97b67485d58823bc73472141f3bec38e2994d6))

## [1.20.2](https://github.com/akiojin/claude-worktree/compare/v1.20.1...v1.20.2) (2025-11-05)

### Bug Fixes

- Bashフックで連結コマンドのgit操作を検知 ([31da6a7](https://github.com/akiojin/claude-worktree/commit/31da6a76d36139ca9a9a84781251f553abf50d6e))

## [1.20.1](https://github.com/akiojin/claude-worktree/compare/v1.20.0...v1.20.1) (2025-11-05)

### Bug Fixes

- limit divergence checks to selected branch ([11e1115](https://github.com/akiojin/claude-worktree/commit/11e1115117aa37e389a3009eb67ebfa0c35cddf3))

# [1.20.0](https://github.com/akiojin/claude-worktree/compare/v1.19.3...v1.20.0) (2025-11-05)

### Bug Fixes

- ブランチ行レンダリングのハイライト表示を調整 ([d20ca27](https://github.com/akiojin/claude-worktree/commit/d20ca27b52be87bb56ee05a716594ecfb5848d01))

### Features

- ブランチ一覧に最終更新時刻を表示 ([c19e504](https://github.com/akiojin/claude-worktree/commit/c19e5043075f8adc89260cb6e5ada5b74c8fb813))

## [1.19.3](https://github.com/akiojin/claude-worktree/compare/v1.19.2...v1.19.3) (2025-11-05)

### Bug Fixes

- rely on GH_TOKEN env directly ([ed523fe](https://github.com/akiojin/claude-worktree/commit/ed523fefe62d9cfe6768723c30a7db6b6722ea00))

## [1.19.2](https://github.com/akiojin/claude-worktree/compare/v1.19.1...v1.19.2) (2025-11-05)

### Bug Fixes

- login gh before enabling auto merge ([9ff2afc](https://github.com/akiojin/claude-worktree/commit/9ff2afc1b9a0883f566dac8d33ce83ac5745f3f6))

## [1.19.1](https://github.com/akiojin/claude-worktree/compare/v1.19.0...v1.19.1) (2025-11-05)

### Bug Fixes

- adjust auto merge workflow permissions ([49a122b](https://github.com/akiojin/claude-worktree/commit/49a122b5ae0eb706baf56e9de8d0b0b339b1537c))
- guard auto merge workflow when token missing ([6834305](https://github.com/akiojin/claude-worktree/commit/68343050fdb74af22c8276d3ed57ba8ff1015e2e))

# [1.19.0](https://github.com/akiojin/claude-worktree/compare/v1.18.1...v1.19.0) (2025-11-05)

### Features

- PR作成時に自動マージを有効化 ([233266a](https://github.com/akiojin/claude-worktree/commit/233266a1e99547c2e6f1ddbf91fbb283b1c84435))

## [1.18.1](https://github.com/akiojin/claude-worktree/compare/v1.18.0...v1.18.1) (2025-11-05)

### Bug Fixes

- heredoc内のgit文字列に誤反応しないようフック検知ロジックを改善 ([8453ce2](https://github.com/akiojin/claude-worktree/commit/8453ce2ef08c39113e06a7d23dcbe0aa5f6a84ce))

# [1.18.0](https://github.com/akiojin/claude-worktree/compare/v1.17.0...v1.18.0) (2025-11-05)

### Bug Fixes

- BatchMergeServiceテストのモック修正とコンパイルエラー解消 ([985cd75](https://github.com/akiojin/claude-worktree/commit/985cd75ab745058b407b7ab5e3d634a7858d74b6))
- exact optional cwd handling in divergence helper ([4e7cbb7](https://github.com/akiojin/claude-worktree/commit/4e7cbb7cf0f76b206abf2ee6201bf1691875ff53))
- 最新コミット順ソートの型エラーを解消 ([b488819](https://github.com/akiojin/claude-worktree/commit/b488819f2604477ea98d45997a83d47d9fba337b))

### Features

- AI起動前にfast-forward pullと競合警告を追加 ([83badeb](https://github.com/akiojin/claude-worktree/commit/83badeb1a046bbcfaf626c7f203ce916d10cb5ab))
- App.tsxにbatch merge機能を統合 ([8aa0dad](https://github.com/akiojin/claude-worktree/commit/8aa0dada19291044a21bf3f9ae320ae8517b4e61))
- auto-pushモード実装（T401-T404） ([f48db05](https://github.com/akiojin/claude-worktree/commit/f48db050e9ab3293dc856c71aa3fb7c23a172eec))
- Bashツールでのgitブランチ操作を禁止するPreToolUseフックを追加 ([9b8c648](https://github.com/akiojin/claude-worktree/commit/9b8c6484babea5be0e19da5b63af8da9ea7252e0))
- BatchMergeService完全実装 (T201-T214) ([2486292](https://github.com/akiojin/claude-worktree/commit/248629247a2abd16984c05a50749a2019ba4ab87))
- dry-runモード実装（T301-T304） ([4378200](https://github.com/akiojin/claude-worktree/commit/43782005238a6b129da7ef24408042ea22dc5888))
- Husky対応を追加してコミット前の品質チェックを自動化 ([df34b61](https://github.com/akiojin/claude-worktree/commit/df34b616fee0927b79ebd61a9b1d46c6650ced99))
- フェーズ2完了 - 型定義とgit操作基盤実装 ([faa9a4d](https://github.com/akiojin/claude-worktree/commit/faa9a4dde9ca0cc24213e79ae74f18621c87c19a))
- ブランチ一覧の最新コミット順ソートを追加 ([b12fa80](https://github.com/akiojin/claude-worktree/commit/b12fa80d1687ab4e2ae56968fb0b9e34f0fc5079))
- ヘッダーに起動ディレクトリ表示機能の仕様を追加 ([b68637f](https://github.com/akiojin/claude-worktree/commit/b68637ff1e39f8a059c19e64416e8463777f2400))
- ヘッダーに起動ディレクトリ表示機能を実装 ([911edad](https://github.com/akiojin/claude-worktree/commit/911edad4fa47bc44961e61455b71283d9cc261ac))
- ヘッダーへの起動ディレクトリ表示の実装タスクを追加 ([4586d19](https://github.com/akiojin/claude-worktree/commit/4586d19a01991fd82d1dc99ed3ed567c403f6899))
- ヘッダーへの起動ディレクトリ表示の実装計画を追加 ([26b442f](https://github.com/akiojin/claude-worktree/commit/26b442f53f16504d0f5b37f76cc3f8372780612c))

# [1.17.0](https://github.com/akiojin/claude-worktree/compare/v1.16.0...v1.17.0) (2025-11-01)

### Features

- Windows向けインストール方法を推奨メッセージに追加 ([d6cda00](https://github.com/akiojin/claude-worktree/commit/d6cda009fdef7e871f23d522475fe332c0a17f90))

# [1.16.0](https://github.com/akiojin/claude-worktree/compare/v1.15.0...v1.16.0) (2025-11-01)

### Features

- bunxフォールバック時に公式インストール方法を推奨 ([e79f151](https://github.com/akiojin/claude-worktree/commit/e79f151d292451c4ad4757770e2b4e053ad33e61))
- bunxフォールバック時のメッセージに2秒待機を追加 ([ba60cad](https://github.com/akiojin/claude-worktree/commit/ba60cad50580e5fa2f52aa54bef92a255c973c03))

# [1.15.0](https://github.com/akiojin/claude-worktree/compare/v1.14.0...v1.15.0) (2025-11-01)

### Features

- Claude Code自動検出機能を追加（US4: ローカルインストール版優先） ([17e3279](https://github.com/akiojin/claude-worktree/commit/17e3279d8d73df11bcd8a0f009a53ab2a74a6e8e))

# [1.14.0](https://github.com/akiojin/claude-worktree/compare/v1.13.0...v1.14.0) (2025-10-31)

### Features

- add git loading indicator with tdd coverage ([ca9faa9](https://github.com/akiojin/claude-worktree/commit/ca9faa9ea85be3abbd8cec8043bb005cf25731d1))
- ブランチ一覧に未プッシュ・PR状態アイコンを追加 ([404af59](https://github.com/akiojin/claude-worktree/commit/404af5934ec9ac8ac9b382422913228fd8707f34))

# [1.13.0](https://github.com/akiojin/claude-worktree/compare/v1.12.3...v1.13.0) (2025-10-31)

### Features

- UIヘッダーにバージョン表示機能を追加 (US2) ([17e8a6d](https://github.com/akiojin/claude-worktree/commit/17e8a6dfb7cb84b7e6bcbb472d17a136ff8b4e45))
- **version:** add CLI version flag (--version/-v) ([351d702](https://github.com/akiojin/claude-worktree/commit/351d7026d97e02ee1c1babcc3d6c0d70dabe14be))

## [1.12.3](https://github.com/akiojin/claude-worktree/compare/v1.12.2...v1.12.3) (2025-10-31)

### Bug Fixes

- Codex CLIのweb検索フラグを正しく有効化 ([7319690](https://github.com/akiojin/claude-worktree/commit/7319690d1d4bee5978b49390f797bc471164b683))

## [1.12.2](https://github.com/akiojin/claude-worktree/compare/v1.12.1...v1.12.2) (2025-10-31)

### Bug Fixes

- 自動更新時のカーソル位置リセット問題を解決 ([ee54c1a](https://github.com/akiojin/claude-worktree/commit/ee54c1a396c1905080f44558c1952daeaa594af1))

## [1.12.1](https://github.com/akiojin/claude-worktree/compare/v1.12.0...v1.12.1) (2025-10-31)

### Bug Fixes

- Codex CLIのweb_search_request対応 ([efe151e](https://github.com/akiojin/claude-worktree/commit/efe151e8d4800b43399bd9e9718a4c777f2ebe8e))

# [1.12.0](https://github.com/akiojin/claude-worktree/compare/v1.11.0...v1.12.0) (2025-10-30)

### Bug Fixes

- ensureGitignoreEntryテストを統合テストに変更 ([2727353](https://github.com/akiojin/claude-worktree/commit/2727353744d5eb23eb7b0cda5329dadd11479253))
- Spec Kitスクリプトのデフォルト動作をブランチ作成なしに変更 ([e7627ec](https://github.com/akiojin/claude-worktree/commit/e7627ecf11c40da4f98a1203d7745a881117f70e))
- Spec Kitスクリプトのブランチ名制約を緩和 ([fb7118a](https://github.com/akiojin/claude-worktree/commit/fb7118a97620d5d0d35df13004bdbcefd83ea2cb))

### Features

- Worktreeディレクトリパスを.git/worktreeから.worktreesに変更 ([da46aa1](https://github.com/akiojin/claude-worktree/commit/da46aa1385fdd420072c12a6f3a70a5400a02675))
- worktree作成時に.gitignoreへ.worktrees/を自動追加 ([aabfaee](https://github.com/akiojin/claude-worktree/commit/aabfaee47aca3660398b1860f2b51b4762f99f57))

## [Unreleased]

### Features

- Add version display in CLI flag (`--version` / `-v`) and UI header
  - CLI: Display package version with `claude-worktree --version` or `-v` flag
  - UI: Show version in all screen headers (e.g., "Claude Worktree v1.12.3")
  - Graceful handling when version cannot be retrieved

- **worktree**: Worktreeディレクトリを`.git/worktree`から`.worktrees`に変更
  - 新規Worktreeは`.worktrees/`配下に作成されます
  - `.gitignore`に`.worktrees/`が自動的に追加されます
  - 既存の`.git/worktree`配下のWorktreeは影響を受けず、引き続き動作します
  - 新旧両方のパスのWorktreeが共存可能です

### Tests

- **worktree**: 後方互換性テストを追加
  - 既存の`.git/worktree`パスのWorktreeが正常に動作することを検証
  - 新旧パスのWorktreeが共存できることを検証

# [1.11.0](https://github.com/akiojin/claude-worktree/compare/v1.10.0...v1.11.0) (2025-10-30)

### Bug Fixes

- realtimeUpdate.test.tsxのテストアプローチを修正 ([54a4502](https://github.com/akiojin/claude-worktree/commit/54a4502a9a614739dc88b5a5be01973656c805ee))

### Features

- リアルタイム更新機能を実装（FR-009対応） ([9dd057a](https://github.com/akiojin/claude-worktree/commit/9dd057a6fe6a62e4e0c86a948826c0f77fcdb145))

# [1.10.0](https://github.com/akiojin/claude-worktree/compare/v1.9.0...v1.10.0) (2025-10-29)

### Features

- cコマンドでベース差分なしブランチもクリーンアップ対象に追加 ([4ff79b7](https://github.com/akiojin/claude-worktree/commit/4ff79b76ebbc2b2d2a40ccf250eba8d1a165abb9))

# [1.9.0](https://github.com/akiojin/claude-worktree/compare/v1.8.0...v1.9.0) (2025-10-29)

### Bug Fixes

- AIToolSelectorScreenテストを非同期読み込みに対応 ([0114eeb](https://github.com/akiojin/claude-worktree/commit/0114eebf82e44f9e71d91a9903993f088fdc959a))

### Features

- カスタムAIツール対応機能を実装（設定管理・UI統合・起動機能） ([bce4292](https://github.com/akiojin/claude-worktree/commit/bce4292a11122556373f787ecddf4a656452e309))
- カスタムツール統合と実行オプション拡張（Phase 4-6完了） ([c769f5d](https://github.com/akiojin/claude-worktree/commit/c769f5dba5a6a7d73b1e88dea7d1d3b7bd8a847f))
- セッション管理拡張とコード品質改善（Phase 7-8完了） ([6e5b1cf](https://github.com/akiojin/claude-worktree/commit/6e5b1cf0ea1382535b21f2b32a9c2689a443789d))

# [1.8.0](https://github.com/akiojin/claude-worktree/compare/v1.7.1...v1.8.0) (2025-10-29)

### Features

- 戻るキーをqからESCに変更、終了はCtrl+Cに統一 ([2b9d82b](https://github.com/akiojin/claude-worktree/commit/2b9d82bca649a1edd06d800c58f629aa469a3074))

## [1.7.1](https://github.com/akiojin/claude-worktree/compare/v1.7.0...v1.7.1) (2025-10-29)

### Bug Fixes

- BranchActionSelectorScreenでqキーで戻る機能と英語化を実装 ([c7860aa](https://github.com/akiojin/claude-worktree/commit/c7860aa2a0c3cbee359c7d830406c6124b4106f4))

# [1.7.0](https://github.com/akiojin/claude-worktree/compare/v1.6.0...v1.7.0) (2025-10-29)

### Bug Fixes

- TypeScript型エラーを修正してビルドを通す ([e9564ee](https://github.com/akiojin/claude-worktree/commit/e9564ee6c95390d8dda0eb73d5f454a1c8596e4d))

### Features

- ブランチ選択後にアクション選択画面を追加（MVP2） ([54eae36](https://github.com/akiojin/claude-worktree/commit/54eae367045945bd45fa4a905f20a96e6a04bd8f))
- 選択したブランチをベースブランチとして新規ブランチ作成に使用 ([2ee8eb5](https://github.com/akiojin/claude-worktree/commit/2ee8eb5b1d06ac2e7b24a7cada91989ec023b3ac))

# [1.6.0](https://github.com/akiojin/claude-worktree/compare/v1.5.0...v1.6.0) (2025-10-29)

### Features

- カレントブランチ選択時にWorktree作成をスキップする機能を実装 ([e6bfec2](https://github.com/akiojin/claude-worktree/commit/e6bfec28562a1a7bfba32c06b7002e34a04d513c))
- 型定義を追加（BranchAction, ScreenType拡張, getCurrentBranch export） ([da38caf](https://github.com/akiojin/claude-worktree/commit/da38cafca4946e272f2836a12a4ccd6f9c5c706a))

# [1.5.0](https://github.com/akiojin/claude-worktree/compare/v1.4.5...v1.5.0) (2025-10-29)

### Features

- ブランチ一覧のソート機能を実装 ([3c23747](https://github.com/akiojin/claude-worktree/commit/3c237474ca30756bc2e06ba958036af38238a6e2))

## [1.4.5](https://github.com/akiojin/claude-worktree/compare/v1.4.4...v1.4.5) (2025-10-27)

### Bug Fixes

- テストファイルを削除してnpm自動公開を確認 ([613f404](https://github.com/akiojin/claude-worktree/commit/613f404003edb576bba5592bb08829377cac5cf1))

## [1.4.4](https://github.com/akiojin/claude-worktree/compare/v1.4.3...v1.4.4) (2025-10-27)

### Bug Fixes

- NPM Token更新後の自動公開を有効化 ([3161265](https://github.com/akiojin/claude-worktree/commit/3161265b370bfbcb62781cd2fa5e8d7107617b43))

## [1.4.3](https://github.com/akiojin/claude-worktree/compare/v1.4.2...v1.4.3) (2025-10-27)

### Bug Fixes

- npm publishでOIDC provenanceを有効化 ([c296899](https://github.com/akiojin/claude-worktree/commit/c2968993240095e4914457d7d7dcf06a2449651f))

## [1.4.2](https://github.com/akiojin/claude-worktree/compare/v1.4.1...v1.4.2) (2025-10-27)

### Bug Fixes

- PRクリーンアップ時の未プッシュ判定をマージ済みブランチに対応 ([b9fe8bb](https://github.com/akiojin/claude-worktree/commit/b9fe8bbd58eb60154a3bebf7d694a1dc9555e2f2))
- semantic-releaseがdetached HEAD状態で動作しない問題を修正 ([5ce7549](https://github.com/akiojin/claude-worktree/commit/5ce7549d903507cb0b1a23fb8fa0238c34e449fb))
- **ui:** stop spinner once cleanup completes ([602b3ce](https://github.com/akiojin/claude-worktree/commit/602b3ceae51b967e885ddb239e739091d06f1f4e))

## [1.4.1](https://github.com/akiojin/claude-worktree/compare/v1.4.0...v1.4.1) (2025-10-27)

### Bug Fixes

- Ink UI終了時にTTYリスナーを解放 ([c6c5392](https://github.com/akiojin/claude-worktree/commit/c6c53921b6d17ffc5cce3cc3bc399ff8bac38683))
- 子プロセス用TTYを安全に引き渡す ([5168007](https://github.com/akiojin/claude-worktree/commit/5168007d0c37ba17fd923741b83728610aa56d8c))

# [1.4.0](https://github.com/akiojin/claude-worktree/compare/v1.3.1...v1.4.0) (2025-10-27)

### Bug Fixes

- Ink UIのTTY制御を安定化 ([290b9e2](https://github.com/akiojin/claude-worktree/commit/290b9e2f183a7fbf4bb1ba4d0ed047381c2c6593))
- TTYフォールバックの標準入出力を引き渡す ([19aaed1](https://github.com/akiojin/claude-worktree/commit/19aaed1ac186d9a6f3b281b65f482c3e10e59500))

### Features

- **ui:** PRクリーンアップ実行中のフィードバックを改善 ([c8f5259](https://github.com/akiojin/claude-worktree/commit/c8f525914112ac770e2aae8a730c127cd9f0d68b))
- **ui:** PRクリーンアップ実行中のフィードバックを改善 ([caa19eb](https://github.com/akiojin/claude-worktree/commit/caa19ebaa341e34dad5dc3a3629a13890f329a5a))
- **ui:** 即時スピナー更新と入力ロックのレスポンス改善 ([43d7577](https://github.com/akiojin/claude-worktree/commit/43d75776a0c6fd007e17004aac1c499668808b48))

## [1.3.1](https://github.com/akiojin/claude-worktree/compare/v1.3.0...v1.3.1) (2025-10-26)

### Bug Fixes

- Bunテスト互換のモック復元処理を整備 ([68f46a0](https://github.com/akiojin/claude-worktree/commit/68f46a0895e673dcfe9db8a7c3ebeb156c2529dd))
- Ink UIショートカットの動作を修正 ([038323b](https://github.com/akiojin/claude-worktree/commit/038323ba1a5d076d66f027bea5bfc61888c9ce01))

# [1.3.0](https://github.com/akiojin/claude-worktree/compare/v1.2.1...v1.3.0) (2025-10-26)

### Features

- SPEC-6d501fd0仕様・計画・タスクの詳細化と品質分析 ([7ff3aa0](https://github.com/akiojin/claude-worktree/commit/7ff3aa03ffa3f3846c8d94d11ae6dcf44ca3498a))

## [1.2.1](https://github.com/akiojin/claude-worktree/compare/v1.2.0...v1.2.1) (2025-10-26)

### Bug Fixes

- Spec Kitのブランチ自動作成を無効化 ([a459682](https://github.com/akiojin/claude-worktree/commit/a459682610fd0553e41bde6815fcef7d68509c3d))

# [1.2.0](https://github.com/akiojin/claude-worktree/compare/v1.1.0...v1.2.0) (2025-10-26)

### Bug Fixes

- Docker環境でのGitリポジトリ検出エラーメッセージを改善 ([338d626](https://github.com/akiojin/claude-worktree/commit/338d626f73d0140af1844b522b710934fc0588d0))
- LintワークフローにMarkdownlintを統合 ([55f446e](https://github.com/akiojin/claude-worktree/commit/55f446ee71a45d958e1b5d5a2e7b74d8047e1a54))
- WorktreeConfig型のエクスポートとフォーマット修正 ([13252a2](https://github.com/akiojin/claude-worktree/commit/13252a2c2e246669212d092e831b3debedd92072))
- WorktreeディレクトリでのisGitRepository()動作を修正 ([4d36898](https://github.com/akiojin/claude-worktree/commit/4d3689846380acc5d2a0075bea41665588550a7e))
- エラー表示にデバッグモード時のスタックトレース表示を追加 ([dd68436](https://github.com/akiojin/claude-worktree/commit/dd68436a0707d0dd808a7a4b0fb076e1b3e757d0))
- リモートブランチ表示のアイコン幅を調整 ([5b7fc35](https://github.com/akiojin/claude-worktree/commit/5b7fc354d3f9dad786ebad06cce74d543e700c4d))
- リリースワークフローの認証設定を追加 ([52683cf](https://github.com/akiojin/claude-worktree/commit/52683cf150cc2be9c4e8d76dbff68cbf89800aac))

### Features

- add git loading indicator with tdd coverage ([#104](https://github.com/akiojin/claude-worktree/issues/104)) ([1432d06](https://github.com/akiojin/claude-worktree/commit/1432d064448cb9496d5fcd3b9e470c6b2ff8c28d))
- skipPermissions選択機能とAIツール終了後のメイン画面復帰を実装 ([63f6f7d](https://github.com/akiojin/claude-worktree/commit/63f6f7db0987fe8d89a63a98bcaf689fa6ec4247))
- ブランチ作成機能を実装（FR-007完全対応） ([88633bf](https://github.com/akiojin/claude-worktree/commit/88633bf10f72680d47dd8efd1014fdc958c99f04))
- ブランチ選択後のワークフロー統合（AIツール選択→実行モード選択→起動） ([fbea71c](https://github.com/akiojin/claude-worktree/commit/fbea71cef09807b19dbfb2cd73f95c79e1fa691e))

# [1.1.0](https://github.com/akiojin/claude-worktree/compare/v1.0.0...v1.1.0) (2025-10-26)

### Bug Fixes

- CHANGELOG.mdの全リストマーカーをアスタリスクに統一 ([d39c01b](https://github.com/akiojin/claude-worktree/commit/d39c01b4de28d018565f336d3a2e06b178ba5920))
- CIエラーを修正（Markdown Lint + Test） ([b9e8b50](https://github.com/akiojin/claude-worktree/commit/b9e8b50a5b8f1fdc7d5c9a2755ca4e09306720ee))
- CIエラー修正（Markdown LintとVitest mock） ([edef82e](https://github.com/akiojin/claude-worktree/commit/edef82e6a721bd87d847172619ccaa1fd8238736))
- Ink.js UIのブランチ表示位置とキーボード操作を修正 ([d88108b](https://github.com/akiojin/claude-worktree/commit/d88108b7458100d207ced5717e1ed1bfb730a3c1))
- qキーとEnterキーが正常に動作するように修正 ([f2cb6b5](https://github.com/akiojin/claude-worktree/commit/f2cb6b5093d53e9cd2069f03a7b03a56ba7f48c3))
- vi.hoistedエラーを修正してテストを全て成功させる ([6b8cac0](https://github.com/akiojin/claude-worktree/commit/6b8cac0d1d585734d2ea49ad255560c707c0e0fe))
- キーボードハンドリング競合とWorktreeアイコン表示を修正 ([2ea4624](https://github.com/akiojin/claude-worktree/commit/2ea46241bd5830d029a8fae5d74ae7522f101e36))

### Features

- カーソルのループ動作を無効化したカスタムSelectコンポーネントを実装 ([10920ce](https://github.com/akiojin/claude-worktree/commit/10920ce0db6f383e26264fa6f69e8dcc0cb4e909))
- カスタムSelectコンポーネントのテスト実装とUI 5カラム表示構造への修正 ([8b65385](https://github.com/akiojin/claude-worktree/commit/8b6538538fd3fb218e86b0f1e85589b0e504d307))

## 1.0.0 (2025-10-26)

### Bug Fixes

- Claude Codeコマンドが見つからない場合の適切なエラーハンドリングを追加 ([372c123](https://github.com/akiojin/claude-worktree/commit/372c1234c18d253c5e67e7f7f3dacb9e4665f260))
- Claude Codeのnpmパッケージ名を修正 ([054b68f](https://github.com/akiojin/claude-worktree/commit/054b68fd59a329b2a1d07085a15473499dc0fe1f))
- Claude Code実行モード選択でqキーで戻れる機能を追加 ([facd56b](https://github.com/akiojin/claude-worktree/commit/facd56b7d7040b0dd5b80e80bc07e16cc1632b91))
- Claude Code実行モード選択でqキー対応とUI簡素化 ([bf418ec](https://github.com/akiojin/claude-worktree/commit/bf418ec6a4c3c804ca641cab5f96be338206be5d))
- Claude Code実行時のエラーハンドリングを改善 ([a17b143](https://github.com/akiojin/claude-worktree/commit/a17b14320b4dac1f546bb364a26d19bf2d2908dd))
- Claude Code実際の表示形式に合わせて履歴表示を修正 ([bc3cb11](https://github.com/akiojin/claude-worktree/commit/bc3cb11bc33aa23d4bd36ceeb1b98e98ebcbf6b1))
- Claude Code履歴タイトル表示を根本的に改善 ([12679da](https://github.com/akiojin/claude-worktree/commit/12679da7c5a870531f8d7c42f1c6fbd2b96bf81d))
- Claude Code履歴メッセージ構造に対応したタイトル抽出 ([86b06a2](https://github.com/akiojin/claude-worktree/commit/86b06a2647480f4eaedcc162b0ce55082483b814))
- Claude Code履歴表示でタイトルがセッションIDしか表示されない問題を修正 ([5f2543f](https://github.com/akiojin/claude-worktree/commit/5f2543f12edabbf07af244498d2b8f084507a4d6))
- claude コマンドのPATH解決問題を修正 ([a9f4627](https://github.com/akiojin/claude-worktree/commit/a9f462795f580a725dd57b87783da103058a494a))
- CLAUDE.mdをclaude-worktreeプロジェクトに適した内容に修正 ([1feda3e](https://github.com/akiojin/claude-worktree/commit/1feda3e1353572818d18b2fe0ee42a04deb8ce4d))
- claude.test.tsをbun vitest互換に書き直し ([50e9f31](https://github.com/akiojin/claude-worktree/commit/50e9f31ac390bfca419f864f56d535101cfe5bcb))
- CodeRabbitレビューコメントへの対応 ([e328a43](https://github.com/akiojin/claude-worktree/commit/e328a43da01f9b50bb8259efb924fda4c2854ca4))
- Codex CLI の resume --last への統一 ([62c1b5a](https://github.com/akiojin/claude-worktree/commit/62c1b5a2deac2358c7cd78a39716649bee028639))
- Codexの権限スキップフラグ表示を修正 ([6143d42](https://github.com/akiojin/claude-worktree/commit/6143d424b7d284b29f3254b3cdb9593bbbf6672d))
- **codex:** 承認/サンドボックス回避フラグをCodex用に切替 ([e0ccb2a](https://github.com/akiojin/claude-worktree/commit/e0ccb2aaa353f0eebeae073abaee372f39b67427))
- developブランチが存在しない場合にmainブランチから分岐するように修正 ([f31bafa](https://github.com/akiojin/claude-worktree/commit/f31bafab0e680201d3751e274f3511d77a222d54))
- Dockerコンテナのentrypoint.shエラーを修正 ([88931d6](https://github.com/akiojin/claude-worktree/commit/88931d63af8669ae08c20a1b06c7614925baa60f))
- Docker環境でのClaude Code実行時のパス問題を修正 ([e55a1b0](https://github.com/akiojin/claude-worktree/commit/e55a1b00f1fcfc093b7ba402c4269b62882a2f69))
- ESLint、Prettier、Markdown Lintのエラーを修正 ([73e3d79](https://github.com/akiojin/claude-worktree/commit/73e3d7957287e1b217669ed8ab4d8ae4c895d38f))
- ESLintエラーを修正 ([d57630d](https://github.com/akiojin/claude-worktree/commit/d57630da1baa4a7548af25fc5771507dce794a2f))
- GitHub CLI認証チェックを修正 ([ffe0834](https://github.com/akiojin/claude-worktree/commit/ffe08349eda1b63405f6b94886f4580b9f112b5b))
- GraphQL APIで自動マージを実行 ([e5f4346](https://github.com/akiojin/claude-worktree/commit/e5f43462b90999c806c54d1913feb0093d835131))
- Ink.js UIの表示とキーボードハンドリングを修正 ([264e750](https://github.com/akiojin/claude-worktree/commit/264e75024b32561c921ada3b9f93565c9aba5543))
- mainブランチから追加されたclaude.test.tsを一時スキップ（bun vitest互換性問題） ([1be1cbf](https://github.com/akiojin/claude-worktree/commit/1be1cbfbd42f81c2e924c7a8bab130ce16ee5fc3))
- markdownlintエラー修正（裸のURL） ([cc74f33](https://github.com/akiojin/claude-worktree/commit/cc74f3393fade609b6bcb3a7536508cce322a641))
- markdownlint違反のエスケープを追加 ([6dc46ed](https://github.com/akiojin/claude-worktree/commit/6dc46edf18962248c994fa431dde37a69141d993))
- node_modulesをmarkdownlintから除外 ([3881b47](https://github.com/akiojin/claude-worktree/commit/3881b478081613c47bc236dde349d901741b3e6c))
- node:fs/promisesとexecaのmockにdefault exportを追加 ([a494c55](https://github.com/akiojin/claude-worktree/commit/a494c55669c80a32a38481e0bfdbcbf50d781d6d))
- npm versionエラーの詳細情報を出力するよう改善 ([b14eaf8](https://github.com/akiojin/claude-worktree/commit/b14eaf841084720363a25a0f2d28f1e165988049))
- npm versionコマンドのエラーハンドリングを改善 ([844cbb7](https://github.com/akiojin/claude-worktree/commit/844cbb7892d6b8de616b011a817fedb7ed4a0685))
- package.jsonの名前を"akiojin/claude-worktree"に変更 ([3f42034](https://github.com/akiojin/claude-worktree/commit/3f420348be83b2c3388faec8a68b8e6b1fe81d9e))
- package.jsonの名前を変更 ([ff406eb](https://github.com/akiojin/claude-worktree/commit/ff406ebc4bd226f67fc62bdecf0b55068dd02374))
- Remove unnecessary '.' argument when launching Claude Code ([28efd7a](https://github.com/akiojin/claude-worktree/commit/28efd7a4e2dbe2529d1ab6aa7345f4f367052ea4))
- session-resume.test.ts の node:os mock に default export を追加 ([f09d7b6](https://github.com/akiojin/claude-worktree/commit/f09d7b6bdbf5fe13d1092102a2cb2a54de572b0f))
- string-width negative value error by adding Math.max protection ([74df748](https://github.com/akiojin/claude-worktree/commit/74df748c6808347217b7b1b6786799ba789971e3))
- stripAnsi関数の位置を修正してimport文の後に移動 ([fe1d43f](https://github.com/akiojin/claude-worktree/commit/fe1d43f81e1ce2eedefc7087ba1da8a146fd1c13))
- T094-T095完了 - テスト修正とフィーチャーフラグ変更 ([d40d8d9](https://github.com/akiojin/claude-worktree/commit/d40d8d9153ac84c702f5e2d72a91c1095dcf0e37))
- UI表示とタイトル抽出の問題を修正 ([b67ec29](https://github.com/akiojin/claude-worktree/commit/b67ec29810075169325d48c928d7b5519f83a24d))
- Windows環境でのClaude Code起動エラーを改善 ([676a0f2](https://github.com/akiojin/claude-worktree/commit/676a0f215945a76e9b8dae2d685707917dae0866))
- Windows環境でのnpx実行エラーを修正 ([0f0075b](https://github.com/akiojin/claude-worktree/commit/0f0075bee654c9d86c57d9d32718c136f9e51c56))
- worktreeパス衝突時のエラーハンドリングを改善 ([#79](https://github.com/akiojin/claude-worktree/issues/79)) ([602008c](https://github.com/akiojin/claude-worktree/commit/602008ccaac99ef394c0c92bbfecbaf9da1fbf28))
- worktree内での実行時の警告表示とパス解決の改善 ([5de57da](https://github.com/akiojin/claude-worktree/commit/5de57dad5208264bb1ac3336506a9c8808a4712d))
- アカウント切り替え機能のデバッグとUI改善 ([0546015](https://github.com/akiojin/claude-worktree/commit/05460156f954cba94d72ad1a0c9aac1653d57fbb))
- アカウント管理UIの改善 ([2c480b9](https://github.com/akiojin/claude-worktree/commit/2c480b9d135943d4cf9184717a85357da2ff94ca))
- ウェルカムメッセージの枠線表示を修正 ([d853d07](https://github.com/akiojin/claude-worktree/commit/d853d076bb7799df45dc8ff7de4055cd4f3bda73))
- ウェルカムメッセージの枠線表示を長いバージョン番号に対応 ([0994961](https://github.com/akiojin/claude-worktree/commit/09949615f479ec0e7c48651300a67cc9ef37b206))
- エラー発生時にユーザー入力を待機するように修正 ([ea34168](https://github.com/akiojin/claude-worktree/commit/ea34168068e1887bbfaf6fb30cc870dbf6a69af4))
- カラム名（ヘッダー）が表示されない問題を修正 ([42b720b](https://github.com/akiojin/claude-worktree/commit/42b720bf73f1bd8e5496615325a76b600021e315))
- タイトル抽出ロジックをシンプル化し、ブランチ記録機能を削除 ([c565b51](https://github.com/akiojin/claude-worktree/commit/c565b51aa353085a15c7f1e155b249954f8c516d))
- バージョン番号表示による枠線のズレを修正 ([65820be](https://github.com/akiojin/claude-worktree/commit/65820be525bf409058598ff5e16352f59a72163c))
- ビルドエラーを修正 ([a252b1d](https://github.com/akiojin/claude-worktree/commit/a252b1d7b07ebe85f5395cbef6ab1b65fab6d9be))
- プレビュー表示前に画面をクリアして見やすさを改善 ([a4d2f56](https://github.com/akiojin/claude-worktree/commit/a4d2f56f6a8b048c0fe9a1e587cf44b474d43c11))
- マージ済みPRクリーンアップ画面でqキーで前の画面に戻れるように修正 ([b87e5a6](https://github.com/akiojin/claude-worktree/commit/b87e5a66c3cfd65d4025565858487086d2fee363))
- リアルタイム更新テストの安定性向上 ([e81e900](https://github.com/akiojin/claude-worktree/commit/e81e9009e0b816bbc4e4e25ffe1bb17ad58b7e68))
- リモートブランチ選択時にローカルブランチが存在しない場合の不具合を修正 ([9910253](https://github.com/akiojin/claude-worktree/commit/9910253e984847209490f01697c3c639398795c6))
- リリースブランチの2つの問題を修正 ([189f6f1](https://github.com/akiojin/claude-worktree/commit/189f6f182ff8db77a53a27d067fdce60a53405cc))
- リリースブランチ作成フローを完全に修正 ([e117b56](https://github.com/akiojin/claude-worktree/commit/e117b56c0e4063481676ed6bc155731bb0391215))
- リリースブランチ検出を正確にするため実際のGitブランチ名を使用 ([ba11b3e](https://github.com/akiojin/claude-worktree/commit/ba11b3e338ba49802893ade8d175cebd5cec5f4c))
- 会話タイトルを最後のメッセージから抽出するように改善 ([e459a67](https://github.com/akiojin/claude-worktree/commit/e459a67956d8b5a79040f24e175306d82dd38a8f))
- 会話プレビューで最新メッセージが見えるように表示順序を改善 ([533f680](https://github.com/akiojin/claude-worktree/commit/533f6805bbc58a7f14dc6a2f245ff5a2c5a08cdc))
- 会話プレビューの「more messages above」を「more messages below」に修正 ([948aaf1](https://github.com/akiojin/claude-worktree/commit/948aaf114ba01b6321eda8f193155e77cb3c3b2d))
- 会話プレビューの表示順序を通常のチャット形式に修正 ([ee69b18](https://github.com/akiojin/claude-worktree/commit/ee69b18d2cce1b6b8c592e16f764fa5b83ec5faf))
- 保護対象ブランチ(main, master, develop)をクリーンアップから除外 ([bac6904](https://github.com/akiojin/claude-worktree/commit/bac690444da6416c52249cd6f239b26caa536ecb))
- 修正と設定の更新 ([2ce23af](https://github.com/akiojin/claude-worktree/commit/2ce23af53ebf4cf26d42d476070298296cafe534))
- 全画面でqキー統一操作に対応 ([62162d8](https://github.com/akiojin/claude-worktree/commit/62162d83b95a9f26b6021733e082043e2a6a0bb2))
- 履歴選択キャンセル時にメニューに戻るように修正 ([59dc3e1](https://github.com/akiojin/claude-worktree/commit/59dc3e1df6fa4b6430a048efb31b98ba657811a7))
- 改行コードをLFに統一 ([cb74c1b](https://github.com/akiojin/claude-worktree/commit/cb74c1bf65f8ebb96557e39180ed4185813198f3))
- 新規Worktree作成時にClaude CodeとCodex CLIを選択可能にする (SPEC-473b3d47 FR-008対応) ([633e91b](https://github.com/akiojin/claude-worktree/commit/633e91b9286b59c322d0b7b644eae2e588f26b10))
- 未使用のインポートを削除 ([0d01774](https://github.com/akiojin/claude-worktree/commit/0d01774db4b74ce24a30430547925919b50e8d8f))
- 残り全テストファイルのmock問題を修正 ([4a69fff](https://github.com/akiojin/claude-worktree/commit/4a69fffe4bac4e26198bb8e27cf9745aa737cea4)), closes [#91](https://github.com/akiojin/claude-worktree/issues/91)
- 独自履歴選択後のclaude -r重複実行を修正 ([fc53467](https://github.com/akiojin/claude-worktree/commit/fc534673709a9cacdd26dc82f5eaf7df8b0bb31d))
- 現在のブランチがCURRENTとして表示されない問題を修正 ([f75cbdc](https://github.com/akiojin/claude-worktree/commit/f75cbdc6fab01663e3161a0cdc139f1a2d34a3ef))
- 自動マージワークフローのトリガー条件を修正 ([baedfeb](https://github.com/akiojin/claude-worktree/commit/baedfeb69b7867ce717305cd2efad33d1cb23f64))

### Features

- -cパラメーターによる前回セッション継続機能を追加 ([d3d300a](https://github.com/akiojin/claude-worktree/commit/d3d300aa10a45eadb4e002edd3d318e4ef9263f1))
- -rパラメーターによるセッション選択機能を追加 ([976e1f4](https://github.com/akiojin/claude-worktree/commit/976e1f4c9c8f790aa2aeb2180e87226306f3bb8c))
- .gitignoreと.mcp.jsonの更新、docker-compose.ymlから不要な環境変数を削除 ([f77670c](https://github.com/akiojin/claude-worktree/commit/f77670ce0263053b0087d258b61cdf5c9df1d1ac))
- @akiojin/spec-kitを導入し、仕様駆動開発をサポート ([1df357c](https://github.com/akiojin/claude-worktree/commit/1df357c53b9837e52b41d732253ef7e10040d7d5))
- Add change tracking and post-Claude Code change management ([a80ed2b](https://github.com/akiojin/claude-worktree/commit/a80ed2b4013a56fd7967ab4c9a2b19701d9a30ad))
- add Spec Kit ([2b9db16](https://github.com/akiojin/claude-worktree/commit/2b9db16934a9874f79abd04c6bfb40bef6fdefd5))
- AIツール選択（Claude/Codex）機能を実装 ([dfd4a4c](https://github.com/akiojin/claude-worktree/commit/dfd4a4cd2cdfbfce5a2d105444e56f097d5e3f37))
- **auto-merge:** PR番号取得、マージ可能性チェック、PRマージステップを実装 (T004-T006) ([fb1d993](https://github.com/akiojin/claude-worktree/commit/fb1d993fe520d7355820bd55e09199644540c166))
- claude -rの表示を大幅改善 ([2ea7147](https://github.com/akiojin/claude-worktree/commit/2ea7147a75c42e52fff2b604ee3af670d04aa036))
- claude -rをグルーピング形式で大幅改善 ([097c01b](https://github.com/akiojin/claude-worktree/commit/097c01b31394a21944832f6f1d744f9ac750a405))
- Claude Codeアカウント切り替え機能を追加 ([5b2762a](https://github.com/akiojin/claude-worktree/commit/5b2762a939746a2d3b6b035b56010e34385f836f))
- Claude CodeをnpxからbunxへComplete移行（SPEC-c0deba7e） ([9094c2d](https://github.com/akiojin/claude-worktree/commit/9094c2d2ee6263e94fd3c41f6663dd988b21790b))
- Claude Code履歴を参照したresume機能を実装 ([4dbdb47](https://github.com/akiojin/claude-worktree/commit/4dbdb47bd1d0966d4bb7413395f42c9c07929086))
- Codex CLIのbunx対応とresumeコマンド整備 ([a4f76d7](https://github.com/akiojin/claude-worktree/commit/a4f76d74403284f7971a48d6ff965a3d0729ab82))
- Codex CLI対応の仕様と実装計画を追加 ([3e9644f](https://github.com/akiojin/claude-worktree/commit/3e9644ff1a2d4b1a759652b3494365cafa45168f))
- docker-compose.ymlにNPMのユーザー情報を追加 ([cf5c343](https://github.com/akiojin/claude-worktree/commit/cf5c343c0f98bd26923aa5ccaefeccf2df8e2cef))
- Docker/root環境でClaude Code自動承認機能を追加 ([cf4e96b](https://github.com/akiojin/claude-worktree/commit/cf4e96bf460b6d382f3da7576083c1308a995bba))
- Git Flowに準拠したリリースブランチ作成機能を実装 ([ad731dd](https://github.com/akiojin/claude-worktree/commit/ad731dd5278b33062bee284377843de2f5870503))
- GitHub CLIのインストールをDockerfileに追加 ([bcfb709](https://github.com/akiojin/claude-worktree/commit/bcfb709d5595efd188a034e704f25ea101ccbe98))
- Git認証設定をentrypoint.shに追加 ([544b5a0](https://github.com/akiojin/claude-worktree/commit/544b5a075c0bc9d9fca52420a0bd985b6d2681e1))
- Initial package structure for claude-worktree ([3f02b3d](https://github.com/akiojin/claude-worktree/commit/3f02b3dcfd92023f2a019a8e0122dc6dd5a215f5))
- Ink.js UI移行のPhase 1完了（セットアップと準備） ([f460e65](https://github.com/akiojin/claude-worktree/commit/f460e65d7237b6d4478af56c5d2e476cc9f13bbb))
- npm versionコマンドと連携したリリースブランチ作成機能を実装 ([897686a](https://github.com/akiojin/claude-worktree/commit/897686ace32544af0adb3ae81e6c491452f6e7b1))
- npx経由でAI CLIを起動するよう変更 ([5e53a2f](https://github.com/akiojin/claude-worktree/commit/5e53a2fb11acc1b4e861f85b7a9c7f12960aa42d))
- Phase 2 開始 - 型定義拡張とカスタムフック実装（進行中） ([93a797e](https://github.com/akiojin/claude-worktree/commit/93a797e56254e6f9231aec8d12bdf6c440d120f4))
- Phase 2基盤実装 - カスタムフック（useTerminalSize, useScreenState） ([80849bc](https://github.com/akiojin/claude-worktree/commit/80849bc2af2fa3aadf2a1a72546c091445eebce7))
- Phase 2基盤実装 - 共通コンポーネント（ErrorBoundary, Select, Confirm, Input） ([7795a4e](https://github.com/akiojin/claude-worktree/commit/7795a4ecbc70601443b9c9f87c669fa4bb3d2ff9))
- Phase 2基盤実装完了 - UI部品コンポーネント（Header, Footer, Stats, ScrollableList） ([d8cd2e2](https://github.com/akiojin/claude-worktree/commit/d8cd2e2b3ddc96fb4a87a8ccb632f7310da534cd))
- Phase 3 T038-T041完了 - BranchListScreen実装 ([ea94fc4](https://github.com/akiojin/claude-worktree/commit/ea94fc4319114359f878db2e3386098f004cee06))
- Phase 3 T042-T044完了 - App component統合とフィーチャーフラグ実装 ([7955a1d](https://github.com/akiojin/claude-worktree/commit/7955a1dac9229594bd8f88b82c086eba6ada706f))
- Phase 3 完了 - 統合テスト・受け入れテスト実装（T045-T051） ([59a2e75](https://github.com/akiojin/claude-worktree/commit/59a2e750143177953302909846f897fcdf1b8c3a))
- Phase 3実装 - useGitDataフック（Git情報取得） ([6464bd6](https://github.com/akiojin/claude-worktree/commit/6464bd68a3fecd62289b6bccffa23347ae4f6904))
- Phase 3開始 - データ変換ロジック実装（branchFormatter, statisticsCalculator） ([b023c3c](https://github.com/akiojin/claude-worktree/commit/b023c3c6124da1724ddaf559e6c8a72074cfcfd3))
- Phase 4 開始 - 画面遷移とWorktree管理画面実装（T052-T055） ([3da7d42](https://github.com/akiojin/claude-worktree/commit/3da7d42e774d0dd42fc9a91f733d7cf09cb6df43))
- Phase 6完了 - Ink.js UI移行成功（成功基準7/8達成） ([c55315f](https://github.com/akiojin/claude-worktree/commit/c55315ff87ccdcf14d3321fbbc45f0c41dd5a747))
- Repository Statisticsセクションを削除 ([bbaf962](https://github.com/akiojin/claude-worktree/commit/bbaf962270bbdf365c01ae85f4f0da44401174b6))
- Repository Statisticsの表デザインを改善 ([178e515](https://github.com/akiojin/claude-worktree/commit/178e51511d1a64af6c3bdcd6960fa8a58869f388))
- Repository Statistics表示をよりコンパクトで見やすいデザインに改善 ([1585685](https://github.com/akiojin/claude-worktree/commit/15856859229114b0d9f7321236565ce73bedc6e6))
- resume機能を大幅強化 ([0e631dd](https://github.com/akiojin/claude-worktree/commit/0e631ddcb95d15de6f1a2aa5dddad56f731fb9d3))
- semantic-release自動リリース機能を実装 ([90fc53e](https://github.com/akiojin/claude-worktree/commit/90fc53e039c039584624a0416b74b2d1ef77c668))
- semantic-release設定を明示化 ([35480fe](https://github.com/akiojin/claude-worktree/commit/35480feb0937aaab4aabdf15fa9b17d52ff6d2a0))
- **specify:** ブランチを作成しない運用へ変更 ([7c809e7](https://github.com/akiojin/claude-worktree/commit/7c809e792995e1ca03e6edadb4b37ebff8e39498))
- T056完了 - WorktreeManager画面遷移統合（mキー） ([2d5c625](https://github.com/akiojin/claude-worktree/commit/2d5c625c5a01d0dc4fdf365fca05d11547a565ad))
- T057-T059完了 - BranchCreatorScreen実装と統合 ([a90b3a8](https://github.com/akiojin/claude-worktree/commit/a90b3a817b1f628db711b3491cf644ac8f59d2f3))
- T060-T062完了 - PRCleanupScreen実装と統合 ([f545f32](https://github.com/akiojin/claude-worktree/commit/f545f32ce0d9f48dd370d277c9a53c39519c3145)), closes [#123](https://github.com/akiojin/claude-worktree/issues/123)
- T063-T071完了 - 全サブ画面実装完了（Phase 4 サブ画面実装完了） ([f8cca21](https://github.com/akiojin/claude-worktree/commit/f8cca214af69cf27c041fe697a6c2e12de0af584))
- T072-T076完了 - Phase 4完全完了！（統合テスト・受け入れテスト実装） ([3c7b8fa](https://github.com/akiojin/claude-worktree/commit/3c7b8fa74cc311aa2a7ba3de594e30dda7e08a23))
- T077-T080完了 - リアルタイム更新機能実装 ([c16c219](https://github.com/akiojin/claude-worktree/commit/c16c21924ce1e765ce5705402c52373c3af53dee))
- T081-T084完了 - パフォーマンス最適化と統合テスト実装 ([50c4868](https://github.com/akiojin/claude-worktree/commit/50c4868248ad7aa1d4a931e6fdf2ed381a999dfb))
- T085-T086完了 - Phase 5完全完了！リアルタイム更新機能実装完了 ([a5a9c4b](https://github.com/akiojin/claude-worktree/commit/a5a9c4bfd221347bce37416736743e6c937692d0))
- T096完了 - レガシーUIコード完全削除 ([f63cb4b](https://github.com/akiojin/claude-worktree/commit/f63cb4bfeb528732f38fa262ab915cae6ed48143))
- T097完了 - @inquirer/prompts依存削除 ([4775588](https://github.com/akiojin/claude-worktree/commit/4775588f4b00252d27d191355475f2f590b44704))
- tasks.mdにCI/CD検証タスク（T105-T106）を追加 & markdownlintエラーを修正 ([1c831d4](https://github.com/akiojin/claude-worktree/commit/1c831d47eba1f225fa0fcae31c11b6f191a6a080)), closes [#89](https://github.com/akiojin/claude-worktree/issues/89)
- UIの改善と表示形式の更新 ([db6ab2a](https://github.com/akiojin/claude-worktree/commit/db6ab2a4e7c92f171e65071cebc95dc0eda58609))
- worktreeに存在しないローカルブランチのクリーンアップ機能を追加 ([4584085](https://github.com/akiojin/claude-worktree/commit/4584085a943c07b53d3416116f69749f15e17492))
- worktree削除時にローカルブランチをリモートにプッシュする機能を追加 ([32a6fae](https://github.com/akiojin/claude-worktree/commit/32a6fae00758a1564be2dde7c43d216c0fbf0c34))
- worktree選択後にClaude Code実行方法を選択できる機能を追加 ([02cd48d](https://github.com/akiojin/claude-worktree/commit/02cd48d90384dd7cb3ed6aa3cfa3efa6bb995500))
- アクセスできないworktreeを明示的に表示し、pnpmへ移行 ([28b0912](https://github.com/akiojin/claude-worktree/commit/28b091260f199e227f68b807f4edfb2af7fa2256))
- キーボードショートカット機能とブランチ名省略表示を実装 ([cf0ac51](https://github.com/akiojin/claude-worktree/commit/cf0ac5186860ceb8ddf5e36805f864f42749c8b0))
- クリーンアップ時にリモートブランチも削除する機能を追加 ([ce61f89](https://github.com/akiojin/claude-worktree/commit/ce61f8934ea1dc0e64bc7e1c1b3bf1dac2717795))
- クリーンアップ時の表示メッセージを改善 ([6d782d8](https://github.com/akiojin/claude-worktree/commit/6d782d8d0184ed8c950292864d5490b7dee40c0c))
- ツール引数パススルーとエラーメッセージを追加 ([e5ef2e1](https://github.com/akiojin/claude-worktree/commit/e5ef2e1558edae3da306bc0108580e1e0f8f3c8d))
- テーブル表示にカラムヘッダーを追加 ([8194570](https://github.com/akiojin/claude-worktree/commit/8194570cb157542cf09935bd6c519e33be29f863))
- バージョン番号をタイトルに表示 ([4d2c08f](https://github.com/akiojin/claude-worktree/commit/4d2c08fb5757376f7c796ff07f1891eba933ab32))
- ブランチ一覧のソート優先度を整理 ([ef29325](https://github.com/akiojin/claude-worktree/commit/ef29325eb11aa1863a63c0a280d0d3665ef4c0a5))
- ブランチ選択UIと操作メニューの視覚的分離を改善 ([0fef2d8](https://github.com/akiojin/claude-worktree/commit/0fef2d8f2dc023f803c467de2bb257115e406a25))
- ブランチ選択カーソル視認性向上 (SPEC-822a2cbf) ([ce2ece5](https://github.com/akiojin/claude-worktree/commit/ce2ece5d6c9ae20f797b358a98cf0635338d642b))
- マージ済みPRクリーンアップ機能の改善 ([c4b5bf8](https://github.com/akiojin/claude-worktree/commit/c4b5bf8269d4708dee7af4efcbc4cc20c3fa12c5))
- マージ済みPRのworktreeとブランチを削除する機能を追加 ([b43937f](https://github.com/akiojin/claude-worktree/commit/b43937f495670b5a01c43d3d3dcd0d39c75356e0))
- メッセージプレビュー表示を大幅改善 ([b2947eb](https://github.com/akiojin/claude-worktree/commit/b2947ebbe3888dfc4d604155d27b72b180f936a4))
- リモートブランチ削除を選択可能にする機能を追加 ([7c604a9](https://github.com/akiojin/claude-worktree/commit/7c604a99af3fc6fe0996fb23a83c4c4473eb7322))
- リリースブランチの自動化を強化 ([8c290f6](https://github.com/akiojin/claude-worktree/commit/8c290f607986de9c75dd2b9f91662ae21311fb60))
- リリースブランチ完了時のworktreeとローカルブランチ自動削除機能を追加 ([d1cddcf](https://github.com/akiojin/claude-worktree/commit/d1cddcf81d34f7e132e42d9c6f39ac784502ee87))
- リリースブランチ終了時に選択肢を提供 ([f4fc85e](https://github.com/akiojin/claude-worktree/commit/f4fc85e8e24336353e06f6f933c22888a43c3536))
- 全画面でqキー統一操作に変更 ([206be92](https://github.com/akiojin/claude-worktree/commit/206be92733416237449378b21e8b766cd1e06944))
- 全画面活用の拡張プレビュー機能を実装 ([409dc53](https://github.com/akiojin/claude-worktree/commit/409dc5306971ee7a4edc01cdbb7585f3811d61bf))
- 新機能の追加と既存機能の改善 ([b10eb29](https://github.com/akiojin/claude-worktree/commit/b10eb29b74d96b1050b37f989e7b1c5df6a7fa88))
- 既存実装に対する包括的な機能仕様書を作成（SPEC-473b3d47） ([bca3698](https://github.com/akiojin/claude-worktree/commit/bca369840855bbff2c235d16b6622af9da69dda6))
- 時間表示を削除してccresume風のプレビュー表示に改善 ([6a7fe4c](https://github.com/akiojin/claude-worktree/commit/6a7fe4ca5e564279255352f25c9278cf2a31f7df))
- 表デザインをモダンでより見やすいスタイルに改善 ([e97351d](https://github.com/akiojin/claude-worktree/commit/e97351dc0013790a42064b3b34cbcab8eba4b229))
- 表デザインをモダンでより見やすいスタイルに改善 ([ca9b2f7](https://github.com/akiojin/claude-worktree/commit/ca9b2f7328acff3259f8762c7bf386f57489408a))

### Reverts

- Claude Codeアカウント切り替え機能を完全に削除 ([7c87127](https://github.com/akiojin/claude-worktree/commit/7c87127a6a39c44561966d833be0733de137ff14))

## [Unreleased]

### Added

- **`.releaserc.json` による semantic-release 設定の明示化**
  - デフォルト設定への暗黙的な依存を排除
  - リリースプロセスの可視化と保守性向上
  - 全6つのプラグイン設定を明示的に定義 (commit-analyzer, release-notes-generator, changelog, npm, git, github)
- semantic-release と必要なプラグインを devDependencies に追加
- 完全なテストカバレッジ（104+ tests）
  - ユニットテスト: Git operations, Worktree management, UI components
  - 統合テスト: Branch selection, Remote branch handling, Branch creation workflows
  - E2Eテスト: Complete user workflows
- 包括的なドキュメント
  - API documentation (docs/api.md)
  - Architecture documentation (docs/architecture.md)
  - Contributing guidelines (CONTRIBUTING.md)
  - Troubleshooting guide (docs/troubleshooting.md)

### Changed

- **🎨 UI Framework Migration to Ink.js (React-based)**: Complete redesign of CLI interface
  - **Before**: inquirer/chalk-based UI (2,522 lines)
  - **After**: Ink.js v6.3.1 + React v19.2.0 (113 lines in index.ts, 92.7% reduction)
  - **Benefits**:
    - Full-screen layout with persistent header, scrollable content, and fixed footer
    - Real-time statistics updates without screen refresh
    - Smooth terminal resize handling
    - Component-based architecture for better maintainability
    - 81.78% test coverage achieved
  - **Dependencies Removed**: @inquirer/core, @inquirer/prompts (2 packages)
  - **Dependencies Added**: ink, react, ink-select-input, ink-text-input
  - **Code Quality**: Simplified from 2,522 lines to ~760 lines (70% reduction target achieved)
- **リリースプロセスのドキュメント化**
  - README.md にリリースプロセスセクションを追加
  - Conventional Commits のガイドライン記載
  - semantic-release の動作説明を追加
  - .releaserc.json の詳細説明を追加
  - リリースプロセスガイド (specs/SPEC-23bb2eed/quickstart.md) へのリンク追加
- テストフレームワークをVitestに移行
- CI/CDパイプラインの強化
- **bunx移行**: Claude Code起動方式をnpxからbunxへ完全移行
  - Claude Code: `bunx @anthropic-ai/claude-code@latest`で起動
  - Codex CLI: 既存のbunx対応を維持
  - UI表示文言をbunx表記へ統一

### Breaking Changes

- **Bun 1.0+が必須**: Claude Code起動にはBun 1.0.0以上が必要
- npx対応の廃止: `npx`経由でのClaude Code起動は非対応
- ユーザーへの移行ガイダンス:
  - Bunインストール: `curl -fsSL https://bun.sh/install | bash` (macOS/Linux)
  - Bunインストール: `powershell -c "irm bun.sh/install.ps1|iex"` (Windows)
  - エラー時に詳細なインストール手順を表示

## [0.6.1] - 2024-09-06

### Fixed

- Docker環境での動作改善
- パスハンドリングの修正

### Added

- Dockerサポートの完全実装
- Docker使用ガイド (docs/docker-usage.md)

## [0.6.0] - 2024-09-06

### Added

- @akiojin/spec-kit統合による仕様駆動開発サポート
- Codex CLI対応
  - Claude CodeとCodex CLIの選択機能
  - ワークツリー起動時のAIツール選択
  - `--tool`オプションによる直接指定

### Changed

- npmコマンドからnpx経由での実行に変更
- npxコマンドを最新版指定に更新

## [0.5.0] - 2024-08-XX

### Added

- セッション管理機能
  - `-c, --continue`: 最後のセッションを継続
  - `-r, --resume`: セッション選択して再開
  - セッション情報の永続化 (~/.config/claude-worktree/sessions.json)

### Changed

- Claude Code統合の改善
- UI/UXの向上

## [0.4.0] - 2024-07-XX

### Added

- GitHub PR統合
  - マージ済みPRの自動検出
  - ブランチとワークツリーの一括クリーンアップ
  - 未プッシュコミットの処理

### Changed

- エラーハンドリングの改善
- パフォーマンスの最適化

## [0.3.0] - 2024-06-XX

### Added

- スマートブランチ作成ワークフロー
  - feature/hotfix/releaseブランチタイプのサポート
  - releaseブランチでの自動バージョン管理
  - package.jsonの自動更新

### Changed

- ブランチタイプの自動検出
- ワークツリーパス生成ロジックの改善

## [0.2.0] - 2024-05-XX

### Added

- ワークツリー管理機能
  - 既存ワークツリーの一覧表示
  - ワークツリーの開く/削除操作
  - ブランチも含めた削除オプション

### Changed

- CLI UIの改善
- エラーメッセージの分かりやすさ向上

## [0.1.0] - 2024-04-XX

### Added

- 対話型ブランチ選択
  - ローカル・リモートブランチの統合表示
  - ブランチタイプ別の視覚的識別
  - 既存ワークツリーの表示
- ワークツリー自動作成
  - ブランチ選択からワークツリー作成まで
  - 自動パス生成 (.git/worktree/)
- Claude Code統合
  - ワークツリー作成後の自動起動
  - 引数パススルー機能
- 変更管理
  - AIツール終了後の未コミット変更検出
  - commit/stash/discard オプション

### Technical

- TypeScript 5.8.3
- Bun 1.3.1+ サポート（必須ランタイム）
- Node.js 18+ サポート（開発ツール向けオプション）
- Git 2.25+ 必須
- execa for Git command execution
- inquirer for interactive prompts

## [0.0.1] - 2024-03-XX

### Added

- 初期リリース
- 基本的なワークツリー管理機能

---

## Release Process

リリースは自動化されています:

1. PRがmainブランチにマージ
2. GitHub Actionsがテスト実行
3. Semantic Releaseがコミットメッセージからバージョンを決定
4. npmに自動公開
5. このCHANGELOG.mdが自動更新
6. GitHubリリースノート自動生成

## Migration Guides

### v0.6.x → v0.7.x (Unreleased)

Breaking changes: なし

新機能:

- テストスイートの追加（ユーザーへの影響なし）
- ドキュメントの拡充

推奨アクション:

- 特になし、通常通りアップグレード可能

### v0.5.x → v0.6.x

Breaking changes: なし

新機能:

- Codex CLI対応
- Docker対応

推奨アクション:

- Codex CLIを使用したい場合は`codex`コマンドをインストール
- Docker環境で使用したい場合はdocs/docker-usage.mdを参照

### v0.4.x → v0.5.x

Breaking changes: なし

新機能:

- セッション管理 (-c, -r オプション)

推奨アクション:

- セッション機能を活用して開発効率を向上

## Deprecation Notices

現在、非推奨となっている機能はありません。

## Known Issues

See [GitHub Issues](https://github.com/akiojin/claude-worktree/issues) for current known issues.

## Links

- [Repository](https://github.com/akiojin/claude-worktree)
- [npm Package](https://www.npmjs.com/package/@akiojin/claude-worktree)
- [Documentation](https://github.com/akiojin/claude-worktree/tree/main/docs)
- [Issue Tracker](https://github.com/akiojin/claude-worktree/issues)
