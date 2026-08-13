import { z } from "zod";

export const MetricsSnapshotSchema = z.object({
  totalIngested: z.number().int().nonnegative(),
  duplicatesDropped: z.number().int().nonnegative(),
  lateRecovered: z.number().int().nonnegative(),
  lateDiscarded: z.number().int().nonnegative(),
  gapsDetected: z.number().int().nonnegative(),
  eventsPerSec: z.number().nonnegative(),
  latencyP50Ms: z.number().nonnegative(),
  latencyP95Ms: z.number().nonnegative(),
  watermarkLagMs: z.number().nonnegative(),
  updatedAt: z.number().int(),
});
export type MetricsSnapshot = z.infer<typeof MetricsSnapshotSchema>;

export function emptyMetrics(): MetricsSnapshot {
  return {
    totalIngested: 0,
    duplicatesDropped: 0,
    lateRecovered: 0,
    lateDiscarded: 0,
    gapsDetected: 0,
    eventsPerSec: 0,
    latencyP50Ms: 0,
    latencyP95Ms: 0,
    watermarkLagMs: 0,
    updatedAt: Date.now(),
  };
}

/** Live ticker entries the consumer streams to the UI — the visible proof that
 * failure handling actually did something, not just a counter going up. */
export const ActivityKindSchema = z.enum([
  "ingested",
  "duplicate-suppressed",
  "late-recovered",
  "late-discarded",
  "gap-detected",
]);
export type ActivityKind = z.infer<typeof ActivityKindSchema>;

export const ActivityEntrySchema = z.object({
  id: z.string(),
  kind: ActivityKindSchema,
  gameId: z.string(),
  eventId: z.string(),
  eventType: z.string(),
  description: z.string(),
  gameClockMs: z.number(),
  processedAt: z.number(),
  lateByMs: z.number().optional(),
});
export type ActivityEntry = z.infer<typeof ActivityEntrySchema>;
