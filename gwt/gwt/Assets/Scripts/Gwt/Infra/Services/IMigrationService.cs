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

    public interface IMigrationService
    {
        UniTask<MigrationState> CheckMigrationNeededAsync(string projectRoot, CancellationToken ct = default);
        UniTask MigrateAsync(string projectRoot, CancellationToken ct = default);
    }
}
