/**
 * Stores Module - SolidJS state management
 *
 * This module exports all stores for the OpenTUI implementation.
 *
 * @see specs/SPEC-d27be71b/spec.md - OpenTUI migration spec
 */

// App Store - Navigation, screen state, app config
export {
  getAppStore,
  appStore,
  appActions,
  type AppState,
} from "./appStore.js";

// Branch Store - Branch list, selection, filtering
export {
  getBranchStore,
  branchStore,
  branchActions,
  type BranchStoreState,
} from "./branchStore.js";

// UI Store - Notifications, terminal size, loading
export {
  getUIStore,
  uiStore,
  uiActions,
  type UIStoreState,
} from "./uiStore.js";
