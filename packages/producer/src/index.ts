import { randomUUID } from "node:crypto";
import { TOPICS, partitionKey, type NhlEvent, type EventEnvelope, type ControlState } from "@nhl/schema";
import { kafkaProducer } from "./kafka.js";
import { controlClient } from "./controlClient.js";
import { decideInjection } from "./failureLab.js";
import { replayEvents } from "./replaySource.js";
import { liveEvents } from "./liveSource.js";

function randomInt(min: number, max: number): number {
  return Math.floor(min + Math.random() * (max - min));
}

// Lazily (re)connects on first send rather than blocking process startup on
// Kafka being reachable — the control plane (and the UI it drives) should
// stay responsive even while the broker is still coming up.
let kafkaReady: Promise<void> | null = null;

function ensureKafkaConnected(): Promise<void> {
  if (!kafkaReady) {
    kafkaReady = kafkaProducer
      .connect()
      .then(() => console.log("[producer] connected to Kafka"))
      .catch((err) => {
        kafkaReady = null;
        throw err;
      });
  }
  return kafkaReady;
}

async function send(event: NhlEvent, injection: EventEnvelope["injection"]) {
  await ensureKafkaConnected();
  const envelope: EventEnvelope = { event, producedAt: Date.now(), injection };
  await kafkaProducer.send({
    topic: TOPICS.RAW_EVENTS,
    messages: [{ key: partitionKey(event.gameId), value: JSON.stringify(envelope) }],
  });
}

/** Applies the current Failure Lab decision to one event: publishes it
 * (immediately, or after an artificial delay to simulate a late arrival), and
 * separately schedules a duplicate publish when the dice say so. Drops never
 * touch Kafka at all — the consumer finds out about them indirectly, via a
 * gap in `sequenceNumber`. */
function publishWithInjection(event: NhlEvent) {
  const decision = decideInjection(controlClient.state.failureLab);

  if (decision.drop) {
    console.log(`[producer] FAILURE LAB: dropped ${event.eventType} (${event.eventId})`);
    return;
  }

  const publishOriginal = () =>
    send(event, {
      isDuplicate: false,
      isLate: decision.lateDelayMs > 0,
      artificialDelayMs: decision.lateDelayMs,
      isReordered: false,
    }).catch((err) => console.error("[producer] send failed", err));

  if (decision.lateDelayMs > 0) {
    console.log(
      `[producer] FAILURE LAB: delaying ${event.eventType} (${event.eventId}) by ${decision.lateDelayMs}ms`,
    );
    setTimeout(publishOriginal, decision.lateDelayMs);
  } else {
    void publishOriginal();
  }

  if (decision.duplicate) {
    const dupDelay = randomInt(200, 1500);
    console.log(`[producer] FAILURE LAB: duplicating ${event.eventType} (${event.eventId})`);
    setTimeout(() => {
      void send(event, { isDuplicate: true, isLate: false, artificialDelayMs: 0, isReordered: false }).catch(
        (err) => console.error("[producer] send failed", err),
      );
    }, dupDelay);
  }
}

/** Buffers events into windows and shuffles each window before publishing,
 * when reorder is enabled — simulates network-level reordering independent of
 * the duplicate/late/drop failures, which all still apply per-event on flush. */
class ReorderBuffer {
  private buffer: NhlEvent[] = [];

  push(event: NhlEvent, windowSize: number) {
    this.buffer.push(event);
    if (this.buffer.length >= windowSize) this.flush(true);
  }

  flush(shuffle: boolean) {
    const batch = this.buffer;
    this.buffer = [];
    if (shuffle) {
      for (let i = batch.length - 1; i > 0; i--) {
        const j = Math.floor(Math.random() * (i + 1));
        [batch[i], batch[j]] = [batch[j], batch[i]];
      }
    }
    for (const event of batch) publishWithInjection(event);
  }
}

const reorderBuffer = new ReorderBuffer();

function handleEvent(event: NhlEvent) {
  const { reorderEnabled, reorderWindow } = controlClient.state.failureLab;
  if (reorderEnabled) {
    reorderBuffer.push(event, reorderWindow);
  } else {
    reorderBuffer.flush(false); // flush any leftovers from when reorder was just turned off
    publishWithInjection(event);
  }
}

let activeAbort: AbortController | null = null;
let activeRunId = "";

async function runSource(state: ControlState) {
  activeAbort?.abort();
  const controller = new AbortController();
  activeAbort = controller;
  const runId = randomUUID();
  activeRunId = runId;

  if (state.replay.mode === "replay" && state.replay.fixture) {
    console.log(`[producer] starting REPLAY mode: ${state.replay.fixture} @ ${state.replay.speed}x`);
    try {
      for await (const event of replayEvents(state.replay.fixture, state.replay.speed, controller.signal)) {
        if (activeRunId !== runId) return;
        handleEvent(event);
      }
      console.log("[producer] replay finished");
    } catch (err) {
      console.error("[producer] replay error", err);
    }
  } else if (state.replay.mode === "live" && state.replay.gameId) {
    console.log(`[producer] starting LIVE mode: game ${state.replay.gameId}`);
    try {
      for await (const event of liveEvents(state.replay.gameId, controller.signal)) {
        if (activeRunId !== runId) return;
        handleEvent(event);
      }
    } catch (err) {
      console.error("[producer] live error", err);
    }
  } else {
    console.log("[producer] idle — waiting for replay/live mode to be selected in the control UI");
  }
}

function sourceKey(state: ControlState): string {
  return `${state.replay.mode}:${state.replay.fixture}:${state.replay.gameId}:${state.replay.speed}`;
}

async function main() {
  controlClient.connect();
  void ensureKafkaConnected().catch((err) =>
    console.error("[producer] could not connect to Kafka yet, will retry on next publish:", err.message),
  );

  // sentinel guarantees the first state push (control-api sends one immediately
  // on connect) always triggers a run, even if it happens to be the idle state.
  let lastKey: string | null = null;
  controlClient.on("state", (state: ControlState) => {
    const key = sourceKey(state);
    if (key !== lastKey) {
      lastKey = key;
      void runSource(state);
    }
  });
}

main().catch((err) => {
  console.error("[producer] fatal error", err);
  process.exit(1);
});

process.on("SIGINT", async () => {
  await kafkaProducer.disconnect().catch(() => {});
  process.exit(0);
});
