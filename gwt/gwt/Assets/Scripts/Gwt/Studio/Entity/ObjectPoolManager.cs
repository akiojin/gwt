using System.Collections.Generic;
using UnityEngine;

namespace Gwt.Studio.Entity
{
    public class ObjectPoolManager : MonoBehaviour
    {
        private readonly Dictionary<string, Queue<GameObject>> _pools = new();

        public GameObject Get(string key, GameObject prefab)
        {
            if (_pools.TryGetValue(key, out var pool) && pool.Count > 0)
            {
                var obj = pool.Dequeue();
                obj.SetActive(true);
                return obj;
            }
            return Instantiate(prefab);
        }

        public void Return(string key, GameObject obj)
        {
            obj.SetActive(false);
            if (!_pools.ContainsKey(key))
                _pools[key] = new Queue<GameObject>();
            _pools[key].Enqueue(obj);
        }
    }
}
