export const TOPICS = {
  RAW_EVENTS: "nhl.events.raw",
} as const;

/** Kafka message key — partitioning by gameId keeps a given game's events on one
 * partition, which matters because Kafka only guarantees order within a partition. */
export function partitionKey(gameId: string): string {
  return gameId;
}
