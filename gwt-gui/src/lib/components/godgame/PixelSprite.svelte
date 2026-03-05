<script lang="ts">
  import type { SpriteData } from './sprites';

  interface Props {
    sprite: SpriteData;
    scale?: number;
    animate?: boolean;
    frameIndex?: number;
    fps?: number;
    class?: string;
  }

  let {
    sprite,
    scale = 2,
    animate = false,
    frameIndex = 0,
    fps = 4,
    class: className = '',
  }: Props = $props();

  let canvas: HTMLCanvasElement | undefined = $state();
  let currentFrame = $state(0);
  let animFrameId = $state(0);

  function drawFrame(ctx: CanvasRenderingContext2D, frame: number[][]) {
    ctx.clearRect(0, 0, sprite.width * scale, sprite.height * scale);
    for (let y = 0; y < frame.length; y++) {
      for (let x = 0; x < frame[y].length; x++) {
        const colorIdx = frame[y][x];
        if (colorIdx === 0) continue;
        const color = sprite.palette[colorIdx];
        if (!color) continue;
        ctx.fillStyle = color;
        ctx.fillRect(x * scale, y * scale, scale, scale);
      }
    }
  }

  $effect(() => {
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const frames = sprite.frames;
    if (frames.length === 0) return;

    if (!animate || frames.length <= 1) {
      const idx = Math.min(frameIndex, frames.length - 1);
      drawFrame(ctx, frames[idx]);
      return;
    }

    // Animated mode
    let lastTime = 0;
    const interval = 1000 / fps;
    let frame = currentFrame % frames.length;

    function tick(time: number) {
      if (!ctx) return;
      if (time - lastTime >= interval) {
        lastTime = time;
        frame = (frame + 1) % frames.length;
        currentFrame = frame;
        drawFrame(ctx, frames[frame]);
      }
      animFrameId = requestAnimationFrame(tick);
    }

    drawFrame(ctx, frames[frame]);
    animFrameId = requestAnimationFrame(tick);

    return () => {
      if (animFrameId) cancelAnimationFrame(animFrameId);
    };
  });
</script>

<canvas
  bind:this={canvas}
  class="pixel-sprite {className}"
  width={sprite.width * scale}
  height={sprite.height * scale}
  aria-hidden="true"
></canvas>

<style>
  .pixel-sprite {
    image-rendering: pixelated;
    display: block;
  }
</style>
