using Cysharp.Threading.Tasks;
using System.Threading;

namespace Gwt.Core.Models
{
    public interface IConfigService
    {
        UniTask<Settings> LoadSettingsAsync(string projectRoot, CancellationToken ct = default);
        UniTask SaveSettingsAsync(string projectRoot, Settings settings, CancellationToken ct = default);
        UniTask<Settings> GetOrCreateSettingsAsync(string projectRoot, CancellationToken ct = default);
        string GetGwtDir(string projectRoot);
    }
}
