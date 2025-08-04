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
    if (process.env.DEBUG_CLAUDE_HISTORY || process.env.CLAUDE_WORKTREE_DEBUG) {
      console.log(`
[DEBUG] ===== Processing file: ${filePath} =====`);
      console.log(`[DEBUG] File basename: ${path.basename(filePath, '.jsonl')}`);
      console.log(`[DEBUG] Message count: ${messages.length}`);
      
      // Log first 3 messages in detail
      console.log(`[DEBUG] First 3 messages:`);
      messages.slice(0, 3).forEach((msg, idx) => {
        console.log(`[DEBUG] Message ${idx + 1}:`);
        console.log(`  - Type: ${typeof msg}`);
        console.log(`  - Keys: ${Object.keys(msg).join(', ')}`);
        console.log(`  - Role: ${msg.role || 'undefined'}`);
        console.log(`  - Content type: ${typeof msg.content}`);
        if (msg.content) {
          if (typeof msg.content === 'string') {
            console.log(`  - Content preview: ${msg.content.substring(0, 100)}...`);
          } else if (Array.isArray(msg.content)) {
            console.log(`  - Content is array with ${msg.content.length} items`);
            if (msg.content[0]) {
              console.log(`  - First item type: ${typeof msg.content[0]}`);
              console.log(`  - First item keys: ${typeof msg.content[0] === 'object' ? Object.keys(msg.content[0]).join(', ') : 'N/A'}`);
            }
          } else {
            console.log(`  - Content is object with keys: ${Object.keys(msg.content).join(', ')}`);
          }
        }
        console.log('');
      });
    }
    
    // Find last user message - Claude Code uses different message structure
    const lastUserMessage = messages.slice().reverse().find(msg => 
      // Claude Code format: type='message' + userType='user'
      (msg.type === 'message' && msg.userType === 'user') ||
      // Nested format: type='user' with message.role='user'
      (msg.type === 'user' && msg.message && msg.message.role === 'user') ||
      // Legacy format
      msg.role === 'user' || msg.role === 'human' || 
      (msg.sender && msg.sender === 'human') ||
      (!msg.role && msg.content) // fallback for messages without explicit role
    );
    
    if (lastUserMessage) {
      let extractedContent = '';
      
      // Extract content based on Claude Code's actual structure
      let messageContent = null;
      
      // For Claude Code format: msg.message.content
      if (lastUserMessage.message && lastUserMessage.message.content) {
        messageContent = lastUserMessage.message.content;
      }
      // For direct message field (string)
      else if (lastUserMessage.message && typeof lastUserMessage.message === 'string') {
        messageContent = lastUserMessage.message;
      }
      // For legacy content field
      else if (lastUserMessage.content) {
        messageContent = lastUserMessage.content;
      }
      
      // Handle different content formats that Claude Code might use
      if (typeof messageContent === 'string') {
        extractedContent = messageContent;
      } else if (Array.isArray(messageContent)) {
        // Handle array of content blocks
        for (const block of messageContent) {
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
      } else if (messageContent && typeof messageContent === 'object') {
        // Handle single content object
        if (messageContent.text) {
          extractedContent = messageContent.text;
        } else if (messageContent.content) {
          extractedContent = messageContent.content;
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
          console.log(`[DEBUG] lastUserMessage structure:`, JSON.stringify(lastUserMessage, null, 2).substring(0, 500));
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
        // Fallback: try to extract from any message content
        let foundTitle = false;
        
        // Try last messages first - more relevant for current context
        for (const msg of messages.slice(-10).reverse()) { // Check last 10 messages in reverse order
          if (msg && msg.content) {
            let content = '';
            
            // Extract content regardless of format
            if (typeof msg.content === 'string') {
              content = msg.content;
            } else if (Array.isArray(msg.content)) {
              for (const item of msg.content) {
                if (typeof item === 'string') {
                  content = item;
                  break;
                } else if (item && typeof item === 'object') {
                  content = item.text || item.content || JSON.stringify(item).substring(0, 100);
                  if (content) break;
                }
              }
            } else if (typeof msg.content === 'object') {
              content = msg.content.text || msg.content.content || JSON.stringify(msg.content).substring(0, 100);
            }
            
            // Clean and extract meaningful text
            if (content && content.length > 10) {
              // Remove common prefixes and clean up
              const cleaned = content
                .replace(/^(Human:|Assistant:|User:|System:|<.*?>|\[.*?\])/gi, '')
                .replace(/^\s*[-#*•]\s*/gm, '') // Remove list markers
                .trim();
              
              if (cleaned.length > 10) {
                // Get first sentence or line
                const firstSentence = cleaned.match(/^[^.!?\n]{10,60}/)?.[0] || cleaned.substring(0, 50);
                if (firstSentence && firstSentence.length > 10) {
                  title = firstSentence.trim() + (firstSentence.length === 50 ? '...' : '');
                  foundTitle = true;
                  break;
                }
              }
            }
          }
        }
        
        // If still no title, use generic title
        if (!foundTitle) {
          // We'll use the file stats later for the date
          title = `Conversation (${messages.length} messages)`;
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
        
        // Extract role and content based on Claude Code's actual structure
        let role: 'user' | 'assistant' = 'user';
        let content = '';
        
        // Determine role based on Claude Code's structure
        if (parsed.type === 'message' && parsed.userType === 'user') {
          role = 'user';
        } else if (parsed.type === 'user') {
          role = 'user';
        } else if (parsed.type === 'assistant') {
          role = 'assistant';
        } else if (parsed.message && parsed.message.role === 'user') {
          role = 'user';
        } else if (parsed.message && parsed.message.role === 'assistant') {
          role = 'assistant';
        } else if (parsed.role === 'user' || parsed.role === 'human') {
          role = 'user';
        } else if (parsed.role === 'assistant') {
          role = 'assistant';
        } else {
          // Default based on message structure
          role = 'assistant';
        }
        
        // Extract content based on Claude Code's structure
        if (parsed.message && parsed.message.content) {
          // For Claude Code format: msg.message.content
          const messageContent = parsed.message.content;
          if (typeof messageContent === 'string') {
            content = messageContent;
          } else if (Array.isArray(messageContent)) {
            // Handle array of content blocks
            for (const block of messageContent) {
              if (typeof block === 'string') {
                content = block;
                break;
              } else if (block && typeof block === 'object') {
                // Claude Code format: {type: "text", text: "..."}
                if (block.type === 'text' && block.text && typeof block.text === 'string') {
                  content = block.text;
                  break;
                } else if (block.type === 'tool_use' && block.name) {
                  // Display tool usage
                  content = `🔧 Used tool: ${block.name}`;
                  break;
                } else if (block.text && typeof block.text === 'string') {
                  content = block.text;
                  break;
                } else if (block.content && typeof block.content === 'string') {
                  content = block.content;
                  break;
                }
              }
            }
          }
        } else if (parsed.message && typeof parsed.message === 'string') {
          // For direct message field (string)
          content = parsed.message;
        } else if (parsed.content) {
          // For legacy content field
          if (typeof parsed.content === 'string') {
            content = parsed.content;
          } else if (Array.isArray(parsed.content)) {
            for (const block of parsed.content) {
              if (typeof block === 'string') {
                content = block;
                break;
              } else if (block && typeof block === 'object') {
                // Claude Code format: {type: "text", text: "..."}
                if (block.type === 'text' && block.text && typeof block.text === 'string') {
                  content = block.text;
                  break;
                } else if (block.type === 'tool_use' && block.name) {
                  // Display tool usage
                  content = `🔧 Used tool: ${block.name}`;
                  break;
                } else if (block.text && typeof block.text === 'string') {
                  content = block.text;
                  break;
                }
              }
            }
          }
        }
        
        return {
          role,
          content,
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