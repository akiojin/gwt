import path from 'path';
import { fileURLToPath } from 'url';
import { access } from 'fs/promises';

export function getCurrentDirname(): string {
  return path.dirname(fileURLToPath(import.meta.url));
}

export async function pathExists(filePath: string): Promise<boolean> {
  try {
    await access(filePath);
    return true;
  } catch {
    return false;
  }
}

export function sanitizePath(input: string): string {
  return input.replace(/[^a-zA-Z0-9-_./]/g, '-');
}

export function formatBranchName(type: string, name: string): string {
  return `${type}/${name}`;
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