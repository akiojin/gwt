using UnityEngine;

namespace Gwt.Studio.Entity
{
    [RequireComponent(typeof(SpriteRenderer))]
    public class CharacterController2D : MonoBehaviour
    {
        [SerializeField] private float _moveSpeed = 3f;
        [SerializeField] private float _idleBlinkInterval = 3f;

        private SpriteRenderer _spriteRenderer;
        private CharacterState _state = CharacterState.Idle;
        private FacingDirection _facing = FacingDirection.Down;
        private Vector3 _targetPosition;
        private float _idleTimer;

        public CharacterState State => _state;
        public FacingDirection Facing => _facing;
        public float MoveSpeed => _moveSpeed;

        protected virtual void Awake()
        {
            _spriteRenderer = GetComponent<SpriteRenderer>();
            _targetPosition = transform.position;
        }

        protected virtual void Update()
        {
            switch (_state)
            {
                case CharacterState.Walking:
                case CharacterState.Entering:
                case CharacterState.Leaving:
                    UpdateMovement();
                    break;
                case CharacterState.Idle:
                    UpdateIdle();
                    break;
            }
        }

        public void SetState(CharacterState newState)
        {
            _state = newState;
        }

        public void MoveTo(Vector3 position)
        {
            _targetPosition = position;
            if (_state == CharacterState.Idle || _state == CharacterState.Working)
                _state = CharacterState.Walking;
        }

        public void SetFacing(FacingDirection direction)
        {
            _facing = direction;
            if (_spriteRenderer != null)
            {
                _spriteRenderer.flipX = direction == FacingDirection.Left;
            }
        }

        public bool HasReachedTarget()
        {
            return Vector3.Distance(transform.position, _targetPosition) < 0.05f;
        }

        private void UpdateMovement()
        {
            if (HasReachedTarget())
            {
                transform.position = _targetPosition;
                if (_state == CharacterState.Walking)
                    _state = CharacterState.Idle;
                return;
            }

            var direction = (_targetPosition - transform.position).normalized;
            transform.position += _moveSpeed * Time.deltaTime * direction;
            UpdateFacingFromDirection(direction);
        }

        private void UpdateIdle()
        {
            _idleTimer += Time.deltaTime;
            if (_idleTimer >= _idleBlinkInterval)
            {
                _idleTimer = 0f;
                StartCoroutine(BlinkAnimation());
            }
        }

        private System.Collections.IEnumerator BlinkAnimation()
        {
            if (_spriteRenderer == null) yield break;

            var originalColor = _spriteRenderer.color;
            _spriteRenderer.color = new Color(originalColor.r, originalColor.g, originalColor.b, 0.7f);
            yield return new WaitForSeconds(0.1f);
            _spriteRenderer.color = originalColor;
        }

        private void UpdateFacingFromDirection(Vector3 direction)
        {
            if (Mathf.Abs(direction.x) > Mathf.Abs(direction.y))
            {
                SetFacing(direction.x > 0 ? FacingDirection.Right : FacingDirection.Left);
            }
            else
            {
                SetFacing(direction.y > 0 ? FacingDirection.Up : FacingDirection.Down);
            }
        }
    }
}
