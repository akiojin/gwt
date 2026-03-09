using Cysharp.Threading.Tasks;
using System;
using System.Threading;

namespace Gwt.Core.Models
{
    public interface IPtyService
    {
        UniTask<string> SpawnAsync(string command, string[] args, string workingDir, int rows, int cols, CancellationToken ct = default);
        UniTask WriteAsync(string paneId, string data, CancellationToken ct = default);
        UniTask ResizeAsync(string paneId, int rows, int cols, CancellationToken ct = default);
        UniTask KillAsync(string paneId, CancellationToken ct = default);
        IObservable<string> GetOutputStream(string paneId);
        PaneStatus GetStatus(string paneId);
    }
}
