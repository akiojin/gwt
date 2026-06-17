// SPEC-3064 Phase 3 (E6a) — File Tree window surface extracted from app.js.
// Owns the per-window file tree state map (directory cache, worktree
// picker, text/hex viewer with dirty tracking), the highlight.js lazy
// loader, the hex editor helpers, renderFileTree / renderFileTreeViewer /
// the worktree picker + discard/conflict modals, the File Tree window
// mount, and the file_tree_* / file_content_* receive() bodies. Pure
// movement from app.js: behavior, DOM output, and WS protocol are
// unchanged; the moved code keeps its original app.js indentation. The
// only textual changes: in-module self-references through
// `*` became direct local calls and
// the mount's `socketTransport.send({ kind: "focus_window", ... })`
// became the injected sendWindowFocus dep.
//
// deps:
// - send(message): forward a frontend event over the WebSocket bridge.
// - makeEl(tag, options, children) / clearChildren(el): shared DOM helpers
//   owned by app.js.
// - focusWindowLocally(windowId): local z-order focus bookkeeping.
// - sendWindowFocus(windowId): backend focus_window notification.
// - windowMap: workspace window element map owned by app.js
//   (workspace-window-manager).
export function createFileTreeSurface({
  send,
  makeEl,
  clearChildren,
  focusWindowLocally,
  sendWindowFocus,
  windowMap,
}) {
      const fileTreeStateMap = new Map();

      function ensureFileTreeState(windowId) {
        if (!fileTreeStateMap.has(windowId)) {
          fileTreeStateMap.set(windowId, {
            loaded: new Map(),
            expanded: new Set(),
            loading: new Set(),
            selectedPath: "",
            error: "",
            // SPEC-2009 amendment: per-window picker + viewer state.
            picker: {
              open: false,
              loading: false,
              entries: [],
              error: "",
            },
            selectedWorktreeId: "",
            selectedWorktreeLabel: "",
            splitterRatio: 0.4,
            viewer: {
              path: "",
              mode: "empty", // empty | text | binary | hex | error | loading
              text: "",
              encoding: "",
              totalSize: 0,
              hexOffset: 0,
              hexBytes: "",
              error: { kind: "", message: "", size: null, limit: null },
              // SPEC-2009 amendment Phase 2: dirty / edit / save state.
              dirty: false,
              originalText: "",
              originalBytes: null, // Uint8Array
              originalEncoding: "",
              originalNewline: "lf",
              originalHasBom: false,
              originalMtime: 0,
              originalSize: 0,
              readOnly: false,
              savedAt: 0,
              saveInFlight: false,
              undoStack: [],
              redoStack: [],
            },
            // Pending navigation queued behind the Discard modal so we can
            // continue or abort after the user resolves the unsaved edit.
            discardModal: {
              open: false,
              pendingAction: null, // { kind: 'switch_file'|'open_picker'|'close_window'|'switch_worktree', payload }
            },
            conflictModal: {
              open: false,
              currentMtime: 0,
              currentSize: 0,
              pendingPayload: null, // SaveFileContent payload that triggered the conflict
            },
          });
        }
        return fileTreeStateMap.get(windowId);
      }

      function requestFileTreeWorktrees(windowId) {
        const state = ensureFileTreeState(windowId);
        state.picker.loading = true;
        state.picker.error = "";
        send({ kind: "list_file_tree_worktrees", id: windowId });
      }

      function selectFileTreeWorktree(windowId, worktreeId) {
        send({
          kind: "select_file_tree_worktree",
          id: windowId,
          worktree_id: worktreeId,
        });
      }

      function requestFileContent(windowId, path, mode, hexOffset = null, hexLength = null) {
        send({
          kind: "load_file_content",
          id: windowId,
          path,
          mode,
          hex_offset: hexOffset,
          hex_length: hexLength,
        });
      }

      function formatHexDump(offset, base64Bytes) {
        let binary;
        try {
          binary = atob(base64Bytes || "");
        } catch (e) {
          return "(invalid hex chunk)";
        }
        const bytes = new Uint8Array(binary.length);
        for (let i = 0; i < binary.length; i += 1) {
          bytes[i] = binary.charCodeAt(i);
        }
        const BYTES_PER_LINE = 16;
        const lines = [];
        for (let i = 0; i < bytes.length; i += BYTES_PER_LINE) {
          const slice = bytes.slice(i, i + BYTES_PER_LINE);
          const lineOffset = (offset + i).toString(16).padStart(8, "0").toUpperCase();
          const hexParts = [];
          const asciiParts = [];
          for (let j = 0; j < BYTES_PER_LINE; j += 1) {
            if (j < slice.length) {
              const b = slice[j];
              hexParts.push(b.toString(16).padStart(2, "0").toUpperCase());
              asciiParts.push(b >= 0x20 && b < 0x7F ? String.fromCharCode(b) : ".");
            } else {
              hexParts.push("  ");
              asciiParts.push(" ");
            }
          }
          lines.push(lineOffset + "  " + hexParts.join(" ") + "  |" + asciiParts.join("") + "|");
        }
        return lines.join("\n");
      }

      function formatBytes(size) {
        if (size === null || size === undefined) {
          return "";
        }
        if (size < 1024) {
          return size + " B";
        }
        if (size < 1024 * 1024) {
          return (size / 1024).toFixed(1) + " KiB";
        }
        return (size / (1024 * 1024)).toFixed(2) + " MiB";
      }


      function openWorktreePicker(windowId) {
        const state = ensureFileTreeState(windowId);
        state.picker.open = true;
        state.picker.entries = [];
        state.picker.error = "";
        renderWorktreePicker(windowId);
        requestFileTreeWorktrees(windowId);
      }

      function closeWorktreePicker(windowId) {
        const state = ensureFileTreeState(windowId);
        state.picker.open = false;
        renderWorktreePicker(windowId);
      }

      function renderWorktreePicker(windowId) {
        const modal = document.getElementById("file-tree-worktree-picker-modal");
        if (!modal) return;
        const shell = modal.querySelector(".modal-shell");
        if (!shell) return;
        const state = ensureFileTreeState(windowId);
        clearChildren(shell);
        if (!state.picker.open) {
          modal.setAttribute("aria-hidden", "true");
          modal.style.display = "none";
          modal.dataset.windowId = "";
          return;
        }
        modal.dataset.windowId = windowId;
        modal.setAttribute("aria-hidden", "false");
        modal.style.display = "flex";

        const header = makeEl("header", { className: "worktree-picker-header" }, [
          makeEl("h2", { text: "Select Worktree" }),
          makeEl("button", {
            className: "icon-button",
            text: "×",
            attrs: { type: "button", "aria-label": "Close picker" },
            dataset: { pickerAction: "cancel" },
          }),
        ]);
        const bodyContainer = makeEl("div", { className: "worktree-picker-body" });
        if (state.picker.loading && state.picker.entries.length === 0) {
          bodyContainer.appendChild(
            makeEl("div", { className: "worktree-picker-empty", text: "Loading worktrees…" }),
          );
        } else if (state.picker.error) {
          bodyContainer.appendChild(
            makeEl("div", { className: "worktree-picker-error", text: state.picker.error }),
          );
        } else if (state.picker.entries.length === 0) {
          bodyContainer.appendChild(
            makeEl("div", {
              className: "worktree-picker-empty",
              text: "No worktrees available. Use Start Work to create a new work.",
            }),
          );
        } else {
          const list = makeEl("div", { className: "worktree-picker-list" });
          for (const entry of state.picker.entries) {
            const row = makeEl(
              "button",
              {
                className: "worktree-picker-row",
                attrs: { type: "button" },
                dataset: { worktreeId: entry.id },
              },
              [
                makeEl("div", { className: "worktree-picker-row-label", text: entry.label }),
                makeEl("div", { className: "worktree-picker-row-meta" }, [
                  makeEl("span", {
                    className: "worktree-picker-kind",
                    text: entry.kind === "bare_main" ? "main" : "workspace",
                  }),
                  entry.is_active
                    ? makeEl("span", { className: "worktree-picker-active", text: "active" })
                    : null,
                  makeEl("span", { className: "worktree-picker-path", text: entry.path }),
                ]),
              ],
            );
            row.addEventListener("click", (event) => {
              event.preventDefault();
              state.selectedWorktreeId = entry.id;
              state.selectedWorktreeLabel = entry.label;
              closeWorktreePicker(windowId);
              selectFileTreeWorktree(windowId, entry.id);
              renderWorktreeTrigger(windowId);
              // Reset tree + viewer state when switching worktree.
              state.loaded.clear();
              state.expanded.clear();
              state.loading.clear();
              state.error = "";
              state.viewer = {
                path: "",
                mode: "empty",
                text: "",
                encoding: "",
                totalSize: 0,
                hexOffset: 0,
                hexBytes: "",
                error: { kind: "", message: "", size: null, limit: null },
              };
              renderFileTreeViewer(windowId);
              renderFileTree(windowId);
            });
            list.appendChild(row);
          }
          bodyContainer.appendChild(list);
        }
        shell.appendChild(header);
        shell.appendChild(bodyContainer);
        shell
          .querySelector('[data-picker-action="cancel"]')
          ?.addEventListener("click", () => closeWorktreePicker(windowId));
      }

      // SPEC-2009 amendment Phase 2b: lazy-load highlight.js once on first
      // text viewer use. Bundled module sits at /assets/highlight/.
      let highlightModulePromise = null;
      function loadHighlightModule() {
        if (!highlightModulePromise) {
          highlightModulePromise = import("/assets/highlight/highlight.min.js")
            .then((mod) => mod.default || mod)
            .catch((err) => {
              console.warn("highlight.js failed to load", err);
              return null;
            });
        }
        return highlightModulePromise;
      }

      const FILE_EXT_TO_LANGUAGE = {
        js: "javascript", mjs: "javascript", cjs: "javascript", jsx: "javascript",
        ts: "typescript", tsx: "typescript",
        rs: "rust", py: "python", rb: "ruby", go: "go", java: "java",
        c: "c", h: "c", cpp: "cpp", cc: "cpp", hpp: "cpp", cs: "csharp",
        sh: "bash", bash: "bash", zsh: "bash", fish: "bash",
        json: "json", yaml: "yaml", yml: "yaml", toml: "ini", ini: "ini",
        md: "markdown", markdown: "markdown",
        html: "xml", htm: "xml", xml: "xml", svg: "xml",
        css: "css", scss: "scss", less: "less",
        sql: "sql", dockerfile: "dockerfile",
        kt: "kotlin", swift: "swift", php: "php", lua: "lua",
        vue: "xml", graphql: "graphql", gql: "graphql",
      };

      function detectLanguageByExtension(path) {
        if (!path) return "";
        const base = String(path).split("/").pop() || "";
        if (/^Dockerfile/i.test(base)) return "dockerfile";
        if (/^Makefile/i.test(base)) return "makefile";
        const dot = base.lastIndexOf(".");
        if (dot < 0) return "";
        const ext = base.slice(dot + 1).toLowerCase();
        return FILE_EXT_TO_LANGUAGE[ext] || "";
      }

      function applySyntaxHighlight(codeEl, text, language) {
        if (!codeEl) return;
        // Always set raw text first so the viewer renders something even
        // before highlight.js loads (or if it fails to load entirely).
        codeEl.textContent = text || "";
        codeEl.className = language ? `hljs language-${language}` : "hljs";
        loadHighlightModule().then((hljs) => {
          if (!hljs) return;
          try {
            if (language && hljs.getLanguage && hljs.getLanguage(language)) {
              const html = hljs.highlight(text || "", { language, ignoreIllegals: true }).value;
              codeEl.innerHTML = html;
              codeEl.className = `hljs language-${language}`;
            } else {
              const result = hljs.highlightAuto(text || "");
              codeEl.innerHTML = result.value;
              codeEl.className = `hljs language-${result.language || "plaintext"}`;
            }
          } catch (e) {
            // Fall back to plain text on highlight failure.
            codeEl.textContent = text || "";
          }
        });
      }

      // SPEC-2009 amendment Phase 2: helper to decode the binary chunk sent
      // by the backend over base64 so the hex viewer can mutate single
      // bytes locally before issuing a save_file_content(mode=hex).
      function decodeBase64ToBytes(b64) {
        let binary;
        try {
          binary = atob(b64 || "");
        } catch (_e) {
          return new Uint8Array();
        }
        const bytes = new Uint8Array(binary.length);
        for (let i = 0; i < binary.length; i += 1) {
          bytes[i] = binary.charCodeAt(i);
        }
        return bytes;
      }

      function encodeBytesToBase64(bytes) {
        if (!bytes || bytes.length === 0) return "";
        let binary = "";
        for (let i = 0; i < bytes.length; i += 1) {
          binary += String.fromCharCode(bytes[i]);
        }
        return btoa(binary);
      }

      function recomputeHexDirty(state) {
        const v = state.viewer;
        if (v.mode !== "hex" || !v.originalBytes) {
          return;
        }
        const current = decodeBase64ToBytes(v.hexBytes || "");
        if (current.length !== v.originalBytes.length) {
          v.dirty = true;
          return;
        }
        for (let i = 0; i < current.length; i += 1) {
          if (current[i] !== v.originalBytes[i]) {
            v.dirty = true;
            return;
          }
        }
        v.dirty = false;
      }

      function requestSaveFileContent(windowId) {
        const state = ensureFileTreeState(windowId);
        const v = state.viewer;
        if (v.saveInFlight || v.readOnly || !v.dirty) {
          return;
        }
        const payload = {
          kind: "save_file_content",
          id: windowId,
          path: v.path,
          mode: v.mode,
          expected_mtime: v.originalMtime,
          expected_size: v.originalSize,
        };
        if (v.mode === "text") {
          payload.text = v.text;
          payload.encoding = (v.originalEncoding || "utf-8").toLowerCase();
          payload.newline = (v.originalNewline || "lf").toLowerCase();
          payload.has_bom = v.originalHasBom;
        } else if (v.mode === "hex") {
          // Hex save sends a single byte at a single offset (replace-only).
          // We pull the dirty byte by diffing against originalBytes.
          const current = decodeBase64ToBytes(v.hexBytes || "");
          let dirtyOffset = -1;
          for (let i = 0; i < current.length; i += 1) {
            if (current[i] !== (v.originalBytes ? v.originalBytes[i] : -1)) {
              dirtyOffset = i;
              break;
            }
          }
          if (dirtyOffset < 0) {
            return;
          }
          payload.hex_offset = v.hexOffset + dirtyOffset;
          payload.hex_byte = current[dirtyOffset];
        } else {
          return;
        }
        v.saveInFlight = true;
        state.lastSavePayload = payload;
        send(payload);
        renderFileTreeViewer(windowId);
      }

      function applyAfterSaveContinuation(windowId) {
        const state = ensureFileTreeState(windowId);
        const pending = state.discardModal && state.discardModal.pendingAction;
        if (!pending || !pending.queuedFromDiscard) return;
        state.discardModal.pendingAction = null;
        runPendingNavigation(windowId, pending);
      }

      function runPendingNavigation(windowId, pending) {
        if (!pending) return;
        switch (pending.kind) {
          case "switch_file":
            beginViewerForFile(windowId, pending.path);
            break;
          case "open_picker":
            openWorktreePicker(windowId);
            break;
          case "switch_worktree":
            // Re-emit the worktree selection that was queued behind the modal.
            selectFileTreeWorktree(windowId, pending.worktreeId);
            break;
          case "close_window":
            // Forward to the backend close path so persistence stays in sync.
            send({ kind: "close_window", id: windowId });
            break;
          default:
            break;
        }
      }

      function beginViewerForFile(windowId, path) {
        const state = ensureFileTreeState(windowId);
        state.viewer = {
          ...state.viewer,
          path,
          mode: "loading",
          text: "",
          encoding: "",
          totalSize: 0,
          hexOffset: 0,
          hexBytes: "",
          error: { kind: "", message: "", size: null, limit: null },
          dirty: false,
          originalText: "",
          originalBytes: null,
          originalEncoding: "",
          originalNewline: "lf",
          originalHasBom: false,
          originalMtime: 0,
          originalSize: 0,
          readOnly: false,
          savedAt: 0,
          saveInFlight: false,
          undoStack: [],
          redoStack: [],
        };
        renderFileTreeViewer(windowId);
        requestFileContent(windowId, path, "text");
      }

      function queueNavigationGuardedByDirty(windowId, pendingAction) {
        const state = ensureFileTreeState(windowId);
        if (state.viewer.dirty) {
          state.discardModal = {
            open: true,
            pendingAction: { ...pendingAction, queuedFromDiscard: true },
          };
          renderDiscardModal(windowId);
          return true;
        }
        return false;
      }

      function closeDiscardModal(windowId) {
        const state = ensureFileTreeState(windowId);
        state.discardModal = { open: false, pendingAction: null };
        renderDiscardModal(windowId);
      }

      function renderDiscardModal(windowId) {
        const modal = document.getElementById("file-tree-discard-modal");
        if (!modal) return;
        const shell = modal.querySelector(".modal-shell");
        if (!shell) return;
        const state = ensureFileTreeState(windowId);
        clearChildren(shell);
        if (!state.discardModal.open) {
          modal.setAttribute("aria-hidden", "true");
          modal.dataset.windowId = "";
          return;
        }
        modal.dataset.windowId = windowId;
        modal.setAttribute("aria-hidden", "false");
        const header = makeEl("header", { className: "discard-modal-header" }, [
          makeEl("h2", { text: "Unsaved changes" }),
        ]);
        const bodyText = makeEl("div", { className: "discard-modal-body" }, [
          makeEl("p", {
            text: `${state.viewer.path} has unsaved changes. Save them or discard before continuing.`,
          }),
        ]);
        const footer = makeEl("footer", { className: "discard-modal-footer" });
        const saveBtn = makeEl("button", {
          className: "wizard-button primary",
          attrs: { type: "button" },
          text: "Save",
        });
        const discardBtn = makeEl("button", {
          className: "wizard-button",
          attrs: { type: "button" },
          text: "Discard",
        });
        const cancelBtn = makeEl("button", {
          className: "wizard-button",
          attrs: { type: "button" },
          text: "Cancel",
        });
        saveBtn.addEventListener("click", () => {
          requestSaveFileContent(windowId);
          // Close modal but keep pending action; resume after file_content_saved.
          state.discardModal.open = false;
          renderDiscardModal(windowId);
        });
        discardBtn.addEventListener("click", () => {
          // Roll back to the original baseline and run the pending action.
          const v = state.viewer;
          if (v.mode === "text") {
            v.text = v.originalText;
          } else if (v.mode === "hex") {
            v.hexBytes = encodeBytesToBase64(v.originalBytes || new Uint8Array());
          }
          v.dirty = false;
          const pending = state.discardModal.pendingAction;
          state.discardModal = { open: false, pendingAction: null };
          renderDiscardModal(windowId);
          runPendingNavigation(windowId, pending);
        });
        cancelBtn.addEventListener("click", () => closeDiscardModal(windowId));
        footer.appendChild(saveBtn);
        footer.appendChild(discardBtn);
        footer.appendChild(cancelBtn);
        shell.appendChild(header);
        shell.appendChild(bodyText);
        shell.appendChild(footer);
      }

      function renderConflictModal(windowId) {
        const modal = document.getElementById("file-tree-conflict-modal");
        if (!modal) return;
        const shell = modal.querySelector(".modal-shell");
        if (!shell) return;
        const state = ensureFileTreeState(windowId);
        clearChildren(shell);
        if (!state.conflictModal.open) {
          modal.setAttribute("aria-hidden", "true");
          modal.dataset.windowId = "";
          return;
        }
        modal.dataset.windowId = windowId;
        modal.setAttribute("aria-hidden", "false");
        const header = makeEl("header", { className: "conflict-modal-header" }, [
          makeEl("h2", { text: "File changed externally" }),
        ]);
        const bodyText = makeEl("div", { className: "conflict-modal-body" }, [
          makeEl("p", {
            text: `${state.viewer.path} was modified outside the editor. Choose how to proceed.`,
          }),
        ]);
        const footer = makeEl("footer", { className: "conflict-modal-footer" });
        const overwriteBtn = makeEl("button", {
          className: "wizard-button primary",
          attrs: { type: "button" },
          text: "Overwrite",
        });
        const reloadBtn = makeEl("button", {
          className: "wizard-button",
          attrs: { type: "button" },
          text: "Reload from disk",
        });
        const cancelBtn = makeEl("button", {
          className: "wizard-button",
          attrs: { type: "button" },
          text: "Cancel",
        });
        overwriteBtn.addEventListener("click", () => {
          // Re-issue the save with the latest expected_metadata so the
          // domain layer skips its conflict gate this time.
          const v = state.viewer;
          v.originalMtime = state.conflictModal.currentMtime;
          v.originalSize = state.conflictModal.currentSize;
          state.conflictModal = { open: false, currentMtime: 0, currentSize: 0, pendingPayload: null };
          renderConflictModal(windowId);
          requestSaveFileContent(windowId);
        });
        reloadBtn.addEventListener("click", () => {
          const v = state.viewer;
          state.conflictModal = { open: false, currentMtime: 0, currentSize: 0, pendingPayload: null };
          renderConflictModal(windowId);
          // Throw away the unsaved edit and re-read from disk.
          beginViewerForFile(windowId, v.path);
        });
        cancelBtn.addEventListener("click", () => {
          state.conflictModal = { open: false, currentMtime: 0, currentSize: 0, pendingPayload: null };
          renderConflictModal(windowId);
        });
        footer.appendChild(overwriteBtn);
        footer.appendChild(reloadBtn);
        footer.appendChild(cancelBtn);
        shell.appendChild(header);
        shell.appendChild(bodyText);
        shell.appendChild(footer);
      }

      function renderFileTreeViewer(windowId) {
        const state = ensureFileTreeState(windowId);
        // The workspace window element exposes its id via `data-id`
        // (see `ensureWindow`), not `data-window-id`. Using the right
        // attribute is required for the viewer DOM lookup to resolve in
        // both production and Playwright fixtures.
        const surface = document.querySelector(
          `[data-id='${CSS.escape(windowId)}'] .file-tree-viewer`,
        );
        if (!surface) return;
        const header = surface.querySelector(".file-tree-viewer-header");
        const body = surface.querySelector(".file-tree-viewer-body");
        if (!header || !body) return;
        clearChildren(header);
        clearChildren(body);
        const v = state.viewer;
        const sizeLabel = v.totalSize ? formatBytes(v.totalSize) : "";
        const headerPath = makeEl("span", { className: "file-tree-viewer-path", text: v.path || "" });
        const dirtyMarker = makeEl("span", {
          className: "file-tree-viewer-dirty",
          text: "●",
        });
        switch (v.mode) {
          case "empty":
            header.appendChild(
              makeEl("span", {
                className: "file-tree-viewer-placeholder",
                text: "No file selected",
              }),
            );
            body.appendChild(
              makeEl("div", {
                className: "file-tree-viewer-empty",
                text: "Select a file to view its contents.",
              }),
            );
            break;
          case "loading":
            header.appendChild(headerPath);
            body.appendChild(
              makeEl("div", { className: "file-tree-viewer-empty", text: "Loading…" }),
            );
            break;
          case "text": {
            header.appendChild(headerPath);
            // SPEC-2009 Phase 2b: dirty marker / Saved badge live in the
            // header as toggleable elements so the input handler can flip
            // their visibility without a full re-render. Keeping the
            // textarea alive across keystrokes is what stops focus loss
            // mid-typing (Phase 2 had a re-render-on-input loop that
            // recreated the textarea each character — visible in headed
            // Playwright but masked by fill() in the headless smoke).
            dirtyMarker.style.display = v.dirty ? "" : "none";
            header.appendChild(dirtyMarker);
            const langBadge = makeEl("span", {
              className: "file-tree-viewer-lang",
              text: detectLanguageByExtension(v.path).toUpperCase() || "PLAIN",
            });
            header.appendChild(langBadge);
            header.appendChild(
              makeEl("span", {
                className: "file-tree-viewer-meta",
                text: (v.encoding || "") + " · " + (v.originalNewline || "lf").toUpperCase() + " · " + sizeLabel,
              }),
            );
            if (v.readOnly) {
              header.appendChild(
                makeEl("span", {
                  className: "file-tree-viewer-readonly",
                  text: "read-only",
                }),
              );
            }
            const saveBtn = makeEl("button", {
              className: "wizard-button file-tree-viewer-save",
              attrs: { type: "button" },
              text: v.saveInFlight ? "Saving…" : "Save",
            });
            const updateSaveBtn = () => {
              saveBtn.textContent = v.saveInFlight ? "Saving…" : "Save";
              if (!v.dirty || v.readOnly || v.saveInFlight) {
                saveBtn.setAttribute("disabled", "");
              } else {
                saveBtn.removeAttribute("disabled");
              }
            };
            updateSaveBtn();
            saveBtn.addEventListener("click", () => requestSaveFileContent(windowId));
            header.appendChild(saveBtn);
            const savedBadge = makeEl("span", {
              className: "file-tree-viewer-saved",
              text: "Saved",
            });
            savedBadge.style.display = v.savedAt && Date.now() - v.savedAt < 2000 ? "" : "none";
            header.appendChild(savedBadge);

            // Overlay editor: highlighted <pre> sits behind a transparent
            // <textarea>. Both share the same monospace metrics so the
            // overlay aligns character-for-character. Scroll syncs in
            // both directions so the highlight follows the caret.
            const wrap = makeEl("div", { className: "file-tree-viewer-editor-wrap" });
            const language = detectLanguageByExtension(v.path);
            const hlPre = makeEl("pre", { className: "file-tree-viewer-hl" });
            const hlCode = makeEl("code", {
              className: language ? `hljs language-${language}` : "hljs",
            });
            hlPre.appendChild(hlCode);
            const textarea = makeEl("textarea", {
              className: "file-tree-viewer-text file-tree-viewer-editor",
              attrs: { spellcheck: "false", wrap: "off" },
            });
            textarea.value = v.text;
            if (v.readOnly || v.saveInFlight) {
              textarea.setAttribute("disabled", "");
            }
            applySyntaxHighlight(hlCode, v.text, language);
            const syncScroll = () => {
              hlPre.scrollTop = textarea.scrollTop;
              hlPre.scrollLeft = textarea.scrollLeft;
            };
            textarea.addEventListener("scroll", syncScroll);
            textarea.addEventListener("input", () => {
              v.text = textarea.value;
              v.dirty = v.text !== v.originalText;
              dirtyMarker.style.display = v.dirty ? "" : "none";
              updateSaveBtn();
              applySyntaxHighlight(hlCode, v.text, language);
              syncScroll();
            });
            wrap.appendChild(hlPre);
            wrap.appendChild(textarea);
            body.appendChild(wrap);
            break;
          }
          case "binary": {
            header.appendChild(headerPath);
            header.appendChild(
              makeEl("span", { className: "file-tree-viewer-meta", text: "binary · " + sizeLabel }),
            );
            if (v.readOnly) {
              header.appendChild(
                makeEl("span", {
                  className: "file-tree-viewer-readonly",
                  text: "read-only",
                }),
              );
            }
            const btn = makeEl("button", {
              className: "wizard-button",
              text: "View as hex",
              attrs: { type: "button" },
              dataset: { viewerAction: "view-as-hex" },
            });
            btn.addEventListener("click", () => {
              v.mode = "loading";
              v.hexOffset = 0;
              v.hexBytes = "";
              renderFileTreeViewer(windowId);
              requestFileContent(windowId, v.path, "hex", 0, 64 * 16);
            });
            header.appendChild(btn);
            body.appendChild(
              makeEl("div", {
                className: "file-tree-viewer-notice",
                text:
                  "Cannot display as text. Use “View as hex” for a 16-byte/row hex dump.",
              }),
            );
            break;
          }
          case "hex": {
            header.appendChild(headerPath);
            if (v.dirty) header.appendChild(dirtyMarker);
            header.appendChild(
              makeEl("span", { className: "file-tree-viewer-meta", text: "hex · " + sizeLabel }),
            );
            if (v.readOnly) {
              header.appendChild(
                makeEl("span", {
                  className: "file-tree-viewer-readonly",
                  text: "read-only",
                }),
              );
            }
            const saveBtn = makeEl("button", {
              className: "wizard-button file-tree-viewer-save",
              attrs: { type: "button" },
              text: v.saveInFlight ? "Saving…" : "Save",
            });
            if (!v.dirty || v.readOnly || v.saveInFlight) {
              saveBtn.setAttribute("disabled", "");
            }
            saveBtn.addEventListener("click", () => requestSaveFileContent(windowId));
            header.appendChild(saveBtn);
            if (v.savedAt && Date.now() - v.savedAt < 2000) {
              header.appendChild(
                makeEl("span", {
                  className: "file-tree-viewer-saved",
                  text: "Saved",
                }),
              );
            }
            // Render hex byte cells inline so we can attach click handlers
            // for single-byte replace edits.
            const container = makeEl("div", { className: "file-tree-viewer-hex" });
            const bytes = decodeBase64ToBytes(v.hexBytes || "");
            const BYTES_PER_LINE = 16;
            for (let line = 0; line < bytes.length; line += BYTES_PER_LINE) {
              const row = makeEl("div", { className: "file-tree-hex-row" });
              const offsetLabel = (v.hexOffset + line)
                .toString(16)
                .padStart(8, "0")
                .toUpperCase();
              row.appendChild(makeEl("span", { className: "file-tree-hex-offset", text: offsetLabel }));
              const bytesContainer = makeEl("span", { className: "file-tree-hex-bytes" });
              const asciiContainer = makeEl("span", { className: "file-tree-hex-ascii" });
              for (let j = 0; j < BYTES_PER_LINE; j += 1) {
                const idx = line + j;
                if (idx < bytes.length) {
                  const b = bytes[idx];
                  const cell = makeEl("button", {
                    className: "file-tree-hex-cell",
                    attrs: { type: "button" },
                    text: b.toString(16).padStart(2, "0").toUpperCase(),
                    dataset: { hexOffset: String(v.hexOffset + idx) },
                  });
                  if (v.readOnly) {
                    cell.setAttribute("disabled", "");
                  } else {
                    cell.addEventListener("click", () => {
                      const input = window.prompt("Replace byte (2 hex digits)", cell.textContent);
                      if (input == null) return;
                      const normalised = input.trim();
                      if (!/^[0-9a-fA-F]{1,2}$/.test(normalised)) {
                        window.alert("Enter 1 or 2 hex digits (0-9, A-F).");
                        return;
                      }
                      const newByte = parseInt(normalised, 16);
                      const prev = bytes[idx];
                      if (prev === newByte) return;
                      bytes[idx] = newByte;
                      v.undoStack.push({ offset: idx, prev });
                      v.redoStack = [];
                      v.hexBytes = encodeBytesToBase64(bytes);
                      recomputeHexDirty(state);
                      renderFileTreeViewer(windowId);
                    });
                  }
                  bytesContainer.appendChild(cell);
                  bytesContainer.appendChild(document.createTextNode(" "));
                  asciiContainer.appendChild(
                    document.createTextNode(b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : "."),
                  );
                } else {
                  bytesContainer.appendChild(document.createTextNode("   "));
                  asciiContainer.appendChild(document.createTextNode(" "));
                }
              }
              row.appendChild(bytesContainer);
              row.appendChild(makeEl("span", { className: "file-tree-hex-divider", text: "|" }));
              row.appendChild(asciiContainer);
              row.appendChild(makeEl("span", { className: "file-tree-hex-divider", text: "|" }));
              container.appendChild(row);
            }
            body.appendChild(container);
            break;
          }
          case "error":
            header.appendChild(headerPath);
            body.appendChild(
              makeEl("div", {
                className: "file-tree-viewer-error",
                text: v.error.message || "Unable to load file",
              }),
            );
            break;
          default:
            header.appendChild(
              makeEl("span", {
                className: "file-tree-viewer-placeholder",
                text: "Unknown state",
              }),
            );
        }
      }


      function requestFileTree(windowId, path = "") {
        const state = ensureFileTreeState(windowId);
        if (state.loading.has(path)) {
          return;
        }
        state.loading.add(path);
        send({
          kind: "load_file_tree",
          id: windowId,
          path: path || null,
        });
      }


      function applyFileTreeSplitterRatio(split, ratio) {
        if (!split) return;
        const clamped = Math.min(0.9, Math.max(0.1, Number(ratio) || 0.4));
        const leftPercent = (clamped * 100).toFixed(2);
        split.style.setProperty("--file-tree-left-ratio", leftPercent + "%");
        split.dataset.leftRatio = String(clamped);
      }

      function renderWorktreeTrigger(windowId) {
        const element = windowMap.get(windowId);
        if (!element) return;
        const trigger = element.querySelector(".file-tree-worktree-trigger");
        if (!trigger) return;
        const state = ensureFileTreeState(windowId);
        trigger.textContent = state.selectedWorktreeId
          ? (state.selectedWorktreeLabel || "Worktree")
          : "Select worktree…";
      }

      function renderFileTree(windowId) {
        const element = windowMap.get(windowId);
        if (!element) {
          return;
        }
        const state = ensureFileTreeState(windowId);
        const list = element.querySelector(".file-tree-list");
        const footer = element.querySelector(".file-tree-footer");
        if (!list || !footer) {
          return;
        }
        list.innerHTML = "";
        footer.textContent = state.selectedPath || ".";

        if (state.error) {
          const errorRow = document.createElement("div");
          errorRow.className = "file-tree-empty workspace-empty-state";
          errorRow.textContent = state.error;
          list.appendChild(errorRow);
        }

        if (!state.loaded.has("")) {
          const loadingRow = document.createElement("div");
          loadingRow.className = "file-tree-empty workspace-empty-state";
          loadingRow.textContent = "Loading repository";
          list.appendChild(loadingRow);
          return;
        }

        function appendRows(parentPath, depth) {
          const entries = state.loaded.get(parentPath) || [];
          for (const entry of entries) {
            const row = document.createElement("div");
            row.className = "file-tree-row";
            // SPEC-2356 — make the row keyboard-navigable. tabindex=0
            // puts the row in the natural Tab order; role="button"
            // tells assistive tech the row is activatable. The keydown
            // handler below mirrors the click handler for Enter/Space.
            row.tabIndex = 0;
            row.setAttribute("role", "button");
            // SPEC-2356 — file tree rows have a selected state but are
            // <div>s, not buttons. aria-current="true" works on any
            // element and announces "current item" to screen readers.
            if (state.selectedPath === entry.path) {
              row.classList.add("selected");
              row.setAttribute("aria-current", "true");
            } else {
              row.removeAttribute("aria-current");
            }
            row.style.paddingLeft = `${12 + depth * 18}px`;

            const expanded = state.expanded.has(entry.path);
            const isDirectory = entry.kind === "directory";
            // SPEC-2356 — directory rows expose collapse state via
            // aria-expanded so screen readers announce "expanded" or
            // "collapsed" alongside the visual ▾/▸ caret. File rows
            // (non-directories) should not expose aria-expanded —
            // that would falsely signal the element is collapsible.
            if (isDirectory) {
              row.setAttribute("aria-expanded", expanded ? "true" : "false");
            } else {
              row.removeAttribute("aria-expanded");
            }
            row.innerHTML = `
              <span class="tree-caret">${isDirectory ? (expanded ? "▾" : "▸") : ""}</span>
              <span class="tree-icon ${isDirectory ? "dir" : "file"}">${isDirectory ? "▣" : "•"}</span>
              <span class="tree-name">${entry.name}</span>
            `;
            const activate = () => {
              state.selectedPath = entry.path;
              if (isDirectory) {
                if (state.expanded.has(entry.path)) {
                  state.expanded.delete(entry.path);
                } else {
                  state.expanded.add(entry.path);
                  if (!state.loaded.has(entry.path)) {
                    requestFileTree(windowId, entry.path);
                  }
                }
              } else {
                // SPEC-2009 amendment Phase 2: gate navigation behind the
                // Discard modal when the viewer has unsaved edits.
                if (
                  queueNavigationGuardedByDirty(windowId, {
                    kind: "switch_file",
                    path: entry.path,
                  })
                ) {
                  return;
                }
                beginViewerForFile(windowId, entry.path);
              }
              renderFileTree(windowId);
            };
            row.addEventListener("click", activate);
            // SPEC-2356 — keyboard activation: Enter and Space invoke the
            // same handler as click so keyboard users can navigate and
            // activate rows without a pointing device.
            row.addEventListener("keydown", (event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                activate();
              }
            });
            list.appendChild(row);

            if (isDirectory && state.expanded.has(entry.path)) {
              if (state.loaded.has(entry.path)) {
                appendRows(entry.path, depth + 1);
              } else {
                const loadingRow = document.createElement("div");
                loadingRow.className = "file-tree-empty workspace-empty-state";
                loadingRow.style.paddingLeft = `${30 + depth * 18}px`;
                loadingRow.textContent = "Loading";
                list.appendChild(loadingRow);
              }
            }
          }
        }

        appendRows("", 0);

        if (list.childElementCount === 0) {
          const emptyRow = document.createElement("div");
          emptyRow.className = "file-tree-empty workspace-empty-state";
          emptyRow.textContent = "No visible files";
          list.appendChild(emptyRow);
        }
      }

      // SPEC-3064 Phase 3 (E6a): File Tree window mount moved verbatim from
      // app.js mountWindowBody (surface === "file-tree" branch).
      function mountFileTreeWindow(windowData, body) {
          // SPEC-2009 amendment: File Tree window now opens with a worktree
          // picker, then renders a single window split into a left directory
          // tree pane and a right file content viewer pane. The legacy
          // `.file-tree-root` class wraps the whole composition so existing
          // styles (and embedded HTML contract tests) still hit.
          const root = makeEl("div", { className: "file-tree-root file-tree-root--split" });
          const toolbar = makeEl("div", {
            className: "file-tree-toolbar workspace-toolbar",
          });
          const pathLabel = makeEl("button", {
            className: "file-tree-path file-tree-worktree-trigger",
            attrs: { type: "button" },
            dataset: { action: "open-worktree-picker" },
            text: "Select worktree…",
          });
          const refreshBtn = makeEl("button", {
            className: "icon-button",
            attrs: { "aria-label": "Refresh tree", type: "button" },
            dataset: { action: "refresh-tree" },
            text: "↻",
          });
          toolbar.appendChild(pathLabel);
          toolbar.appendChild(refreshBtn);

          const split = makeEl("div", { className: "file-tree-split" });
          const pane = makeEl("div", { className: "file-tree-pane" });
          const scroll = makeEl("div", { className: "file-tree-scroll workspace-scroll" });
          const list = makeEl("div", { className: "file-tree-list" });
          scroll.appendChild(list);
          pane.appendChild(scroll);
          pane.appendChild(makeEl("div", { className: "file-tree-footer", text: "." }));

          const splitter = makeEl("div", {
            className: "file-tree-splitter",
            attrs: { role: "separator", "aria-orientation": "vertical", tabindex: "0" },
            dataset: { action: "drag-splitter" },
          });

          const viewer = makeEl("div", { className: "file-tree-viewer" });
          viewer.appendChild(makeEl("div", { className: "file-tree-viewer-header" }));
          viewer.appendChild(makeEl("div", { className: "file-tree-viewer-body" }));

          split.appendChild(pane);
          split.appendChild(splitter);
          split.appendChild(viewer);

          root.appendChild(toolbar);
          root.appendChild(split);
          clearChildren(body);
          body.appendChild(root);

          // Apply initial splitter ratio.
          const initialState = ensureFileTreeState(
            windowData.id,
          );
          applyFileTreeSplitterRatio(split, initialState.splitterRatio);

          body.addEventListener("mousedown", () => {
            focusWindowLocally(windowData.id);
            sendWindowFocus(windowData.id);
          });

          pathLabel.addEventListener("click", (event) => {
            event.stopPropagation();
            openWorktreePicker(windowData.id);
          });

          refreshBtn.addEventListener("click", (event) => {
            event.stopPropagation();
            const state = ensureFileTreeState(
              windowData.id,
            );
            if (!state.selectedWorktreeId) {
              openWorktreePicker(windowData.id);
              return;
            }
            state.loaded.clear();
            state.expanded.clear();
            state.loading.clear();
            state.error = "";
            requestFileTree(windowData.id, "");
            renderFileTree(windowData.id);
          });

          // Splitter drag: pointer events keep the handler small and ignore
          // the canvas pan/zoom because the modal capture absorbs them.
          splitter.addEventListener("pointerdown", (event) => {
            event.preventDefault();
            splitter.setPointerCapture(event.pointerId);
            const onMove = (moveEvent) => {
              const rect = split.getBoundingClientRect();
              if (rect.width <= 0) return;
              const ratio = (moveEvent.clientX - rect.left) / rect.width;
              const clamped = Math.min(0.9, Math.max(0.1, ratio));
              const state = ensureFileTreeState(
                windowData.id,
              );
              state.splitterRatio = clamped;
              applyFileTreeSplitterRatio(split, clamped);
            };
            const onUp = () => {
              splitter.releasePointerCapture(event.pointerId);
              splitter.removeEventListener("pointermove", onMove);
              splitter.removeEventListener("pointerup", onUp);
              splitter.removeEventListener("pointercancel", onUp);
            };
            splitter.addEventListener("pointermove", onMove);
            splitter.addEventListener("pointerup", onUp);
            splitter.addEventListener("pointercancel", onUp);
          });

          // Initial state: prompt for worktree selection. The picker fires
          // the first directory load once the user picks.
          if (!initialState.selectedWorktreeId) {
            pathLabel.textContent = "Select worktree…";
            openWorktreePicker(windowData.id);
          } else {
            pathLabel.textContent = initialState.selectedWorktreeLabel || "Worktree";
            if (!initialState.loaded.has("")) {
              requestFileTree(windowData.id, "");
            }
          }
          renderFileTree(windowData.id);
          renderFileTreeViewer(windowData.id);

          // SPEC-2009 amendment Phase 2 FR-033/041: Ctrl+S / Cmd+S triggers
          // a save when focus is inside this File Tree window so other
          // windows' inputs are not stolen. Bound on the window body element
          // because keydown bubbles up from the textarea / hex cells.
          body.addEventListener("keydown", (event) => {
            if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
              if (event.shiftKey) return; // leave Save-As (future) alone
              event.preventDefault();
              requestSaveFileContent(windowData.id);
            }
          });
      }

      // SPEC-3064 Phase 3 (E6a): receive() bodies for file_tree_* /
      // file_content_* events moved verbatim from app.js; the case arms in
      // app.js delegate here.
      function applyFileTreeReceiveEvent(event) {
        switch (event.kind) {
          case "file_tree_entries": {
            const state = ensureFileTreeState(
              event.id,
            );
            state.loaded.set(event.path, event.entries);
            state.loading.delete(event.path);
            state.error = "";
            renderFileTree(event.id);
            break;
          }
          case "file_tree_error": {
            const state = ensureFileTreeState(
              event.id,
            );
            state.loading.delete(event.path);
            state.error = event.message;
            renderFileTree(event.id);
            break;
          }
          case "file_tree_worktrees": {
            const state = ensureFileTreeState(
              event.id,
            );
            state.picker.entries = Array.isArray(event.entries) ? event.entries : [];
            state.picker.loading = false;
            state.picker.error = "";
            renderWorktreePicker(event.id);
            break;
          }
          case "file_tree_worktree_selected": {
            const state = ensureFileTreeState(
              event.id,
            );
            state.selectedWorktreeId = event.worktree_id || "";
            const selectedEntry = state.picker.entries.find(
              (entry) => entry.id === state.selectedWorktreeId,
            );
            state.selectedWorktreeLabel =
              (selectedEntry && selectedEntry.label) || state.selectedWorktreeLabel;
            renderWorktreeTrigger(event.id);
            // After selection, refresh tree contents.
            state.loaded.clear();
            state.expanded.clear();
            state.loading.clear();
            state.error = "";
            requestFileTree(event.id, "");
            renderFileTree(event.id);
            break;
          }
          case "file_tree_worktree_error": {
            const state = ensureFileTreeState(
              event.id,
            );
            state.picker.open = true;
            state.picker.loading = false;
            state.picker.error = event.message || "Unable to enumerate worktrees";
            renderWorktreePicker(event.id);
            break;
          }
          case "file_content_text": {
            const state = ensureFileTreeState(
              event.id,
            );
            const text = event.text || "";
            const newline = (event.newline || "lf").toString();
            state.viewer = {
              ...state.viewer,
              path: event.path,
              mode: "text",
              text,
              encoding: (event.encoding || "").toString().toUpperCase(),
              totalSize: event.total_size || 0,
              hexOffset: 0,
              hexBytes: "",
              error: { kind: "", message: "", size: null, limit: null },
              dirty: false,
              originalText: text,
              originalBytes: null,
              originalEncoding: (event.encoding || "utf-8").toString(),
              originalNewline: newline,
              originalHasBom: Boolean(event.has_bom),
              originalMtime: Number(event.mtime || 0),
              originalSize: Number(event.total_size || 0),
              readOnly: Boolean(event.read_only),
              savedAt: 0,
              saveInFlight: false,
              undoStack: [],
              redoStack: [],
            };
            renderFileTreeViewer(event.id);
            break;
          }
          case "file_content_hex": {
            const state = ensureFileTreeState(
              event.id,
            );
            const bytes = decodeBase64ToBytes(event.bytes_b64 || "");
            state.viewer = {
              ...state.viewer,
              path: event.path,
              mode: "hex",
              text: "",
              encoding: "",
              totalSize: event.total_size || 0,
              hexOffset: event.offset || 0,
              hexBytes: event.bytes_b64 || "",
              error: { kind: "", message: "", size: null, limit: null },
              dirty: false,
              originalText: "",
              originalBytes: bytes,
              originalEncoding: "",
              originalNewline: "lf",
              originalHasBom: false,
              originalMtime: Number(event.mtime || 0),
              originalSize: Number(event.total_size || 0),
              readOnly: Boolean(event.read_only),
              savedAt: 0,
              saveInFlight: false,
              undoStack: [],
              redoStack: [],
            };
            renderFileTreeViewer(event.id);
            break;
          }
          case "file_content_saved": {
            const state = ensureFileTreeState(
              event.id,
            );
            const v = state.viewer;
            v.dirty = false;
            v.saveInFlight = false;
            v.savedAt = Date.now();
            v.originalMtime = Number(event.new_mtime || 0);
            v.originalSize = Number(event.new_size || 0);
            // Snapshot current edit as the new baseline.
            if (v.mode === "text") {
              v.originalText = v.text;
            } else if (v.mode === "hex") {
              v.originalBytes = decodeBase64ToBytes(v.hexBytes || "");
            }
            // Resume any pending navigation queued behind the Discard modal.
            applyAfterSaveContinuation(event.id);
            renderFileTreeViewer(event.id);
            break;
          }
          case "file_content_save_error": {
            const state = ensureFileTreeState(
              event.id,
            );
            const v = state.viewer;
            v.saveInFlight = false;
            const kind = (event.error_kind || "").toString();
            if (kind === "conflict") {
              state.conflictModal = {
                open: true,
                currentMtime: Number(event.current_mtime || 0),
                currentSize: Number(event.current_size || 0),
                pendingPayload: state.lastSavePayload || null,
              };
              renderConflictModal(event.id);
            } else {
              v.error = {
                kind,
                message: event.message || "",
                size: event.current_size || null,
                limit: null,
              };
              renderFileTreeViewer(event.id);
            }
            // Either way the queued navigation should not silently proceed
            // on failure; we keep the dirty edit and let the user retry.
            const pending = state.discardModal && state.discardModal.pendingAction;
            if (pending && pending.queuedFromDiscard) {
              state.discardModal.pendingAction = null;
            }
            break;
          }
          case "file_content_error": {
            const state = ensureFileTreeState(
              event.id,
            );
            const errorKind = (event.error_kind || "").toString();
            // SPEC-2009 amendment FR-026/029: binary detection is reported as
            // an error variant from the file_content domain. The GUI flips
            // the viewer into a "binary" notice (with a hex affordance) when
            // the user attempted a text read; everything else surfaces the
            // raw notice.
            if (errorKind === "binary_not_text" && state.viewer.mode === "loading") {
              state.viewer = {
                ...state.viewer,
                path: event.path,
                mode: "binary",
                text: "",
                encoding: "",
                totalSize: event.size || state.viewer.totalSize || 0,
                hexOffset: 0,
                hexBytes: "",
                error: { kind: errorKind, message: event.message || "", size: event.size, limit: event.limit },
              };
            } else {
              state.viewer = {
                ...state.viewer,
                path: event.path,
                mode: "error",
                text: "",
                encoding: "",
                totalSize: event.size || 0,
                hexOffset: 0,
                hexBytes: "",
                error: { kind: errorKind, message: event.message || "", size: event.size, limit: event.limit },
              };
            }
            renderFileTreeViewer(event.id);
            break;
          }
          default:
            break;
        }
      }

      return {
        fileTreeStateMap,
        ensureFileTreeState,
        requestFileTree,
        renderFileTree,
        renderFileTreeViewer,
        openWorktreePicker,
        closeWorktreePicker,
        renderWorktreePicker,
        requestFileContent,
        requestSaveFileContent,
        renderDiscardModal,
        renderConflictModal,
        queueNavigationGuardedByDirty,
        beginViewerForFile,
        applyAfterSaveContinuation,
        mountFileTreeWindow,
        applyFileTreeReceiveEvent,
      };
}
