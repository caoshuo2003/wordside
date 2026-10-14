// Cache complete dictionary JSON in memory only. Restarting clears it.
class MemoryCache {
  constructor({ ttlMs = 30 * 60 * 1000, maxEntries = 1000, maxBytes = 32 * 1024 * 1024, now = Date.now } = {}) {
    this.ttlMs = ttlMs;
    this.maxEntries = maxEntries;
    this.maxBytes = maxBytes;
    this.now = now;
    this.entries = new Map();
    this.bytes = 0;
  }
  remove(key) {
    const value = this.entries.get(key);
    if (value) { this.bytes -= value.bytes; this.entries.delete(key); }
  }
  get(key) {
    const value = this.entries.get(key);
    if (!value) return undefined;
    if (this.now() >= value.expires) { this.remove(key); return undefined; }
    this.entries.delete(key);
    this.entries.set(key, value);
    return JSON.parse(value.json);
  }
  set(key, result) {
    const json = JSON.stringify(result);
    const bytes = Buffer.byteLength(json);
    this.remove(key);
    if (bytes > this.maxBytes || this.maxEntries <= 0 || this.ttlMs <= 0) return;
    const now = this.now();
    for (const [entry, value] of this.entries) if (now >= value.expires) this.remove(entry);
    while (this.entries.size >= this.maxEntries || this.bytes + bytes > this.maxBytes) {
      this.remove(this.entries.keys().next().value);
    }
    this.entries.set(key, { json, bytes, expires: now + this.ttlMs });
    this.bytes += bytes;
  }
  clear() { this.entries.clear(); this.bytes = 0; }
}
module.exports = { MemoryCache };
