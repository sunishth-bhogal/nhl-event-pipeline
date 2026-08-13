import { z } from "zod";

export const EventTypeSchema = z.enum([
  "goal",
  "shot-on-goal",
  "missed-shot",
  "blocked-shot",
  "penalty",
  "delayed-penalty",
  "faceoff",
  "hit",
  "giveaway",
  "takeaway",
  "period-start",
  "period-end",
  "stoppage",
  "game-end",
]);
export type EventType = z.infer<typeof EventTypeSchema>;

export const PlayerRefSchema = z.object({
  id: z.number(),
  name: z.string(),
  role: z.string(), // e.g. "scorer", "assist", "goalie", "hitter", "hittee"
});
export type PlayerRef = z.infer<typeof PlayerRefSchema>;

/**
 * The canonical, domain-level NHL event. `gameClockMs` is EVENT TIME (see
 * gameClock.ts) — the moment the play actually happened in the game. This is
 * distinct from any processing-time field, which only ever lives on the
 * envelope, never here, so the two concepts can't be accidentally conflated.
 */
export const NhlEventSchema = z.object({
  eventId: z.string().uuid(),
  gameId: z.string(),
  sequenceNumber: z.number().int().nonnegative(),
  eventType: EventTypeSchema,
  period: z.number().int().min(1),
  periodTimeRemaining: z.string(),
  gameClockMs: z.number().int().nonnegative(),
  team: z.object({ id: z.number(), abbrev: z.string() }).nullable(),
  players: z.array(PlayerRefSchema).default([]),
  description: z.string(),
  details: z.record(z.string(), z.unknown()).default({}),
  source: z.enum(["live", "replay"]),
});
export type NhlEvent = z.infer<typeof NhlEventSchema>;

/**
 * What actually goes on the wire. `producedAt` is PROCESSING TIME — the
 * wall-clock instant the producer published this record, which is what the
 * consumer uses to measure end-to-end latency and to detect lateness relative
 * to event time. `injection` is metadata the Failure Lab stamps on so the
 * consumer/UI can label what was artificially done to this publish, without
 * that ever leaking into the domain event itself.
 */
export const EventEnvelopeSchema = z.object({
  event: NhlEventSchema,
  producedAt: z.number().int(), // epoch ms
  injection: z
    .object({
      isDuplicate: z.boolean().default(false),
      isLate: z.boolean().default(false),
      artificialDelayMs: z.number().int().nonnegative().default(0),
      isReordered: z.boolean().default(false),
    })
    .default({}),
});
export type EventEnvelope = z.infer<typeof EventEnvelopeSchema>;

export function parseEnvelope(raw: unknown): EventEnvelope {
  return EventEnvelopeSchema.parse(raw);
}
