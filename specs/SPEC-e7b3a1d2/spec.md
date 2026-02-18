# 機能仕様: ウィンドウ・タブ切り替えショートカット

**仕様ID**: `SPEC-e7b3a1d2`
**作成日**: 2026-02-15
**更新日**: 2026-02-15
**ステータス**: 確定
**カテゴリ**: GUI
**依存仕様**:

- SPEC-f490dded（ターミナルタブ — タブの種類が増えるため、タブ切り替えの対象に影響）

**入力**: ユーザー説明: "ウィンドウ切り替え・タブ切り替えのキーボードショートカットを追加し、README にキーボードショートカット一覧を記載する"

## 背景

- 現在の GWT では複数ウィンドウ間の切り替えは Window メニューからの選択のみ
- macOS のネイティブ Cmd+\` によるウィンドウ切り替えが Tauri アプリで動作しない
- タブ間の切り替えもマウスクリックのみで、キーボードだけで操作を完結できない
- ブラウザや IDE では Cmd+Shift+\[/\] によるタブ切り替えが標準的で、ユーザーが期待する操作
- README にキーボードショートカット一覧が記載されておらず、ユーザーが利用可能なショートカットを発見しにくい
- Window メニューに macOS 標準の Minimize / Zoom / Bring All to Front が存在しない

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - タブ切り替えショートカット (優先度: P0)

ユーザーとして、Cmd+Shift+\[/\] で同一ウィンドウ内のタブを順方向・逆方向に切り替えたい。

**独立したテスト**: 複数タブが開いた状態で Cmd+Shift+\] を押下し、次のタブに切り替わること

**受け入れシナリオ**:

1. **前提条件** 3つのタブ（Summary, Agent1, Agent2）が開いており Agent1 がアクティブ、**操作** Cmd+Shift+\] を押下、**期待結果** Agent2 タブがアクティブになる
2. **前提条件** 3つのタブが開いており Agent1 がアクティブ、**操作** Cmd+Shift+\[ を押下、**期待結果** Summary タブがアクティブになる
3. **前提条件** 最右端のタブがアクティブ、**操作** Cmd+Shift+\] を押下、**期待結果** 何も起きない（ラップしない）
4. **前提条件** 最左端のタブ（Summary）がアクティブ、**操作** Cmd+Shift+\[ を押下、**期待結果** 何も起きない（ラップしない）
5. **前提条件** タブが1つのみ、**操作** Cmd+Shift+\[/\] を押下、**期待結果** 何も起きない
6. **前提条件** xterm.js ターミナルがフォーカスされている、**操作** Cmd+Shift+\] を押下、**期待結果** xterm.js がキーを飲み込まず、タブ切り替えが正常に動作する（Tauri accelerator によるキャプチャ）

---

### ユーザーストーリー 2 - タブ巡回順序の一貫性 (優先度: P0)

ユーザーとして、タブの巡回順序がタブバーの表示順（左→右）と一致し、D&D で並べ替えた順序にも追従してほしい。

**独立したテスト**: タブを D&D で並べ替えた後に Cmd+Shift+\] で次のタブへ移動し、表示順に一致すること

**受け入れシナリオ**:

1. **前提条件** タブ順序が \[Summary, Agent1, Agent2\] で Summary がアクティブ、**操作** Cmd+Shift+\] を押下、**期待結果** Agent1 がアクティブになる
2. **前提条件** タブを D&D で \[Summary, Agent2, Agent1\] に並べ替えて Summary がアクティブ、**操作** Cmd+Shift+\] を押下、**期待結果** Agent2 がアクティブになる（並べ替え後の順序に追従）

---

### ユーザーストーリー 3 - Window メニューへのナビゲーション項目追加 (優先度: P0)

ユーザーとして、Window メニューに Previous Tab / Next Tab / Previous Window / Next Window の項目が表示され、ショートカットキーが確認できるようにしたい。

**独立したテスト**: Window メニューに各ナビゲーション項目が accelerator 表示付きで存在すること

**受け入れシナリオ**:

1. **前提条件** アプリが起動している、**操作** Window メニューを開く、**期待結果** メニュー最上部に "Previous Tab" (Cmd+Shift+\[) / "Next Tab" (Cmd+Shift+\]) が表示され、その下に "Previous Window" / "Next Window" が表示される
2. **前提条件** タブが1つのみ、**操作** Window メニューから "Next Tab" を選択、**期待結果** 何も起きない
3. **前提条件** macOS で実行中、**操作** Window メニューを開く、**期待結果** Minimize (Cmd+M) / Zoom / Bring All to Front が含まれている

---

### ユーザーストーリー 4 - ウィンドウ切り替えショートカット (優先度: P1)

ユーザーとして、Cmd+\` で複数の GWT ウィンドウ間を MRU 順で切り替えたい。

**独立したテスト**: 2つのウィンドウを開き、Cmd+\` で別ウィンドウにフォーカスが移ること

**受け入れシナリオ**:

1. **前提条件** ウィンドウ A, B が開いており A がフォーカス中、**操作** Cmd+\` を押下、**期待結果** ウィンドウ B にフォーカスが移る
2. **前提条件** ウィンドウ A, B, C が開いており C が最後にフォーカスされた、**操作** A で Cmd+\` を押下、**期待結果** C（MRU で直前のウィンドウ）にフォーカスが移る
3. **前提条件** ウィンドウ A が表示中、B がトレイに非表示、**操作** A で Cmd+\` を押下、**期待結果** B が show+focus され前面に復元される
4. **前提条件** ウィンドウが1つのみ、**操作** Cmd+\` を押下、**期待結果** 何も起きない

---

### ユーザーストーリー 5 - ウィンドウ逆方向切り替え (優先度: P1)

ユーザーとして、Cmd+Shift+\` で MRU リストを逆方向に巡回したい。

**独立したテスト**: 3つのウィンドウで Cmd+Shift+\` を押下して逆方向に巡回すること

**受け入れシナリオ**:

1. **前提条件** ウィンドウ A, B, C が開いており A がフォーカス中、MRU 順が \[A, C, B\]、**操作** Cmd+Shift+\` を押下、**期待結果** B（MRU 逆方向）にフォーカスが移る

---

### ユーザーストーリー 6 - macOS 標準 Window メニュー項目 (優先度: P1)

ユーザーとして、macOS の Window メニューに Minimize / Zoom / Bring All to Front が含まれ、macOS ユーザーの期待に沿った操作ができるようにしたい。

**独立したテスト**: macOS で Window メニューに Minimize (Cmd+M), Zoom, Bring All to Front が表示されること

**受け入れシナリオ**:

1. **前提条件** macOS でアプリが起動、**操作** Window メニューを開く、**期待結果** Minimize (Cmd+M), Zoom が表示される
2. **前提条件** macOS でアプリが起動、**操作** Minimize を選択、**期待結果** ウィンドウが最小化される
3. **前提条件** macOS で複数ウィンドウが開いている、**操作** "Bring All to Front" を選択、**期待結果** すべての GWT ウィンドウが前面に表示される

---

### ユーザーストーリー 7 - README キーボードショートカット一覧 (優先度: P1)

ユーザーとして、README に全キーボードショートカットの包括的な一覧が記載されていることを確認したい。

**独立したテスト**: README.md / README.ja.md にキーボードショートカット一覧セクションが存在し、既存・新規ショートカットが網羅されていること

**受け入れシナリオ**:

1. **前提条件** なし、**操作** README.md を確認、**期待結果** 以下が記載されている: Cmd+N (New Window), Cmd+O (Open Project), Cmd+C/V (Copy/Paste), Cmd+Shift+K (Cleanup Worktrees), Cmd+, (Preferences), Cmd+Shift+\[/\] (Previous/Next Tab), Cmd+\`/Cmd+Shift+\` (Window switching), Cmd+M (Minimize)
2. **前提条件** なし、**操作** README.ja.md を確認、**期待結果** README.md と同等の内容が日本語で記載されている

## エッジケース

- Summary パネルはタブ巡回の対象に含まれる（通常のタブと同等に扱う）
- Tauri accelerator で `CmdOrCtrl+Shift+[` がサポートされない場合 → 別のキーに変更（例: CmdOrCtrl+Shift+PageUp/PageDown）
- Tauri accelerator で `CmdOrCtrl+Backquote` がサポートされない場合 → 別のキーに変更
- MRU リストが空の場合（ウィンドウが1つのみ）→ Cmd+\` は何もしない
- 非表示ウィンドウのみが残っている場合 → show+focus で復元する
- MRU 履歴はメモリ内のみで管理し、アプリ再起動時はリセットされる（ウィンドウ作成順から再構築）
- Windows/Linux では CmdOrCtrl 修飾子により Ctrl+Shift+\[/\] / Ctrl+\` として動作する

## 要件 *(必須)*

### 機能要件

- **FR-001**: Window メニューに "Previous Tab" 項目を追加し、`CmdOrCtrl+Shift+[` accelerator を割り当てる
- **FR-002**: Window メニューに "Next Tab" 項目を追加し、`CmdOrCtrl+Shift+]` accelerator を割り当てる
- **FR-003**: Previous Tab / Next Tab はタブバーの表示順（左→右）で巡回する。D&D による並べ替え済み順序に追従する
- **FR-004**: 端のタブでの巡回はラップせず停止する（最左で Previous → 何もしない、最右で Next → 何もしない）
- **FR-005**: Summary パネルはタブ巡回の対象に含める
- **FR-006**: タブが1つのみの場合は Previous Tab / Next Tab ともに何もしない
- **FR-007**: Tauri メニュー accelerator として登録し、xterm.js フォーカス中でもキャプチャ可能にする
- **FR-008**: Cmd+\`（`CmdOrCtrl+Backquote`）でウィンドウを MRU 順に切り替える（Tauri accelerator 検証後、未サポートの場合は別キーに変更）
- **FR-009**: Cmd+Shift+\`（`CmdOrCtrl+Shift+Backquote`）で MRU 逆方向にウィンドウを切り替える
- **FR-010**: ウィンドウ切り替え時、非表示（hide 状態）のウィンドウも対象に含め、選択時に show+focus する
- **FR-011**: ウィンドウが1つのみの場合は Cmd+\` は何もしない
- **FR-012**: Window メニューに "Next Window" / "Previous Window" 項目を追加し、対応する accelerator を表示する
- **FR-013**: AppState にウィンドウフォーカス履歴（MRU リスト）を追加する。メモリ内のみ、再起動時リセット
- **FR-014**: macOS の Window メニューに Minimize (Cmd+M) / Zoom / Bring All to Front を追加する
- **FR-015**: Window メニューの構造: ナビゲーション項目 → セパレータ → タブ一覧 → セパレータ → Minimize/Zoom → セパレータ → ウィンドウ一覧 → セパレータ → Bring All to Front
- **FR-016**: README.md / README.ja.md にキーボードショートカット包括一覧セクションを追加する（既存ショートカット含む）

### 非機能要件

- **NFR-001**: タブ切り替えの応答は即座（<50ms）であること
- **NFR-002**: MRU リストの管理はウィンドウ数に対して O(n) 以内であること
- **NFR-003**: ウィンドウ切り替え時の show+focus は 200ms 以内に完了すること

## 制約と仮定

- Tauri v2 の accelerator パーサーが `CmdOrCtrl+Shift+[` / `CmdOrCtrl+Shift+]` をサポートしている（サポートされない場合は別キーに変更）
- Tauri v2 の accelerator パーサーが `CmdOrCtrl+Backquote` をサポートしている可能性がある（要検証。サポートされない場合は別キーに変更）
- MRU 順のウィンドウ切り替えには、AppState にウィンドウフォーカス履歴の追跡が必要
- MRU 履歴はメモリ内のみ。アプリ再起動時はウィンドウ作成順で再初期化
- 非表示ウィンドウの show は Tauri の `window.show()` + `window.set_focus()` で実現可能
- macOS 標準 Window メニュー項目は `#[cfg(target_os = "macos")]` で条件コンパイル

## 成功基準 *(必須)*

- **SC-001**: Cmd+Shift+\[/\] でタブバーの表示順に従ったタブ切り替えが動作する
- **SC-002**: xterm.js フォーカス中でも Cmd+Shift+\[/\] でタブ切り替えが動作する（accelerator によるキャプチャ）
- **SC-003**: Window メニューに全ナビゲーション項目が表示され、accelerator が確認できる
- **SC-004**: Cmd+\` / Cmd+Shift+\` で MRU 順にウィンドウが切り替わる（Tauri 対応確認後）
- **SC-005**: 非表示ウィンドウも切り替え対象に含まれ、show+focus で復元される
- **SC-006**: macOS で Minimize / Zoom / Bring All to Front が Window メニューに表示され動作する
- **SC-007**: README.md / README.ja.md に既存含む全ショートカットの包括一覧が記載されている
