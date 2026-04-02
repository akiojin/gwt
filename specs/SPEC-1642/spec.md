# Docker 実行ターゲットとサービス検出

> **Canonical Boundary**: SPEC-1552 が Docker / DevContainer の runtime support を担当し、本 SPEC は gwt-tui からの検出・サービス選択・ホストフォールバック導線だけを扱う。

## Background

- gwt は docker compose と devcontainer を検出し、エージェントをコンテナ内で起動できる。
- 既存の SPEC-1642 はコンテナ監視やネットワーク管理まで含み、`SPEC-1552` と責務が衝突していた。
- 本 SPEC は起動時の UX に絞り、Docker を launch target としてどう選ばせるかを定義する。

## User Stories

### US-1: Docker / DevContainer を launch target として検出する

開発者として、プロジェクトを開いた時点で Docker 関連設定が検出され、コンテナ内起動の可否を判断したい。

### US-2: サービスを選んでエージェントを起動する

開発者として、複数サービスがある場合でも起動前に対象サービスを選び、その環境でエージェントを動かしたい。

### US-3: Docker が使えない場合にホスト実行へ戻る

開発者として、Docker daemon やサービスに問題があっても、ホスト実行へ安全にフォールバックしたい。

## Acceptance Scenarios

1. docker compose または devcontainer 設定があるリポジトリを開くと、起動フローでコンテナ内実行候補が提示される。
2. 複数サービスがある場合、Agent 起動前にサービス選択 UI が表示される。
3. サービス選択後、選択結果が launch config に反映され、コンテナ内で Agent が起動する。
4. Docker が利用できない場合、理由を表示したうえでホスト実行を選び直せる。
5. compose/devcontainer がない場合、Docker 導線は表示されず通常のホスト実行だけが使える。

## Edge Cases

- compose は存在するが対象サービスが停止中または build 失敗している。
- docker daemon が停止中で、検出は成功するが起動は失敗する。
- devcontainer と compose が同時に存在し、優先する launch target を明示する必要がある。

## Functional Requirements

- FR-001: docker compose / devcontainer 設定を launch target 候補として検出する。
- FR-002: 複数サービスがある場合は Agent 起動前に選択 UI を出す。
- FR-003: 選択した launch target を Agent 起動設定へ渡す。
- FR-004: Docker 利用不可時はホスト実行へ戻るためのエラーと再選択導線を提供する。
- FR-005: コンテナ監視・ネットワーク管理・手動 lifecycle 操作は本 SPEC の対象外とする。

## Success Criteria

- Docker 対応プロジェクトで launch target が正しく検出される。
- 複数サービス時に誤起動なく対象を選べる。
- Docker 失敗時でも gwt がクラッシュせずホスト実行へ戻れる。
