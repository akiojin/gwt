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