using System;
using System.IO;
using System.Runtime.InteropServices;

namespace Gwt.Core.Services.Pty
{
    public class PlatformShellDetector : IPlatformShellDetector
    {
        public string DetectDefaultShell()
        {
            if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
                return DetectWindowsShell();

            return DetectUnixShell();
        }

        public string[] GetShellArgs(string shell)
        {
            if (shell == null)
                throw new ArgumentNullException(nameof(shell));

            var name = Path.GetFileNameWithoutExtension(shell).ToLowerInvariant();
            return name switch
            {
                "cmd" => new[] { "/Q" },
                "powershell" or "pwsh" => new[] { "-NoLogo", "-NoProfile" },
                _ => Array.Empty<string>()
            };
        }

        public bool IsShellAvailable(string shell)
        {
            if (string.IsNullOrEmpty(shell))
                return false;

            if (Path.IsPathRooted(shell))
                return File.Exists(shell);

            // For non-rooted paths on Unix, check common locations
            if (!RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
            {
                string[] searchPaths = { "/bin", "/usr/bin", "/usr/local/bin" };
                foreach (var dir in searchPaths)
                {
                    if (File.Exists(Path.Combine(dir, shell)))
                        return true;
                }
            }

            return false;
        }

        private static string DetectUnixShell()
        {
            var shellEnv = Environment.GetEnvironmentVariable("SHELL");
            if (!string.IsNullOrEmpty(shellEnv) && File.Exists(shellEnv))
                return shellEnv;

            if (RuntimeInformation.IsOSPlatform(OSPlatform.OSX))
            {
                if (File.Exists("/bin/zsh")) return "/bin/zsh";
                if (File.Exists("/bin/bash")) return "/bin/bash";
            }
            else
            {
                if (File.Exists("/bin/bash")) return "/bin/bash";
                if (File.Exists("/bin/sh")) return "/bin/sh";
            }

            return "/bin/sh";
        }

        private static string DetectWindowsShell()
        {
            // Prefer PowerShell 7+ (pwsh)
            var pwsh = FindInPath("pwsh.exe");
            if (pwsh != null) return pwsh;

            // Fallback to Windows PowerShell
            var psPath = Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.System),
                "WindowsPowerShell", "v1.0", "powershell.exe");
            if (File.Exists(psPath)) return psPath;

            // Fallback to cmd
            var cmdPath = Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.System),
                "cmd.exe");
            if (File.Exists(cmdPath)) return cmdPath;

            return "cmd.exe";
        }

        private static string FindInPath(string executable)
        {
            var pathEnv = Environment.GetEnvironmentVariable("PATH");
            if (string.IsNullOrEmpty(pathEnv)) return null;

            var separator = RuntimeInformation.IsOSPlatform(OSPlatform.Windows) ? ';' : ':';
            foreach (var dir in pathEnv.Split(separator))
            {
                var fullPath = Path.Combine(dir.Trim(), executable);
                if (File.Exists(fullPath)) return fullPath;
            }

            return null;
        }
    }
}
