# セッション保存・復元

> **Canonical Boundary**: 本 SPEC はセッション metadata と restore contract の正本である。設定ファイル全般は `SPEC-1542`、タブ構成は `SPEC-1654`、一時 PTY scrollback は永続化対象外とする。

## Background

- gwt は Shell / Agent タブの session metadata、session id、設定を保存し、次回起動時の復元に利用する。
- 既存の SPEC-1648 は raw terminal state の完全復元まで含むように読め、現在の実装境界とズレている。
- 本 SPEC は永続化するデータを session metadata に限定し、terminal transcript の長期保存は扱わない。

## User Stories

### US-1: 起動時に前回のセッション metadata を復元する

開発者として、再起動後に開いていたタブや関連 session id を回復したい。

### US-2: 設定と履歴を分けて管理する

開発者として、config / session history / agent history を役割ごとに分けて扱いたい。

### US-3: 永続化しないデータを明確にしたい

開発者として、raw PTY transcript のような一時データを誤って復元対象としないようにしたい。

## Acceptance Scenarios

1. gwt 再起動後に session metadata から Shell / Agent タブが再構成される。
2. 復元時、Agent は保存済み session id を使って resume/continue できる。
3. 設定ファイルと session history が別ファイルとして管理される。
4. raw PTY scrollback は session lifetime の一時データとして扱われ、再起動後の必須復元対象には含めない。
5. 不正データや古い schema は安全に無視または移行できる。

## Edge Cases

- session file はあるが実 worktree が削除されている。
- resume 先の agent session id が既に無効化されている。
- 古い設定 schema と新しい session schema が混在する。

## Functional Requirements

- FR-001: session metadata と設定ファイルを分離して永続化する。
- FR-002: 起動時に session metadata を読み込み、タブ構成を再生成する。
- FR-003: Agent resume に必要な session id / tool / branch 情報を保存する。
- FR-004: raw PTY transcript は永続化対象外とする。
- FR-005: schema 変更時に後方互換または安全な読み飛ばしを提供する。

## Success Criteria

- 再起動後の session restore 契約が明確になる。
- 一時データと永続データの境界がぶれない。
- Session store / profile store / logs の責務が重ならない。
