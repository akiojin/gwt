# Issue #6301: Host Work probe の区間計測

## 範囲

この配送単位は追加caseの計測工程（T-6301-R2）である。旧PR #6370の有限60秒期限、40秒遅延成功、認証、正規Work照合、404限定fallbackは維持する。根因修復、実稼働Hostへの反映、親SPEC #5390全体の性能達成は、この診断の追加だけでは証明できない。

比較した案は、新しい診断ストア、Host全体の詳細ログ、既存probeの相関spanである。既存logging/transportを再利用し変更範囲を保つため、probeの相関spanを選んだ。元のunknown workspace.updateや旧Blocked generationは再送していない。

## 履歴の一次証拠

調査base/HEADは `c807ba6beb677812893030ed8001e94d88543c23`。fetch後に差分はなく、旧PR #6370のsourceはdevelopに含まれる。引継ぎの未配信sourceという記述を現在の事実とは扱わない。

| 観測 | 一次証拠 | 解釈の限界 |
|---|---|---|
| 旧Session `638426f5-8a22-40f2-888d-41408b50c83a` の60秒timeout 3件 | errors.2026-10-31.jsonl の1247/1268/1293行、UTC 01:57:11 / 01:59:03 / 02:02:07 | probeの内部停止区間は記録されていない |
| 対応する操作wall 64.199 / 62.346 / 63.258秒 | perf-2026-10-31.jsonl の2422/2450/2491行 | CLI全体の時間であり、Host内処理時間ではない |
| 同Sessionのhook認証成功 | gwt.log.2026-10-31 の116643行以降 | probe自身の認証成功とは別の観測 |
| probe完了accessはJST 09:00:57の200/11.769秒の次が14:50:20の200/1.114秒 | 同ログ1105/393347行 | 完了時だけの記録なので、間の欠落は入口未到達を証明しない |
| 近傍のGit show-toplevelに50.507〜59.202秒、rev-parseに最大60.772秒 | 同ログのprocess spawn791/797/798/853/854/831 | request/Session相関がなく、probeの原因とは断定できない |

[historical-primary.json](historical-primary.json) は該当record・原本path/行/SHA256を保持する。HOME prefixを伏せたGitのUTF-8/LF保存形のSHA256は `e42850ce90f4a47f723ec55f67a6ed0942c62f77c5c64ac791d86cefb5aea2b5`。[historical-process.json](historical-process.json) は近傍のprocess64行で、保存形のSHA256は `b2729afde40072902300a6b18a79d43318bb26b758975998abf57f1c8a2b0936`。正規化前のWindows抽出物のSHA256は、それぞれ `5576d7f663451c96881e9bface1420b4ae896303b238678323d013e113eebfe5` / `4ede1d33107f8fd8072ded63105fe91010d7fd12ea68537eeba5653eaca6aaa4` であり、原本logのSHA256・行番号は保持し、path/binary内の個人HOME prefixを`<HOME>`へ伏せた以外のrecord内容は同じである。

旧Session ledger/generationには初期Active owner4093/gen-b6c811e958941a649289a7c1があり、正規Workにも当該Sessionとの対応が残る。後のBlocked/Stoppedを理由に元の症状を無効にはしない。旧source e18d534a2と現baseの間にはruntime supervisorの変更もあり、現在の成功を単一変更の効果とはしない。

現own Session `b41bf2fa-f5bf-4e73-8762-5007b61238a6` / gen-986b89c6f93d29357fda6648はActive/bound。Windowsでbootstrapしたcheckout gwtdのbuild.startは、同installed Host PID38952に対して成功した。これは今回の起動可用性の観測であり、当時の負荷条件や追加AC-5の修復前後比較ではない。

## 追加する診断

probeだけにserver生成request IDのspanを置く。入口、認証前/後、blocking workerへのdispatch/開始、Execution検証、target検証、正規Work照会前/後、検証完了、worker完了、reply生成を同じspanで記録する。timestamp差から区間を比較でき、replyにはHTTP statusと全elapsedを残す。replyはサーバーがresponseを生成した時点であり、client受領を保証する通知ではない。

認証後だけdurable Session ID/owner番号を記録する。bearer、request body、cwd、credential、provider IDは診断に含めない。public protocol/schemaやdeadline、authority判定、fallback条件は変えない。

## 検証と残件

既存のcurrent binding正負テストは1 PASS。追加した実HTTPテストのREDは、正規Active Session/Workで200 receiptと認証拒否が成立した後に、入口診断がないことを理由として失敗した（0 PASS / 1 FAIL、10.85秒）。

GREENでは成功の全12区間と認証拒否の3区間が相関し、Work snapshot不変とJSON decode後のtoken/cwd非出力を確認した（1 PASS、18.46秒。fixture準備と両HTTP要求を含むtest全体時間）。独立reviewはblocking 0件で、原本3ログのSHA/28recordとprocess64行の一致も確認した。Work照会診断はprobe span内に限定した。

PR #6633 reviewで、専用targetだけを有効にするとmodule targetのspanが無効になり相関を失うことを確認した。同じ実HTTP testを`off,gwt.work_materialization_probe=info`にしてRED（1 FAIL、9.24秒、入口相関の欠落）を固定し、spanと全probe stageのtargetを統一してGREEN（1 PASS、9.83秒）を確認。既存12/3区間・副作用なし・secret非出力に加え、replyの200/401と整数elapsedを検証する。0msは有効な値として許容する。review修正の独立確認もblocking 0件である。これらの時間はtest全体であり、元60秒停止の修復前後比較ではない。

候補Hostの実HTTP・同HEAD canonical・独立review・必須CI/coverage・PR/develop包含の結果は、確定後にIssue #6301とPRへ記録する。source配送、候補Host実測、installed反映を別の証拠として扱う。再発時には同requestの最後のphaseを起点に最小修復箇所を確定し、旧AC-1〜3の履歴を保持したままAC-4〜7を評価する。Issue #4093 T-4と親のfleet/CPU/RSSを今回だけで完了にしない。

User Verification Result: n/a (autonomous)

Agent Visual Check: n/a (no UI surface)
