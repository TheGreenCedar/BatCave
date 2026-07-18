import type { RuntimeQuery, RuntimeSnapshot } from "./types";

export class AcceptedRuntimeControls {
  private publicationSeq = -1;
  private query: RuntimeQuery;
  private sampleIntervalMs: number;

  constructor(query: RuntimeQuery, sampleIntervalMs: number) {
    this.query = { ...query };
    this.sampleIntervalMs = sampleIntervalMs;
  }

  observe(snapshot: RuntimeSnapshot): void {
    if (snapshot.publication_seq < this.publicationSeq) return;
    this.publicationSeq = snapshot.publication_seq;
    this.query = { ...snapshot.settings.query };
    this.sampleIntervalMs = snapshot.settings.sample_interval_ms;
  }

  acceptedQuery(): RuntimeQuery {
    return { ...this.query };
  }

  acceptedSampleIntervalMs(): number {
    return this.sampleIntervalMs;
  }
}
