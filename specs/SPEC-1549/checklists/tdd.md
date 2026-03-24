### EditMode

1. `LeadSessionData` シリアライゼーション
   - `LeadSessionData` を JSON にシリアライズ → デシリアライズし、全フィールドが復元されること
   - 空の `ConversationHistory` / `TaskAssignments` でもエラーにならないこと
   - `HandoverDocument` が null の場合でもシリアライズ/デシリアライズが成功すること
2. `LeadCandidate` 選択
   - 候補一覧が3名（男性・女性・中性）返却されること
   - 各候補に `DisplayName`, `Personality`, `SpriteKey`, `VoiceKey`, `ToneSample` が設定されていること
   - `Personality` が `Professional` / `Friendly` / `Strict` のいずれかであること
3. 引継ぎドキュメント生成
   - `HandoverDocument` に `Summary`, `SpecStates`, `TaskProgress`, `AgentStatuses`, `OngoingPlans` が含まれること
   - 空の会話履歴でも引継ぎドキュメントが生成されること（最低限 Summary が非空）
   - `GeneratedAt` がドキュメント生成時刻と一致すること
4. 性格タイプ（Professional/Friendly/Strict）によるプロンプト変更
   - `Professional` 選択時のシステムプロンプトに丁寧語・敬語指示が含まれること
   - `Friendly` 選択時のシステムプロンプトにカジュアル語・絵文字指示が含まれること
   - `Strict` 選択時のシステムプロンプトに命令口調・簡潔指示が含まれること
   - AI性能パラメータ（model, temperature, max_tokens等）は全性格タイプで同一であること

### PlayMode

1. Lead 雇用→巡回開始
   - Lead 候補を選択して雇用するとスタジオ内に Lead スプライトが出現すること
   - 雇用後、Lead がスタジオ内を巡回（歩行アニメーション）を開始すること
   - HUD 入力フィールドが有効化され、指示入力を受け付けること
2. Lead 解雇→引継ぎ→退場
   - Lead 解雇を実行すると引継ぎドキュメントが生成されること
   - 新しい Lead を雇用すると引継ぎドキュメントが新 Lead のコンテキストにロードされること
   - 前の Lead スプライトがドアへ歩行→消滅し、新 Lead がドアから入場すること
3. SPEC 生成フロー
   - ユーザーが機能要望を入力すると Lead がインタビューを開始すること
   - インタビュー完了後に SPEC ドラフトが生成されること
   - SPEC 確定後に GitHub Issue が作成されること（`gwt-spec` ラベル付き）
