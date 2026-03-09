using System.Collections.Generic;
using UnityEngine;

namespace Gwt.Studio.Entity
{
    public class LeadCharacter : MonoBehaviour
    {
        [SerializeField] private CharacterController2D _character;
        [SerializeField] private SpriteRenderer _speechBubble;
        [SerializeField] private float _patrolWaitTime = 2f;
        [SerializeField] private float _speechBubbleDuration = 3f;

        private List<Vector3> _patrolPoints = new();
        private int _currentPatrolIndex;
        private float _waitTimer;
        private bool _isPatrolling;
        private float _speechTimer;

        private void Awake()
        {
            if (_character == null)
                _character = GetComponent<CharacterController2D>();
        }

        public void SetPatrolPoints(List<Vector3> points)
        {
            _patrolPoints = points ?? new List<Vector3>();
            _currentPatrolIndex = 0;
        }

        public void StartPatrol()
        {
            if (_patrolPoints.Count == 0) return;
            _isPatrolling = true;
            MoveToNextPatrolPoint();
        }

        public void StopPatrol()
        {
            _isPatrolling = false;
        }

        public void ShowSpeechBubble(string text)
        {
            if (_speechBubble != null)
            {
                _speechBubble.gameObject.SetActive(true);
                _speechTimer = _speechBubbleDuration;
            }
        }

        private void Update()
        {
            if (_character == null) return;

            UpdateSpeechBubble();

            if (!_isPatrolling || _patrolPoints.Count == 0) return;

            if (_character.HasReachedTarget())
            {
                _waitTimer += Time.deltaTime;
                if (_waitTimer >= _patrolWaitTime)
                {
                    _waitTimer = 0f;
                    _currentPatrolIndex = (_currentPatrolIndex + 1) % _patrolPoints.Count;
                    MoveToNextPatrolPoint();
                }
            }
        }

        private void MoveToNextPatrolPoint()
        {
            if (_currentPatrolIndex < _patrolPoints.Count)
            {
                _character.MoveTo(_patrolPoints[_currentPatrolIndex]);
            }
        }

        private void UpdateSpeechBubble()
        {
            if (_speechBubble == null || !_speechBubble.gameObject.activeSelf) return;

            _speechTimer -= Time.deltaTime;
            if (_speechTimer <= 0f)
            {
                _speechBubble.gameObject.SetActive(false);
            }
        }
    }
}
