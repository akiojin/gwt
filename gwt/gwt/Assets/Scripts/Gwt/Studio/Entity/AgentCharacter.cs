using UnityEngine;

namespace Gwt.Studio.Entity
{
    public class AgentCharacter : MonoBehaviour
    {
        [SerializeField] private CharacterController2D _character;
        [SerializeField] private SpriteRenderer _statusBubble;
        [SerializeField] private SpriteRenderer _issueMarker;
        [SerializeField] private SpriteRenderer _prBadge;
        [SerializeField] private Transform _entrancePoint;
        [SerializeField] private Transform _exitPoint;

        private Vector3 _deskPosition;
        private string _agentId;
        private string _assignedBranch;

        public string AgentId => _agentId;
        public string AssignedBranch => _assignedBranch;

        private void Awake()
        {
            if (_character == null)
                _character = GetComponent<CharacterController2D>();
        }

        public void Initialize(string agentId, string branch, Vector3 deskPosition)
        {
            _agentId = agentId;
            _assignedBranch = branch;
            _deskPosition = deskPosition;
        }

        public void Hire(Vector3 entrancePosition)
        {
            transform.position = entrancePosition;
            _character.SetState(CharacterState.Entering);
            _character.MoveTo(_deskPosition);
        }

        public void Fire(Vector3 exitPosition)
        {
            _character.SetState(CharacterState.Leaving);
            _character.MoveTo(exitPosition);
        }

        public void ShowStatusBubble(bool show)
        {
            if (_statusBubble != null)
                _statusBubble.gameObject.SetActive(show);
        }

        public void ShowIssueMarker(bool show)
        {
            if (_issueMarker != null)
                _issueMarker.gameObject.SetActive(show);
        }

        public void ShowPrBadge(bool show)
        {
            if (_prBadge != null)
                _prBadge.gameObject.SetActive(show);
        }

        private void Update()
        {
            if (_character == null) return;

            if (_character.State == CharacterState.Entering && _character.HasReachedTarget())
            {
                _character.SetState(CharacterState.Working);
            }
        }
    }
}
