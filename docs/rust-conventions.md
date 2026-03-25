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

## 5. Structural Design

### 5.1 Small Functions + Single Responsibility

Each function does one thing. The top-level flow reads like a script.

```rust
// ✅ Main flow reads like a recipe
fn process(path: &str) -> Result<()> {
    let raw = load(path)?;
    let parsed = parse(raw)?;
    let validated = validate(parsed)?;
    save(validated)?;
    Ok(())
}

// Each step is independently testable, composable, and replaceable.
fn load(path: &str) -> Result<Vec<u8>> { /* one job: read bytes */ }
fn parse(raw: Vec<u8>) -> Result<Config> { /* one job: deserialize */ }
fn validate(cfg: Config) -> Result<Config> { /* one job: check invariants */ }
fn save(cfg: Config) -> Result<()> { /* one job: persist */ }

// ❌ God function — loads, parses, validates, transforms, saves, logs, all in one
fn do_everything(path: &str) -> Result<()> {
    let bytes = std::fs::read(path)?;
    let cfg: Config = serde_json::from_slice(&bytes)?;
    if cfg.port < 1024 { return Err(anyhow!("bad port")); }
    let transformed = /* 50 lines of logic */;
    std::fs::write("out.json", serde_json::to_vec(&transformed)?)?;
    tracing::info!("done");
    Ok(())
}
```

**Rule**: If a function has more than one reason to change, split it.

### 5.2 Parse, Don't Validate

Validation returns a bool — the knowledge lives only in your head. Parsing transforms input into a stricter type — the compiler remembers for you.

```rust
// ❌ Validate: caller must remember to check every time
fn send_email(addr: &str) -> Result<()> {
    if !addr.contains('@') { return Err(anyhow!("invalid email")); }
    // ... every function that takes an email repeats this check
}

// ✅ Parse: validity encoded in the type, checked once at the boundary
struct Email(String);

impl Email {
    fn parse(input: &str) -> Result<Self> {
        if input.contains('@') && input.len() > 3 {
            Ok(Email(input.to_owned()))
        } else {
            Err(anyhow!("invalid email: {input}"))
        }
    }

    fn as_str(&self) -> &str { &self.0 }
}

fn send_email(addr: &Email) -> Result<()> {
    // addr is always valid — no runtime check needed
}
```

**Rule**: Parse at the system boundary (user input, API response, file read). Inner functions accept parsed types, never raw strings.

### 5.3 Make Illegal States Unrepresentable

Use enums and type privacy so invalid states cannot be constructed.

```rust
// ❌ Optional fields → runtime checks everywhere
struct Connection {
    state: String,           // "connecting", "connected", "closed"
    socket: Option<Socket>,  // only Some when connected — but compiler doesn't know
    error: Option<String>,   // only Some when closed — but compiler doesn't know
}

// ✅ Enum makes invalid states impossible
enum Connection {
    Connecting { addr: SocketAddr },
    Connected { socket: Socket },
    Closed { reason: String },
}

// You can't have a socket in the Connecting state or an error in Connected.
// Pattern matching forces you to handle every state.
impl Connection {
    fn send(&self, data: &[u8]) -> Result<()> {
        match self {
            Connection::Connected { socket } => socket.write(data),
            Connection::Connecting { .. } => Err(anyhow!("not yet connected")),
            Connection::Closed { reason } => Err(anyhow!("closed: {reason}")),
        }
    }
}
```

**Rule**: If two fields are mutually exclusive, they belong in separate enum variants, not as `Option`s on the same struct.

### 5.4 Module Privacy as API Boundary

Keep constructors private. Expose only validated factory functions.

```rust
mod config {
    pub struct Port(u16);  // field is private — can't construct from outside

    impl Port {
        pub fn new(value: u16) -> Result<Self, String> {
            if value >= 1024 {
                Ok(Port(value))
            } else {
                Err(format!("port {value} requires root"))
            }
        }

        pub fn get(&self) -> u16 { self.0 }
    }
}

// Outside the module:
// Port(80)       → compile error, field is private
// Port::new(80)  → Err at runtime, caught at boundary
// Port::new(8080) → Ok, guaranteed valid from here on
```

### 5.5 Thin main, Fat lib

`main.rs` is a thin shell — parse CLI args, set up tracing, call into `lib.rs`. Business logic lives in the library crate.

```rust
// main.rs — thin
fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    tracing_subscriber::init();
    myapp::run(args)
}

// lib.rs — fat
pub fn run(args: Args) -> Result<()> {
    let config = Config::load(&args.config_path)?;
    let server = Server::build(config)?;
    server.start()
}
```

**Why**: Library code is testable, reusable, and can be called from multiple binary targets.

---

## 6. Code Quality

### 6.1 clone() Audit

Every `.clone()` must answer: **why can't a reference work?**

| Acceptable | Not acceptable |
|------------|----------------|
| `Arc::clone()` — zero-copy refcount | `String` clone just to satisfy borrow checker → fix lifetimes |
| Cross-`spawn` needs `'static` | `Vec<T>` cloned then read-only → use `&[T]` |
| Small `serde_json::Value` | Large struct cloned for one field → extract field first |

### 6.2 Naming

| Type | Rule | Example |
|------|------|---------|
| Trait | Adjective or capability | `Streamable`, `Retryable` |
| Enum variant | Noun | `ErrorKind::NotFound` |
| Builder method | Field name | `.api_key()`, `.model()` |
| Bool-returning method | `is_` / `has_` / `can_` | `.is_empty()`, `.has_tools()` |
| Conversion method | `into_` / `as_` / `to_` | `.into_inner()`, `.as_str()`, `.to_string()` |

### 6.3 Documentation

- Public API: `///` required
- `unsafe` / `expect`: `// SAFETY:` comment
- Complex algorithms: link to design doc

### 6.4 Build Verification

After each logical unit, verify immediately:

```bash
cargo check --workspace
cargo clippy --workspace -- -D warnings
cargo test --workspace
cargo fmt --all -- --check
```

All must pass before moving to the next module.

---

## 7. Advanced Patterns

### 7.1 Tower-Style Middleware — Composable Async Layers

Wrap services with generic middleware. Each layer adds one concern (timeout, logging, retry) without coupling.

```rust
use std::task::{Context, Poll};
use std::time::Duration;
use tower::Service;

struct Timeout<S> {
    inner: S,
    duration: Duration,
}

impl<S, Req> Service<Req> for Timeout<S>
where
    S: Service<Req>,
    S::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    type Response = S::Response;
    type Error = Box<dyn std::error::Error + Send + Sync>;
    type Future = TimeoutFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: Req) -> Self::Future {
        TimeoutFuture {
            inner: self.inner.call(req),
            sleep: tokio::time::sleep(self.duration),
        }
    }
}

// Stack layers: Timeout<Retry<RateLimit<MyService>>>
// Each layer is independent, testable, reusable.
```

**Rule**: One concern per layer. Compose via wrapping, not inheritance.

### 7.2 GATs — Lifetime-Bound Associated Types

Generic Associated Types enable zero-allocation lending patterns.

```rust
trait LendingIterator {
    type Item<'a> where Self: 'a;
    fn next(&mut self) -> Option<Self::Item<'_>>;
}

// Zero-copy CSV parser: yields borrowed slices from internal buffer
struct CsvParser<'input> {
    remaining: &'input str,
}

impl<'input> LendingIterator for CsvParser<'input> {
    type Item<'a> = &'a str where 'input: 'a;

    fn next(&mut self) -> Option<Self::Item<'_>> {
        let (field, rest) = self.remaining.split_once(',')?;
        self.remaining = rest;
        Some(field)
    }
}

// Pre-GATs this required boxing or collecting into Vec.
```

**Use when**: Iterator items borrow from the iterator itself. Avoids allocation.

### 7.3 Arena Allocation — Batch Allocate, Single Free

Pre-allocate a region. Bump-allocate many objects. Free all at once when the arena drops.

```rust
use bumpalo::Bump;

fn parse_document(input: &str) -> Vec<&Node> {
    let arena = Bump::new();

    // Thousands of small allocations — O(1) each, no individual frees
    let nodes: Vec<&Node> = input.lines()
        .map(|line| arena.alloc(parse_node(line)))
        .collect();

    process(&nodes);
    nodes
    // arena drops here — single deallocation for all nodes
}
```

**Use when**: Many short-lived objects with shared lifetime (compilers, parsers, per-request state).
**Don't use**: Long-lived objects with independent lifetimes.

### 7.4 Lock-Free Atomics — Wait-Free Progress

Atomic CAS loops for high-contention counters and flags. No mutex, no blocking.

```rust
use std::sync::atomic::{AtomicU64, Ordering};

struct Stats {
    requests: AtomicU64,
    errors: AtomicU64,
}

impl Stats {
    fn record_request(&self) {
        self.requests.fetch_add(1, Ordering::Relaxed);
    }

    fn record_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> (u64, u64) {
        // Acquire ensures we see all prior writes
        let req = self.requests.load(Ordering::Acquire);
        let err = self.errors.load(Ordering::Acquire);
        (req, err)
    }
}

// Ordering cheat sheet:
// Relaxed  — atomicity only, fastest (counters, flags)
// Acquire  — see all writes before the paired Release
// Release  — make all prior writes visible to Acquire readers
// SeqCst   — total order, slowest (rarely needed)
```

**Rule**: `Relaxed` for independent counters. `Acquire`/`Release` pairs for synchronization. `SeqCst` only when total ordering is required.

### 7.5 Pin + pin_project — Safe Self-Referential Futures

Use `pin_project` crate to safely project pinned fields. Never write raw `Pin::new_unchecked`.

```rust
use pin_project::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

#[pin_project]
struct TimedFuture<F> {
    start: std::time::Instant,
    #[pin]  // pinned: cannot move after first poll
    inner: F,
}

impl<F: Future> Future for TimedFuture<F> {
    type Output = (F::Output, std::time::Duration);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();  // safe pin projection
        match this.inner.poll(cx) {
            Poll::Ready(output) => {
                Poll::Ready((output, this.start.elapsed()))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
```

**Rule**: `#[pin]` on fields that are `!Unpin` (futures, streams). Use `pin_project` — never raw unsafe pinning.

### 7.6 Const Generics — Compile-Time Parameterization

Types parameterized by constant values. Zero-cost, no heap.

```rust
struct RingBuffer<T, const N: usize> {
    data: [Option<T>; N],
    head: usize,
    len: usize,
}

impl<T, const N: usize> RingBuffer<T, N> {
    const fn new() -> Self
    where
        T: Copy,
    {
        RingBuffer {
            data: [None; N],
            head: 0,
            len: 0,
        }
    }

    fn push(&mut self, item: T) {
        let idx = (self.head + self.len) % N;
        self.data[idx] = Some(item);
        if self.len < N { self.len += 1; }
        else { self.head = (self.head + 1) % N; }
    }
}

// Different sizes = different types, checked at compile time
let small: RingBuffer<u8, 16> = RingBuffer::new();
let large: RingBuffer<u8, 4096> = RingBuffer::new();
```

**Use when**: Fixed-size containers, compile-time feature flags, dimensional analysis.

### 7.7 Phantom Types — Zero-Cost Type Tags

Tag types with metadata that exists only at compile time. Zero runtime cost.

```rust
use std::marker::PhantomData;

struct Meters;
struct Seconds;

struct Quantity<Unit> {
    value: f64,
    _unit: PhantomData<Unit>,
}

impl<U> Quantity<U> {
    fn new(value: f64) -> Self {
        Quantity { value, _unit: PhantomData }
    }
}

// Type-safe operations
fn speed(distance: Quantity<Meters>, time: Quantity<Seconds>) -> f64 {
    distance.value / time.value
}

let d = Quantity::<Meters>::new(100.0);
let t = Quantity::<Seconds>::new(9.58);
let v = speed(d, t);

// speed(t, d) → compile error: expected Meters, got Seconds
```

**Beyond typestate**: Phantom types tag units, permissions, ownership, format — any compile-time distinction.

### 7.8 Procedural Macros — Compile-Time Code Generation

Derive macros eliminate boilerplate. Use `syn` + `quote` for robust codegen.

```rust
// In a proc-macro crate
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Data, Fields};

#[proc_macro_derive(Validate)]
pub fn derive_validate(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let validations = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => fields.named.iter().map(|f| {
                let field_name = &f.ident;
                quote! {
                    if self.#field_name.is_empty() {
                        errors.push(format!("{} is empty", stringify!(#field_name)));
                    }
                }
            }).collect::<Vec<_>>(),
            _ => vec![],
        },
        _ => vec![],
    };

    let expanded = quote! {
        impl #name {
            pub fn validate(&self) -> Result<(), Vec<String>> {
                let mut errors = Vec::new();
                #(#validations)*
                if errors.is_empty() { Ok(()) } else { Err(errors) }
            }
        }
    };

    TokenStream::from(expanded)
}

// Usage:
// #[derive(Validate)]
// struct User { name: String, email: String }
// user.validate()?;
```

**Rule**: Prefer derive macros over attribute macros. Keep generated code inspectable (`cargo expand`).

---

## Pattern Selection Guide

| Problem | Pattern | Section |
|---------|---------|---------|
| Sequential API constraints | Typestate | 1.1 |
| Methods on foreign types | Extension trait | 1.2 |
| Raw string IDs getting mixed up | Newtype | 1.4 |
| Runtime validation repeated everywhere | Parse, don't validate | 5.2 |
| Optional fields that are mutually exclusive | Enum variants | 5.3 |
| Composable async middleware | Tower Service | 7.1 |
| Zero-copy iteration | GATs / LendingIterator | 7.2 |
| Many short-lived allocations | Arena | 7.3 |
| High-contention counters | Lock-free atomics | 7.4 |
| Custom futures / self-ref types | Pin + pin_project | 7.5 |
| Fixed-size containers | Const generics | 7.6 |
| Compile-time unit/tag safety | Phantom types | 7.7 |
| Boilerplate elimination | Proc macros | 7.8 |
