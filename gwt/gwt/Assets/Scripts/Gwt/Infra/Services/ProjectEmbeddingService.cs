using System;
using System.Collections.Generic;
using System.Linq;

namespace Gwt.Infra.Services
{
    public interface IProjectEmbeddingService
    {
        bool IsAvailable { get; }
        int Dimensions { get; }
        List<float> EmbedTerms(IEnumerable<SemanticTokenWeight> terms);
    }

    public class ProjectEmbeddingService : IProjectEmbeddingService
    {
        public bool IsAvailable => true;
        public int Dimensions { get; }

        public ProjectEmbeddingService(int dimensions = 64)
        {
            Dimensions = Math.Max(8, dimensions);
        }

        public List<float> EmbedTerms(IEnumerable<SemanticTokenWeight> terms)
        {
            var vector = new float[Dimensions];
            if (terms == null)
                return vector.ToList();

            foreach (var term in terms)
            {
                if (term == null || string.IsNullOrWhiteSpace(term.Token) || term.Weight <= 0f)
                    continue;

                var primaryIndex = StableHash(term.Token) % Dimensions;
                vector[primaryIndex] += term.Weight;

                // A second bucket reduces collisions while keeping the vector small.
                var secondaryIndex = StableHash($"{term.Token}:secondary") % Dimensions;
                vector[secondaryIndex] += term.Weight * 0.5f;
            }

            Normalize(vector);
            return vector.ToList();
        }

        private static int StableHash(string value)
        {
            unchecked
            {
                var hash = 2166136261;
                foreach (var c in value)
                {
                    hash ^= c;
                    hash *= 16777619;
                }

                return (int)(hash & 0x7fffffff);
            }
        }

        private static void Normalize(float[] vector)
        {
            var norm = 0f;
            for (var i = 0; i < vector.Length; i++)
                norm += vector[i] * vector[i];

            if (norm <= 0f)
                return;

            var scale = 1f / UnityEngine.Mathf.Sqrt(norm);
            for (var i = 0; i < vector.Length; i++)
                vector[i] *= scale;
        }
    }
}
