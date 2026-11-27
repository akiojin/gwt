import { useStoreSelector } from "../store/useStoreSelector.js";

const selectNotice = notice => notice;

// Persistent project-state load diagnostics, cleared only after a successful retry.
export function WorkspaceStateNotice({ store, send }) {
  const notice = useStoreSelector(store, selectNotice, { frame: true });
  if (!notice) return null;
  return (
    <div className="workspace-state-notice" role="alert">
      <span>{`${notice.path}: ${notice.message}`}</span>
      <button type="button" onClick={() => send({ kind: "retry_workspace_state_load" })}>Retry</button>
    </div>
  );
}
