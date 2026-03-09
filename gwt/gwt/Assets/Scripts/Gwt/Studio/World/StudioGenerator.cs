using System.Collections.Generic;
using UnityEngine;
using UnityEngine.Tilemaps;

namespace Gwt.Studio.World
{
    public class StudioGenerator : MonoBehaviour
    {
        [SerializeField] private Tilemap _floorTilemap;
        [SerializeField] private TileBase _floorTile;
        [SerializeField] private GameObject _deskPrefab;
        [SerializeField] private Transform _deskContainer;
        [SerializeField] private float _deskSpawnDuration = 0.3f;

        private readonly Dictionary<string, GameObject> _deskObjects = new();

        public void GenerateStudio(StudioLayout layout)
        {
            GenerateFloor(layout.Width, layout.Height);

            foreach (var desk in layout.Desks)
            {
                AddDesk(desk);
            }
        }

        public void AddDesk(DeskSlot desk)
        {
            if (desk.AssignedBranch == null || _deskObjects.ContainsKey(desk.AssignedBranch))
                return;

            var position = new Vector3(desk.GridPosition.x, desk.GridPosition.y, 0);
            var parent = _deskContainer != null ? _deskContainer : transform;
            var deskObj = _deskPrefab != null
                ? Instantiate(_deskPrefab, position, Quaternion.identity, parent)
                : CreatePlaceholderDesk(position, parent);

            deskObj.name = $"Desk_{desk.AssignedBranch}";

            if (desk.IsRemote)
            {
                SetAlpha(deskObj, 0.5f);
            }

            _deskObjects[desk.AssignedBranch] = deskObj;
            StartCoroutine(ScaleAnimation(deskObj.transform, Vector3.zero, Vector3.one, _deskSpawnDuration));
        }

        public void RemoveDesk(string branch)
        {
            if (!_deskObjects.TryGetValue(branch, out var deskObj))
                return;

            _deskObjects.Remove(branch);
            StartCoroutine(ScaleAnimationThenDestroy(deskObj.transform, deskObj.transform.localScale, Vector3.zero, _deskSpawnDuration));
        }

        private void GenerateFloor(int width, int height)
        {
            if (_floorTilemap == null || _floorTile == null) return;

            for (int x = 0; x < width; x++)
            {
                for (int y = 0; y < height; y++)
                {
                    _floorTilemap.SetTile(new Vector3Int(x, y, 0), _floorTile);
                }
            }
        }

        private static GameObject CreatePlaceholderDesk(Vector3 position, Transform parent)
        {
            var obj = new GameObject("Desk");
            obj.transform.SetParent(parent);
            obj.transform.position = position;

            var sr = obj.AddComponent<SpriteRenderer>();
            sr.color = new Color(0.6f, 0.4f, 0.2f);

            return obj;
        }

        private static void SetAlpha(GameObject obj, float alpha)
        {
            var sr = obj.GetComponent<SpriteRenderer>();
            if (sr != null)
            {
                var c = sr.color;
                c.a = alpha;
                sr.color = c;
            }
        }

        private System.Collections.IEnumerator ScaleAnimation(Transform target, Vector3 from, Vector3 to, float duration)
        {
            float elapsed = 0f;
            target.localScale = from;
            while (elapsed < duration)
            {
                elapsed += Time.deltaTime;
                float t = Mathf.Clamp01(elapsed / duration);
                target.localScale = Vector3.Lerp(from, to, t);
                yield return null;
            }
            target.localScale = to;
        }

        private System.Collections.IEnumerator ScaleAnimationThenDestroy(Transform target, Vector3 from, Vector3 to, float duration)
        {
            yield return ScaleAnimation(target, from, to, duration);
            Destroy(target.gameObject);
        }
    }
}
