namespace Gwt.Core.Services.Pty
{
    public interface IPlatformShellDetector
    {
        string DetectDefaultShell();
        string[] GetShellArgs(string shell);
        bool IsShellAvailable(string shell);
    }
}
