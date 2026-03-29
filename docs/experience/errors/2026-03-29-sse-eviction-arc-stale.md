# 2026-03-29 · SSE client eviction didn't work — Arc held old pool

## Context
SSE stream errors recurring despite a "fix" in 2857ce5 that added
`client_registry.remove()` when an SSE stream body error was detected.
Error appeared 4 times before root cause was found.

## Root Cause
`build_candidate()` was called **once before** the `for attempt` loop:

```rust
for (provider, model) in &candidates {
    let candidate = self.build_candidate(provider, model);  // Arc<Client> captured once

    for attempt in self.retry_attempts() {
        // ... SSE stream error detected
        self.client_registry.remove(provider, key, base);  // removes from HashMap
        continue;  // retry — but candidate.client still points to old pool!
    }
}
```

`ClientRegistry::remove()` deletes the HashMap entry, but `candidate.client`
holds an `Arc<Client>` to the old pool. The Arc keeps the old client alive
and all retries use the same stale connection.

## Fix
Move `build_candidate()` inside the attempt loop:

```rust
for (provider, model) in &candidates {
    for attempt in self.retry_attempts() {
        let candidate = self.build_candidate(provider, model);  // fresh lookup each time
        // after eviction: get_or_create() creates a new client
        // normal case: get_or_create() returns the cached client (no overhead)
    }
}
```

Also extracted `SSE_STREAM_ERROR_PREFIX` as a `pub(crate) const` so the
error construction and detection can't silently diverge.

## Rule
When using a cache with `Arc` values: evicting from the cache does NOT
invalidate existing `Arc` clones. If the caller holds an `Arc` across
a retry loop, they'll see the old value regardless of eviction. Always
re-fetch from the cache inside the retry loop.
