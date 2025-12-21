import { test, expect } from "@playwright/test";

/**
 * Web UI 全機能ウォークスルーテスト
 *
 * すべてのページとインタラクションをヘッドレスでテストします。
 */

test.describe("Web UI Walkthrough", () => {
  test.describe("ブランチ一覧ページ", () => {
    test("ページが正常に表示される", async ({ page }) => {
      await page.goto("/");

      // ヘッダーが表示される
      await expect(page.getByText("gwt Control Center")).toBeVisible();
      await expect(page.getByText("WORKTREE DASHBOARD")).toBeVisible();

      // メトリクスカードが表示される
      await expect(page.getByText("Total Branches")).toBeVisible();
      await expect(page.getByText("Active Worktrees")).toBeVisible();
    });

    test("検索機能が動作する", async ({ page }) => {
      await page.goto("/");

      // 検索ボックスを取得
      const searchBox = page.getByPlaceholder(
        "Search branches by name, type, or commit...",
      );
      await expect(searchBox).toBeVisible();

      // 「feature」で検索
      await searchBox.fill("feature");

      // フィルタリング結果が表示される（件数が変わる）
      // 検索後、featureを含むブランチのみが表示される
      const featureCards = page.locator("text=feature/").first();
      await expect(featureCards).toBeVisible();
    });

    test("ブランチカードから詳細ページに遷移できる", async ({ page }) => {
      await page.goto("/");

      // View Details リンクをクリック
      const viewDetailsLink = page.getByRole("link", {
        name: "View Details →",
      });
      await viewDetailsLink.first().click();

      // 詳細ページに遷移
      await expect(page.getByText("BRANCH DETAIL")).toBeVisible();
    });
  });

  test.describe("ブランチ詳細ページ", () => {
    test("直接URLアクセスでページが表示される（SPAフォールバック）", async ({
      page,
    }) => {
      // URLエンコードされたブランチ名で直接アクセス
      await page.goto("/develop");

      // ページが表示される（404にならない）
      await expect(page.getByText("BRANCH DETAIL")).toBeVisible();
    });

    test("AIツール起動UIが表示される（Worktreeあり）", async ({ page }) => {
      // Worktreeがあるブランチを使用
      await page.goto("/feature%2Fchrome-extension");

      // ツールランチャーセクションが表示される
      await expect(
        page.getByRole("heading", { name: "AIツール起動" }),
      ).toBeVisible();

      // AIツール選択ラベルが表示される（exact: trueで厳密マッチ）
      await expect(page.getByText("AIツール", { exact: true })).toBeVisible();

      // 起動モード選択が表示される（exact: trueで厳密マッチ）
      await expect(page.getByText("起動モード", { exact: true })).toBeVisible();

      // セッション起動ボタンが表示される
      await expect(
        page.getByRole("button", { name: "セッションを起動" }),
      ).toBeVisible();
    });

    test("ナビゲーションリンクが機能する", async ({ page }) => {
      await page.goto("/develop");

      // ブランチ一覧へのリンクをクリック
      await page.getByRole("link", { name: "← ブランチ一覧" }).click();

      // ブランチ一覧ページに戻る
      await expect(page.getByText("gwt Control Center")).toBeVisible();
    });

    test("設定ページへのリンクが機能する", async ({ page }) => {
      // Worktreeがあるブランチを使用
      await page.goto("/feature%2Fchrome-extension");

      // カスタムツール設定リンクをクリック
      await page.getByRole("link", { name: "カスタムツール設定" }).click();

      // 設定ページに遷移
      await expect(page.getByText("環境変数の管理")).toBeVisible();
    });
  });

  test.describe("設定ページ", () => {
    test("直接URLアクセスでページが表示される（SPAフォールバック）", async ({
      page,
    }) => {
      await page.goto("/config");

      // ページが表示される（404にならない）
      await expect(page.getByText("環境変数の管理")).toBeVisible();
      await expect(page.getByText("CONFIG")).toBeVisible();
    });

    test("環境変数セクションが表示される", async ({ page }) => {
      await page.goto("/config");

      // 共通環境変数セクションが表示される（見出しを使って一意に特定）
      await expect(
        page.getByRole("heading", { name: "共通環境変数" }),
      ).toBeVisible();

      // ツール固有の環境変数セクションが表示される（見出しを使って一意に特定）
      await expect(
        page.getByRole("heading", { name: "ツール固有の環境変数" }),
      ).toBeVisible();
    });

    test("変数追加ボタンが機能する", async ({ page }) => {
      await page.goto("/config");

      // 変数追加ボタンをクリック
      await page.getByRole("button", { name: "変数を追加" }).click();

      // 新しい入力行が追加される（キー入力欄が増える）
      const keyInputs = page.getByPlaceholder("EXAMPLE_KEY");
      await expect(keyInputs.first()).toBeVisible();
    });

    test("保存ボタンが表示される", async ({ page }) => {
      await page.goto("/config");

      // 保存ボタンが表示される
      await expect(page.getByRole("button", { name: "保存" })).toBeVisible();
    });

    test("ブランチ一覧へのリンクが機能する", async ({ page }) => {
      await page.goto("/config");

      // ブランチ一覧へのリンクをクリック
      await page.getByRole("link", { name: "← ブランチ一覧へ" }).click();

      // ブランチ一覧ページに戻る
      await expect(page.getByText("gwt Control Center")).toBeVisible();
    });
  });

  test.describe("ページ間ナビゲーション", () => {
    test("一覧 → 詳細 → 設定 → 一覧 のフローが機能する", async ({ page }) => {
      // 1. ブランチ一覧ページ
      await page.goto("/");
      await expect(page.getByText("gwt Control Center")).toBeVisible();

      // 2. 詳細ページへ遷移
      await page.getByRole("link", { name: "View Details →" }).first().click();
      await expect(page.getByText("BRANCH DETAIL")).toBeVisible();

      // 3. 設定ページへ遷移
      await page.getByRole("link", { name: "カスタムツール設定" }).click();
      await expect(page.getByText("環境変数の管理")).toBeVisible();

      // 4. ブランチ一覧に戻る
      await page.getByRole("link", { name: "← ブランチ一覧へ" }).click();
      await expect(page.getByText("gwt Control Center")).toBeVisible();
    });
  });

  test.describe("エラーハンドリング", () => {
    test("存在しないAPIエンドポイントは404を返す", async ({ page }) => {
      const response = await page.request.get("/api/nonexistent");
      expect(response.status()).toBe(404);
    });

    test("存在しないページはSPAフォールバックでindex.htmlを返す", async ({
      page,
    }) => {
      await page.goto("/nonexistent-page");
      // SPAフォールバックにより、index.htmlが返される
      // React Routerがハンドルし、何らかのUIが表示される
      // （このテストはSPAフォールバックが機能していることを確認）
      const response = await page.request.get("/nonexistent-page");
      expect(response.status()).toBe(200);
    });
  });
});
