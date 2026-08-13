import { z } from "zod";

/**
 * Failure Lab toggles. These live on the control-api and get pushed to the
 * producer over a WebSocket; the producer is the injection point because
 * that's the one place in the pipeline that can still choose to duplicate,
 * delay, drop, or reorder a publish before it becomes "real" on the wire.
 */
export const FailureLabStateSchema = z.object({
  duplicateRate: z.number().min(0).max(1).default(0),
  dropRate: z.number().min(0).max(1).default(0),
  lateRate: z.number().min(0).max(1).default(0),
  lateDelayMsMin: z.number().int().nonnegative().default(2000),
  lateDelayMsMax: z.number().int().nonnegative().default(8000),
  reorderEnabled: z.boolean().default(false),
  reorderWindow: z.number().int().min(2).max(10).default(4),
});
export type FailureLabState = z.infer<typeof FailureLabStateSchema>;

export const ReplayStateSchema = z.object({
  mode: z.enum(["idle", "live", "replay"]).default("idle"),
  fixture: z.string().nullable().default(null),
  speed: z.number().positive().default(1),
  gameId: z.string().nullable().default(null),
});
export type ReplayState = z.infer<typeof ReplayStateSchema>;

export const ControlStateSchema = z.object({
  failureLab: FailureLabStateSchema,
  replay: ReplayStateSchema,
});
export type ControlState = z.infer<typeof ControlStateSchema>;

export function defaultControlState(): ControlState {
  return {
    failureLab: FailureLabStateSchema.parse({}),
    replay: ReplayStateSchema.parse({}),
  };
}
