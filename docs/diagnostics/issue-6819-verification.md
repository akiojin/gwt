# Issue #6819: Windows recovery worker の source authority

担当: サチ。起動経路は `execution.status` の `launch_route: autonomous`。

## 基準と原因

調査開始時の fetch 後の HEAD / origin/develop はともに
`54d2e9f7ad733ac8737d8bd7daac5113603f1913`。このブランチに未配信の
ソース差分はなく、base 上の `current_hook_context` は元 agent child との
通常の親子関係だけを認識していた。Windows recovery は
`Win32_Process.Create` で worker を起動するため、その関係を持たない。

Issue #6806 の既存の正規記録 `vrr-b8179a141f5a485f9738f649d4848208` は
GUI 起動前に `source_foreign` で失敗している。今回も元の通常 tool 経路から
同じ checkout の doctor を実行すると、`current / normal / converged / proceed`、
理由・診断ともに空だった。設定の収束と recovery の実行関係を区別する。

## 修正と安全性

元 caller 内の専用 local named-pipe broker が、手元の exact recovery Job に
OS から得た接続元 PID が属するかだけを返す。接続側は OS から得た server PID と
元 caller の native 生成時刻・生存・agent ancestry を前後に確認する。
固定 version / nonce、期限付き通信、query-only Job handle を使う。
descriptor は接続先の手掛かりであり、単独の許可にはならない。

通常の process ancestry と、Session / worktree / runtime incarnation / 正常 hook
完了 / ECR authority の照合は維持する。同じ包含判定を runtime 失効検査にも使い、
停止された元 runtime の配下からの検証を拒否する。broker を終了・join してから
元 Job を閉じる。隔離 GUI には source descriptor を渡さない。

private worker は dispatch 前に locator を一度固定する。これは接続先の snapshot であり、
許可の cache ではない。以後も native peer / 生成時刻 / 生存 / membership を毎回確認する。
失効検査の正負で同じ locator を使い、認証不能を「子孫ではない」という証明に変換しない。
正の許可は確認済み true だけ、停止 subtree の除外は認証済み false だけを認める。

初案の raw Job handle descriptor は独立レビューで破棄した。
`job_cpu::observe` が無関係の Job の照会ハンドルも一時的に保持するため、
ハンドルの保持だけはその Job を正規 recovery 用に作った証明にならない。
この誤認を拒否する回帰検証を残す。

追加 RED は、初回成功後に環境変数を消すと source 判定が true から false へ変わることを
実証した。locator 固定と checked result により、途中の削除・差し替えと通信不能を区別する。

## 検証境界

- native tests: 正規 Job member、foreign / non-member、偽の locator / server / birth / nonce、
  source broker の停止、locator 削除・差し替え、および照会ハンドルで Job 設定を変更できないこと。
  source broker の停止と、元 process 自体の終了は別の観測として扱う。
- 既存 hook / doctor と runtime revocation の回帰検証。
- `scripts/verify-issue-6819.ps1 -RequireRecovery`: checkout の fresh binaries と HOME を使い、
  recovery descriptor の継承、typed preflight、実 Chromium headed の dark / light、
  既存の input-disconnect / agent-window-scroll の機能・console/page error assertions と cleanup。
- Issue #6806 の元の全 matrix は、本 checkout の GUI に対して独立して診断する。
  continuity test の元 blob は `71d67db6a85f40afd9d087db77a62796182c78e4`。
  入力修復の source は取り込まない。preflight の通過と、別 owner の入力修復の合格を区別する。

User Verification Result: n/a (autonomous)

## 配送前の観測

基準 HEAD `54d2e9f7a` に本修正を加えた working tree で確認した。

| 確認 | 実測 |
| --- | --- |
| foreign Job query handle の旧誤許可 | 拒否すべき probe が true となる RED |
| locator 削除の旧挙動 | 初回 true → unset 後 false の RED |
| native module（実物を読む一時 harness） | 最終 3 PASS、locator 保持・checked unknown / unrelated を含む |
| query-only Job handle | 1 PASS、設定変更が拒否され kill-on-close を保持 |
| 既存 hook / preflight、失効と新 incarnation | 旧 broker GREEN binary で 15 + 1 PASS。最終 canonical で再実行する |
| 原 Issue #6806 全 matrix の通常 caller 診断 | 実 Chromium headed、22 PASS / 2 FAIL、4.7 分 |

通常 caller の typed preflight は `current / normal / converged / proceed`。
scroll / disconnected-input の 22 件は dark / light とも機能 assertion と
console / page error ゼロの assertion を通過した。

continuity の 2 件は未配信の入力修復に対応する receipt 順序で失敗した。
dark は開始時に `ASCII-tocn` / `r-olastrt日本語-control-start`、light は
120 秒 checkpoint で `ACISI-s日本語-stream-2tream-` / `2` を受理した。
入力修復の 5 ファイル・456 行はこの checkout に取り込んでいない。
原 assertion / timeout は変えておらず、入力合格・3 分継続合格とは報告しない。
証拠は Issue #6806 comment `6697782194` に返した。

直接診断の HOME は `target/issue-6819-home-5eec882ca3d64468bf254f2ff6fb628a`。
`preflight.json`、synthetic input receipt、Playwright screenshots、`shutdown.json`
を保存する。正規 recovery の原 24 件診断と、本修正の derived pre-pr 記録は分離し、
失敗した入力診断を PASS に変更しない。最終 committed HEAD の canonical 記録・CI・
develop 包含の証拠は PR と Issue #6819 の配送コメントに保存する。

カバレッジは必須 CI の exact PR head で判定する。適用 gate は outer shell 除外後の
gwt-core + gwt が 90%、その他 domain crates が 80%。ローカルでは未測定。
