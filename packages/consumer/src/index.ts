import { Kafka, logLevel } from "kafkajs";
import { TOPICS, parseEnvelope } from "@nhl/schema";
import { processEnvelope, oldestPendingAgeMs } from "./watermark.js";
import { metrics } from "./metrics.js";
import { startServer } from "./server.js";

const BROKERS = (process.env.KAFKA_BROKERS ?? "localhost:9092").split(",");

const kafka = new Kafka({
  clientId: "nhl-consumer",
  brokers: BROKERS,
  logLevel: logLevel.NOTHING,
  retry: { retries: 8, initialRetryTime: 500 },
});

const consumer = kafka.consumer({ groupId: "nhl-pipeline-consumer" });

/** Idempotent — safe to call every startup. Ensures the topic exists before we
 * subscribe, so a fresh `docker compose up` + `npm run dev` doesn't race the
 * consumer against topic auto-creation (which only fires on a producer
 * request, not a consumer's metadata fetch). */
async function ensureTopic() {
  const admin = kafka.admin();
  await admin.connect();
  try {
    await admin.createTopics({
      topics: [{ topic: TOPICS.RAW_EVENTS, numPartitions: 3, replicationFactor: 1 }],
    });
  } finally {
    await admin.disconnect();
  }
}

async function startConsuming() {
  await ensureTopic();
  await consumer.connect();
  console.log("[consumer] connected to Kafka");
  await consumer.subscribe({ topic: TOPICS.RAW_EVENTS, fromBeginning: true });

  await consumer.run({
    eachMessage: async ({ message }) => {
      if (!message.value) return;
      try {
        const envelope = parseEnvelope(JSON.parse(message.value.toString()));
        processEnvelope(envelope);
      } catch (err) {
        console.error("[consumer] failed to process message", err);
      }
    },
  });
}

async function main() {
  metrics.setWatermarkLagProvider(oldestPendingAgeMs);
  startServer();

  // Retries rather than crashing on startup errors (e.g. broker still coming
  // up, topic metadata not yet propagated) — the observability API above is
  // already serving, so the dashboard stays responsive while this retries.
  for (;;) {
    try {
      await startConsuming();
      return;
    } catch (err) {
      console.error("[consumer] startup failed, retrying in 3s:", (err as Error).message);
      await new Promise((resolve) => setTimeout(resolve, 3000));
    }
  }
}

main().catch((err) => {
  console.error("[consumer] fatal error", err);
  process.exit(1);
});

process.on("SIGINT", async () => {
  await consumer.disconnect().catch(() => {});
  process.exit(0);
});
