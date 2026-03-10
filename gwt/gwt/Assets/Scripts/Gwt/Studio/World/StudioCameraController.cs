using UnityEngine;
using UnityEngine.InputSystem;

namespace Gwt.Studio.World
{
    public class StudioCameraController : MonoBehaviour
    {
        [SerializeField] private Camera _camera;
        [SerializeField] private float _panSpeed = 5f;
        [SerializeField] private float _zoomSpeed = 2f;
        [SerializeField] private float _minZoom = 3f;
        [SerializeField] private float _maxZoom = 12f;
        [SerializeField] private float _followSmoothing = 5f;

        private Transform _followTarget;
        private bool _isFollowing;
        private Vector3 _dragOrigin;
        private bool _isDragging;

        private void Awake()
        {
            if (_camera == null)
                _camera = GetComponent<Camera>();
        }

        private void Update()
        {
            if (_camera == null) return;

            HandleZoom();

            if (_isFollowing && _followTarget != null)
            {
                FollowTarget();
            }
            else
            {
                HandleKeyboardPan();
                HandleMousePan();
            }
        }

        public void SetFollowTarget(Transform target)
        {
            _followTarget = target;
            _isFollowing = target != null;
        }

        public void StopFollowing()
        {
            _isFollowing = false;
            _followTarget = null;
        }

        private void HandleKeyboardPan()
        {
            var keyboard = Keyboard.current;
            if (keyboard == null) return;

            var input = Vector3.zero;

            if (keyboard.wKey.isPressed || keyboard.upArrowKey.isPressed) input.y += 1;
            if (keyboard.sKey.isPressed || keyboard.downArrowKey.isPressed) input.y -= 1;
            if (keyboard.aKey.isPressed || keyboard.leftArrowKey.isPressed) input.x -= 1;
            if (keyboard.dKey.isPressed || keyboard.rightArrowKey.isPressed) input.x += 1;

            if (input != Vector3.zero)
            {
                _isFollowing = false;
                transform.position += _panSpeed * Time.deltaTime * input.normalized;
            }
        }

        private void HandleMousePan()
        {
            var mouse = Mouse.current;
            if (mouse == null) return;

            if (mouse.middleButton.wasPressedThisFrame)
            {
                _isDragging = true;
                _dragOrigin = _camera.ScreenToWorldPoint(mouse.position.ReadValue());
            }

            if (mouse.middleButton.isPressed && _isDragging)
            {
                var diff = _dragOrigin - _camera.ScreenToWorldPoint(mouse.position.ReadValue());
                transform.position += diff;
                _isFollowing = false;
            }

            if (mouse.middleButton.wasReleasedThisFrame)
            {
                _isDragging = false;
            }
        }

        private void HandleZoom()
        {
            var mouse = Mouse.current;
            if (mouse == null) return;

            float scroll = mouse.scroll.y.ReadValue() / 120f;
            if (Mathf.Abs(scroll) > 0.01f)
            {
                _camera.orthographicSize = Mathf.Clamp(
                    _camera.orthographicSize - scroll * _zoomSpeed,
                    _minZoom,
                    _maxZoom
                );
            }
        }

        private void FollowTarget()
        {
            var targetPos = new Vector3(
                _followTarget.position.x,
                _followTarget.position.y,
                transform.position.z
            );
            transform.position = Vector3.Lerp(transform.position, targetPos, Time.deltaTime * _followSmoothing);
        }
    }
}
