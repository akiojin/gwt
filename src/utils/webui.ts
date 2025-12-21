import * as net from "node:net";

export function resolveWebUiPort(
  portEnv: string | undefined = process.env.PORT,
  defaultPort = 3001,
): number {
  if (!portEnv) {
    return defaultPort;
  }

  const port = parseInt(portEnv, 10);
  if (Number.isNaN(port) || port <= 0 || port > 65535) {
    return defaultPort;
  }

  return port;
}

/**
 * 指定ポートが使用中かどうかをチェック
 * @param port - チェックするポート番号
 * @returns 使用中ならtrue、利用可能ならfalse
 */
export async function isPortInUse(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const server = net.createServer();

    server.once("error", (err: NodeJS.ErrnoException) => {
      if (err.code === "EADDRINUSE") {
        resolve(true);
      } else {
        // Other errors (EACCES, etc.) - treat as port not in use
        resolve(false);
      }
    });

    server.once("listening", () => {
      server.close(() => resolve(false));
    });

    server.listen(port, "127.0.0.1");
  });
}
