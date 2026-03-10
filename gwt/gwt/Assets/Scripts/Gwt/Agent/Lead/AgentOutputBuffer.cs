using System.Collections.Generic;
using Gwt.Agent.Services;

namespace Gwt.Agent.Lead
{
    public class AgentOutputBuffer
    {
        readonly Dictionary<string, Queue<string>> _buffers = new();
        readonly int _maxLines;

        public AgentOutputBuffer(IAgentService agentService, int maxLines = 100)
        {
            _maxLines = maxLines;
            agentService.OnAgentOutput += HandleOutput;
        }

        void HandleOutput(string sessionId, string output)
        {
            if (!_buffers.TryGetValue(sessionId, out var queue))
            {
                queue = new Queue<string>();
                _buffers[sessionId] = queue;
            }

            foreach (var line in output.Split('\n'))
            {
                queue.Enqueue(line);
                while (queue.Count > _maxLines)
                    queue.Dequeue();
            }
        }

        public string GetRecentOutput(string sessionId, int lines = 20)
        {
            if (!_buffers.TryGetValue(sessionId, out var queue))
                return string.Empty;

            var recent = new List<string>();
            var arr = queue.ToArray();
            var start = System.Math.Max(0, arr.Length - lines);
            for (var i = start; i < arr.Length; i++)
                recent.Add(arr[i]);

            return string.Join("\n", recent);
        }

        public void Clear(string sessionId)
        {
            _buffers.Remove(sessionId);
        }
    }
}
