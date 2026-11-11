# Issue #7007: タスク中心 GUI の設計

この成果物は設計とモックであり、gwt の実装ではない。PM 裁定
`d873504f-5389-4d09-805e-29f2645290e3` に従い、React 基盤は Issue #6943、
画面の実装はその基盤配信後の Issue #7007 が担当する。製品の受け入れ条件は未完了のまま保持する。

## 成果物

- [操作できるモック](mock.html): Before / After、Home / Decide / Progress / Log、Create task、dark / light、判断待ちなし／更新待ちの例。
- [Home・dark](home-dark.png) / [Home・light](home-light.png)
- [Decide・dark](decide-dark.png) / [Decide・light](decide-light.png)
- [Before の再構成](before-dark.png)、[判断待ちなし](empty-light.png)、[更新待ち](stale-dark.png)
- [モック検証結果](preview-check.json)

モックは架空のタスク・名前・件数を使う。Before は Issue 本文とソースからの再構成であり、
実機のスクリーンショットではない。ボタンは画面を移動するか説明を表示するだけで、
タスク作成・判断送信・Issue の変更は行わない。HTML をブラウザで開くと操作できる。
同じ checkout の Operator 色・タイポグラフィ token とフォントを相対参照するため、
HTML だけを切り離さず repository 内で開く。

## Goal / Non-goals / Acceptance / What stays untouched

Goal: 利用者が Home から状況と次の行動を把握し、依頼・進行確認・判断・記録へ移れる設計を具体化する。

Non-goals: この設計フェーズでの製品 UI 実装、React 導入、Astryx 採否、モバイルの実装、
develop 全体の main 配信、状態モデルの再設計、既存権限を越える判断操作。

設計フェーズの受け入れ: UX の10問題を解法・owner・製品ACへ対応付け、英語モックで
主要4面と欠損／空状態を示し、依存関係を含む plan/tasks を Issue の正本 section に保存して読み戻す。

What stays untouched: `crates/gwt/web/`、Rust、ビルド／CI、現在の起動・キュー・権限・判断 API。
このディレクトリはアプリへ埋め込まれない設計資料だけを含む。

## UX 棚卸し

根拠: Issue #7007 の v9.157.0 実機観察と、`origin/main` の
`46b11ca92ad967ef41bfe4f291fbf9a3fae5a668` のソース監査。
比較対象の checkout は `1ea2034245937780bb65644e71f2440f905c25b9`。
ソース読取と実機観察を区別し、新しい実機測定を行ったとは主張しない。

| 問題 | 利用者への影響 | 設計の対処 | 製品AC | 実装責任 |
| --- | --- | --- | --- | --- |
| 1. 指標が第一印象 | 次の行動が分からない | Home の先頭を判断待ちと作成入口にする | AC-1, AC-8 | #7007 |
| 2. 略語・暗号的な状態 | 状態を推測する必要 | 平易な statusbar / tab title、同じ状態要約を読む | AC-1, AC-6, AC-7 | #7007 / #5016 |
| 3. ISO/UTC 時刻 | 新しさが読めない | 相対時刻とローカル絶対時刻、詳細で正確な時刻 | AC-4 | #7007 |
| 4. 通知の重複 | 本当に重要なものが埋まる | 共通モデルのグループを1行、回数と最終発生を表示 | AC-3, AC-6 | #5016 → #7007 |
| 5. 判断が backlog に混在 | 必要な操作を見落とす | Decide に未解決の人判断だけを集約 | AC-1, AC-2 | #7007 / #3431 / #6859 |
| 6. 内部エラーだけ表示 | 影響と復旧方法が分からない | 事象・影響・可能な次操作＋展開可能な原文 | AC-5, AC-7 | #7007 |
| 7. 面ごとの数字が不一致 | 情報を信頼できない | Home/Decide/Log/タブ/strip で同一 snapshot の派生要約を使う | AC-6 | #5016 → #7007 |
| 8. 内部IDが露出 | タスクの目的が読みにくい | request-task の目的と担当を先頭、内部IDは詳細に限定 | AC-7 | #7007、#4777 と調整 |
| 9. 操作名と意図が不一致 | 押した結果を予測できない | Create task / View progress / Review decision 等、対象と行動を明示 | AC-1, AC-9 | #7007、Issues名称は #5333 |
| 10. カードが過密 | 状態を一目で把握できない | 目的・担当・現在の段階・詰まりの理由を優先、指標は詳細へ | AC-1, AC-7, AC-8 | #7007、#4777 と調整 |

`works-surface.js` は取得不能な cost/landed の常時注記と `verification/last_action` の直接表示、
`app.js` の windowCommandErrorModel は内部 message の直接表示、`notification-center.js` は
通常 record の非集約と keyed upsert の混在を確認した。これらが変更境界の根拠となる。
既存通知はローカル時刻化された部分もあるため、全画面で未対応とは扱わない。

## 情報設計と画面契約

| 面 | 最初に答える問い | 主要情報と操作 | 詳細へ移すもの |
| --- | --- | --- | --- |
| Home | 今どうなっていて、次に何をすればよいか | 判断待ち要約、Create task、進行中タスクの抜粋、更新の新しさ | トークン、費用、内部状態、全キュー |
| Decide | 私の判断を何が待っているか | 理由、影響、選択肢、根拠、対象タスクへのリンク | 原文、内部ID、実行履歴 |
| Progress | 誰が何をどこまで進めているか | タスク目的、担当、段階、詰まりの理由、次の行動 | branch、model、利用量、検証の詳細 |
| Log | 何が起きたか | Action required / Completed / Information、同じ通知の回数、相対時刻 | 全発生記録、原文、正確な時刻 |

Home を新しい初期入口とし、Issues / Agents / Board / terminal の既存機能は専門操作として残す。
Create task は #3431 の request-task 作成経路へ接続し、別のタスク概念や独立キューは作らない。
Progress は request-task を主語にするが、未関連の Work は別枠で目的と担当を表示し、勝手に
同じタスクへまとめない。カードの抜粋数と全体件数を混同しない。

タブタイトル例: `gwt — 2 decisions · 3 in progress`。
statusbar 例: `Auto-run: On · 9 queued · 3 in progress`。
判断が不要なら `No decisions waiting`、読取失敗なら `Unable to refresh decisions` とする。
未知を0件にしない。前回値を残す場合は古い時刻と stale 状態を明記する。
更新待ちは、判断を再送可能か不明な状態なので操作を無効化し、更新の再試行を提示する。

## 判断・状態・エラーの境界

- **Decide は権限を増やさない。** backend が対象・revision・許可された選択肢を示す場合だけ
  送信可能にする。単なる文章の選択肢は PM の既存返信経路へ誘導する。未配信 API を仮定しない。
  `protocol.rs` の expected_revision / operation_id / actor 検査を維持する。
- 後で読む操作は未解決のまま保持する。表示から隠すことを acceptance・close・解決とみなさない。
  成功応答／新 snapshot で解決が確認できた時点で行と件数を同時に更新する。
- waiting / interrupted / error と人間の判断待ちは同義ではない。型付けされた要求に基づく。
- 通知のグループ同一性、回数、未読、解決状態は #5016 のモデルの責任。
  surface ごとに文面 hash を作らない。別 project/owner の同文や解決後の再発を混ぜない。
- 取得不能の費用等は省略する。判断データの取得不能・操作失敗は隠さず、影響と次操作を示す。
- エラーの翻訳は既知の型に対して行い、不明エラーは「The action could not be completed」
  と対象操作・詳細への入口を残す。保存状態が不明なのに「work is kept」、不明な復旧に
  「Restart fixes this」と断定しない。モックの resize 文面も実装時は error type と照合する。
- モックの選択肢は代表例であり backend の新 API を定義しない。汎用の Close ボタンは設けない。

## 表示・アクセシビリティ・端状態

英語 UI、既存 Operator token / typography、色と文字の両方による状態表示を使う。
モーダルが必要なら既存 shared modal primitive と ARIA 契約を使い、専用 shell は作らない。
Home/Decide を keyboard だけで往復でき、行の解決時にフォーカスを次の行または見出しへ移す。
カウント更新の読み上げはまとめ、継続更新でフォーカスを奪わない。

空・読取失敗・前回値・操作送信中・権限なし・revision競合・解決済み・再接続・project切替を
実装テストに含める。相対時刻は長時間開いたままでも更新し、日をまたぐ場合や未来時刻は
安全なローカル絶対表示に戻す。原文の UTC は詳細で保持する。

## 選択した方式

比較した方式は (A) 現行 vanilla 画面への見た目パッチ、(B) 本 Issue で React 基盤を新設、
(C) #6943 配信後に既存ストア購読の React 面として新設。
**C を採用**: 所有を重複させず、同じ状態モデルと一度の描画移行で受け入れを満たせる。
この設計のために新しい UI framework、state store、API、常設の設定層は追加しない。

## 検証の意味と限界

モックの Chromium headed 確認は見た目・画面遷移・空/古い状態・console/page error の確認であり、
製品の E2E や UX 成功を証明しない。特に「10秒以内」は初見の利用者によるタスク理解評価が必要で、
自動テストの速度や agent の自己申告で合格にしない。自律実行では人の確認を要求せず、未測定のまま記録する。
製品実装後は browser-check の checkout binary + fresh HOME で dark/light headed E2E を
verify.run に登録する。対象SHAとスコープを記録し、coverage は必須CIへ委ねる。

User Verification Result: n/a (autonomous)

## 依存と引継ぎ

Blocked-by: #6943

この依存は **React 画面実装**に適用し、設計作業は完了できる。
#6943 の基盤の AC-1/AC-2/AC-3/AC-4 相当が配信済みで、store購読・island mount・埋め込み・
再現可能buildが使えることを再開時に確認する。基盤のAPI・配置はその配信結果に合わせる。
#5016 の human summary と通知集約、#3431 の request-task/判断経路は必要能力ごとに配信証跡を確認する。
足りなければ当該ownerへ差し戻し、本Issueで互換実装を作らない。

Issue 正本の plan/tasks はこの資料とモックへのリンクを持つ。
設計完了は Issue 全体の完了ではない。9製品ACは実装・検証・配信の証拠が揃うまで未チェック。
