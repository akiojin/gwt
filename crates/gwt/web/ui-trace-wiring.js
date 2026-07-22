export const UI_TRACE_EVENT = Object.freeze({
  applyStatus: "apply_status",
  applyViewport: "apply_viewport",
  fitTerminal: "fit_terminal",
  pointerCancelIgnored: "pointer_cancel_ignored",
  pointerCaptureFailed: "pointer_capture_failed",
  pointerCaptureSet: "pointer_capture_set",
  pointerDownIgnored: "pointer_down_ignored",
  pointerDragCancel: "pointer_drag_cancel",
  pointerDragEnd: "pointer_drag_end",
  pointerDragMove: "pointer_drag_move",
  pointerLostCapture: "pointer_lost_capture",
  pointerMoveIgnored: "pointer_move_ignored",
  pointerPanCancel: "pointer_pan_cancel",
  pointerPanEnd: "pointer_pan_end",
  pointerPanMove: "pointer_pan_move",
  pointerPanStart: "pointer_pan_start",
  pointerResizeCancel: "pointer_resize_cancel",
  pointerResizeEnd: "pointer_resize_end",
  pointerResizeMove: "pointer_resize_move",
  pointerResizeStart: "pointer_resize_start",
  pointerUpIgnored: "pointer_up_ignored",
  renderAppState: "render_app_state",
  renderWorkspace: "render_workspace",
  resizePointermoveApply: "resize_pointermove_apply",
  resizePointermoveFrame: "resize_pointermove_frame",
  resizePointermoveFrameScheduled: "resize_pointermove_frame_scheduled",
  terminalActivation: "terminal_activation",
  terminalInputEnqueue: "terminal_input_enqueue",
  terminalVisibilityReveal: "terminal_visibility_reveal",
  writeOutput: "write_output",
});

const START_COMMAND = Object.freeze({
  group: "Diagnostics",
  id: "diagnostics-ui-trace-start",
  label: "Start UI Trace",
});

const STOP_COMMAND = Object.freeze({
  group: "Diagnostics",
  id: "diagnostics-ui-trace-stop",
  label: "Stop UI Trace",
});

export function createUiTraceSavePayload(trace) {
  return {
    kind: "save_ui_trace",
    trace,
  };
}

export function createUiTraceWiring({
  profiler,
  send,
  alert = () => {},
  log = () => {},
} = {}) {
  if (!profiler || typeof profiler !== "object") {
    throw new TypeError("createUiTraceWiring requires a profiler");
  }
  if (typeof send !== "function") {
    throw new TypeError("createUiTraceWiring requires a send callback");
  }

  let activeEpoch = null;

  function traceUi(kind, fields = {}) {
    profiler.record(kind, fields);
  }

  function tracePointer(kind, event, fields = {}) {
    profiler.recordPointer(kind, event, fields);
  }

  function traceMeasure(kind, fields, callback) {
    return profiler.measure(kind, fields, callback);
  }

  function isTracing() {
    if (typeof profiler.isActive !== "function") {
      return false;
    }
    return Boolean(profiler.isActive());
  }

  function currentEpoch() {
    if (!isTracing()) {
      return null;
    }
    return activeEpoch;
  }

  function start() {
    const trace = profiler.start();
    activeEpoch = Symbol("ui-trace-epoch");
    log(`[ui-trace] started ${trace.session_id}`);
    return trace;
  }

  function stop() {
    const trace = profiler.stop();
    activeEpoch = null;
    if (!trace) {
      alert("UI trace is not running.");
      return null;
    }
    const payload = createUiTraceSavePayload(trace);
    send(payload);
    return payload;
  }

  function registerPalette(palette) {
    if (!palette || typeof palette.register !== "function") {
      return false;
    }
    palette.register({
      ...START_COMMAND,
      handler: start,
    });
    palette.register({
      ...STOP_COMMAND,
      handler: stop,
    });
    return true;
  }

  return {
    currentEpoch,
    isTracing,
    registerPalette,
    start,
    stop,
    traceMeasure,
    tracePointer,
    traceUi,
  };
}
