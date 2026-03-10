using System;
using Cysharp.Threading.Tasks;
using Gwt.Lifecycle.Services;
using UnityEngine;
using UnityEngine.SceneManagement;

namespace Gwt.Studio.UI
{
    public class ProjectSceneTransitionController : MonoBehaviour
    {
        [SerializeField] private string _studioSceneName = "StudioScene";
        [SerializeField] private float _fadeDuration = 0.15f;

        private CanvasGroup _fadeCanvasGroup;

        public bool IsTransitioning { get; private set; }
        public string LastProjectPath { get; private set; } = string.Empty;

        protected virtual void Awake()
        {
            DontDestroyOnLoad(gameObject);
            EnsureFadeOverlay();
        }

        public virtual async UniTask<bool> TransitionToProjectAsync(ProjectInfo project)
        {
            if (project == null || string.IsNullOrWhiteSpace(project.Path))
                return false;

            if (IsTransitioning)
                return false;

            IsTransitioning = true;
            LastProjectPath = project.Path;

            try
            {
                await FadeAsync(1f);

                var activeScene = SceneManager.GetActiveScene();
                var useSingleReload = activeScene.name == _studioSceneName;
                var loadMode = useSingleReload ? LoadSceneMode.Single : LoadSceneMode.Additive;

                var loadOperation = SceneManager.LoadSceneAsync(_studioSceneName, loadMode);
                if (loadOperation == null)
                    return false;

                while (!loadOperation.isDone)
                    await UniTask.Yield(PlayerLoopTiming.Update);

                if (loadMode == LoadSceneMode.Additive)
                {
                    var newScene = SceneManager.GetSceneByName(_studioSceneName);
                    if (newScene.IsValid())
                        SceneManager.SetActiveScene(newScene);

                    if (activeScene.IsValid() && activeScene.isLoaded && activeScene != newScene)
                    {
                        var unload = SceneManager.UnloadSceneAsync(activeScene);
                        if (unload != null)
                        {
                            while (!unload.isDone)
                                await UniTask.Yield(PlayerLoopTiming.Update);
                        }
                    }
                }

                await FadeAsync(0f);
                return true;
            }
            finally
            {
                IsTransitioning = false;
            }
        }

        private void EnsureFadeOverlay()
        {
            if (_fadeCanvasGroup != null)
                return;

            var canvasObject = new GameObject("ProjectSceneTransitionOverlay");
            canvasObject.transform.SetParent(transform, false);

            var canvas = canvasObject.AddComponent<Canvas>();
            canvas.renderMode = RenderMode.ScreenSpaceOverlay;
            canvas.sortingOrder = short.MaxValue;

            canvasObject.AddComponent<UnityEngine.UI.GraphicRaycaster>();
            _fadeCanvasGroup = canvasObject.AddComponent<CanvasGroup>();
            _fadeCanvasGroup.alpha = 0f;
            _fadeCanvasGroup.interactable = false;
            _fadeCanvasGroup.blocksRaycasts = false;

            var imageObject = new GameObject("Fade");
            imageObject.transform.SetParent(canvasObject.transform, false);
            var rect = imageObject.AddComponent<RectTransform>();
            rect.anchorMin = Vector2.zero;
            rect.anchorMax = Vector2.one;
            rect.offsetMin = Vector2.zero;
            rect.offsetMax = Vector2.zero;
            var image = imageObject.AddComponent<UnityEngine.UI.Image>();
            image.color = Color.black;
        }

        private async UniTask FadeAsync(float targetAlpha)
        {
            EnsureFadeOverlay();
            if (_fadeCanvasGroup == null)
                return;

            var startAlpha = _fadeCanvasGroup.alpha;
            if (Mathf.Approximately(startAlpha, targetAlpha))
            {
                _fadeCanvasGroup.alpha = targetAlpha;
                return;
            }

            var duration = Mathf.Max(0.01f, _fadeDuration);
            var elapsed = 0f;
            while (elapsed < duration)
            {
                elapsed += Time.unscaledDeltaTime <= 0f ? 0.016f : Time.unscaledDeltaTime;
                _fadeCanvasGroup.alpha = Mathf.Lerp(startAlpha, targetAlpha, Mathf.Clamp01(elapsed / duration));
                await UniTask.Yield(PlayerLoopTiming.Update);
            }

            _fadeCanvasGroup.alpha = targetAlpha;
        }
    }
}
