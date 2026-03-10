namespace Gwt.Agent.Services.SkillRegistration
{
    public readonly struct ManagedAsset
    {
        public readonly string RelativePath;
        public readonly string Body;
        public readonly bool Executable;
        public readonly bool RewriteForProject;

        public ManagedAsset(string relativePath, string body, bool executable, bool rewriteForProject)
        {
            RelativePath = relativePath;
            Body = body;
            Executable = executable;
            RewriteForProject = rewriteForProject;
        }
    }
}
