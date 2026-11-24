import { useEffect, useRef } from "react";
import type { RefObject } from "react";

/** Latest audio frame handed from the Tauri event listener to the canvas. */
export interface LevelFeed {
  level: number;
  bins: number[];
  /** performance.now() at the moment the event was received. */
  at: number;
}

interface VisualizerProps {
  feedRef: RefObject<LevelFeed | null>;
  /** Listening keeps the loop alive and pulls in fresh bins. */
  active: boolean;
}

const FALLBACK_BAR_COUNT = 64;
const SMOOTHING = 0.35;
const MIN_BAR_HEIGHT = 3;

function clamp01(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}

/** Pill-shaped bar with rounded caps, drawn bottom-up. */
function fillRoundedBar(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
): void {
  const radius = Math.min(width / 2, height / 2);
  ctx.beginPath();
  ctx.moveTo(x + radius, y);
  ctx.lineTo(x + width - radius, y);
  ctx.quadraticCurveTo(x + width, y, x + width, y + radius);
  ctx.lineTo(x + width, y + height - radius);
  ctx.quadraticCurveTo(x + width, y + height, x + width - radius, y + height);
  ctx.lineTo(x + radius, y + height);
  ctx.quadraticCurveTo(x, y + height, x, y + height - radius);
  ctx.lineTo(x, y + radius);
  ctx.quadraticCurveTo(x, y, x + radius, y);
  ctx.closePath();
  ctx.fill();
}

export function Visualizer({ feedRef, active }: VisualizerProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const activeRef = useRef(active);
  activeRef.current = active;

  // Values persist across effect restarts so "transcribing" can ease the bars
  // down from wherever the last listening frame left them.
  const barsRef = useRef<number[]>([]);
  const targetsRef = useRef<number[]>([]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    let disposed = false;
    let raf = 0;

    const measure = (): { width: number; height: number } => {
      // Layout pixels, not getBoundingClientRect: the entrance animation scales
      // the capsule, and the backing store must not follow that transform.
      const width = Math.max(1, canvas.clientWidth);
      const height = Math.max(1, canvas.clientHeight);
      const dpr = Math.min(3, Math.max(1, window.devicePixelRatio || 1));
      const pixelWidth = Math.round(width * dpr);
      const pixelHeight = Math.round(height * dpr);
      if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
        canvas.width = pixelWidth;
        canvas.height = pixelHeight;
      }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      return { width, height };
    };

    let size = measure();

    const ensureLength = (count: number): void => {
      const bars = barsRef.current;
      const targets = targetsRef.current;
      while (bars.length < count) bars.push(0);
      while (targets.length < count) targets.push(0);
      if (bars.length > count) bars.length = count;
      if (targets.length > count) targets.length = count;
    };

    /** Returns true while any bar is still meaningfully in motion. */
    const draw = (): boolean => {
      const { width, height } = size;
      ctx.clearRect(0, 0, width, height);

      const feed = feedRef.current;
      const listening = activeRef.current;
      const fresh = listening && feed !== null && performance.now() - feed.at < 180;
      const count = Math.min(96, Math.max(8, feed?.bins.length ?? FALLBACK_BAR_COUNT));

      ensureLength(count);
      const bars = barsRef.current;
      const targets = targetsRef.current;

      // A faint rail anchors the bars and keeps an empty state from floating.
      ctx.fillStyle = "rgba(245, 242, 234, 0.07)";
      ctx.fillRect(0, height - 1, width, 1);

      const slot = width / count;
      const barWidth = Math.max(1.5, slot * 0.62);
      const baseline = height - 1.5;
      const maxHeight = Math.max(2, height - 5);

      let moving = false;

      for (let index = 0; index < count; index += 1) {
        let target: number;
        if (listening && fresh && feed) {
          target = clamp01(feed.bins[index] ?? 0);
        } else {
          // Gentle decay when frames stop arriving or the mic closes.
          target = (targets[index] ?? 0) * 0.9;
        }
        targets[index] = target;

        const previous = bars[index] ?? 0;
        const current = previous + (target - previous) * SMOOTHING;
        bars[index] = current;
        if (current > 0.006) moving = true;

        const barHeight = Math.max(MIN_BAR_HEIGHT, current * maxHeight);
        const x = index * slot + (slot - barWidth) / 2;
        const y = baseline - barHeight;
        const alpha = 0.34 + 0.58 * Math.min(1, current * 1.35);

        ctx.fillStyle = `rgba(245, 242, 234, ${alpha.toFixed(3)})`;
        if (current > 0.42) {
          // Loud bands pick up the moss tint as a soft halo, not a colour swap.
          ctx.shadowColor = `rgba(118, 168, 134, ${(0.34 + current * 0.55).toFixed(3)})`;
          ctx.shadowBlur = 7 + current * 16;
        } else {
          ctx.shadowColor = "transparent";
          ctx.shadowBlur = 0;
        }
        fillRoundedBar(ctx, x, y, barWidth, barHeight);
      }

      ctx.shadowColor = "transparent";
      ctx.shadowBlur = 0;

      return moving;
    };

    const frame = (): void => {
      if (disposed) return;
      const moving = draw();
      if (activeRef.current || moving) {
        raf = requestAnimationFrame(frame);
      } else {
        raf = 0;
      }
    };

    const observer = new ResizeObserver(() => {
      if (disposed) return;
      size = measure();
      draw();
    });
    observer.observe(canvas);

    frame();

    return () => {
      disposed = true;
      observer.disconnect();
      if (raf !== 0) {
        cancelAnimationFrame(raf);
        raf = 0;
      }
    };
  }, [active, feedRef]);

  return <canvas ref={canvasRef} className="overlay-visualizer" aria-hidden="true" />;
}
