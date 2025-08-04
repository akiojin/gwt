import { homedir } from 'node:os';
import { readdir, readFile, stat } from 'node:fs/promises';
import path from 'node:path';

/**
 * Claude Code conversation session information
 */
export interface ClaudeConversation {
  id: string;
  title: string;
  lastActivity: number;
  messageCount: number;
  projectPath: string;
  filePath: string;
  summary?: string;
}

/**
 * Claude Code history manager error
 */
export class ClaudeHistoryError extends Error {
  constructor(message: string, public cause?: unknown) {
    super(message);
    this.name = 'ClaudeHistoryError';
  }
}

/**
 * Get Claude Code configuration directory
 */
function getClaudeConfigDir(): string {
  return path.join(homedir(), '.claude');
}

/**
 * Get Claude Code projects directory
 */
function getClaudeProjectsDir(): string {
  return path.join(getClaudeConfigDir(), 'projects');
}

/**
 * Check if Claude Code is configured on this system
 */
export async function isClaudeHistoryAvailable(): Promise<boolean> {
  try {
    const projectsDir = getClaudeProjectsDir();
    const stats = await stat(projectsDir);
    return stats.isDirectory();
  } catch {
    return false;
  }
}

/**
 * Parse a JSONL conversation file
 */
async function parseConversationFile(filePath: string): Promise<ClaudeConversation | null> {
  try {
    const content = await readFile(filePath, 'utf-8');
    const lines = content.trim().split('\n').filter(line => line.trim());
    
    if (lines.length === 0) {
      return null;
    }

    // Parse messages to extract information
    const messages = lines.map(line => {
      try {
        return JSON.parse(line);
      } catch {
        return null;
      }
    }).filter(Boolean);

    if (messages.length === 0) {
      return null;
    }

    // Extract conversation metadata
    const firstMessage = messages[0];
    const lastMessage = messages[messages.length - 1];
    
    // Generate conversation title from first user message or file name
    let title = 'Untitled Conversation';
    const firstUserMessage = messages.find(msg => msg.role === 'user');
    if (firstUserMessage && firstUserMessage.content) {
      const content = typeof firstUserMessage.content === 'string' 
        ? firstUserMessage.content 
        : firstUserMessage.content[0]?.text || '';
      
      // Extract first line or truncate long content
      const firstLine = content.split('\n')[0].trim();
      title = firstLine.length > 60 ? firstLine.substring(0, 57) + '...' : firstLine;
    }

    // If still no good title, use file name
    if (!title || title === 'Untitled Conversation') {
      const fileName = path.basename(filePath, '.jsonl');
      title = fileName.replace(/^\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2}_/, '') || 'Conversation';
    }

    // Extract project path from file path
    const projectsDir = getClaudeProjectsDir();
    const relativePath = path.relative(projectsDir, filePath);
    const projectPath = path.dirname(relativePath);

    // Get file stats for last activity time
    const stats = await stat(filePath);
    
    return {
      id: path.basename(filePath, '.jsonl'),
      title: title,
      lastActivity: stats.mtime.getTime(),
      messageCount: messages.length,
      projectPath: projectPath === '.' ? 'root' : projectPath,
      filePath: filePath,
      summary: generateSummary(messages)
    };
  } catch (error) {
    console.error(`Failed to parse conversation file ${filePath}:`, error);
    return null;
  }
}

/**
 * Generate a summary from conversation messages
 */
function generateSummary(messages: any[]): string {
  const userMessages = messages.filter(msg => msg.role === 'user').slice(0, 3);
  const topics = userMessages.map(msg => {
    const content = typeof msg.content === 'string' 
      ? msg.content 
      : msg.content[0]?.text || '';
    
    const firstLine = content.split('\n')[0].trim();
    return firstLine.length > 30 ? firstLine.substring(0, 27) + '...' : firstLine;
  });
  
  return topics.length > 0 ? topics.join(' • ') : 'No summary available';
}

/**
 * Get all Claude Code conversations
 */
export async function getAllClaudeConversations(): Promise<ClaudeConversation[]> {
  if (!(await isClaudeHistoryAvailable())) {
    throw new ClaudeHistoryError('Claude Code history is not available on this system');
  }

  try {
    const conversations: ClaudeConversation[] = [];
    const projectsDir = getClaudeProjectsDir();
    
    // Recursively scan for .jsonl files
    await scanDirectoryForConversations(projectsDir, conversations);
    
    // Sort by last activity (most recent first)
    conversations.sort((a, b) => b.lastActivity - a.lastActivity);
    
    return conversations;
  } catch (error) {
    throw new ClaudeHistoryError('Failed to scan Claude Code conversations', error);
  }
}

/**
 * Recursively scan directory for conversation files
 */
async function scanDirectoryForConversations(
  dirPath: string, 
  conversations: ClaudeConversation[]
): Promise<void> {
  try {
    const entries = await readdir(dirPath, { withFileTypes: true });
    
    for (const entry of entries) {
      const fullPath = path.join(dirPath, entry.name);
      
      if (entry.isDirectory()) {
        // Recursively scan subdirectories
        await scanDirectoryForConversations(fullPath, conversations);
      } else if (entry.isFile() && entry.name.endsWith('.jsonl')) {
        // Parse conversation file
        const conversation = await parseConversationFile(fullPath);
        if (conversation) {
          conversations.push(conversation);
        }
      }
    }
  } catch (error) {
    // Continue scanning even if one directory fails
    console.error(`Failed to scan directory ${dirPath}:`, error);
  }
}

/**
 * Get conversations filtered by project/worktree path
 */
export async function getConversationsForProject(worktreePath: string): Promise<ClaudeConversation[]> {
  const allConversations = await getAllClaudeConversations();
  
  // Extract project name from worktree path
  const projectName = path.basename(worktreePath);
  
  return allConversations.filter(conversation => {
    // Match by project path or conversation mentions the project
    return conversation.projectPath.includes(projectName) ||
           conversation.title.toLowerCase().includes(projectName.toLowerCase()) ||
           conversation.summary?.toLowerCase().includes(projectName.toLowerCase());
  });
}

/**
 * Launch Claude Code with a specific conversation
 */
export async function launchClaudeWithConversation(
  worktreePath: string, 
  conversation: ClaudeConversation,
  options: { skipPermissions?: boolean } = {}
): Promise<void> {
  const { launchClaudeCode } = await import('./claude.js');
  
  // Launch Claude Code in the worktree with the conversation file
  // Note: This might need adjustment based on how Claude Code handles specific conversation files
  // For now, we'll use the standard launch and let Claude Code handle the session
  await launchClaudeCode(worktreePath, {
    mode: 'resume',
    skipPermissions: options.skipPermissions ?? false
  });
}