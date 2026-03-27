### Background

Assistant startup analysis は動くが、現在の UI は `Analyzing project...` としか出さず、何をしているかが分からない。また、project を開くたびに同じ初回解析を毎回 LLM でやり直しており、起動体験と API コストの両方が無駄になる。さらに、analysis 結果が markdown で返っても transcript 側で plain text 表示されるため、見出しや箇条書きが崩れる。さらに startup path は user request 前の自動処理なので、write-capable tool を使わせず read-only に限定する必要がある。

### User Scenarios

**S1: 起動直後に何をしているか分かる**
- Given: Assistant tab を開く
- When: startup analysis が進行中
- Then: transcript に現在の分析ステップが assistant message として表示される

**S2: 同じ repository 状態ならキャッシュを使う**
- Given: 前回と同じ repository 状態で Assistant tab を再度開く
- When: startup analysis を開始する
- Then: 永続化されたキャッシュを読み、LLM を再実行せずに前回の要約を再利用する

**S3: repository 状態が変わったら再解析する**
- Given: branch / HEAD / working tree 状態が変わっている
- When: Assistant tab を開く
- Then: キャッシュ不一致として LLM 解析を再実行し、新しい結果でキャッシュを更新する

**S4: startup analysis の markdown が読める**
- Given: startup analysis result が markdown を含む
- When: Assistant transcript に表示される
- Then: heading / list / code block が markdown として描画される

### Requirements

- **FR-001**: startup analysis 中のステップを assistant transcript 上で可視化すること
- **FR-002**: startup analysis cache を file-based に永続化すること
- **FR-003**: cache key は少なくとも current branch、HEAD revision、working tree dirty state を含むこと
- **FR-004**: cache hit 時は LLM を呼ばず、cached summary を transcript に出すこと
- **FR-005**: cache miss 時だけ startup analysis を実行し、成功結果を cache file に保存すること
- **FR-006**: startup analysis では read-only tool set だけを LLM に公開し、write-capable tool は禁止すること
- **FR-007**: startup analysis の内部 prompt は transcript に表示しないこと
- **FR-008**: startup analysis で assistant が返す markdown は transcript で markdown として描画すること
- **FR-009**: 既存の手動 message send flow は壊さないこと

### Success Criteria

- **SC-001**: startup analysis 中に step message が見える
- **SC-002**: cache hit で cached summary が表示され、LLM call count が増えない
- **SC-003**: cache miss で fresh summary が表示され、cache file が更新される
- **SC-004**: startup analysis path で write-capable tool が使えない
- **SC-005**: startup analysis result の markdown が heading / list として描画される
- **SC-006**: Rust / frontend tests が追加または更新され通過する
