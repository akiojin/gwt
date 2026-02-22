# 機能仕様: macOS で断続的にターミナル入力不能になる事象の抑止

**仕様ID**: `SPEC-748cead5`
**作成日**: 2026-02-22
**更新日**: 2026-02-22
**ステータス**: 確定
**カテゴリ**: GUI

**入力**: ユーザー説明: "gwtが所々固まります。画面更新されているが、入力を受け付けない。現在はmacOSのみ確認。"

## 背景

- ターミナル出力は継続しているのに、キー入力が受け付けられない状態が断続的に発生する。
- `MainArea` の terminal 表示制御が tab 切替ごとに ready 待ちへ戻るため、条件競合時に入力対象が `pointer-events: none` になりうる。
- `TerminalView` は tab 有効化直後の再フォーカスは実施しているが、ウィンドウ再フォーカスや pointerdown 起点の復帰経路が弱い。

## ユーザーシナリオとテスト

### ユーザーストーリー 1 - 一度表示済みの terminal は再切替で即入力したい (優先度: P0)

開発者として、一度 ready になった terminal tab へ戻るときに待機状態へ戻らず、すぐ入力可能であってほしい。

**独立したテスト**: terminal A/B を往復したとき、A に戻るタイミングで `.terminal-wrapper.active` が即時復帰すること。

### ユーザーストーリー 2 - フォーカス復帰時に terminal 入力を取り戻したい (優先度: P0)

開発者として、window focus 復帰や terminal 領域クリック時に入力フォーカスが terminal へ戻ってほしい。

**独立したテスト**: `pointerdown` と `window focus` の各イベントで `Terminal.focus()` が呼ばれること。

## 要件

### 機能要件

- **FR-001**: システムは、一度 ready 済みの terminal tab へ再切替した場合、ready 待機状態に戻してはならない。
- **FR-002**: システムは、ready fallback で可視化した tab も ready 済みとして扱わなければならない。
- **FR-003**: システムは、window focus 復帰時に active terminal のフォーカス復帰を試行しなければならない。
- **FR-004**: システムは、terminal 領域 pointerdown 時に active terminal のフォーカス復帰を試行しなければならない。

### 非機能要件

- **NFR-001**: 既存の tab D&D / close / shortcut / copy-paste 挙動を変更しないこと。
- **NFR-002**: 対応は全 OS 共通コードで行い、OS 分岐を増やさないこと。

## 成功基準

- **SC-001**: `MainArea.test.ts` に再切替即時表示の回帰テストが追加され、通過する。
- **SC-002**: `TerminalView.test.ts` に pointerdown / window focus のフォーカス復帰テストが追加され、通過する。
- **SC-003**: 既存の `MainArea` / `TerminalView` / `systemMonitor` テストが回帰なく通過する。
