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
* support gemini and qwen session resume ([9cc993d](https://github.com/akiojin/gwt/commit/9cc993df4eb5ba9a1ea1c3be89e64dbca3071929))
* クイックスタートでツール別の直近設定を提示 ([9904992](https://github.com/akiojin/gwt/commit/99049928061105e48fadb103bab423fdb4e7ecf4))
* セッションID再開対応（Codex/Claude/Gemini/Qwen） ([a2d50ef](https://github.com/akiojin/gwt/commit/a2d50ef9f48e18045b6c5764b38c6f567f6d0d83))
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
