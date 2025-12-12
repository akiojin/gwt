export function resolveWebUiPort(
  portEnv: string | undefined = process.env.PORT,
  defaultPort = 3000,
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

