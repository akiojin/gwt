# バグ修正仕様: macOS リリースの署名/公証を必須化して Gatekeeper エラーを防止

**仕様ID**: `SPEC-e0e11640`
**作成日**: 2026-02-24
**更新日**: 2026-02-24
**ステータス**: ドラフト
**カテゴリ**: GUI
**依存仕様**:

- なし

**入力**: ユーザー説明: "macOS リリースで署名と公証を必須化し Gatekeeper で壊れている判定を回避する"

## 背景

- v7.12.0 の macOS インストール後に「壊れているため開けません」エラーが発生。
- 署名が `adhoc`、`TeamIdentifier` 未設定、`com.apple.quarantine` 付与の状態では Gatekeeper が起動を拒否する。
- リリースワークフローに署名/公証の必須手順がないため、今後も同様の事故が再発する。

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - macOS で正常起動できる (優先度: P0)

macOS ユーザーとして、リリース版の gwt をインストール後に通常起動したい。

**独立したテスト**: 署名/公証済み DMG を Gatekeeper 評価に通し、起動可能であることを確認する。

**受け入れシナリオ**:

1. **前提条件** Release DMG を生成済み、**操作** `spctl --assess --verbose=4` を実行、**期待結果** `accepted` になる。
2. **前提条件** `gwt.app` を評価、**操作** `codesign -dv --verbose=4` を実行、**期待結果** `Developer ID Application` 署名と `TeamIdentifier` が付与されている。

---

### ユーザーストーリー 2 - 署名/公証の欠落はリリースで検出される (優先度: P1)

リリース担当として、署名・公証が欠落した成果物が公開されないようにしたい。

**独立したテスト**: Release workflow が必要なシークレットを検出し、欠落時に macOS ジョブを失敗させることを確認する。

**受け入れシナリオ**:

1. **前提条件** macOS 署名シークレット未設定、**操作** Release workflow を実行、**期待結果** macOS ジョブが `missing signing/notarization secrets` で失敗する。

## エッジケース

- 署名対象のバンドルにリソース欠落がある場合、Gatekeeper が `code has no resources but signature indicates they must be present` を返す。
- 署名/公証済みでも `com.apple.quarantine` が付与されるため、`spctl` 判定で受け入れられる状態が必要。

## 要件 *(必須)*

### 機能要件

- **FR-001**: Release workflow は macOS で `Developer ID Application` 署名を必須とする。
- **FR-002**: Release workflow は macOS DMG を notarytool で公証し、staple する。
- **FR-003**: 署名/公証に必要なシークレットが欠落した場合、macOS release job は失敗する。

### 非機能要件

- **NFR-001**: 署名/公証は macOS ジョブ内で完結し、他プラットフォームには影響しない。

## 制約と仮定

- 署名/公証に必要な Apple Developer ID 証明書と Notary API Key が GitHub Secrets に登録済みであることを前提とする。
- 署名は `APPLE_CERT_APP_BASE64` / `APPLE_CERTIFICATE_PASSWORD` の証明書で行い、identity はキーチェーンから自動選択する。
- 公証は `APPLE_ID` / `APPLE_ID_PASSWORD` / `APPLE_TEAM_ID` を利用する。

## 成功基準 *(必須)*

- **SC-001**: Release で生成された DMG が Gatekeeper に受理される。
- **SC-002**: 署名/公証が未設定の場合に macOS release job が失敗して公開を防止する。
