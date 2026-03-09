using Cysharp.Threading.Tasks;
using System.Collections.Generic;
using System.Threading;

namespace Gwt.Core.Models
{
    public interface ISessionService
    {
        UniTask<Session> CreateSessionAsync(string worktreePath, string branch, CancellationToken ct = default);
        UniTask<Session> GetSessionAsync(string sessionId, CancellationToken ct = default);
        UniTask<List<Session>> ListSessionsAsync(string projectRoot, CancellationToken ct = default);
        UniTask UpdateSessionAsync(Session session, CancellationToken ct = default);
        UniTask DeleteSessionAsync(string sessionId, CancellationToken ct = default);
    }
}
