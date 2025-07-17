import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { access, readFile } from 'fs/promises';
import { existsSync } from 'fs';
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

/**
 * Docker環境で実行されているかどうかを検出
 */
export function isRunningInDocker(): boolean {
  // Docker環境の一般的な指標をチェック
  return existsSync('/.dockerenv') || 
         (process.env.container !== undefined) ||
         existsSync('/run/.containerenv');
}

/**
 * Docker環境用にworktreeパスを変換
 * ホストのパスをコンテナ内のパスに変換する
 */
export function convertPathForDocker(hostPath: string): string {
  if (!isRunningInDocker()) {
    return hostPath;
  }
  
  // Docker compose で /claude-worktree にマウントされている想定
  const containerRoot = '/claude-worktree';
  
  // パスがすでにコンテナ内のパスの場合はそのまま返す
  if (hostPath.startsWith(containerRoot)) {
    return hostPath;
  }
  
  // ホストパスから .git/worktree/ 部分を探す
  const worktreeIndex = hostPath.indexOf('.git/worktree/');
  if (worktreeIndex !== -1) {
    // .git/worktree/ 以降の部分を取得
    const relativePath = hostPath.substring(worktreeIndex);
    return path.join(containerRoot, relativePath);
  }
  
  // それ以外の場合は変換できないのでそのまま返す
  return hostPath;
}