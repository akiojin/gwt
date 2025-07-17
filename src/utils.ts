import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { access, readFile } from 'fs/promises';
export function getCurrentDirname(): string {
  return path.dirname(fileURLToPath(import.meta.url));
}




export class AppError extends Error {
  constructor(message: string, public cause?: unknown) {
    super(message);
    this.name = 'AppError';
  }
}

export function setupExitHandlers(): void {
  // Handle Ctrl+C gracefully
  process.on('SIGINT', () => {
    console.log('\n\n👋 Goodbye!');
    process.exit(0);
  });

  // Handle other termination signals
  process.on('SIGTERM', () => {
    console.log('\n\n👋 Goodbye!');
    process.exit(0);
  });
}

export function handleUserCancel(error: unknown): never {
  if (error && typeof error === 'object' && 'name' in error) {
    if (error.name === 'ExitPromptError' || error.name === 'AbortPromptError') {
      console.log('\n\n👋 Operation cancelled. Goodbye!');
      process.exit(0);
    }
  }
  throw error;
}

interface PackageJson {
  version: string;
  name?: string;
}

export async function getPackageVersion(): Promise<string | null> {
  try {
    const currentDir = getCurrentDirname();
    const packageJsonPath = path.resolve(currentDir, '..', 'package.json');
    
    const packageJsonContent = await readFile(packageJsonPath, 'utf-8');
    const packageJson: PackageJson = JSON.parse(packageJsonContent);
    
    return packageJson.version || null;
  } catch {
    return null;
  }
}