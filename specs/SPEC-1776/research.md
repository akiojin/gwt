# Research: SPEC-1776 — Cross-Spec Comparison Matrix

## Purpose

`SPEC-1776` は単独で全機能を上書きしない。親 SPEC として、旧TUI、現行 `gwt-tui`、現行 `gwt-core`、関連 child SPEC を横断して比較し、新しい TUI の優先順位だけを定義する。

## Comparison Matrix

| Concern | 旧TUI (`v6.30.3`) | 現行 `gwt-tui` | 現行 `gwt-core` | 新目標 |
|---|---|---|---|---|
| Primary entry | `branch list` 中心 | session/tab surface が強い | Git / launch data は再利用可能 | `Branches` を primary entry に戻す |
| Session multiplicity | 基本 `1ブランチ = 1セッション` | 同一ブランチ複数セッション可 | session persistence あり | `1ブランチ = Nセッション` を正式化 |
| Session layout | `tmux` pane / focus | tab + single-pane寄り | `native PTY` / scrollback あり | `equal grid` + `maximize` |
| Session visibility from branch rows | 行内で起動中 agent が見える | 情報はあるが UX が散る | branch/session linkage は利用可 | 各行は `session count` のみに絞る |
| Enter behavior | focus / show hidden / wizard | tab 前提 | launch primitives あり | `no / one / many` の 3 分岐 |
| Hidden pane concept | あり | なし | 不要 | 廃止 |
| Management workspace | 旧TUIは tab ではない | tabbed management | data loaders あり | tabbed management を採用 |
| Initial management tabs | 旧TUIは統合的 | Branches / SPECs / Issues / Versions / Settings / Logs | backend support は広い | 初回は `Branches / SPECs / Issues / Profiles` |
| Terminal basis | `tmux` + terminal | `native PTY` + `vt100` | `terminal::*` 完備 | 現行 terminal 基盤を維持 |
| Quick Start | あり | child 実装あり | session store / detect あり | Branches flow に接続維持 |
| SPECs tab | 旧TUIには直接対応薄い | local artifact viewer あり | local SPEC loader あり | tab 維持、parent navigation に同期 |
| Issues tab | GitHub Issue 導線あり | GitHub Issue detail あり | issue cache / linkage あり | tab 維持、launch entry 維持 |
| Profiles | Settings 内の env 管理 | Settings 内に存在 | persistence あり | `Profiles = Env profiles` として独立タブ化 |
| Settings | 強い設定面あり | 実装済み | persistence あり | 後続 |
| Logs | 旧TUIに存在 | 実装済み | log reader あり | 後続 |
| Versions | 旧TUIには薄い | 実装済み | git metadata あり | 後続 |
| AI summary | 旧TUIに存在 | 現行でも文脈あり | summary infrastructure あり | 後続 |

## Why Not Revert gwt-cli

- 旧TUIコードは `tmux` multi-mode に深く結合している
- 現行 `gwt-core` は `terminal::*` と `session_store` / `session_watcher` に置換済み
- 単純リバートでは backend contract が崩れ、戻したあとに再度 large refactor が必要になる
- したがって、`旧TUIの UX` を抽出し、現行 backend の上で再構成するほうが筋が良い

## Cross-Spec Follow-Ups

| SPEC | Needed Sync |
|---|---|
| `SPEC-1654` | shell/session model を `tab-first` から `branch-first + permanent multi-mode` へ修正 |
| `SPEC-1770` | `Enter`、management toggle、grid/maximize 操作ポリシーを同期 |
| `SPEC-1777` | SPECs tab の parent navigation と launch entry を同期 |
| `SPEC-1782` | `1ブランチ = Nセッション` 前提へ Quick Start 導線を同期 |
| `SPEC-1542`, `SPEC-1656` | `Profiles = Env profiles` wording を必要なら明示 |

## Coverage Audit Status

| SPEC | Coverage Status | Why |
|---|---|---|
| `SPEC-1541` | reference only | terminal basis は維持し、parent UX では上書きしない |
| `SPEC-1579` | audit required | `gwt-spec-ops` / `gwt-spec-implement` など workflow skill 契約の正本 |
| `SPEC-1636` | reference only | Assistant interrupt semantics は今回の主変更対象ではない |
| `SPEC-1643` | audit required | Issues search/discovery contract が management tabs と噛み合う必要がある |
| `SPEC-1644` | reference only | local git/worktree owner。branch-first でも正本は維持 |
| `SPEC-1646` | reference only | agent catalog / launch contract owner |
| `SPEC-1648` | reference only | multi-session persistence の owner |
| `SPEC-1654` | sync required | shell/session model が parent UX と明確にずれている |
| `SPEC-1714` | audit required | Branches / Issues の linkage source として必須 |
| `SPEC-1770` | sync required | shortcut / grid / maximize 操作が親方針で変わる |
| `SPEC-1777` | sync required | SPECs tab の navigation / launch entry を parent 方針に合わせる |
| `SPEC-1779` | reference only | custom agent UI は後続だが owner として inventory に含める |
| `SPEC-1782` | sync required | `1ブランチ = Nセッション` により Branches flow が変わる |
| `SPEC-1786` | audit required | hooks confirm が新しい branch enter selector にぶつかる |
| `SPEC-1787` | audit required | SPEC-first workflow / initialization と branch-first UX の整合確認が必要 |
| `SPEC-1354` | audit required | Issues detail contract の canonical owner |
| `SPEC-1542`, `SPEC-1656` | audit required | Profiles persistence wording を `Profiles = Env profiles` に揃える必要がある |

## Embedded Workflow Skill Coverage

- `gwt-spec-ops`
- `gwt-spec-implement`
- `gwt-spec-plan`
- `gwt-spec-tasks`
- `gwt-spec-analyze`
- `gwt-spec-register`
- `gwt-spec-search`
- `gwt-issue-register`
- `gwt-issue-resolve`

These are considered covered through `SPEC-1579` and `SPEC-1787`. `SPEC-1776` does not redefine their workflow contract; it only constrains how `Branches`, `SPECs`, and `Issues` surfaces hand users into those flows.

## Workflow Contract Audit

### `SPEC-1579`

- Status: `reference only`
- Reason: `gwt-spec-ops`, `gwt-spec-implement`, `gwt-spec-plan`, `gwt-spec-tasks`, `gwt-spec-analyze` の ownership と stop condition はすでに `SPEC-1579` が正本として持っている
- Result: `SPEC-1776` 側で workflow contract を再定義する必要はない
- Constraint on parent UX: `SPECs` / `Issues` / `Branches` から workflow へ handoff するとき、owner は常に `SPEC-1579` / `SPEC-1787` で定義された visible workflow owner を使う

### `SPEC-1787`

- Status: `sync required`
- Reason: 現行文面には `Branch-centric workflow — Work should start from SPEC/Issue, not from Branches tab branch selection` とあり、今回の `branch-first primary entry` 方針と衝突している
- Result: branch-first UX と SPEC-first workflow を両立するように wording を更新する必要がある
- Required sync:
  - `Branches` は primary entry だが、SPEC/Issue launch は first-class のまま維持する
  - `SPEC-first workflow` は workflow owner として維持し、UI entry surface の優先順位だけ parent UX に合わせる
  - initialization 後にどのタブへ着地するかも、`Branches` / `SPECs` の方針と整合させて再確認する

## Accepted Product Decisions

- 旧TUIの使いやすさは `branch list` と操作意味にある
- `tmux` は不要
- 新しい唯一の運用モデルは `permanent multi-mode`
- session workspace は `equal grid` を通常形、`maximize + tabs` を集中形にする
- 管理画面は `tabbed management workspace`
- 初回完成条件は `Branches / SPECs / Issues / Profiles`
- `AI summary` は後続
