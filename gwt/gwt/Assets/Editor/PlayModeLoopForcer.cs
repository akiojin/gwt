#if UNITY_EDITOR
using UnityEditor;
using UnityEngine;

/// <summary>
/// Forces continuous PlayerLoop execution in Play Mode without forcing the
/// editor into a paused single-step state.
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

        // Queue another player loop tick without mutating the editor pause state.
        // EditorApplication.Step() immediately pauses Play Mode and was trapping
        // the editor in a play+pause state on Unity 6000.3.10.
        if (!EditorApplication.isPaused)
            EditorApplication.QueuePlayerLoopUpdate();
    }
}
#endif
