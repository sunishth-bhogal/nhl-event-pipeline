import { EventEmitter } from "node:events";
import {
  type ControlState,
  type FailureLabState,
  type ReplayState,
  defaultControlState,
} from "@nhl/schema";

/** Single in-memory source of truth for the control plane. Fine for a
 * single-instance dev/demo control-api; a real deployment would back this
 * with something durable (etcd/Consul/a Kafka compacted topic) so multiple
 * control-api replicas agree. */
class ControlStore extends EventEmitter {
  private state: ControlState = defaultControlState();

  get(): ControlState {
    return this.state;
  }

  patchFailureLab(patch: Partial<FailureLabState>): ControlState {
    this.state = {
      ...this.state,
      failureLab: { ...this.state.failureLab, ...patch },
    };
    this.emit("change", this.state);
    return this.state;
  }

  patchReplay(patch: Partial<ReplayState>): ControlState {
    this.state = {
      ...this.state,
      replay: { ...this.state.replay, ...patch },
    };
    this.emit("change", this.state);
    return this.state;
  }
}

export const controlStore = new ControlStore();
