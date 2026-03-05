import type { ProjectIssue } from '$lib/types';

export function getMockIssues(): ProjectIssue[] {
  return [
    {
      id: 'mock-issue-1',
      githubIssueNumber: 42,
      githubIssueUrl: 'https://github.com/example/repo/issues/42',
      title: 'Login UI Redesign',
      status: 'in_progress',
      tasks: [
        {
          id: 'mock-task-1-1',
          name: 'Design login form layout',
          status: 'completed',
          developers: [
            {
              id: 'dev-claude-1',
              agentType: 'claude',
              paneId: 'pane-1',
              status: 'completed',
              worktree: { branchName: 'feat/login-form', path: '/tmp/wt-1' },
            },
          ],
          testStatus: 'passed',
          retryCount: 0,
        },
        {
          id: 'mock-task-1-2',
          name: 'Implement OAuth integration',
          status: 'running',
          developers: [
            {
              id: 'dev-claude-2',
              agentType: 'claude',
              paneId: 'pane-2',
              status: 'running',
              worktree: { branchName: 'feat/oauth', path: '/tmp/wt-2' },
            },
            {
              id: 'dev-codex-1',
              agentType: 'codex',
              paneId: 'pane-3',
              status: 'running',
              worktree: { branchName: 'feat/oauth-api', path: '/tmp/wt-3' },
            },
          ],
          testStatus: 'running',
          retryCount: 0,
        },
        {
          id: 'mock-task-1-3',
          name: 'Write E2E tests',
          status: 'pending',
          developers: [],
          testStatus: 'not_run',
          retryCount: 0,
        },
        {
          id: 'mock-task-1-4',
          name: 'Accessibility audit',
          status: 'completed',
          developers: [
            {
              id: 'dev-gemini-1',
              agentType: 'gemini',
              paneId: 'pane-4',
              status: 'completed',
              worktree: { branchName: 'feat/a11y', path: '/tmp/wt-4' },
            },
          ],
          testStatus: 'passed',
          retryCount: 0,
        },
      ],
    },
    {
      id: 'mock-issue-2',
      githubIssueNumber: 58,
      githubIssueUrl: 'https://github.com/example/repo/issues/58',
      title: 'API Refactor to REST v2',
      status: 'planned',
      tasks: [
        {
          id: 'mock-task-2-1',
          name: 'Define new endpoint schema',
          status: 'ready',
          developers: [
            {
              id: 'dev-codex-2',
              agentType: 'codex',
              paneId: 'pane-5',
              status: 'starting',
              worktree: { branchName: 'feat/api-schema', path: '/tmp/wt-5' },
            },
          ],
          testStatus: 'not_run',
          retryCount: 0,
        },
        {
          id: 'mock-task-2-2',
          name: 'Migrate database queries',
          status: 'pending',
          developers: [],
          retryCount: 0,
        },
        {
          id: 'mock-task-2-3',
          name: 'Update client SDK',
          status: 'pending',
          developers: [],
          retryCount: 0,
        },
      ],
    },
    {
      id: 'mock-issue-3',
      githubIssueNumber: 71,
      githubIssueUrl: 'https://github.com/example/repo/issues/71',
      title: 'Integration Test Suite',
      status: 'pending',
      tasks: [
        {
          id: 'mock-task-3-1',
          name: 'Set up test framework',
          status: 'pending',
          developers: [],
          retryCount: 0,
        },
        {
          id: 'mock-task-3-2',
          name: 'Write smoke tests',
          status: 'pending',
          developers: [],
          retryCount: 0,
        },
      ],
    },
  ];
}
