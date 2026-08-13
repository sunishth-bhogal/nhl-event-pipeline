import { Kafka, logLevel } from "kafkajs";

const BROKERS = (process.env.KAFKA_BROKERS ?? "localhost:9092").split(",");

export const kafka = new Kafka({
  clientId: "nhl-producer",
  brokers: BROKERS,
  logLevel: logLevel.NOTHING,
  retry: { retries: 8, initialRetryTime: 500 },
});

export const kafkaProducer = kafka.producer({ allowAutoTopicCreation: true });
