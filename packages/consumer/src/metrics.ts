import type { MetricsSnapshot } from "@nhl/schema";

const LATENCY_SAMPLE_CAP = 500;
const RATE_WINDOW_MS = 5000;

function percentile(sorted: number[], p: number): number {
  if (sorted.length === 0) return 0;
  const idx = Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length));
  return sorted[idx];
}

/** Tracks the counters the Failure Lab exists to make visible, plus real
 * end-to-end latency (consumer processedAt − producer producedAt, both actual
 * wall-clock epoch ms) sampled from live traffic rather than assumed. */
export class MetricsTracker {
  private totalIngested = 0;
  private duplicatesDropped = 0;
  private lateRecovered = 0;
  private lateDiscarded = 0;
  private gapsDetected = 0;
  private latencySamples: number[] = [];
  private recentTimestamps: number[] = [];
  private watermarkLagProvider: () => number = () => 0;

  setWatermarkLagProvider(fn: () => number) {
    this.watermarkLagProvider = fn;
  }

  recordIngested(latencyMs: number) {
    this.totalIngested++;
    this.recentTimestamps.push(Date.now());
    this.latencySamples.push(latencyMs);
    if (this.latencySamples.length > LATENCY_SAMPLE_CAP) this.latencySamples.shift();
  }

  recordDuplicate() {
    this.duplicatesDropped++;
  }

  recordLateRecovered() {
    this.lateRecovered++;
  }

  recordLateDiscarded() {
    this.lateDiscarded++;
  }

  recordGapDetected() {
    this.gapsDetected++;
  }

  snapshot(): MetricsSnapshot {
    const now = Date.now();
    this.recentTimestamps = this.recentTimestamps.filter((t) => now - t <= RATE_WINDOW_MS);
    const sorted = [...this.latencySamples].sort((a, b) => a - b);
    return {
      totalIngested: this.totalIngested,
      duplicatesDropped: this.duplicatesDropped,
      lateRecovered: this.lateRecovered,
      lateDiscarded: this.lateDiscarded,
      gapsDetected: this.gapsDetected,
      eventsPerSec: this.recentTimestamps.length / (RATE_WINDOW_MS / 1000),
      latencyP50Ms: percentile(sorted, 50),
      latencyP95Ms: percentile(sorted, 95),
      watermarkLagMs: this.watermarkLagProvider(),
      updatedAt: now,
    };
  }

  reset() {
    this.totalIngested = 0;
    this.duplicatesDropped = 0;
    this.lateRecovered = 0;
    this.lateDiscarded = 0;
    this.gapsDetected = 0;
    this.latencySamples = [];
    this.recentTimestamps = [];
  }
}

export const metrics = new MetricsTracker();
