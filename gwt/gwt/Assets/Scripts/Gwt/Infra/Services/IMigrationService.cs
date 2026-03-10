using Cysharp.Threading.Tasks;
using System.Threading;

namespace Gwt.Infra.Services
{
    public enum MigrationState
    {
        NotNeeded,
        Available,
        InProgress,
        Completed,
        Failed
    }

    [System.Serializable]
    public class MigrationPlan
    {
        public string ProjectRoot;
        public string GwtDir;
        public string BackupDir;
        public System.Collections.Generic.List<string> TomlFiles = new();
        public System.Collections.Generic.List<string> JsonTargets = new();
    }

    [System.Serializable]
    public class MigrationResult
    {
        public MigrationState State;
        public System.Collections.Generic.List<string> ConvertedFiles = new();
        public System.Collections.Generic.List<string> SkippedFiles = new();
        public string ErrorMessage;
        public string BackupDir;
    }

    [System.Serializable]
    public class TomlToJsonMapping
    {
        public string SourcePath;
        public string DestinationPath;
        public string BackupPath;
        public string FileKind;
    }

    public interface IMigrationService
    {
        MigrationResult LastResult { get; }
        UniTask<MigrationState> CheckMigrationNeededAsync(string projectRoot, CancellationToken ct = default);
        UniTask MigrateAsync(string projectRoot, CancellationToken ct = default);
    }
}
