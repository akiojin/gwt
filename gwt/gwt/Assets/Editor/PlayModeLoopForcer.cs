#if UNITY_EDITOR
using UnityEditor;
using UnityEngine;

/// <summary>
/// Forces continuous PlayerLoop execution in Play Mode via Step+Unpause cycle.
/// Workaround for Unity 6 where the PlayerLoop halts when the Game view lacks
/// OS-level window focus (common in CLI-driven development).
/// </summary>
[InitializeOnLoad]
static class PlayModeLoopForcer
{
    private static bool _active;

    static PlayModeLoopForcer()
    {
        if (!PlayerSettings.runInBackground)
            PlayerSettings.runInBackground = true;

        EditorApplication.playModeStateChanged += OnPlayModeChanged;
        EditorApplication.update += OnEditorUpdate;
    }

    static void OnPlayModeChanged(PlayModeStateChange state)
    {
        if (state == PlayModeStateChange.EnteredPlayMode)
        {
            _active = true;
            Application.runInBackground = true;
        }
        else if (state == PlayModeStateChange.ExitingPlayMode)
        {
            _active = false;
        }
    }

    static void OnEditorUpdate()
    {
        if (!_active || !EditorApplication.isPlaying)
            return;

        if (EditorApplication.isPaused)
        {
            EditorApplication.isPaused = false;
        }

        EditorApplication.Step();
    }
}
#endif
