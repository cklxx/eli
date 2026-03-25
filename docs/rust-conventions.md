# Rust Coding Conventions

> All new code and refactors must comply. Agents (Codex / Claude) read this before writing code.

---

## 1. Type System / Trait Design

### 1.1 Typestate — Compile-Time State Machines

APIs with ordering constraints encode states via PhantomData. Invalid transitions = compile error, zero runtime cost.

```rust
use std::marker::PhantomData;

struct Draft;
struct Published;
struct Archived;

struct Article<S> {
    title: String,
    body: String,
    _state: PhantomData<S>,
}

impl Article<Draft> {
    fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Article { title: title.into(), body: body.into(), _state: PhantomData }
    }

    fn publish(self) -> Article<Published> {
        Article { title: self.title, body: self.body, _state: PhantomData }
    }
}

impl Article<Published> {
    fn archive(self) -> Article<Archived> {
        Article { title: self.title, body: self.body, _state: PhantomData }
    }
}

// Article<Archived>::publish() doesn't exist → caught at compile time
```

**Use when**: states ≤ 4 and transition graph is linear/tree.
**Don't use when**: states > 4 or cyclic transitions → fall back to enum + match.

### 1.2 Extension Traits — Adding Methods to Foreign Types

Project-level methods on std/external types use extension traits, not free functions.

```rust
trait StrExt {
    fn is_blank(&self) -> bool;
    fn truncate_to(&self, max: usize) -> &str;
}

impl StrExt for str {
    fn is_blank(&self) -> bool {
        self.trim().is_empty()
    }

    fn truncate_to(&self, max: usize) -> &str {
        match self.char_indices().nth(max) {
            Some((idx, _)) => &self[..idx],
            None => self,
        }
    }
}

// Call: "hello world".truncate_to(5)
// Not:  truncate_str("hello world", 5)
```

**Naming**: `{Type}Ext` — `ValueExt`, `StrExt`, `SliceExt`.

### 1.3 Sealed Traits — Controlling Implementation Rights

Publicly usable but externally unimplementable. Prevents downstream crates from adding unexpected impls.

```rust
mod sealed {
    pub trait Sealed {}
}

pub trait Plugin: sealed::Sealed {
    fn name(&self) -> &str;
    fn execute(&self);
}

pub struct InternalPlugin;
impl sealed::Sealed for InternalPlugin {}
impl Plugin for InternalPlugin {
    fn name(&self) -> &str { "internal" }
    fn execute(&self) { /* ... */ }
}

// External crates can't impl Sealed → can't impl Plugin
```

**Use for**: framework-level traits not intended for user implementation.

### 1.4 Newtypes — Eliminating Primitive Obsession

Same-type values with different semantics must use newtype wrappers.

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChannelId(pub String);

impl From<String> for SessionId {
    fn from(s: String) -> Self { Self(s) }
}

impl AsRef<str> for SessionId {
    fn as_ref(&self) -> &str { &self.0 }
}

// Compiler prevents mix-ups:
// fn route(session: SessionId, channel: ChannelId)
// route(channel_id, session_id) → compile error
```

---

## 2. Error Handling

### 2.1 Two-Layer Model

| Layer | Tool | Purpose |
|-------|------|---------|
| Library | `thiserror` | Structured enum, callers can match |
| Application | `anyhow` | Flexible context, fast bail |

Don't merge them. Libraries expose precise errors; applications wrap with anyhow.

```rust
// Library: precise
#[derive(Debug, thiserror::Error)]
enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(#[from] std::num::ParseIntError),
    #[error("validation: {0}")]
    Validation(String),
}

// Application: flexible
fn main() -> anyhow::Result<()> {
    let port = load_port("config.txt")?;  // ConfigError → anyhow auto-converts
    Ok(())
}
```

### 2.2 No Bare unwrap in Production

Production path `unwrap()` count = 0. Only allowed in:
1. Test code (`#[cfg(test)]`)
2. Provably infallible → `expect("SAFETY: reason")`
3. Program init config validation (startup failure = acceptable)

| Scenario | Replacement |
|----------|-------------|
| `option.unwrap()` | `.ok_or_else(\|\| Error::...)?` |
| `result.unwrap()` | `result?` or `.map_err(...)?` |
| `map.get(k).unwrap()` | `.ok_or(Error::NotFound(k))?` |
| Provably safe | `expect("SAFETY: regex is a static literal")` |

```rust
// ✅
fn load_port(path: &str) -> Result<u16, ConfigError> {
    let content = std::fs::read_to_string(path)?;
    let port: u16 = content.trim().parse()?;
    if port < 1024 {
        return Err(ConfigError::Validation(format!("port {port} < 1024")));
    }
    Ok(port)
}

// ❌
fn load_port(path: &str) -> u16 {
    std::fs::read_to_string(path).unwrap().trim().parse().unwrap()
}
```

### 2.3 Combinators First — Result as Monad

Prefer `and_then` / `map_err` / `or_else` over nested match.

```rust
// ✅ Flat
std::env::var(key)
    .map_err(|_| format!("env {key} not set"))
    .and_then(|v| if v.is_empty() { Err(format!("{key} empty")) } else { Ok(v) })

// ❌ Nested
match std::env::var(key) {
    Ok(v) => {
        if v.is_empty() { Err(format!("{key} empty")) }
        else { Ok(v) }
    }
    Err(_) => Err(format!("env {key} not set")),
}
```

### 2.4 Option → Result Bridge

`ok_or_else()` for lazy error construction, `ok_or()` for zero-cost.

```rust
users.iter()
    .find(|u| u.role == "admin")
    .map(|u| u.name.as_str())
    .ok_or_else(|| anyhow::anyhow!("no admin user"))
```

---

## 3. Iterators / Functional Composition

### 3.1 No Hand-Written Index Loops

`for i in 0..vec.len()` with `vec[i]` → iterator.

```rust
// ✅ windows
data.windows(3)
    .map(|w| w.iter().sum::<f64>() / 3.0)
    .collect::<Vec<_>>()

// ❌ index
for i in 0..data.len() - 2 {
    result.push((data[i] + data[i+1] + data[i+2]) / 3.0);
}
```

### 3.2 flat_map to Flatten Nesting

Nested loops → `flat_map` + `enumerate`.

```rust
// ✅
matrix.iter().enumerate()
    .flat_map(|(r, row)| {
        row.iter().enumerate().map(move |(c, &val)| (r, c, val))
    })
    .find(|&(_, _, val)| val == target)
    .map(|(r, c, _)| (r, c))

// ❌
for r in 0..matrix.len() {
    for c in 0..matrix[r].len() {
        if matrix[r][c] == target { return Some((r, c)); }
    }
}
```

### 3.3 scan — Stateful Map

Use `scan` when accumulating state, not a mut accumulator.

```rust
// Prefix sums
let sums: Vec<i64> = nums.iter()
    .scan(0i64, |acc, &x| { *acc += x; Some(*acc) })
    .collect();
```

### 3.4 chunks for Batch Processing

```rust
ids.chunks(batch_size)
    .map(|batch| {
        let values = batch.iter().map(|id| format!("({id})")).collect::<Vec<_>>().join(", ");
        format!("INSERT INTO tasks (id) VALUES {values};")
    })
    .collect::<Vec<_>>()
```

### 3.5 Custom Iterators

Lazy sequences implement `Iterator`. Use `checked_add` / `checked_mul` to auto-terminate on overflow.

```rust
struct Fib(u64, u64);

impl Fib {
    fn new() -> Self { Fib(0, 1) }
}

impl Iterator for Fib {
    type Item = u64;
    fn next(&mut self) -> Option<u64> {
        let val = self.0;
        let new = self.0.checked_add(self.1)?;  // overflow → None → iteration stops
        self.0 = self.1;
        self.1 = new;
        Some(val)
    }
}

// Fib::new().take(20).filter(|n| n % 2 == 0).sum::<u64>()
```

---

## 4. Concurrency / Async Patterns

### 4.1 select! + Timeout — All External IO Must Have Timeouts

```rust
use std::time::Duration;

async fn fetch_with_timeout(url: &str, timeout_ms: u64) -> Result<String, String> {
    tokio::select! {
        result = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok::<_, String>(format!("response from {url}"))
        } => result,
        _ = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {
            Err(format!("timeout after {timeout_ms}ms"))
        }
    }
}
```

### 4.2 Fan-out / Fan-in — JoinSet Over Vec<JoinHandle>

```rust
// ✅ JoinSet manages automatically
let mut set = tokio::task::JoinSet::new();
for url in urls {
    set.spawn(async move { fetch(&url, 3000).await });
}
let mut results = Vec::with_capacity(set.len());
while let Some(result) = set.join_next().await {
    results.push(result.unwrap_or_else(|e| Err(format!("join: {e}"))));
}

// ❌ Manual handle collection
let handles: Vec<_> = urls.iter().map(|u| tokio::spawn(fetch(u))).collect();
```

### 4.3 Semaphore for Concurrency Control

Rate-limit external API calls with `Semaphore`.

```rust
use std::sync::Arc;
use tokio::sync::Semaphore;

async fn rate_limited_fetch(urls: Vec<String>, max_concurrent: usize) {
    let sem = Arc::new(Semaphore::new(max_concurrent));
    let mut handles = Vec::new();

    for url in urls {
        let permit = sem.clone().acquire_owned().await.unwrap();
        handles.push(tokio::spawn(async move {
            let _permit = permit;  // released on drop
            fetch_with_timeout(&url, 3000).await
        }));
    }

    for h in handles {
        let _ = h.await;
    }
}
```

### 4.4 Minimal Mutex Hold — No IO Under Lock

Lock only for read + clone; compute and IO outside the lock. Never `.await` inside `std::sync::Mutex`.

```rust
// ✅ Clone out, then process
let existing = {
    let guard = cache.lock().await;
    guard.get(&key).cloned()
};  // guard dropped, lock released

match existing {
    Some(v) => v,
    None => {
        let value = compute_expensive(&key).await;  // outside lock
        let mut guard = cache.lock().await;
        guard.entry(key).or_insert(value).clone()
    }
}

// ❌ IO under lock
let mut guard = cache.lock().await;
let value = fetch_from_api(&key).await;  // everyone waits
guard.insert(key, value);
```

### 4.5 Bounded Channels First

`mpsc::channel(capacity)` preferred. Unbounded only when production rate is known and finite.

```rust
let (tx_raw, mut rx_raw) = tokio::sync::mpsc::channel::<i32>(64);
let (tx_out, mut rx_out) = tokio::sync::mpsc::channel::<String>(64);

// Producer
tokio::spawn(async move {
    for i in 0..100 { let _ = tx_raw.send(i).await; }
});

// Transformer
tokio::spawn(async move {
    while let Some(n) = rx_raw.recv().await {
        let _ = tx_out.send(format!("item-{}", n * 2)).await;
    }
});

// Consumer
while let Some(item) = rx_out.recv().await {
    println!("{item}");
}
```

### 4.6 CancellationToken Over abort

Cooperative cancellation preferred. `abort()` only when no cleanup is needed.

```rust
// ✅ Cooperative
tokio::select! {
    _ = do_work() => {},
    _ = token.cancelled() => { cleanup().await; }
}

// ⚠️ Force cancel, may lose state
handle.abort();
```

---

## 5. Code Quality

### 5.1 clone() Audit

Every `.clone()` must answer: **why can't a reference work?**

| Acceptable | Not acceptable |
|------------|----------------|
| `Arc::clone()` — zero-copy refcount | `String` clone just to satisfy borrow checker → fix lifetimes |
| Cross-`spawn` needs `'static` | `Vec<T>` cloned then read-only → use `&[T]` |
| Small `serde_json::Value` | Large struct cloned for one field → extract field first |

### 5.2 Naming

| Type | Rule | Example |
|------|------|---------|
| Trait | Adjective or capability | `Streamable`, `Retryable` |
| Enum variant | Noun | `ErrorKind::NotFound` |
| Builder method | Field name | `.api_key()`, `.model()` |
| Bool-returning method | `is_` / `has_` / `can_` | `.is_empty()`, `.has_tools()` |
| Conversion method | `into_` / `as_` / `to_` | `.into_inner()`, `.as_str()`, `.to_string()` |

### 5.3 Documentation

- Public API: `///` required
- `unsafe` / `expect`: `// SAFETY:` comment
- Complex algorithms: link to design doc

### 5.4 Build Verification

After each logical unit, verify immediately:

```bash
cargo check --workspace
cargo clippy --workspace -- -D warnings
cargo test --workspace
cargo fmt --all -- --check
```

All must pass before moving to the next module.
