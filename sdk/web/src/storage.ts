import type { BehaviorRecord } from "./types.js";

const STORAGE_KEY = "chill.buffer.v1";
export class DurableBuffer {
  private records: BehaviorRecord[];
  constructor(private readonly storage: Storage | undefined, private readonly maximum: number) {
    try { const value = storage?.getItem(STORAGE_KEY); this.records = value ? JSON.parse(value) as BehaviorRecord[] : []; }
    catch { this.records = []; }
  }
  push(record: BehaviorRecord): void { this.records.push(record); if (this.records.length > this.maximum) this.records.splice(0, this.records.length - this.maximum); this.persist(); }
  peek(count: number): readonly BehaviorRecord[] { return this.records.slice(0, count); }
  remove(count: number): void { this.records.splice(0, count); this.persist(); }
  clear(): void { this.records = []; this.persist(); }
  get length(): number { return this.records.length; }
  private persist(): void { try { this.storage?.setItem(STORAGE_KEY, JSON.stringify(this.records)); } catch { /* Memory remains bounded when storage is unavailable. */ } }
}
