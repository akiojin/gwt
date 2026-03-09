using System.Collections.Generic;
using UnityEngine;

namespace Gwt.Studio.Entity
{
    public static class SimplePathfinder
    {
        private class Node
        {
            public Vector2Int Position;
            public Node Parent;
            public int G;
            public int H;
            public int F => G + H;
        }

        private static readonly Vector2Int[] CardinalDirections =
        {
            Vector2Int.up, Vector2Int.down, Vector2Int.left, Vector2Int.right
        };

        private static readonly Vector2Int[] AllDirections =
        {
            Vector2Int.up, Vector2Int.down, Vector2Int.left, Vector2Int.right,
            new(1, 1), new(1, -1), new(-1, 1), new(-1, -1)
        };

        public static List<Vector2Int> FindPath(
            Vector2Int start,
            Vector2Int end,
            HashSet<Vector2Int> obstacles,
            int gridWidth,
            int gridHeight,
            bool allowDiagonal = false)
        {
            if (start == end) return new List<Vector2Int> { start };
            if (obstacles != null && obstacles.Contains(end)) return new List<Vector2Int>();

            var directions = allowDiagonal ? AllDirections : CardinalDirections;
            var openSet = new SortedList<int, List<Node>>();
            var closedSet = new HashSet<Vector2Int>();
            var startNode = new Node { Position = start, G = 0, H = Heuristic(start, end) };

            AddToOpenSet(openSet, startNode);

            while (openSet.Count > 0)
            {
                var current = PopBest(openSet);
                if (current.Position == end)
                    return ReconstructPath(current);

                closedSet.Add(current.Position);

                foreach (var dir in directions)
                {
                    var neighbor = current.Position + dir;

                    if (neighbor.x < 0 || neighbor.x >= gridWidth ||
                        neighbor.y < 0 || neighbor.y >= gridHeight)
                        continue;

                    if (closedSet.Contains(neighbor))
                        continue;

                    if (obstacles != null && obstacles.Contains(neighbor))
                        continue;

                    int moveCost = (dir.x != 0 && dir.y != 0) ? 14 : 10;
                    int newG = current.G + moveCost;

                    var neighborNode = new Node
                    {
                        Position = neighbor,
                        Parent = current,
                        G = newG,
                        H = Heuristic(neighbor, end)
                    };

                    AddToOpenSet(openSet, neighborNode);
                }
            }

            return new List<Vector2Int>();
        }

        private static int Heuristic(Vector2Int a, Vector2Int b)
        {
            return (Mathf.Abs(a.x - b.x) + Mathf.Abs(a.y - b.y)) * 10;
        }

        private static void AddToOpenSet(SortedList<int, List<Node>> openSet, Node node)
        {
            if (!openSet.TryGetValue(node.F, out var list))
            {
                list = new List<Node>();
                openSet[node.F] = list;
            }
            list.Add(node);
        }

        private static Node PopBest(SortedList<int, List<Node>> openSet)
        {
            var firstKey = openSet.Keys[0];
            var list = openSet[firstKey];
            var best = list[0];
            list.RemoveAt(0);
            if (list.Count == 0)
                openSet.Remove(firstKey);
            return best;
        }

        private static List<Vector2Int> ReconstructPath(Node node)
        {
            var path = new List<Vector2Int>();
            while (node != null)
            {
                path.Add(node.Position);
                node = node.Parent;
            }
            path.Reverse();
            return path;
        }
    }
}
