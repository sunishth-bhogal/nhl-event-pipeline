import type { FailureLabState } from "@nhl/schema";

export interface InjectionDecision {
  drop: boolean;
  duplicate: boolean;
  lateDelayMs: number;
}

function randomInt(min: number, max: number): number {
  return Math.floor(min + Math.random() * (max - min));
}

/** Pure decision function so it's trivially unit-testable and so the Failure
 * Lab's behavior doesn't depend on Kafka/network state at all. */
export function decideInjection(state: FailureLabState): InjectionDecision {
  const drop = Math.random() < state.dropRate;
  const duplicate = !drop && Math.random() < state.duplicateRate;
  const isLate = !drop && Math.random() < state.lateRate;
  const lateDelayMs = isLate ? randomInt(state.lateDelayMsMin, state.lateDelayMsMax) : 0;
  return { drop, duplicate, lateDelayMs };
}
