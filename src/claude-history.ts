import { homedir } from 'node:os';
import { readdir, readFile, stat } from 'node:fs/promises';
import path from 'node:path';

/**
 * Claude Code conversation session information
 */
export interface ClaudeConversation {
  id: string;
  sessionId?: string; // Claude Code session ID for --resume command
  title: string;
  lastActivity: number;
  messageCount: number;
  projectPath: string;
  filePath: string;
  summary?: string;
}

/**
 * Message structure for conversation details
 */
export interface ClaudeMessage {
  role: 'user' | 'assistant';
  content: string | Array<{ text: string; type?: string }>;
  timestamp?: number;
}

/**
 * Detailed conversation with full message history
 */
export interface DetailedClaudeConversation extends ClaudeConversation {
  messages: ClaudeMessage[];
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
    
    // Extract session ID from messages (look for session_id, id, or conversation_id fields)
    let sessionId: string | undefined;
    for (const message of messages) {
      if (message.session_id) {
        sessionId = message.session_id;
        break;
      } else if (message.conversation_id) {
        sessionId = message.conversation_id;
        break;
      } else if (message.id && typeof message.id === 'string' && message.id.length > 10) {
        // If ID looks like a session ID (longer string), use it
        sessionId = message.id;
        break;
      }
    }
    
    // If no session ID found in messages, try to extract from filename
    if (!sessionId) {
      const fileName = path.basename(filePath, '.jsonl');
      // Look for UUID-like patterns in filename
      const uuidMatch = fileName.match(/([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})/i);
      if (uuidMatch) {
        sessionId = uuidMatch[1];
      } else if (fileName.length > 20) {
        // Use filename as session ID if it's long enough
        sessionId = fileName;
      }
    }
    
    // Generate conversation title from first user message or file name
    let title = 'Untitled Conversation';
    
    // Debug: Log raw messages for investigation
    if (process.env.DEBUG_CLAUDE_HISTORY) {
      console.log(`[DEBUG] Processing file: ${filePath}`);
      console.log(`[DEBUG] File basename: ${path.basename(filePath, '.jsonl')}`);
      console.log(`[DEBUG] Message count: ${messages.length}`);
      console.log(`[DEBUG] First message:`, JSON.stringify(messages[0], null, 2));
      console.log(`[DEBUG] All message roles:`, messages.map(m => m.role || 'no-role'));
    }
    
    // Find first user message - be more flexible about role matching
    const firstUserMessage = messages.find(msg => 
      msg.role === 'user' || msg.role === 'human' || 
      (msg.sender && msg.sender === 'human') ||
      (!msg.role && msg.content) // fallback for messages without explicit role
    );
    
    if (firstUserMessage && firstUserMessage.content) {
      let extractedContent = '';
      
      // Handle different content formats that Claude Code might use
      if (typeof firstUserMessage.content === 'string') {
        extractedContent = firstUserMessage.content;
      } else if (Array.isArray(firstUserMessage.content)) {
        // Handle array of content blocks
        for (const block of firstUserMessage.content) {
          if (typeof block === 'string') {
            extractedContent = block;
            break;
          } else if (block && typeof block === 'object') {
            // Handle content blocks with type and text properties
            if (block.text && typeof block.text === 'string') {
              extractedContent = block.text;
              break;
            } else if (block.content && typeof block.content === 'string') {
              extractedContent = block.content;
              break;
            }
          }
        }
      } else if (firstUserMessage.content && typeof firstUserMessage.content === 'object') {
        // Handle single content object
        if (firstUserMessage.content.text) {
          extractedContent = firstUserMessage.content.text;
        } else if (firstUserMessage.content.content) {
          extractedContent = firstUserMessage.content.content;
        }
      }
      
      // Clean and format the extracted content
      if (extractedContent) {
        // Remove system prompts or meta information
        const cleanContent = extractedContent
          .replace(/^(<.*?>|System:|Assistant:|Human:)/i, '')
          .trim();
          
        // Extract first meaningful line
        const firstLine = cleanContent.split('\n')[0]?.trim() || '';
        
        if (firstLine.length > 0) {
          title = firstLine.length > 60 ? firstLine.substring(0, 57) + '...' : firstLine;
        }
        
        // Debug: Log title extraction
        if (process.env.DEBUG_CLAUDE_HISTORY) {
          console.log(`[DEBUG] Extracted title: "${title}" from content: "${extractedContent.substring(0, 100)}..."`);
          console.log(`[DEBUG] firstUserMessage structure:`, JSON.stringify(firstUserMessage, null, 2).substring(0, 500));
        }
      }
    }

    // If still no good title, try alternative extraction methods
    if (!title || title === 'Untitled Conversation') {
      // Try to extract from filename patterns
      const fileName = path.basename(filePath, '.jsonl');
      
      // Remove timestamp patterns and use remaining text
      const cleanFileName = fileName.replace(/^\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2}_/, '');
      
      if (cleanFileName && cleanFileName.length > 0 && !cleanFileName.match(/^[0-9a-f-]+$/i)) {
        // Only use filename if it's not just a UUID
        title = cleanFileName.replace(/[-_]/g, ' ').trim();
        title = title.charAt(0).toUpperCase() + title.slice(1);
      } else {
        // If we only have UUID or nothing meaningful, create a better fallback
        if (messages.length > 0) {
          // Try to create title from first few messages
          const firstMessages = messages.slice(0, 3);
          const keywords = [];
          
          for (const msg of firstMessages) {
            if (msg.content) {
              let content = '';
              if (typeof msg.content === 'string') {
                content = msg.content;
              } else if (Array.isArray(msg.content) && msg.content[0]) {
                content = msg.content[0].text || msg.content[0].content || '';
              }
              
              // Extract meaningful words from content
              const words = content.split(/\s+/).slice(0, 5).join(' ');
              if (words && words.length > 10) {
                keywords.push(words.substring(0, 30) + '...');
                break;
              }
            }
          }
          
          title = keywords.length > 0 ? keywords[0]! : 'Recent conversation';
        } else {
          title = 'Empty conversation';
        }
      }
    }

    // Get file stats for last activity time
    const stats = await stat(filePath);
    
    // Extract project path from file path
    const projectsDir = getClaudeProjectsDir();
    const relativePath = path.relative(projectsDir, filePath);
    const projectPath = path.dirname(relativePath);
    
    const result: ClaudeConversation = {
      id: path.basename(filePath, '.jsonl'),
      title: title,
      lastActivity: stats.mtime.getTime(),
      messageCount: messages.length,
      projectPath: projectPath === '.' ? 'root' : projectPath,
      filePath: filePath,
      summary: generateSummary(messages)
    };
    
    // Only add sessionId if it exists
    if (sessionId) {
      result.sessionId = sessionId;
    }
    
    return result;
  } catch (error) {
    console.error(`Failed to parse conversation file ${filePath}:`, error);
    return null;
  }
}

/**
 * Get detailed conversation with all messages
 */
export async function getDetailedConversation(conversation: ClaudeConversation): Promise<DetailedClaudeConversation | null> {
  try {
    const content = await readFile(conversation.filePath, 'utf-8');
    const lines = content.trim().split('\n').filter(line => line.trim());
    
    if (lines.length === 0) {
      return null;
    }

    // Parse all messages
    const messages: ClaudeMessage[] = lines.map(line => {
      try {
        const parsed = JSON.parse(line);
        return {
          role: parsed.role || 'user',
          content: parsed.content || '',
          timestamp: parsed.timestamp || Date.now()
        };
      } catch {
        return null;
      }
    }).filter(Boolean) as ClaudeMessage[];

    if (messages.length === 0) {
      return null;
    }

    return {
      ...conversation,
      messages
    };
  } catch (error) {
    console.error(`Failed to get detailed conversation:`, error);
    return null;
  }
}

/**
 * Generate a summary from conversation messages
 */
function generateSummary(messages: any[]): string {
  // Find user messages with flexible role matching
  const userMessages = messages.filter(msg => 
    msg.role === 'user' || msg.role === 'human' || 
    (msg.sender && msg.sender === 'human') ||
    (!msg.role && msg.content)
  ).slice(0, 3);
  
  const topics = userMessages.map(msg => {
    let content = '';
    
    // Handle different content formats
    if (typeof msg.content === 'string') {
      content = msg.content;
    } else if (Array.isArray(msg.content)) {
      // Handle array of content blocks
      for (const block of msg.content) {
        if (typeof block === 'string') {
          content = block;
          break;
        } else if (block && typeof block === 'object') {
          if (block.text && typeof block.text === 'string') {
            content = block.text;
            break;
          } else if (block.content && typeof block.content === 'string') {
            content = block.content;
            break;
          }
        }
      }
    } else if (msg.content && typeof msg.content === 'object') {
      if (msg.content.text) {
        content = msg.content.text;
      } else if (msg.content.content) {
        content = msg.content.content;
      }
    }
    
    const firstLine = content.split('\n')[0]?.trim() || '';
    return firstLine.length > 30 ? firstLine.substring(0, 27) + '...' : firstLine;
  }).filter(topic => topic.length > 0);
  
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