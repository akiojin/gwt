using System.Collections.Generic;

namespace Gwt.Core.Models
{
    [System.Serializable]
    public class GitHubLabel
    {
        public string Name;
        public string Color;
    }

    [System.Serializable]
    public class GitHubAssignee
    {
        public string Login;
        public string AvatarUrl;
    }

    [System.Serializable]
    public class GitHubMilestone
    {
        public string Title;
        public int Number;
    }

    [System.Serializable]
    public class GitHubIssue
    {
        public long Number;
        public string Title;
        public string UpdatedAt;
        public List<GitHubLabel> Labels = new();
        public string Body;
        public string State;
        public string HtmlUrl;
        public List<GitHubAssignee> Assignees = new();
        public int CommentsCount;
        public GitHubMilestone Milestone;
    }

    [System.Serializable]
    public class FetchIssuesResult
    {
        public List<GitHubIssue> Issues = new();
        public bool HasNextPage;
    }

    [System.Serializable]
    public class PullRequest
    {
        public long Number;
        public string Title;
        public string HeadBranch;
        public string State;
        public string BaseBranch;
        public string Url;
        public string UpdatedAt;
    }

    [System.Serializable]
    public class WorkflowRunInfo
    {
        public string WorkflowName;
        public long RunId;
        public string Status;
        public string Conclusion;
        public bool? IsRequired;
    }

    [System.Serializable]
    public class ReviewInfo
    {
        public string Reviewer;
        public string State;
    }

    [System.Serializable]
    public class ReviewComment
    {
        public string Author;
        public string Body;
        public string FilePath;
        public long? Line;
        public string CodeSnippet;
        public string CreatedAt;
    }

    [System.Serializable]
    public class PrStatusInfo
    {
        public long Number;
        public string Title;
        public string State;
        public string Url;
        public string Mergeable;
        public string Author;
        public string BaseBranch;
        public string HeadBranch;
        public List<string> Labels = new();
        public List<string> Assignees = new();
        public string Milestone;
        public List<long> LinkedIssues = new();
        public List<WorkflowRunInfo> CheckSuites = new();
        public List<ReviewInfo> Reviews = new();
        public List<ReviewComment> ReviewComments = new();
        public long ChangedFilesCount;
        public long Additions;
        public long Deletions;
    }
}
