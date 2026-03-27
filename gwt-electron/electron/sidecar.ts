import { spawn, type ChildProcess } from 'node:child_process';
import path from 'node:path';
import { app } from 'electron';

let sidecarProcess: ChildProcess | null = null;

function getSidecarPath(): string {
  const isDev = !app.isPackaged;

  if (isDev) {
    // In dev mode, look for the cargo-built binary
    const projectRoot = path.resolve(__dirname, '..', '..');
    const binaryName = process.platform === 'win32' ? 'gwt-server.exe' : 'gwt-server';
    return path.join(projectRoot, 'target', 'debug', binaryName);
  }

  // In production, look in extraResources
  const resourcesPath = process.resourcesPath;
  const binaryName = process.platform === 'win32' ? 'gwt-server.exe' : 'gwt-server';
  return path.join(resourcesPath, binaryName);
}

export function spawnSidecar(): Promise<number> {
  return new Promise((resolve, reject) => {
    const sidecarPath = getSidecarPath();
    console.log(`Spawning sidecar: ${sidecarPath}`);

    try {
      sidecarProcess = spawn(sidecarPath, [], {
        stdio: ['ignore', 'pipe', 'pipe'],
        env: { ...process.env },
      });
    } catch (err) {
      reject(new Error(`Failed to spawn sidecar at ${sidecarPath}: ${err}`));
      return;
    }

    let resolved = false;
    const timeout = setTimeout(() => {
      if (!resolved) {
        resolved = true;
        reject(new Error('Sidecar did not report port within 10 seconds'));
      }
    }, 10_000);

    sidecarProcess.stdout?.on('data', (data: Buffer) => {
      const output = data.toString();
      console.log(`[sidecar stdout] ${output.trimEnd()}`);

      if (!resolved) {
        const match = output.match(/GWT_SERVER_PORT=(\d+)/);
        if (match) {
          resolved = true;
          clearTimeout(timeout);
          resolve(parseInt(match[1], 10));
        }
      }
    });

    sidecarProcess.stderr?.on('data', (data: Buffer) => {
      console.error(`[sidecar stderr] ${data.toString().trimEnd()}`);
    });

    sidecarProcess.on('error', (err) => {
      console.error('Sidecar process error:', err);
      if (!resolved) {
        resolved = true;
        clearTimeout(timeout);
        reject(err);
      }
    });

    sidecarProcess.on('exit', (code, signal) => {
      console.log(`Sidecar exited with code=${code} signal=${signal}`);
      sidecarProcess = null;
      if (!resolved) {
        resolved = true;
        clearTimeout(timeout);
        reject(new Error(`Sidecar exited prematurely (code=${code}, signal=${signal})`));
      }
    });
  });
}

export function killSidecar(): void {
  if (sidecarProcess && !sidecarProcess.killed) {
    console.log('Killing sidecar process...');
    sidecarProcess.kill('SIGTERM');

    // Force kill after 3 seconds if still alive
    setTimeout(() => {
      if (sidecarProcess && !sidecarProcess.killed) {
        console.log('Force killing sidecar process...');
        sidecarProcess.kill('SIGKILL');
      }
    }, 3_000);
  }
}

export function isSidecarRunning(): boolean {
  return sidecarProcess !== null && !sidecarProcess.killed;
}
