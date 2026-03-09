using UnityEngine;

namespace Gwt.Studio.World
{
    public enum AtmosphereState
    {
        Normal,
        CISuccess,
        CIFail
    }

    public class AtmosphereController : MonoBehaviour
    {
        [SerializeField] private Camera _mainCamera;
        [SerializeField] private float _transitionSpeed = 2f;

        private AtmosphereState _currentState = AtmosphereState.Normal;
        private Color _targetColor;

        private static readonly Color NormalColor = new(0.15f, 0.15f, 0.2f);
        private static readonly Color SuccessColor = new(0.2f, 0.25f, 0.2f);
        private static readonly Color FailColor = new(0.3f, 0.1f, 0.1f);

        private void Awake()
        {
            if (_mainCamera == null)
                _mainCamera = Camera.main;
            _targetColor = NormalColor;
        }

        private void Update()
        {
            if (_mainCamera == null) return;

            _mainCamera.backgroundColor = Color.Lerp(
                _mainCamera.backgroundColor,
                _targetColor,
                Time.deltaTime * _transitionSpeed
            );
        }

        public void SetState(AtmosphereState state)
        {
            _currentState = state;
            _targetColor = state switch
            {
                AtmosphereState.CISuccess => SuccessColor,
                AtmosphereState.CIFail => FailColor,
                _ => NormalColor,
            };
        }

        public AtmosphereState CurrentState => _currentState;
    }
}
