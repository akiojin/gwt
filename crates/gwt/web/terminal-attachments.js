// SPEC-3064 Phase 3 (E2) — terminal attachments & clipboard surface
// extracted from app.js: clipboard image paste, browser/native file drop
// bridges, attachment upload progress UI, the upload token + XHR path,
// clipboard read/write/copy helpers, and the per-terminal handler
// installers called from createTerminalRuntime. Pure movement from app.js:
// behavior, DOM output, and the WS protocol (paste_image_uploaded /
// attach_files / attachment_progress) are unchanged; the moved code keeps
// its original app.js indentation.
import { classifyTerminalCopyKeyEvent } from "/terminal-copy-shortcut.js";
import { createTerminalContextMenuController } from "/terminal-context-menu.js";
import { createUiStateStore } from "/ui-state-store.js";

// deps:
// - send(message): forward a frontend event over the WebSocket bridge.
// - terminalMap / windowMap: the live windowId -> terminal runtime / DOM
//   element registries owned by app.js (stable Map references).
// - workspaceWindowById(id) / isAgentWindowPreset(preset) /
//   workspaceWindowElement(id): workspace lookups owned by app.js.
// - scheduleTerminalViewportRefresh(id): viewport refresh scheduler owned
//   by the app.js terminal host.
// - TERMINAL_SELECTION_DRAG_THRESHOLD: drag-vs-click pixel threshold const
//   shared with the app.js terminal selection handling.
export function createTerminalAttachments({
  send,
  terminalMap,
  windowMap,
  workspaceWindowById,
  isAgentWindowPreset,
  workspaceWindowElement,
  scheduleTerminalViewportRefresh,
  TERMINAL_SELECTION_DRAG_THRESHOLD,
}) {
      const SUPPORTED_IMAGE_PASTE_MIME_TYPES = new Set([
        "image/png",
        "image/jpeg",
        "image/webp",
      ]);
      const MAX_FILE_DROP_COUNT = 16;
      const FILE_DROP_AGENT_TARGET_MESSAGE = "Drop files on a running Agent window.";
      const FILE_DROP_COUNT_MESSAGE = `Drop up to ${MAX_FILE_DROP_COUNT} files at once.`;
      const FILE_DROP_UPLOAD_FAILURE_MESSAGE = "Could not upload dropped file.";
      const IMAGE_PASTE_UPLOAD_FAILURE_MESSAGE = "Could not upload pasted image.";

      function findClipboardImagePasteItem(items) {
        for (const item of Array.from(items || [])) {
          if (
            item?.kind === "file" &&
            SUPPORTED_IMAGE_PASTE_MIME_TYPES.has(item.type)
          ) {
            return item;
          }
        }
        return null;
      }

      function defaultImagePasteFilename(mimeType) {
        switch (mimeType) {
          case "image/jpeg":
            return "clipboard-image.jpg";
          case "image/webp":
            return "clipboard-image.webp";
          case "image/png":
          default:
            return "clipboard-image.png";
        }
      }

      function uploadFileFromImageBlob(blob, mimeType, filename) {
        const type = mimeType || blob?.type || "";
        const name = filename || blob?.name || defaultImagePasteFilename(type);
        if (typeof File === "function" && (!(blob instanceof File) || !blob.name)) {
          return new File([blob], name, { type });
        }
        return blob;
      }

      function dataTransferHasFiles(dataTransfer) {
        const types = Array.from(dataTransfer?.types || []);
        return types.includes("Files") || Boolean(dataTransfer?.files?.length);
      }

      function droppedFilesWithinCountLimit(files) {
        return files.length <= MAX_FILE_DROP_COUNT;
      }

      function droppedFilesValidationFailure(files) {
        if (!droppedFilesWithinCountLimit(files)) {
          return FILE_DROP_COUNT_MESSAGE;
        }
        return null;
      }

      function showFileDropAlert(message) {
        if (typeof window.alert === "function") {
          window.alert(message);
        }
      }

      function totalFileBytes(files) {
        return files.reduce((total, file) => total + (file.size || 0), 0);
      }

      function displayAttachmentBasename(filename) {
        const value = String(filename || "").trim();
        const parts = value.split(/[\\/]+/).filter(Boolean);
        return parts.at(-1) || "file";
      }

      function attachmentFileCountLabel(count) {
        return `${count} ${count === 1 ? "file" : "files"}`;
      }

      function createAttachmentOperationId() {
        const random =
          typeof globalThis.crypto?.randomUUID === "function"
            ? globalThis.crypto.randomUUID()
            : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
        return `attachment-${random}`;
      }

      const attachmentProgressControllers = new Map();
      const attachmentProgressModel = createUiStateStore(null);
      // This surface shares the app's document lifetime. Keep only the latest
      // received snapshot; upload controllers and timers stay outside the model.
      attachmentProgressModel.subscribe(progress => progress, progress => {
        if (progress) renderAttachmentProgress(progress);
      });

      function ensureAttachmentProgressSurface(windowId) {
        const terminalHost = terminalMap.get(windowId)?.terminalContainer?.parentElement;
        const host = terminalHost?.dataset.agentId === windowId
          ? terminalHost : workspaceWindowElement(windowId) || document.body;
        let surface = host.querySelector(".attachment-progress");
        if (surface) {
          return surface;
        }
        surface = document.createElement("div");
        surface.className = "attachment-progress";
        surface.hidden = true;
        surface.setAttribute("role", "status");
        surface.setAttribute("aria-live", "polite");
        surface.innerHTML = `
          <div class="attachment-progress__row">
            <span class="attachment-progress__label"></span>
          </div>
          <div class="attachment-progress__track" role="progressbar" aria-valuemin="0" aria-valuemax="100">
            <div class="attachment-progress__bar"></div>
          </div>
        `;
        host.appendChild(surface);
        return surface;
      }

      function attachmentPhaseLabel(phase, fallback = "") {
        switch (phase) {
          case "queued":
            return "Queued";
          case "staging":
            return "Staging";
          case "injecting":
            return "Injecting";
          case "attached":
            return "Attached";
          case "failed":
            return fallback || "Could not attach";
          default:
            return fallback || "Uploading";
        }
      }

      function createAttachmentProgressController(windowId, files, operationId = createAttachmentOperationId()) {
        const surface = ensureAttachmentProgressSurface(windowId);
        const label = surface.querySelector(".attachment-progress__label");
        const track = surface.querySelector(".attachment-progress__track");
        const bar = surface.querySelector(".attachment-progress__bar");
        const abortController = new AbortController();
        const fileCount = files.length;
        const state = {
          phase: "Uploading",
          filename: fileCount === 1 ? displayAttachmentBasename(files[0]?.name) : "",
          totalBytes: totalFileBytes(files),
          loadedBytes: 0,
          failed: false,
          done: false,
          visible: true,
        };

        function percent() {
          if (state.totalBytes <= 0) {
            return state.done ? 100 : 0;
          }
          return Math.max(
            0,
            Math.min(100, Math.round((state.loadedBytes / state.totalBytes) * 100)),
          );
        }

        function render() {
          if (!state.visible) {
            return;
          }
          const value = percent();
          const filename = state.filename ? ` · ${state.filename}` : "";
          const suffix = state.totalBytes > 0 ? ` · ${value}%` : "";
          surface.hidden = false;
          surface.dataset.state = state.failed ? "error" : state.done ? "done" : "active";
          label.textContent = `${state.phase} ${attachmentFileCountLabel(fileCount)}${filename}${suffix}`;
          track.setAttribute("aria-valuenow", String(value));
          bar.style.width = `${value}%`;
        }

        function showNow() {
          state.visible = true;
          render();
        }

        render();

        function setPhase(phase) {
          state.phase = phase;
          render();
        }

        function setUploadProgress(loadedBytes, totalBytes = null) {
          if (Number.isFinite(totalBytes) && totalBytes >= 0) {
            state.totalBytes = totalBytes;
          }
          state.loadedBytes = Math.max(0, loadedBytes || 0);
          render();
        }

        function succeed() {
          state.done = true;
          state.phase = "Attached";
          state.loadedBytes = state.totalBytes;
          if (state.visible) {
            render();
            setTimeout(() => {
              if (surface.dataset.state === "done") {
                surface.hidden = true;
                attachmentProgressControllers.delete(operationId);
              }
            }, 700);
          }
        }

        function fail(message) {
          state.failed = true;
          state.phase = message;
          showNow();
        }

        function applyBackendProgress(event) {
          if (event.filename) {
            state.filename = displayAttachmentBasename(event.filename);
          }
          if (Number.isFinite(event.bytes_total)) {
            state.totalBytes = event.bytes_total;
          }
          if (Number.isFinite(event.bytes_done)) {
            state.loadedBytes = event.bytes_done;
          }
          if (event.phase === "failed") {
            fail(event.message || "Could not attach");
            return;
          }
          if (event.phase === "attached") {
            succeed();
            return;
          }
          setPhase(attachmentPhaseLabel(event.phase));
        }

        const controller = {
          operationId,
          signal: abortController.signal,
          setPhase,
          setUploadProgress,
          succeed,
          fail,
          applyBackendProgress,
        };
        attachmentProgressControllers.set(operationId, controller);
        return controller;
      }

      function handleAttachmentProgress(event) {
        if (!event?.operation_id) return;
        attachmentProgressModel.update(() => ({
          id: event.id,
          operation_id: event.operation_id,
          phase: event.phase,
          filename: event.filename,
          bytes_total: event.bytes_total,
          bytes_done: event.bytes_done,
          message: event.message,
        }));
      }

      function renderAttachmentProgress(event) {
        const operationId = event?.operation_id || "";
        if (!operationId) {
          return;
        }
        let controller = attachmentProgressControllers.get(operationId);
        if (!controller) {
          controller = createAttachmentProgressController(
            event.id,
            [
              {
                name: event.filename || "file",
                size: event.bytes_total || 0,
              },
            ],
            operationId,
          );
        }
        controller.applyBackendProgress(event);
      }

      function attachmentFilesFromNativePaths(paths) {
        return paths.map((path) => ({
          name: displayAttachmentBasename(path),
          size: 0,
        }));
      }

      let attachmentUploadTokenPromise = null;

      async function attachmentUploadToken() {
        if (!attachmentUploadTokenPromise) {
          attachmentUploadTokenPromise = fetch("/internal/attachment-upload-token", {
            credentials: "same-origin",
          }).then(async (response) => {
            if (!response.ok) {
              throw new Error(`attachment upload token failed: ${response.status}`);
            }
            const payload = await response.json();
            if (!payload?.token) {
              throw new Error("attachment upload token missing");
            }
            return payload.token;
          }).catch((error) => {
            // A transient failure must not poison every later user paste/drop.
            attachmentUploadTokenPromise = null;
            throw error;
          });
        }
        return attachmentUploadTokenPromise;
      }

      function uploadAttachmentFile(file, { onProgress, signal } = {}) {
        if (typeof window.__gwtAttachmentUploader === "function") {
          return window.__gwtAttachmentUploader({ file, onProgress, signal });
        }
        return new Promise((resolve, reject) => {
          void attachmentUploadToken()
            .then((token) => {
              if (signal?.aborted) {
                reject(new Error("upload aborted"));
                return;
              }
              const params = new URLSearchParams({
                filename: file.name || "file",
                size: String(file.size || 0),
              });
              if (file.type) {
                params.set("mime_type", file.type);
              }
              const xhr = new XMLHttpRequest();
              xhr.open("POST", `/internal/attachments/upload?${params.toString()}`);
              xhr.setRequestHeader("x-gwt-upload-token", token);
              // Keep failure bodies too: storage errors are plain-text responses.
              xhr.responseType = "text";
              xhr.upload.onprogress = (event) => {
                onProgress?.({
                  loaded: event.loaded || 0,
                  total: event.lengthComputable ? event.total : file.size || 0,
                });
              };
              xhr.onload = () => {
                if (xhr.status < 200 || xhr.status >= 300) {
                  const detail = typeof xhr.response === "string" ? xhr.response : xhr.responseText;
                  reject(new Error(`HTTP ${xhr.status}: ${detail || xhr.statusText || "attachment upload failed"}`));
                  return;
                }
                try {
                  resolve(typeof xhr.response === "object" && xhr.response
                    ? xhr.response : JSON.parse(xhr.responseText || xhr.response || "{}"));
                } catch (error) {
                  reject(new Error(`Invalid attachment upload response (HTTP ${xhr.status}): ${error.message}`));
                }
              };
              xhr.onerror = () => reject(new Error("attachment upload network error (HTTP 0)"));
              xhr.onabort = () => reject(new Error("upload aborted"));
              signal?.addEventListener("abort", () => xhr.abort(), { once: true });
              xhr.send(file);
            })
            .catch(reject);
        });
      }

      async function uploadFilesAsAttachments(files, progress) {
        const totalBytes = totalFileBytes(files);
        let completedBytes = 0;
        const attachments = [];
        for (const file of files) {
          const uploaded = await uploadAttachmentFile(file, {
            signal: progress.signal,
            onProgress: ({ loaded }) => {
              progress.setUploadProgress(completedBytes + (loaded || 0), totalBytes);
            },
          });
          completedBytes += file.size || uploaded?.size || 0;
          progress.setUploadProgress(
            totalBytes > 0 ? Math.min(completedBytes, totalBytes) : 0,
            totalBytes,
          );
          attachments.push({
            source: "uploaded",
            upload_id: uploaded.upload_id,
            filename: uploaded.filename || file.name || "file",
            mime_type: uploaded.mime_type ?? (file.type || null),
            size: uploaded.size ?? file.size ?? 0,
          });
        }
        return attachments;
      }

      async function uploadPastedImage(windowId, blob, { mimeType, filename } = {}) {
        const file = uploadFileFromImageBlob(blob, mimeType, filename);
        const progress = createAttachmentProgressController(windowId, [file]);
        try {
          const uploaded = await uploadAttachmentFile(file, {
            signal: progress.signal,
            onProgress: ({ loaded, total }) => progress.setUploadProgress(loaded || 0, total),
          });
          progress.setPhase("Queued");
          send({
            kind: "paste_image_uploaded",
            id: windowId,
            operation_id: progress.operationId,
            upload_id: uploaded.upload_id,
            mime_type: uploaded.mime_type ?? file.type ?? mimeType ?? "",
            filename: uploaded.filename || file.name || filename || null,
            size: uploaded.size ?? file.size ?? 0,
          });
        } catch (error) {
          const message = `${IMAGE_PASTE_UPLOAD_FAILURE_MESSAGE} ${error?.message || String(error)}`;
          progress.fail(message);
          send({ kind: "attachment_upload_failed", id: windowId,
            operation_id: progress.operationId, message });
          showFileDropAlert(message);
        }
      }

      async function readNavigatorClipboardItems() {
        if (!navigator.clipboard?.read) {
          return [];
        }
        return navigator.clipboard.read();
      }

      async function readNavigatorClipboardText() {
        if (!navigator.clipboard?.readText) {
          return "";
        }
        return navigator.clipboard.readText();
      }

      async function writeClipboardText(text, restoreFocus = null) {
        if (!text) {
          return false;
        }
        if (navigator.clipboard?.writeText) {
          try {
            await navigator.clipboard.writeText(text);
            restoreFocus?.();
            return true;
          } catch (_error) {
            // Fall back to a temporary textarea when the async clipboard API is unavailable.
          }
        }

        const textarea = document.createElement("textarea");
        textarea.value = text;
        textarea.setAttribute("readonly", "");
        textarea.style.position = "fixed";
        textarea.style.top = "-1000px";
        textarea.style.left = "-1000px";
        textarea.style.opacity = "0";
        document.body.appendChild(textarea);
        textarea.focus();
        textarea.select();

        try {
          return document.execCommand("copy");
        } catch (_error) {
          return false;
        } finally {
          textarea.remove();
          restoreFocus?.();
        }
      }

      async function copyTerminalSelection(windowId, { clearSelectionAfterCopy = false } = {}) {
        const runtime = terminalMap.get(windowId);
        if (!runtime || !runtime.terminal.hasSelection()) {
          return false;
        }
        const selection = runtime.terminal.getSelection();
        if (!selection) {
          return false;
        }
        const copied = await writeClipboardText(selection, () => runtime.terminal.focus());
        if (copied && clearSelectionAfterCopy) {
          runtime.terminal.clearSelection();
        }
        return copied;
      }

      async function copyTerminalOverlayMessage(windowId) {
        const element = windowMap.get(windowId);
        const messageEl = element?.querySelector(".terminal-overlay .overlay-message");
        if (!messageEl) {
          return false;
        }
        return writeClipboardText(messageEl.textContent, () => {
          terminalMap.get(windowId)?.terminal.focus();
        });
      }

      function updateTerminalOverlayCopyState(overlay) {
        const button = overlay?.querySelector(".overlay-copy-button");
        const messageEl = overlay?.querySelector(".overlay-message");
        if (!button || !messageEl) {
          return;
        }
        const hasMessage = Boolean(messageEl.textContent);
        button.hidden = !hasMessage;
        button.disabled = !hasMessage;
      }

      function installTerminalCopyHandlers(windowId, terminalRoot, terminal) {
        const copyState = {
          mouseDown: false,
          dragged: false,
          startX: 0,
          startY: 0,
        };

        function resetCopyState() {
          copyState.mouseDown = false;
          copyState.dragged = false;
          copyState.startX = 0;
          copyState.startY = 0;
        }

        const handleMouseDown = (event) => {
          if (event.button !== 0) {
            return;
          }
          copyState.mouseDown = true;
          copyState.dragged = false;
          copyState.startX = event.clientX;
          copyState.startY = event.clientY;
        };

        const handleMouseMove = (event) => {
          if (!copyState.mouseDown || copyState.dragged) {
            return;
          }
          const movedX = Math.abs(event.clientX - copyState.startX);
          const movedY = Math.abs(event.clientY - copyState.startY);
          if (
            movedX >= TERMINAL_SELECTION_DRAG_THRESHOLD ||
            movedY >= TERMINAL_SELECTION_DRAG_THRESHOLD
          ) {
            copyState.dragged = true;
          }
        };

        const handleMouseUp = (event) => {
          if (!copyState.mouseDown) {
            return;
          }
          const shouldCopy = event.button === 0 && copyState.dragged;
          resetCopyState();
          if (!shouldCopy) {
            return;
          }
          requestAnimationFrame(() => {
            if (!terminal.hasSelection()) {
              return;
            }
            void copyTerminalSelection(windowId);
          });
        };

        const handleWindowBlur = () => {
          resetCopyState();
        };

        terminal.attachCustomKeyEventHandler((event) => {
          const copyDecision = classifyTerminalCopyKeyEvent(event, {
            hasSelection: terminal.hasSelection(),
          });
          if (!copyDecision.copy) {
            return true;
          }
          event.preventDefault();
          event.stopPropagation();
          if (!terminal.hasSelection()) {
            return false;
          }
          void copyTerminalSelection(windowId, {
            clearSelectionAfterCopy: copyDecision.clearSelectionAfterCopy,
          });
          return false;
        });

        terminalRoot.addEventListener("mousedown", handleMouseDown);
        window.addEventListener("mousemove", handleMouseMove, true);
        window.addEventListener("mouseup", handleMouseUp, true);
        window.addEventListener("blur", handleWindowBlur);

        return () => {
          terminal.attachCustomKeyEventHandler(() => true);
          terminalRoot.removeEventListener("mousedown", handleMouseDown);
          window.removeEventListener("mousemove", handleMouseMove, true);
          window.removeEventListener("mouseup", handleMouseUp, true);
          window.removeEventListener("blur", handleWindowBlur);
        };
      }

      function installTerminalImagePasteHandlers(windowId, terminalRoot, terminal) {
        const handlePaste = (event) => {
          const item = findClipboardImagePasteItem(event.clipboardData?.items);
          if (!item) {
            return;
          }
          const file = item.getAsFile?.();
          if (!file) {
            return;
          }
          event.preventDefault();
          event.stopPropagation();

          void uploadPastedImage(windowId, file, {
            mimeType: file.type || item.type,
            filename: file.name || null,
          }).finally(() => {
            terminal.focus();
          });
        };

        terminalRoot.addEventListener("paste", handlePaste, true);
        return () => {
          terminalRoot.removeEventListener("paste", handlePaste, true);
        };
      }

      function installTerminalFileDropHandlers(windowId, terminalRoot, terminal) {
        const handleDragOver = (event) => {
          if (!dataTransferHasFiles(event.dataTransfer)) {
            return;
          }
          event.preventDefault();
          event.stopPropagation();
          event.dataTransfer.dropEffect = "copy";
        };

        const handleDrop = (event) => {
          const files = Array.from(event.dataTransfer?.files || []);
          if (files.length === 0) {
            return;
          }
          event.preventDefault();
          event.stopPropagation();
          if (!isAgentWindowPreset(workspaceWindowById(windowId)?.preset)) {
            showFileDropAlert(FILE_DROP_AGENT_TARGET_MESSAGE);
            terminal.focus();
            return;
          }
          const failure = droppedFilesValidationFailure(files);
          if (failure) {
            showFileDropAlert(failure);
            terminal.focus();
            return;
          }

          void sendDroppedFileAttachments(windowId, files).finally(() => {
            terminal.focus();
          });
        };

        terminalRoot.addEventListener("dragover", handleDragOver, true);
        terminalRoot.addEventListener("drop", handleDrop, true);
        return () => {
          terminalRoot.removeEventListener("dragover", handleDragOver, true);
          terminalRoot.removeEventListener("drop", handleDrop, true);
        };
      }

      function installTerminalContextMenuHandlers(windowId, terminalRoot, terminal) {
        const controller = createTerminalContextMenuController({
          document,
          window,
          terminalRoot,
          readClipboardText: readNavigatorClipboardText,
          readClipboardItems: readNavigatorClipboardItems,
          supportedImageTypes: SUPPORTED_IMAGE_PASTE_MIME_TYPES,
          pasteText: (text) => terminal.paste(text),
          pasteImage: ({ blob, mimeType, filename }) =>
            uploadPastedImage(windowId, blob, { mimeType, filename }),
          focusTerminal: () => terminal.focus(),
        });
        return () => {
          controller.dispose();
        };
      }

      function installTerminalViewportRefreshHandlers(windowId, terminal) {
        const viewportScrollDisposable = terminal.onScroll(() => {
          scheduleTerminalViewportRefresh(windowId);
        });

        return () => {
          viewportScrollDisposable.dispose();
        };
      }

      function attachmentWindowIdFromElement(element) {
        const tile = element?.closest?.(".agent-tile[data-agent-id]");
        if (tile) {
          return tile.closest(".agents-surface--preview") ? null : tile.dataset.agentId;
        }
        return element?.closest?.(".workspace-window")?.dataset?.id || null;
      }

      function terminalWindowIdFromPoint(x, y) {
        if (!Number.isFinite(x) || !Number.isFinite(y)) {
          return null;
        }
        const target = document.elementFromPoint(x, y);
        const terminalRoot = target?.closest?.(".terminal-root");
        const windowId = attachmentWindowIdFromElement(terminalRoot);
        if (!windowId || !terminalMap.has(windowId)) {
          return null;
        }
        return windowId;
      }

      function workspaceWindowIdFromDropEvent(event) {
        const targetWindowId = attachmentWindowIdFromElement(event.target);
        if (targetWindowId) {
          return targetWindowId;
        }
        const x = Number(event.clientX);
        const y = Number(event.clientY);
        if (!Number.isFinite(x) || !Number.isFinite(y)) {
          return null;
        }
        const pointTarget = document.elementFromPoint(x, y);
        return attachmentWindowIdFromElement(pointTarget);
      }

      function eventTargetsPmReports(event) {
        const windowId = workspaceWindowIdFromDropEvent(event);
        return Boolean(windowId && workspaceWindowElement(windowId)
          ?.querySelector(".pm-chat:not(.pm-chat--logs)"));
      }

      function agentWindowIdFromDropEvent(event) {
        const windowId = workspaceWindowIdFromDropEvent(event);
        if (!windowId || !terminalMap.has(windowId)) {
          return null;
        }
        const windowData = workspaceWindowById(windowId);
        if (!isAgentWindowPreset(windowData?.preset)) {
          return null;
        }
        return windowId;
      }

      function eventTargetsTerminalRoot(event) {
        return Boolean(event.target?.closest?.(".terminal-root"));
      }

      function focusTerminalForWindow(windowId) {
        terminalMap.get(windowId)?.terminal.focus();
      }

      async function sendDroppedFileAttachments(windowId, files) {
        const failure = droppedFilesValidationFailure(files);
        if (failure) {
          showFileDropAlert(failure);
          focusTerminalForWindow(windowId);
          return;
        }

        const progress = createAttachmentProgressController(windowId, files);
        try {
          const attachments = await uploadFilesAsAttachments(files, progress);
          progress.setPhase("Queued");
          send({
            kind: "attach_files",
            id: windowId,
            operation_id: progress.operationId,
            files: attachments,
          });
        } catch (_error) {
          progress.fail(FILE_DROP_UPLOAD_FAILURE_MESSAGE);
          showFileDropAlert(FILE_DROP_UPLOAD_FAILURE_MESSAGE);
        } finally {
          focusTerminalForWindow(windowId);
        }
      }

      function installBrowserFileDropBridge() {
        const handleDragOver = (event) => {
          if (!dataTransferHasFiles(event.dataTransfer) || eventTargetsTerminalRoot(event)) {
            return;
          }
          event.preventDefault();
          event.stopPropagation();
          if (!eventTargetsPmReports(event) && agentWindowIdFromDropEvent(event)) {
            event.dataTransfer.dropEffect = "copy";
          } else {
            event.dataTransfer.dropEffect = "none";
          }
        };

        const handleDrop = (event) => {
          if (!dataTransferHasFiles(event.dataTransfer) || eventTargetsTerminalRoot(event)) {
            return;
          }
          const files = Array.from(event.dataTransfer?.files || []);
          if (files.length === 0) {
            return;
          }
          event.preventDefault();
          event.stopPropagation();
          if (eventTargetsPmReports(event)) return;
          const windowId = agentWindowIdFromDropEvent(event);
          if (!windowId) {
            showFileDropAlert(FILE_DROP_AGENT_TARGET_MESSAGE);
            return;
          }
          void sendDroppedFileAttachments(windowId, files);
        };

        window.addEventListener("dragover", handleDragOver, true);
        window.addEventListener("drop", handleDrop, true);
      }

      function installNativeFileDropBridge() {
        window.addEventListener("gwt:native-file-drop", (event) => {
          const detail = event.detail || {};
          const paths = Array.isArray(detail.paths)
            ? detail.paths.filter((path) => typeof path === "string" && path.length > 0)
            : [];
          if (paths.length === 0) {
            return;
          }
          const windowId = terminalWindowIdFromPoint(Number(detail.x), Number(detail.y));
          if (!windowId) {
            return;
          }
          const progress = createAttachmentProgressController(
            windowId,
            attachmentFilesFromNativePaths(paths),
          );
          progress.setPhase("Queued");
          send({
            kind: "attach_files",
            id: windowId,
            operation_id: progress.operationId,
            files: paths.map((path) => ({
              source: "native_path",
              path,
            })),
          });
          terminalMap.get(windowId)?.terminal.focus();
        });
      }

      return {
        updateTerminalOverlayCopyState,
        copyTerminalOverlayMessage,
        installTerminalCopyHandlers,
        installTerminalImagePasteHandlers,
        installTerminalFileDropHandlers,
        installTerminalContextMenuHandlers,
        installTerminalViewportRefreshHandlers,
        handleAttachmentProgress,
        installBrowserFileDropBridge,
        installNativeFileDropBridge,
      };
}
