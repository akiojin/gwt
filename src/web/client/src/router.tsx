import React from "react";
import { createBrowserRouter } from "react-router-dom";
import { BranchListPage } from "./pages/BranchListPage";
import { BranchDetailPage } from "./pages/BranchDetailPage";
import { ConfigManagementPage } from "./pages/ConfigManagementPage";

/**
 * React Router設定
 *
 * URL構造:
 * - / - ブランチ一覧（ホーム）
 * - /:branchName - 個別ブランチ詳細（例: /feature-webui, /feature%2Fwebui）
 */
export const router = createBrowserRouter([
  {
    path: "/",
    element: <BranchListPage />,
  },
  {
    path: "/:branchName",
    element: <BranchDetailPage />,
  },
  {
    path: "/config",
    element: <ConfigManagementPage />,
  },
]);
