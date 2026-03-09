using Cysharp.Threading.Tasks;
using Gwt.Core.Models;
using System;
using System.Collections.Generic;
using System.IO;
using System.Threading;
using UnityEngine;

namespace Gwt.Core.Services.Config
{
    public class SessionService : ISessionService
    {
        const string SessionsDirName = "sessions";
        const string GwtDirName = ".gwt";

        public async UniTask<Session> CreateSessionAsync(string worktreePath, string branch, CancellationToken ct = default)
        {
            var session = new Session
            {
                Id = Guid.NewGuid().ToString(),
                WorktreePath = worktreePath,
                Branch = branch,
                CreatedAt = DateTime.UtcNow.ToString("o"),
                UpdatedAt = DateTime.UtcNow.ToString("o"),
                Status = AgentStatusValue.Unknown,
                LastActivityAt = DateTime.UtcNow.ToString("o")
            };

            await SaveSessionFileAsync(session, ct);
            return session;
        }

        public async UniTask<Session> GetSessionAsync(string sessionId, CancellationToken ct = default)
        {
            var path = GetSessionFilePath(sessionId);
            if (!File.Exists(path))
                return null;

            await UniTask.SwitchToThreadPool();
            ct.ThrowIfCancellationRequested();
            var json = await File.ReadAllTextAsync(path, ct);
            await UniTask.SwitchToMainThread(ct);

            return JsonUtility.FromJson<Session>(json);
        }

        public async UniTask<List<Session>> ListSessionsAsync(string projectRoot, CancellationToken ct = default)
        {
            var dir = GetSessionsDir();
            if (!Directory.Exists(dir))
                return new List<Session>();

            await UniTask.SwitchToThreadPool();
            ct.ThrowIfCancellationRequested();

            var files = Directory.GetFiles(dir, "*.json");
            var sessions = new List<Session>();

            foreach (var file in files)
            {
                ct.ThrowIfCancellationRequested();
                var json = await File.ReadAllTextAsync(file, ct);
                await UniTask.SwitchToMainThread(ct);
                var session = JsonUtility.FromJson<Session>(json);
                if (session != null)
                    sessions.Add(session);
                await UniTask.SwitchToThreadPool();
            }

            await UniTask.SwitchToMainThread(ct);
            return sessions;
        }

        public async UniTask UpdateSessionAsync(Session session, CancellationToken ct = default)
        {
            session.UpdatedAt = DateTime.UtcNow.ToString("o");
            await SaveSessionFileAsync(session, ct);
        }

        public async UniTask DeleteSessionAsync(string sessionId, CancellationToken ct = default)
        {
            var path = GetSessionFilePath(sessionId);

            await UniTask.SwitchToThreadPool();
            ct.ThrowIfCancellationRequested();

            if (File.Exists(path))
                File.Delete(path);

            await UniTask.SwitchToMainThread(ct);
        }

        async UniTask SaveSessionFileAsync(Session session, CancellationToken ct)
        {
            var dir = GetSessionsDir();
            if (!Directory.Exists(dir))
                Directory.CreateDirectory(dir);

            var path = GetSessionFilePath(session.Id);
            var tmpPath = path + ".tmp";
            var json = JsonUtility.ToJson(session, true);

            await UniTask.SwitchToThreadPool();
            ct.ThrowIfCancellationRequested();
            await File.WriteAllTextAsync(tmpPath, json, ct);

            if (File.Exists(path))
                File.Delete(path);
            File.Move(tmpPath, path);

            await UniTask.SwitchToMainThread(ct);
        }

        static string GetSessionsDir()
        {
            var home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
            return Path.Combine(home, GwtDirName, SessionsDirName);
        }

        static string GetSessionFilePath(string sessionId)
        {
            return Path.Combine(GetSessionsDir(), sessionId + ".json");
        }
    }
}
