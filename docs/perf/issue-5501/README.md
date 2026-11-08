# Issue #5501: PTY 出力と daemon publish の CPU 比較

Windows、同じ checkout の debug build、実 Chromium の headed 実行で計測した。
before は `166dfefc94b13f4b3124b02e015c7e61a05f2c68`、after は本 PR の Rust 差分である。
branch / worktree の作成や切替は行っていない。before の再 build 中は差分を保全し、
`finally` で after source を復元した。測定後は after binaries の SHA-256 も照合した。

## 条件と一次証跡

- 各 sample は fresh HOME、PM 自動起動 off、Issue Monitor off。
- checkout の `gwtd` で隔離 daemon を明示起動し、所有 PID と認証済み `probe=ok` を確認した。
- 実 Shell 8 pane で同じ Node spinner を実行した。20ms 間隔、512 bytes の padding と連番、cursor redraw。
- `daemon.subscribe(runtime_output)` でも 8 pane 全ての実出力を受信してから CPU sampling を始めた。
- `GetThreadTimes` の kernel + user CPU 秒を、初期 thread と名前が一致する publish worker ごとに差分取得した。
- 取得開始・終了の native thread snapshot、実測秒数、各秒の traffic、GUI / daemon の PID を JSON に保存した。
- dark / light は独立した fresh instance の測定値。共有 host 上の短時間測定であり、release build や実 agent 8 窓・60 分の測定は Issue #5385 の範囲である。

初期の daemon 不在時の試行は下表に含めていない。

| Build / theme | 実測秒数 | main CPU 秒 | publish worker CPU 秒 |
| --- | ---: | ---: | ---: |
| [before / dark](before-dark.json) | 10.580 | 1.531250 | 3.812500 |
| [after / dark](after-dark.json) | 10.498 | 0.296875 | 0.109375 |
| [before / light](before-light.json) | 10.477 | 1.531250 | 2.531250 |
| [after / light](after-light.json) | 10.619 | 1.000000 | 0.171875 |

2 theme の合計 CPU 秒を合計実測秒数で割ると、main は **0.14544 → 0.06141 core（57.77% 減）**、
publish worker は **0.30127 → 0.01332 core（95.58% 減）**。
両 thread の合計は **0.44670 → 0.07473 core（83.27% 減）** だった。

## 出力の進み方

| Build / theme | GUI raw chunks | GUI raw bytes | preview 件数 | daemon relay bytes |
| --- | ---: | ---: | ---: | ---: |
| before / dark | 1,469 | 832,923 | 1,453 | 144,018 |
| after / dark | 1,398 | 792,666 | 288 | 798,336 |
| before / light | 1,608 | 911,736 | 1,593 | 288,603 |
| after / light | 1,500 | 851,067 | 349 | 857,871 |

sampling 境界の前後に batched frame がまたがるため、GUI / daemon の bytes は完全一致しない。
GUI raw 出力は 82,854 → 77,839 bytes/秒、daemon relay は **20,545 → 78,430 bytes/秒**。
before は生成量に転送が追いつかず、停止後の `DONE` が daemon observer に 15 秒以内に届かない RED を再現した。
after は raw、preview、xterm、daemon observer の全てで 8 pane の `DONE` が収束した。
console / page error はゼロ。dark / light の PNG も検証終了後に確認した。

## 実装と受け入れの対応

- AC-1: chunk 受信時は raw bytes / stream sequence を転送し、deadline 到達時の screen render 1 回を preview、承認、trust、quota、API error の検出へ共有する。
- AC-2: 最初の chunk から 150ms の deadline を保持する。連続出力で期限を延長せず、status 前に残りを flush する。旧 incarnation、close / reseed、中間 relay frame の preview を回帰テストで確認した。daemon queue は 4KiB / slot の上限を維持する。
- AC-3: worker ごとに runtime を 1 個保持し、root ごとの認証済み接続を再利用する。失敗した frame は再送せず、次回に authority を再解決する。負の結果は 1 秒、各 cache は 32 root を上限とする。control の fallback 判定と承認 overlay の独立 queue は既存の契約を維持する。
- AC-4: 上記の実測値と 4 JSON が一次証跡。接続中の転送を実際に観測している。

関連 Rust は端末側 112 件と publisher 4 件が PASS。
正式な pre-PR 検証は `verify.plan(mode=pre-pr)` と `verify.run` を使用し、
fmt、clippy、関連 Rust、headed dark / light を記録する。
workspace 全体の Rust tests と coverage 90% / 80% は必須 CI に委ねる。

User Verification Result: n/a (autonomous)

Agent Visual Check: pass

## 再実行

同 checkout の対象 build を `target/debug/gwt.exe` / `gwtd.exe` に用意し、
pinned Playwright 1.49.1 の Windows dependencies をインストールして実行する。
依存 cache と `resolve-playwright.cjs` は `%LOCALAPPDATA%/Temp/gwt-playwright-win-1.49.1` に置く。
正式検証が差し替える `TEMP` からは解決しない。
helper は隔離 daemon / GUI を所有し、失敗時も両方を停止する。

```powershell
pwsh -NoProfile -File scripts/diagnostics/issue-5501-browser-check.ps1 -Label after --headed
```

before は backlog が後続 theme に残るため、各 theme を独立起動して採取する。
`-Label before --project chromium-dark` または `chromium-light` を使用する。
before の最終 daemon 出力 assertion は既知の RED、after は両 theme とも PASS が必要である。

| Binary | SHA-256 |
| --- | --- |
| before gwt | `8FD7676A5FA6469AC3445484B22ADA6C11DD3CE18604531BD824BF9C7A41E3A9` |
| before gwtd | `E516E6CD85F6C61726D1FC52D45DF16EED9E7E897D809348B5040EEA6651BF00` |
| after gwt | `A939E2CE9699558A6AD92B9A77CF8EAA53F2DC1976CE59AE3E2CF1496F870C21` |
| after gwtd | `4405D01B91C62A4CEE452E927DC2C534470D01A7D1EE9DF277DCFA135B334919` |
