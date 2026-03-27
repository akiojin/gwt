### Background

AssistantPanel のチャット表示で 2 つの UX 崩れがある。1 つ目はメッセージ本文中の改行が保持されず、複数行の内容が 1 行に潰れて表示されること。2 つ目はユーザー入力が `assistant_send_message` 完了まで履歴に反映されず、送信直後に自分の発言が見えないこと。コードや箇条書き、複数段落の応答が読みにくく、送信後のフィードバックも遅れる。加えて、送信済み user message を `ArrowUp` / `ArrowDown` で辿れないため、直前の入力を再利用しにくい。

### User Scenarios

**S1: Assistant の複数行応答を読む**
- Given: Assistant が改行を含む本文を返す
- When: AssistantPanel のメッセージ一覧に表示する
- Then: 本文中の改行がそのまま視認できる

**S2: ユーザーの複数行入力を見返す**
- Given: ユーザーが複数行テキストを送信する
- When: 送信済みメッセージがチャット履歴に表示される
- Then: 改行位置が維持される

**S3: 送信直後に自分の入力を確認する**
- Given: Assistant session が有効で、ユーザーがメッセージを送信する
- When: `assistant_send_message` の応答を待っている
- Then: user message が即座に履歴へ表示され、待機中であることも視認できる

**S4: 直前の user input を再利用する**
- Given: 現セッションで user message を複数回送信済み
- When: composer で `ArrowUp` / `ArrowDown` を押す
- Then: 送信済み user message 履歴を遡れる
- And: 履歴突入前に編集中だった draft は、最新位置へ戻った時に復元される

### Requirements

- **FR-001**: `AssistantPanel` のメッセージ本文は改行を保持して表示すること
- **FR-002**: 長い単語やコード片で横スクロールを強制しないこと
- **FR-003**: user message は送信直後に楽観表示されること
- **FR-004**: `assistant_send_message` 成功時は backend の正式 state へ同期すること
- **FR-005**: `assistant_send_message` の応答待ち中は推論中インジケーターを表示すること
- **FR-006**: `assistant_send_message` 失敗時は楽観表示をロールバックし、入力テキストを復元すること
- **FR-007**: 既存の Enter/Shift+Enter/IME 送信挙動を変えないこと
- **FR-008**: 履歴対象は現セッションで正常送信した user message のみとすること
- **FR-009**: `ArrowUp` は caret が先頭行にある時だけ、`ArrowDown` は caret が末尾行にある時だけ履歴移動すること
- **FR-010**: 履歴移動に入る前の未送信 draft は、最新位置へ戻った時に復元すること
- **FR-011**: テキスト選択中や修飾キー付きの上下カーソルでは履歴移動を発火しないこと

### Success Criteria

- **SC-001**: 改行を含む assistant message が複数行で描画される
- **SC-002**: 改行を含む user message が複数行で描画される
- **SC-003**: メッセージ送信直後に user message が履歴へ表示される
- **SC-004**: 応答待ち中に推論中インジケーターが表示される
- **SC-005**: `ArrowUp` で現セッションの送信済み user message を遡れる
- **SC-006**: `ArrowDown` で新しい履歴へ戻り、末尾を抜けると元の draft が復元される
- **SC-007**: 対象コンポーネントのテストが追加され通過する
