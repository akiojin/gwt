const SPINNER_FRAMES = [
  "⠋",
  "⠙",
  "⠹",
  "⠸",
  "⠼",
  "⠴",
  "⠦",
  "⠧",
  "⠇",
  "⠏",
] as const;
const FRAME_INTERVAL_MS = 80;

function isWritableStream(
  stream: NodeJS.WriteStream | undefined,
): stream is NodeJS.WriteStream {
  return Boolean(stream && typeof stream.write === "function");
}

/**
 * CLI用の簡易スピナーを開始する。
 * 戻り値の関数を呼び出すとスピナーが停止して行をクリアします。
 */
export function startSpinner(message: string): () => void {
  const stream = process.stdout;

  if (!isWritableStream(stream) || stream.isTTY === false) {
    return () => {};
  }

  let frameIndex = 0;
  let active = true;
  const padding = " ".repeat(message.length + 2);

  const render = () => {
    if (!active) return;
    const frame = SPINNER_FRAMES[frameIndex];
    frameIndex = (frameIndex + 1) % SPINNER_FRAMES.length;
    stream.write(`\r${frame} ${message}`);
  };

  render();
  const timer = setInterval(render, FRAME_INTERVAL_MS);

  const stop = () => {
    if (!active) return;
    active = false;
    clearInterval(timer);
    stream.write(`\r${padding}\r`);
  };

  return stop;
}
