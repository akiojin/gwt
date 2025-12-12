interface SynapseNode {
  x: number;
  y: number;
  vx: number;
  vy: number;
  radius: number;
}

interface SynapseEdge {
  from: number;
  to: number;
  pulse: number;
  speed: number;
}

const NODE_COUNT = 32;
const LINKS_PER_NODE = 3;

const rand = (limit: number) => Math.random() * limit;

function createNodes(
  width: number,
  height: number,
  count: number,
): SynapseNode[] {
  return Array.from({ length: count }, () => ({
    x: rand(width),
    y: rand(height),
    vx: (Math.random() - 0.5) * 0.5,
    vy: (Math.random() - 0.5) * 0.5,
    radius: 2 + Math.random() * 3,
  }));
}

function createEdges(nodeCount: number, linksPerNode: number): SynapseEdge[] {
  const edges: SynapseEdge[] = [];
  for (let i = 0; i < nodeCount; i += 1) {
    for (let j = 0; j < linksPerNode; j += 1) {
      const target = Math.floor(Math.random() * nodeCount);
      if (target !== i) {
        edges.push({
          from: i,
          to: target,
          pulse: Math.random(),
          speed: 0.002 + Math.random() * 0.005,
        });
      }
    }
  }
  return edges;
}

function updateNodes(nodes: SynapseNode[], width: number, height: number) {
  for (const node of nodes) {
    node.x += node.vx;
    node.y += node.vy;

    if (node.x < 0 || node.x > width) node.vx *= -1;
    if (node.y < 0 || node.y > height) node.vy *= -1;

    node.x = Math.max(0, Math.min(width, node.x));
    node.y = Math.max(0, Math.min(height, node.y));
  }
}

function drawNetwork(
  ctx: CanvasRenderingContext2D,
  nodes: SynapseNode[],
  edges: SynapseEdge[],
  bounds: { width: number; height: number },
) {
  ctx.clearRect(0, 0, bounds.width, bounds.height);
  const connectionRadius = Math.min(bounds.width, bounds.height) * 0.4;

  edges.forEach((edge) => {
    const source = nodes[edge.from];
    const target = nodes[edge.to];
    if (!source || !target) return;

    const dx = target.x - source.x;
    const dy = target.y - source.y;
    const distance = Math.hypot(dx, dy);
    if (distance > connectionRadius) {
      return;
    }
    const strength = 1 - distance / connectionRadius;

    const gradient = ctx.createLinearGradient(
      source.x,
      source.y,
      target.x,
      target.y,
    );
    gradient.addColorStop(0, `rgba(94,234,212,${0.15 + 0.4 * strength})`);
    gradient.addColorStop(1, `rgba(14,165,233,${0.1 + 0.35 * strength})`);
    ctx.lineWidth = 0.5 + strength * 2;
    ctx.strokeStyle = gradient;
    ctx.beginPath();
    ctx.moveTo(source.x, source.y);
    ctx.lineTo(target.x, target.y);
    ctx.stroke();

    edge.pulse = (edge.pulse + edge.speed) % 1;
    const pulseX = source.x + dx * edge.pulse;
    const pulseY = source.y + dy * edge.pulse;
    ctx.fillStyle = `rgba(236,72,153,${0.3 + strength * 0.5})`;
    ctx.beginPath();
    ctx.arc(pulseX, pulseY, 2 + strength * 3, 0, Math.PI * 2);
    ctx.fill();
  });

  nodes.forEach((node) => {
    const glow = ctx.createRadialGradient(
      node.x,
      node.y,
      0,
      node.x,
      node.y,
      node.radius * 4,
    );
    glow.addColorStop(0, "rgba(255,255,255,0.9)");
    glow.addColorStop(0.4, "rgba(45,212,191,0.5)");
    glow.addColorStop(1, "transparent");
    ctx.fillStyle = glow;
    ctx.beginPath();
    ctx.arc(node.x, node.y, node.radius * 4, 0, Math.PI * 2);
    ctx.fill();

    ctx.fillStyle = "rgba(14,165,233,0.9)";
    ctx.beginPath();
    ctx.arc(node.x, node.y, node.radius, 0, Math.PI * 2);
    ctx.fill();
  });
}

function initSynapseField(canvas: HTMLCanvasElement) {
  const ctx = canvas.getContext("2d");
  if (!ctx) {
    throw new Error("Canvas context is not available");
  }

  let nodes = createNodes(canvas.clientWidth, canvas.clientHeight, NODE_COUNT);
  let edges = createEdges(nodes.length, LINKS_PER_NODE);
  let frame = 0;

  const resize = () => {
    const dpr = window.devicePixelRatio || 1;
    const { clientWidth, clientHeight } = canvas;
    canvas.width = clientWidth * dpr;
    canvas.height = clientHeight * dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    nodes = createNodes(clientWidth, clientHeight, NODE_COUNT);
    edges = createEdges(nodes.length, LINKS_PER_NODE);
  };

  resize();

  const render = () => {
    updateNodes(nodes, canvas.clientWidth, canvas.clientHeight);
    drawNetwork(ctx, nodes, edges, {
      width: canvas.clientWidth,
      height: canvas.clientHeight,
    });
    frame = requestAnimationFrame(render);
  };

  frame = requestAnimationFrame(render);
  window.addEventListener("resize", resize);

  return () => {
    cancelAnimationFrame(frame);
    window.removeEventListener("resize", resize);
  };
}

function ready(fn: () => void) {
  if (
    document.readyState === "complete" ||
    document.readyState === "interactive"
  ) {
    fn();
    return;
  }
  document.addEventListener("DOMContentLoaded", fn, { once: true });
}

ready(() => {
  const canvas = document.getElementById("synapse-canvas");
  if (!(canvas instanceof HTMLCanvasElement)) {
    console.error("synapse-canvas not found");
    return;
  }
  initSynapseField(canvas);
});
