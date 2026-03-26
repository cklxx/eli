# Rust Anti-Patterns Reference

Comprehensive catalog of BAD vs GOOD patterns across 10 categories. Sourced from multiple articles, the Rust Design Patterns book, Clippy documentation, and community discussions.

---

## 1. Performance Anti-Patterns

### 1.1 Unnecessary `.clone()` in Hot Paths

**BAD:**
```rust
fn process_data(data: String, lookup: &HashMap<String, String>) {
    let key = data.clone(); // unnecessary heap allocation
    if let Some(value) = lookup.get(&key) {
        println!("Found: {}", value);
    }
}

fn main() {
    let map = HashMap::from([("key".into(), "value".into())]);
    let input = "key".to_string();
    for _ in 0..10_000 {
        process_data(input.clone(), &map); // 10k clones
    }
}
```

**GOOD:**
```rust
fn process_data(data: &str, lookup: &HashMap<String, String>) {
    if let Some(value) = lookup.get(data) {
        println!("Found: {}", value);
    }
}

fn main() {
    let map = HashMap::from([("key".into(), "value".into())]);
    let input = "key".to_string();
    for _ in 0..10_000 {
        process_data(&input, &map); // zero allocations
    }
}
```

**Why it matters:** Each `.clone()` on a `String` or `Vec` triggers a heap allocation. In tight loops, this dominates runtime. Pass `&str` or `&[T]` instead.

### 1.2 Vec Without Pre-allocated Capacity

**BAD:**
```rust
let mut vec = Vec::new();
for i in 0..1000 {
    vec.push(i); // triggers ~10 reallocations (0→4→8→16→32→...→1024)
}
```

**GOOD:**
```rust
let mut vec = Vec::with_capacity(1000);
for i in 0..1000 {
    vec.push(i); // single allocation, zero reallocations
}
```

**Why it matters:** Each reallocation copies the entire buffer. Known sizes should always use `with_capacity`. Same applies to `HashMap::with_capacity`.

### 1.3 Unnecessary Intermediate `collect()`

**BAD:**
```rust
let nopes: Vec<_> = bleeps.iter().map(boop).collect();
let frungies: Vec<_> = nopes.iter().filter(|x| **x > MIN_THRESHOLD).collect();
```

**GOOD:**
```rust
let frungies: Vec<_> = bleeps.iter()
    .map(boop)
    .filter(|x| *x > MIN_THRESHOLD)
    .collect(); // single allocation, lazy evaluation
```

**Why it matters:** Each `collect()` forces a full allocation + evaluation. Chain iterators lazily; only `collect()` once at the end.

### 1.4 Allocating Strings When Literals Suffice

**BAD:**
```rust
let message = format!("Error occurred"); // heap allocation for a constant string
```

**GOOD:**
```rust
let message = "Error occurred"; // zero-cost &str
```

**Why it matters:** `format!` always allocates a `String`. If the content is static, just use a `&str` literal.

### 1.5 Reading Lines with Per-line Allocation

**BAD:**
```rust
for line in reader.lines() {
    let line = line.unwrap(); // allocates a new String per line
    process(&line);
}
```

**GOOD:**
```rust
let mut line = String::new();
while reader.read_line(&mut line).unwrap() > 0 {
    process(&line);
    line.clear(); // reuse the allocation
}
```

**Why it matters:** For large files, the BAD version makes N allocations. The GOOD version makes at most 1 (plus possible growth).

### 1.6 Unbuffered I/O and Unlocked stdout

**BAD:**
```rust
for line in lines {
    println!("{}", line); // locks stdout on EVERY call
    writeln!(file, "{}", line).unwrap(); // unbuffered writes
}
```

**GOOD:**
```rust
let mut buf = BufWriter::new(file);
let mut lock = io::stdout().lock(); // lock once
for line in lines {
    writeln!(lock, "{}", line).unwrap();
    writeln!(buf, "{}", line).unwrap();
}
```

**Why it matters:** `println!` acquires a mutex on stdout for every call. Locking once and using `BufWriter` for files can yield 10-100x speedups in I/O-heavy code.

### 1.7 Indexed Loops Instead of Iterators

**BAD:**
```rust
for i in 0..xs.len() {
    let x = xs[i]; // bounds check on every access
    // ...
}
```

**GOOD:**
```rust
for x in &xs {
    // no bounds check, enables auto-vectorization
}
```

**Why it matters:** Iterators eliminate bounds checks and enable LLVM to auto-vectorize. Indexed access can't prove to the optimizer that all indices are in bounds.

### 1.8 `clone()` When `clone_from()` Can Reuse Buffers

**BAD:**
```rust
let mut v1: Vec<u32> = Vec::with_capacity(99);
let v2: Vec<u32> = vec![1, 2, 3];
v1 = v2.clone(); // v1's 99-capacity allocation is dropped, new one created
```

**GOOD:**
```rust
let mut v1: Vec<u32> = Vec::with_capacity(99);
let v2: Vec<u32> = vec![1, 2, 3];
v1.clone_from(&v2); // reuses v1's existing allocation
assert_eq!(v1.capacity(), 99);
```

**Why it matters:** `clone_from` avoids deallocating the destination then allocating a fresh one. Especially important in loops.

---

## 2. Error Handling Anti-Patterns

### 2.1 `unwrap()` Everywhere

**BAD:**
```rust
fn read_file(path: &str) -> String {
    std::fs::read_to_string(path).unwrap() // panics in production
}

fn get_config(key: &str) -> String {
    let config: HashMap<&str, &str> = [("port", "8080")].into_iter().collect();
    config.get(key).unwrap().to_string() // panics on missing key
}
```

**GOOD:**
```rust
fn read_file(path: &str) -> Result<String, std::io::Error> {
    std::fs::read_to_string(path) // caller decides how to handle
}

fn get_config(key: &str) -> Result<String, ConfigError> {
    let config: HashMap<&str, &str> = [("port", "8080")].into_iter().collect();
    config
        .get(key)
        .ok_or_else(|| ConfigError::MissingKey(key.to_string()))
        .map(|s| s.to_string())
}
```

**Why it matters:** `unwrap()` turns a recoverable error into a process-killing panic. Production code should propagate errors with `?` and let callers decide.

### 2.2 Stringly-Typed Errors

**BAD:**
```rust
fn parse_config(input: &str) -> Result<Config, String> {
    if input.is_empty() {
        return Err("config is empty".to_string());
    }
    // ...
    Err(format!("unknown field: {}", field))
}

// Caller has to string-match:
match parse_config(input) {
    Err(e) if e.contains("empty") => { /* ... */ }
    Err(e) => { /* now what? */ }
    Ok(c) => { /* ... */ }
}
```

**GOOD:**
```rust
#[derive(Debug, thiserror::Error)]
enum ConfigError {
    #[error("config is empty")]
    Empty,
    #[error("unknown field: {0}")]
    UnknownField(String),
    #[error("parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

fn parse_config(input: &str) -> Result<Config, ConfigError> {
    if input.is_empty() {
        return Err(ConfigError::Empty);
    }
    // ...
}

// Caller matches exhaustively:
match parse_config(input) {
    Err(ConfigError::Empty) => { /* ... */ }
    Err(ConfigError::UnknownField(f)) => { /* ... */ }
    Err(ConfigError::Parse(e)) => { /* ... */ }
    Ok(c) => { /* ... */ }
}
```

**Why it matters:** String errors are unstructured, non-exhaustive, fragile (typos in matching), and can't carry typed context. Use `thiserror` for libraries, `anyhow` for applications.

### 2.3 Swallowing Errors

**BAD:**
```rust
fn process(path: &str) {
    let _ = std::fs::remove_file(path); // silently ignores failure
    if let Ok(data) = std::fs::read_to_string(path) {
        // ... but if this fails, we just skip silently
    }
}
```

**GOOD:**
```rust
fn process(path: &str) -> Result<(), AppError> {
    std::fs::remove_file(path)?; // propagates
    let data = std::fs::read_to_string(path)?;
    // ...
    Ok(())
}
```

**Why it matters:** Silent error swallowing causes mysterious failures downstream. Either propagate, log, or explicitly document why ignoring is safe.

### 2.4 Using `Box<dyn Error>` in Library APIs

**BAD (in a library):**
```rust
pub fn parse(input: &str) -> Result<Ast, Box<dyn std::error::Error>> {
    // callers can't match on the error type
}
```

**GOOD (library):**
```rust
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("unexpected token: {0}")]
    UnexpectedToken(String),
    #[error("unterminated string at line {0}")]
    UnterminatedString(usize),
}

pub fn parse(input: &str) -> Result<Ast, ParseError> {
    // callers can match exhaustively
}
```

**GOOD (application):**
```rust
// anyhow::Error is fine at the application level
fn main() -> anyhow::Result<()> {
    let ast = parse(input).context("failed to parse config")?;
    Ok(())
}
```

**Why it matters:** Libraries should expose structured error types so callers can programmatically handle failures. `Box<dyn Error>` erases that information. Use `thiserror` for libraries, `anyhow` for applications.

### 2.5 Providing No Context on Error Propagation

**BAD:**
```rust
fn load_config() -> Result<Config, anyhow::Error> {
    let text = std::fs::read_to_string("config.toml")?; // "No such file or directory"
    let config: Config = toml::from_str(&text)?;         // no idea WHICH step failed
    Ok(config)
}
```

**GOOD:**
```rust
fn load_config() -> Result<Config, anyhow::Error> {
    let text = std::fs::read_to_string("config.toml")
        .context("failed to read config.toml")?;
    let config: Config = toml::from_str(&text)
        .context("failed to parse config.toml as TOML")?;
    Ok(config)
}
```

**Why it matters:** Raw `?` gives you the low-level error message but no call-site context. `.context()` adds the "what was I trying to do" layer.

---

## 3. Ownership/Borrowing Mistakes

### 3.1 Cloning to Silence the Borrow Checker

**BAD:**
```rust
let mut x = 5;
let y = &mut (x.clone()); // "fixes" borrow error by cloning
println!("{x}");
*y += 1; // modifies the CLONE, not x -- probably a bug
```

**GOOD:**
```rust
let mut x = 5;
x += 1; // just modify x directly
println!("{x}");
```

**Why it matters:** If you clone just to make the borrow checker stop complaining, you've likely introduced a logic bug. The clone is a separate value; mutations won't be reflected.

### 3.2 Unnecessary `Arc<Mutex<T>>` Everywhere

**BAD:**
```rust
// "I can't figure out lifetimes, so everything gets Arc<Mutex>"
struct App {
    config: Arc<Mutex<Config>>,
    db: Arc<Mutex<Database>>,
    cache: Arc<Mutex<Cache>>,
}

fn handle_request(app: &App) {
    let config = app.config.lock().unwrap();
    let db = app.db.lock().unwrap(); // potential deadlock
    // ...
}
```

**GOOD:**
```rust
struct App {
    config: Config,        // owned, immutable after init
    db: Database,          // passed as &self
    cache: Mutex<Cache>,   // only the actually-shared thing
}

fn handle_request(app: &App) {
    let cache_entry = app.cache.lock().unwrap();
    // config and db don't need locking at all
}
```

**Why it matters:** `Arc<Mutex<T>>` is expensive (atomic ops + locking) and introduces deadlock risk. Use it only where you genuinely need shared mutable state across threads. Often restructuring ownership removes the need.

### 3.3 Fighting Lifetimes with Owned Types

**BAD:**
```rust
fn process_data(data: Vec<i32>) {
    // takes ownership unnecessarily
}

fn main() {
    let data = vec![1, 2, 3, 4, 5];
    process_data(data.clone()); // clone just because process_data takes ownership
    println!("{:?}", data);     // need data again
}
```

**GOOD:**
```rust
fn process_data(data: &[i32]) {
    // borrows -- no allocation needed
}

fn main() {
    let data = vec![1, 2, 3, 4, 5];
    process_data(&data);
    println!("{:?}", data); // still available
}
```

**Why it matters:** Accept `&T` or `&[T]` when you only need to read. Take ownership (`T`) only when you need to store/move the value.

### 3.4 Using Rc/Arc When a Simple Reference Works

**BAD:**
```rust
use std::sync::Arc;

fn print_name(name: Arc<String>) {
    println!("{}", name);
}

fn main() {
    let name = Arc::new("Alice".to_string());
    print_name(name.clone());
}
```

**GOOD:**
```rust
fn print_name(name: &str) {
    println!("{}", name);
}

fn main() {
    let name = "Alice".to_string();
    print_name(&name);
}
```

**Why it matters:** `Arc` adds atomic reference counting overhead. If the lifetime is clear and there's no shared ownership, a plain reference is zero-cost.

### 3.5 Self-Referential Structs via Raw Pointers

**BAD:**
```rust
struct SelfRef {
    data: String,
    ptr: *const String, // points to self.data -- breaks on move!
}

impl SelfRef {
    fn new(data: String) -> Self {
        let mut s = SelfRef { data, ptr: std::ptr::null() };
        s.ptr = &s.data; // will dangle after move
        s
    }
}
```

**GOOD:**
```rust
// Use handles/indices/offsets instead
struct Document {
    paragraphs: Vec<String>,
    current: usize, // index, not pointer
}

// Or use Pin for truly self-referential types
// Or use a crate like `ouroboros` or `self_cell`
```

**Why it matters:** Rust moves values in memory. Raw pointers to self become dangling after any move. Use indices, `Pin`, or purpose-built crates.

---

## 4. API Design Anti-Patterns

### 4.1 Accepting `&String` / `&Vec<T>` Instead of `&str` / `&[T]`

**BAD:**
```rust
fn greet(name: &String) {
    println!("Hello, {}", name);
}

fn sum(numbers: &Vec<i32>) -> i32 {
    numbers.iter().sum()
}
```

**GOOD:**
```rust
fn greet(name: &str) {
    println!("Hello, {}", name);
}

fn sum(numbers: &[i32]) -> i32 {
    numbers.iter().sum()
}
```

**Why it matters:** `&str` accepts `&String`, `&str`, string literals, and slices of `String`. `&String` only accepts `&String`. The narrow type limits callers for no benefit.

### 4.2 Stringly-Typed APIs

**BAD:**
```rust
fn color_me(input: &str, color: &str) { /* ... */ }

fn main() {
    color_me("surprised", "blue");
    color_me("surprised", "bleu"); // typo compiles fine, fails at runtime
}
```

**GOOD:**
```rust
enum Color { Red, Green, Blue, LightGoldenRodYellow }

fn color_me(input: &str, color: Color) { /* ... */ }

fn main() {
    color_me("surprised", Color::Blue);
    // color_me("surprised", Color::Bleu); // compile error!
}
```

**Why it matters:** Enums give you exhaustive matching, IDE autocompletion, and compile-time typo prevention. Strings give you none of that.

### 4.3 Concrete Types Where Generics Should Be Used

**BAD:**
```rust
fn open_file(path: PathBuf) {
    // caller must allocate a PathBuf
}

open_file(PathBuf::from("/tmp/foo")); // forced allocation
```

**GOOD:**
```rust
fn open_file(path: impl AsRef<Path>) {
    let path = path.as_ref();
    // ...
}

open_file("/tmp/foo");        // &str works
open_file(Path::new("/tmp")); // Path works
open_file(PathBuf::from("x")); // PathBuf works
```

**Why it matters:** Using conversion traits (`AsRef`, `Into`) makes APIs ergonomic. Callers don't need unnecessary allocations or conversions.

### 4.4 No Builder Pattern for Complex Construction

**BAD:**
```rust
pub fn connect(
    host: &str,
    port: u16,
    timeout: Duration,
    retry: bool,
    max_retries: u32,
    tls: bool,
    cert_path: Option<&str>,
) -> Connection { /* ... */ }

// Caller confusion:
connect("localhost", 8080, Duration::from_secs(30), true, 3, false, None);
```

**GOOD:**
```rust
let conn = ConnectionBuilder::new("localhost", 8080)
    .timeout(Duration::from_secs(30))
    .retry(3)
    .tls(false)
    .build()?;
```

**Why it matters:** Builders are self-documenting, support optional parameters with defaults, and can validate invariants in `build()`.

### 4.5 Not Implementing Standard Traits

**BAD:**
```rust
struct UserId(u64);

// No Debug, no Display, no PartialEq, no Hash...
// Can't println!, can't use in HashMap, can't compare
```

**GOOD:**
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct UserId(u64);

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "user:{}", self.0)
    }
}
```

**Why it matters:** Missing `Debug` makes debugging painful. Missing `PartialEq` prevents testing with `assert_eq!`. Missing `Hash` prevents use in `HashMap`. Derive the "standard set" unless you have a reason not to.

### 4.6 Eagerly Evaluating Defaults

**BAD:**
```rust
// default is computed even when option is Some
my_option.unwrap_or(expensive_default())
```

**GOOD:**
```rust
// closure is only called when option is None
my_option.unwrap_or_else(|| expensive_default())

// or with map_or_else
my_option.map_or_else(compute_default, process_value)
```

**Why it matters:** `unwrap_or` evaluates its argument unconditionally. Use `_else` variants for expensive computations.

---

## 5. Async Anti-Patterns

### 5.1 Blocking I/O in Async Functions

**BAD:**
```rust
async fn read_config() -> String {
    // std::fs blocks the tokio worker thread!
    std::fs::read_to_string("config.json").unwrap()
}
```

**GOOD (async I/O):**
```rust
async fn read_config() -> Result<String, std::io::Error> {
    tokio::fs::read_to_string("config.json").await
}
```

**GOOD (spawn_blocking):**
```rust
async fn read_config() -> Result<String, std::io::Error> {
    tokio::task::spawn_blocking(|| {
        std::fs::read_to_string("config.json")
    })
    .await
    .unwrap()
}
```

**Why it matters:** One blocking call on a tokio worker thread stalls ALL tasks on that thread. Use async I/O or offload to the blocking thread pool.

### 5.2 `std::thread::sleep` in Async Code

**BAD:**
```rust
async fn delay_then_print(timer: i32) {
    println!("Start timer {}.", timer);
    std::thread::sleep(Duration::from_secs(1)); // blocks the runtime!
    println!("Timer {} done.", timer);
}

// These run SEQUENTIALLY despite join!, taking 3 seconds:
tokio::join!(delay_then_print(1), delay_then_print(2), delay_then_print(3));
```

**GOOD:**
```rust
async fn delay_then_print(timer: i32) {
    println!("Start timer {}.", timer);
    tokio::time::sleep(Duration::from_secs(1)).await; // yields to runtime
    println!("Timer {} done.", timer);
}

// These run CONCURRENTLY, taking ~1 second:
tokio::join!(delay_then_print(1), delay_then_print(2), delay_then_print(3));
```

**Why it matters:** `std::thread::sleep` freezes the entire worker thread. `tokio::time::sleep` yields control, allowing other tasks to run.

### 5.3 Over-Spawning Micro-Tasks

**BAD:**
```rust
async fn process_many(items: Vec<usize>) {
    for i in items {
        tokio::spawn(process_item(i)); // 100k spawns = 100k task allocations
    }
}
```

**GOOD:**
```rust
async fn process_many(items: Vec<usize>, batch_size: usize) {
    for chunk in items.chunks(batch_size) {
        let chunk = chunk.to_owned();
        tokio::spawn(async move {
            for i in chunk {
                process_item(i).await;
            }
        });
    }
}
```

**Why it matters:** Each `spawn` has overhead: allocation, queueing, waking, context switching. Batch work into fewer tasks.

### 5.4 `block_on` Inside the Runtime

**BAD:**
```rust
fn bad_sync_call(rt: &Runtime) -> u32 {
    rt.block_on(do_async_work()) // deadlocks if called from a runtime thread
}
```

**GOOD:**
```rust
fn safe_sync_call(rt: &Runtime) -> u32 {
    let (tx, rx) = tokio::sync::oneshot::channel();
    rt.spawn(async move {
        let res = do_async_work().await;
        let _ = tx.send(res);
    });
    rx.blocking_recv().expect("async task failed")
}
```

**Why it matters:** `block_on` from within the runtime steals a worker thread that may be needed by the future it's waiting on, causing deadlock.

### 5.5 No Backpressure or Timeouts

**BAD:**
```rust
let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel(); // unbounded = OOM risk

tokio::spawn(async move {
    while let Some(id) = rx.recv().await {
        call_downstream(id).await; // no timeout -- hangs forever on slow downstream
    }
});
```

**GOOD:**
```rust
let (tx, mut rx) = tokio::sync::mpsc::channel(100); // bounded backpressure

tokio::spawn(async move {
    while let Some(id) = rx.recv().await {
        match tokio::time::timeout(Duration::from_secs(2), call_downstream(id)).await {
            Ok(Ok(val)) => println!("ok: {val}"),
            Ok(Err(e)) => eprintln!("error: {e}"),
            Err(_) => eprintln!("timeout for {id}"),
        }
    }
});
```

**Why it matters:** Unbounded channels can OOM under load. Missing timeouts leave zombie tasks consuming resources indefinitely.

### 5.6 CPU-Bound Work on the Async Runtime

**BAD:**
```rust
async fn hash_password(password: String) -> String {
    // CPU-intensive work blocks the runtime
    argon2::hash_password(&password)
}
```

**GOOD:**
```rust
async fn hash_password(password: String) -> String {
    tokio::task::spawn_blocking(move || {
        argon2::hash_password(&password)
    })
    .await
    .unwrap()
}

// Or use rayon for parallel CPU work:
async fn parallel_sum(nums: Vec<i32>) -> i32 {
    let (tx, rx) = tokio::sync::oneshot::channel();
    rayon::spawn(move || {
        let sum: i32 = nums.par_iter().sum();
        let _ = tx.send(sum);
    });
    rx.await.expect("rayon task panicked")
}
```

**Why it matters:** Rule of thumb: if it takes >100us of CPU time without an `.await`, offload it. The async runtime's threads are for I/O multiplexing, not number crunching.

---

## 6. Type System Misuse

### 6.1 Stringly-Typed Code

**BAD:**
```rust
fn handle_event(name: &str, data: &str) {
    match name {
        "click" => { /* ... */ }
        "hover" => { /* ... */ }
        "clack" => { /* typo, no compile error */ }
        _ => {} // silent fallthrough
    }
}
```

**GOOD:**
```rust
enum Event {
    Click { x: i32, y: i32 },
    Hover { x: i32, y: i32 },
    KeyPress { key: char },
}

fn handle_event(event: Event) {
    match event {
        Event::Click { x, y } => { /* ... */ }
        Event::Hover { x, y } => { /* ... */ }
        Event::KeyPress { key } => { /* ... */ }
        // compiler errors on missing variants
    }
}
```

**Why it matters:** Strings bypass the type system entirely. Enums give exhaustive matching, associated data, and zero-cost dispatch.

### 6.2 Not Using Newtypes

**BAD:**
```rust
fn create_user(name: String, email: String, age: u32, zip_code: u32) {
    // ...
}

// Easy to swap arguments:
create_user(name, email, zip_code, age); // compiles, but age and zip are swapped!
```

**GOOD:**
```rust
#[derive(Debug, Clone)]
struct Email(String);
#[derive(Debug, Clone, Copy)]
struct Age(u32);
#[derive(Debug, Clone, Copy)]
struct ZipCode(u32);

fn create_user(name: String, email: Email, age: Age, zip: ZipCode) {
    // ...
}

// create_user(name, email, ZipCode(12345), Age(30)); // compile error: wrong order
create_user(name, email, Age(30), ZipCode(12345)); // correct
```

**Why it matters:** Newtypes are zero-cost wrappers that prevent argument swapping, add semantic meaning, and let you implement traits selectively. "Make illegal states unrepresentable."

### 6.3 Boolean Parameters

**BAD:**
```rust
fn render(content: &str, visible: bool, editable: bool, highlighted: bool) {
    // ...
}

render("hello", true, false, true); // what do these bools mean?
```

**GOOD:**
```rust
enum Visibility { Visible, Hidden }
enum Editability { Editable, ReadOnly }
enum Highlight { On, Off }

fn render(content: &str, vis: Visibility, edit: Editability, hl: Highlight) {
    // ...
}

render("hello", Visibility::Visible, Editability::ReadOnly, Highlight::On);
```

**Why it matters:** Boolean params are unreadable at call sites. Enums are self-documenting and prevent accidental swaps. Clippy's `fn_params_excessive_bools` catches this.

### 6.4 Using Option Where Result Is Appropriate

**BAD:**
```rust
fn find_user(id: u64) -> Option<User> {
    // Returns None for: not found, db error, timeout, permission denied
    // Caller has no idea WHY it failed
}
```

**GOOD:**
```rust
fn find_user(id: u64) -> Result<Option<User>, DbError> {
    // Ok(Some(user)) = found
    // Ok(None) = not found (legitimate)
    // Err(e) = actual error
}
```

**Why it matters:** `Option` erases failure reasons. `Result<Option<T>, E>` distinguishes "absent" from "error."

### 6.5 State Machines as Enums Without Type-State Encoding

**BAD:**
```rust
struct Connection {
    state: ConnectionState,
    // ...
}

enum ConnectionState { Idle, Connecting, Connected, Closed }

impl Connection {
    fn send(&self, data: &[u8]) {
        assert!(self.state == ConnectionState::Connected); // runtime check
        // ...
    }
}
```

**GOOD:**
```rust
struct Idle;
struct Connected { stream: TcpStream }
struct Closed;

struct Connection<S> { state: S }

impl Connection<Idle> {
    fn connect(self, addr: &str) -> Result<Connection<Connected>, Error> { /* ... */ }
}

impl Connection<Connected> {
    fn send(&self, data: &[u8]) -> Result<(), Error> { /* ... */ }
    fn close(self) -> Connection<Closed> { /* ... */ }
}

// Connection<Idle>.send() won't compile -- invalid state transition caught at compile time
```

**Why it matters:** Type-state pattern moves state machine invariants from runtime assertions to compile-time checks. Invalid transitions become type errors.

---

## 7. Unsafe Misuse

### 7.1 Creating Multiple Mutable References Through Raw Pointers

**BAD:**
```rust
let mut x = 10;
let ptr: *mut i32 = &mut x;
unsafe {
    let r1 = &mut *ptr;
    let r2 = &mut *ptr; // UB: two mutable references to same memory
    *r1 = 20;
    *r2 = 30;
}
```

**GOOD:**
```rust
let mut x = 10;
let ptr: *mut i32 = &mut x;
unsafe {
    // Work with raw pointers directly, don't create overlapping references
    ptr.write(20);
    let val = ptr.read();
}
```

**Why it matters:** The Rust compiler assumes mutable references are exclusive. Violating this causes undefined behavior even in `unsafe` blocks -- the optimizer makes assumptions that break your code.

### 7.2 Using Assignment Instead of `ptr::write`

**BAD:**
```rust
let t_ptr: *mut OldType = /* ... */;
let u_ptr: *mut NewType = t_ptr.cast();
unsafe {
    *u_ptr = new_value; // drops the old value AS NewType, but it's actually OldType!
}
```

**GOOD:**
```rust
let t_ptr: *mut OldType = /* ... */;
let u_ptr: *mut NewType = t_ptr.cast();
unsafe {
    u_ptr.write(new_value); // doesn't drop the old value
}
```

**Why it matters:** `*ptr = val` runs the destructor on the old value. If the pointer was cast from a different type, it calls the wrong destructor -- UB and likely corruption.

### 7.3 Using `as` for Pointer Casts

**BAD:**
```rust
let ptr: *mut T = /* ... */;
let other: *mut U = ptr as *mut U; // silently changes mutability, no safety checks
```

**GOOD:**
```rust
let ptr: *mut T = /* ... */;
let other: *mut U = ptr.cast::<U>(); // explicit, preserves mutability
```

**Why it matters:** `as` casts are a "big hammer" that can silently change pointer mutability. `.cast()` preserves const/mut and is more explicit about intent.

### 7.4 Unsafe When Safe Alternatives Exist

**BAD:**
```rust
// "I need to transform a Vec in place"
fn map_in_place<T, U, F: FnMut(T) -> U>(v: Vec<T>, mut f: F) -> Vec<U> {
    unsafe {
        let (ptr, len, cap) = v.into_raw_parts();
        for i in 0..len {
            let t = ptr.add(i).read();
            let u = f(t);
            (ptr.add(i) as *mut U).write(u);
        }
        Vec::from_raw_parts(ptr as *mut U, len, cap)
    }
}
```

**GOOD:**
```rust
fn map_in_place<T, U, F: FnMut(T) -> U>(v: Vec<T>, f: F) -> Vec<U> {
    v.into_iter().map(f).collect() // Rust optimizes this to in-place when layouts match
}
```

**Why it matters:** The safe version is shorter, correct, panic-safe, and the compiler often optimizes it to the same machine code. Don't reach for `unsafe` until you've proven the safe version is too slow.

### 7.5 Missing Safety Comments and Miri Testing

**BAD:**
```rust
unsafe {
    // no explanation of why this is sound
    std::slice::from_raw_parts(ptr, len)
}
```

**GOOD:**
```rust
// SAFETY: `ptr` was obtained from `Vec::as_ptr()` and `len` is within
// the Vec's allocation. The Vec is alive for 'a, so the slice is valid.
unsafe {
    std::slice::from_raw_parts(ptr, len)
}

// In CI:
// cargo miri test
```

**Why it matters:** Clippy's `undocumented_unsafe_blocks` lint exists for a reason. Every `unsafe` block should explain its safety invariants. Run `cargo miri test` to dynamically detect UB.

---

## 8. Common Clippy Warnings People Ignore

### 8.1 `clippy::needless_return`

**BAD:**
```rust
fn add(x: i32, y: i32) -> i32 {
    return x + y;
}
```

**GOOD:**
```rust
fn add(x: i32, y: i32) -> i32 {
    x + y
}
```

### 8.2 `clippy::approx_constant`

**BAD:**
```rust
fn circle_area(radius: f64) -> f64 {
    let pi = 3.14;
    pi * radius * radius
}
```

**GOOD:**
```rust
fn circle_area(radius: f64) -> f64 {
    std::f64::consts::PI * radius * radius
}
```

### 8.3 `clippy::redundant_clone`

**BAD:**
```rust
fn process(s: String) {
    let name = s.clone(); // s is never used again -- clone is redundant
    println!("{}", name);
}
```

**GOOD:**
```rust
fn process(s: String) {
    println!("{}", s); // just use s directly
}
```

### 8.4 `clippy::needless_collect`

**BAD:**
```rust
let names: Vec<_> = items.iter().map(|x| x.name()).collect();
for name in names.iter() {
    println!("{}", name);
}
```

**GOOD:**
```rust
for name in items.iter().map(|x| x.name()) {
    println!("{}", name);
}
```

### 8.5 `clippy::manual_map`

**BAD:**
```rust
let opt = match some_option {
    Some(x) => Some(x + 1),
    None => None,
};
```

**GOOD:**
```rust
let opt = some_option.map(|x| x + 1);
```

### 8.6 `clippy::box_collection`

**BAD:**
```rust
struct Foo {
    data: Box<Vec<i32>>, // Vec is already heap-allocated
}
```

**GOOD:**
```rust
struct Foo {
    data: Vec<i32>, // no need for double indirection
}
```

### 8.7 `clippy::borrowed_box`

**BAD:**
```rust
fn process(data: &Box<[u8]>) { // unnecessary indirection
    // ...
}
```

**GOOD:**
```rust
fn process(data: &[u8]) { // direct slice reference
    // ...
}
```

### 8.8 `clippy::from_over_into`

**BAD:**
```rust
impl Into<String> for MyType {
    fn into(self) -> String {
        format!("{:?}", self)
    }
}
```

**GOOD:**
```rust
impl From<MyType> for String {
    fn from(val: MyType) -> Self {
        format!("{:?}", val)
    }
}
```

**Why:** Implementing `From` automatically gives you `Into`. Implementing `Into` does NOT give you `From`.

### 8.9 `clippy::cast_lossless`

**BAD:**
```rust
let x: u8 = 42;
let y = x as u32; // silent lossy potential in other directions
```

**GOOD:**
```rust
let x: u8 = 42;
let y = u32::from(x); // explicitly infallible widening
```

### 8.10 `clippy::wildcard_imports`

**BAD:**
```rust
use std::collections::*; // imports everything, unclear what's used
```

**GOOD:**
```rust
use std::collections::{HashMap, HashSet, BTreeMap};
```

**Why:** Wildcard imports make it impossible to tell where a name comes from, cause shadowing issues, and can break with upstream changes.

---

## 9. Testing Anti-Patterns

### 9.1 Tests That Depend on Other Tests' State

**BAD:**
```rust
static mut COUNTER: i32 = 0;

#[test]
fn test_increment() {
    unsafe { COUNTER += 1; }
    assert_eq!(unsafe { COUNTER }, 1);
}

#[test]
fn test_depends_on_increment() {
    // Assumes test_increment ran first -- test order is NOT guaranteed
    assert_eq!(unsafe { COUNTER }, 1);
}
```

**GOOD:**
```rust
#[test]
fn test_increment() {
    let mut counter = 0;
    counter += 1;
    assert_eq!(counter, 1);
}

#[test]
fn test_independent() {
    let mut counter = 0;
    counter += 1;
    assert_eq!(counter, 1); // own state, no dependency
}
```

### 9.2 Testing Implementation Details Instead of Behavior

**BAD:**
```rust
#[test]
fn test_internal_cache_structure() {
    let svc = Service::new();
    svc.process("hello");
    // Reaches into private internals:
    assert_eq!(svc.cache.inner.len(), 1);
    assert_eq!(svc.cache.inner[0].key, "hello");
}
```

**GOOD:**
```rust
#[test]
fn test_caching_behavior() {
    let svc = Service::new();
    let r1 = svc.process("hello");
    let r2 = svc.process("hello"); // should hit cache
    assert_eq!(r1, r2);
    // Test behavior, not structure
}
```

### 9.3 Over-DRY Tests (Unreadable Abstractions)

**BAD:**
```rust
fn setup() -> (Database, User, Config) {
    // 50 lines of setup shared by every test
}

#[test]
fn test_user_login() {
    let (db, user, config) = setup(); // what does this test ACTUALLY need?
    // ...
}
```

**GOOD:**
```rust
#[test]
fn test_user_login() {
    let db = TestDb::new(); // explicit, minimal setup
    let user = User { name: "alice".into(), active: true };
    assert!(db.authenticate(&user, "password123").is_ok());
}
```

**Why:** Tests should be DAMP (Descriptive And Meaningful Phrases), not DRY. Each test should make its setup and assertions obvious without reading helper functions.

### 9.4 No Assertion Messages

**BAD:**
```rust
#[test]
fn test_parsing() {
    let result = parse("input");
    assert!(result.is_ok()); // failure says "assertion failed" -- useless
}
```

**GOOD:**
```rust
#[test]
fn test_parsing() {
    let result = parse("input");
    assert!(result.is_ok(), "parse('input') failed: {:?}", result.err());
}

// Or even better -- use assert_eq for structured output:
#[test]
fn test_parsing() {
    let result = parse("input").expect("parse failed");
    assert_eq!(result.value, 42, "expected value=42 for input 'input'");
}
```

### 9.5 Not Testing Error Paths

**BAD:**
```rust
#[test]
fn test_read_file() {
    let content = read_file("test.txt").unwrap();
    assert!(!content.is_empty());
    // what about: missing file? permission denied? corrupt data?
}
```

**GOOD:**
```rust
#[test]
fn test_read_file_success() {
    let content = read_file("tests/fixtures/valid.txt").unwrap();
    assert!(!content.is_empty());
}

#[test]
fn test_read_file_not_found() {
    let err = read_file("nonexistent.txt").unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
}

#[test]
fn test_read_file_invalid_utf8() {
    let err = read_file("tests/fixtures/binary.bin").unwrap_err();
    assert!(matches!(err, AppError::InvalidEncoding(_)));
}
```

---

## 10. Cargo/Dependency Anti-Patterns

### 10.1 Wildcard Dependencies

**BAD:**
```toml
[dependencies]
serde = "*"           # any version -- breaks on major bumps
tokio = ">=1.0"       # open-ended -- same problem
rand = "0"            # matches 0.anything
```

**GOOD:**
```toml
[dependencies]
serde = "1.0"         # compatible with 1.x, won't jump to 2.0
tokio = "1.38"        # specific minor, auto-updates patches
rand = "0.8"          # pinned to 0.8.x range
```

**Why it matters:** Wildcard deps are rejected by crates.io. Open-ended ranges can pull in breaking changes. Pin to major.minor.

### 10.2 Enabling All Default Features

**BAD:**
```toml
[dependencies]
tokio = "1"  # pulls in ALL default features (io, net, time, fs, macros, rt-multi-thread...)
reqwest = "0.12"  # pulls in default-tls, cookies, gzip...
```

**GOOD:**
```toml
[dependencies]
tokio = { version = "1", default-features = false, features = ["rt", "macros", "net"] }
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
```

**Why it matters:** Extra features mean extra compile time, bigger binaries, and larger attack surface. Only enable what you use.

### 10.3 Not Auditing Dependencies

**BAD:**
```bash
cargo build  # 400 transitive dependencies, never audited
```

**GOOD:**
```bash
cargo audit           # check for known vulnerabilities
cargo deny check      # license compliance + advisory checks
cargo tree --duplicates  # find duplicate crate versions
cargo udeps           # find unused dependencies
```

**Why it matters:** Supply chain attacks are real. A single vulnerable transitive dependency compromises your entire application. Automate `cargo audit` in CI.

### 10.4 Duplicate Dependencies

**BAD:**
```
# cargo tree shows:
# rand v0.7.3
# rand v0.8.5
# Two different versions of the same crate -- wasted compile time + binary bloat
```

**GOOD:**
```bash
# Regularly run:
cargo tree --duplicates

# Then align versions:
cargo update -p rand
# Or add workspace-level dependency resolution
```

### 10.5 No Lock File in Applications

**BAD (for applications):**
```gitignore
# .gitignore
Cargo.lock  # DON'T ignore this for applications!
```

**GOOD:**
```
# For applications: commit Cargo.lock for reproducible builds
# For libraries: .gitignore Cargo.lock (let downstream resolve)
```

**Why it matters:** Without `Cargo.lock`, `cargo build` on a different machine or CI may resolve different dependency versions, causing "works on my machine" bugs.

---

## Quick Reference: Cow for Avoiding Unnecessary Allocations

A pattern that cuts across many categories:

**BAD:**
```rust
fn process(input: &str) -> String {
    if input.contains("bad") {
        input.replace("bad", "good") // allocates only when needed -- OK
    } else {
        input.to_string() // allocates even when nothing changed!
    }
}
```

**GOOD:**
```rust
use std::borrow::Cow;

fn process(input: &str) -> Cow<'_, str> {
    if input.contains("bad") {
        Cow::Owned(input.replace("bad", "good"))
    } else {
        Cow::Borrowed(input) // zero allocation in the common case
    }
}
```

**Why it matters:** `Cow` avoids allocation when borrowing is sufficient, and only allocates when mutation is needed. Great for functions that "usually" return the input unchanged.

---

## Sources

- [The 7 Rust Anti-Patterns Killing Your Performance (2025)](https://medium.com/solo-devs/the-7-rust-anti-patterns-that-are-secretly-killing-your-performance-and-how-to-fix-them-in-2025-dcebfdef7b54)
- [Rust Design Patterns Book -- Anti-Patterns](https://rust-unofficial.github.io/patterns/anti_patterns/index.html)
- [Don't Make These Mistakes When Writing Rust](https://dev.to/leapcell/dont-make-these-mistakes-when-writing-rust-40o0)
- [9 Rust Pitfalls Every Developer Should Know](https://leapcell.io/blog/nine-rust-pitfalls)
- [Elegant Library APIs in Rust](https://deterministic.space/elegant-apis-in-rust.html)
- [What Not to Do in Rust (Sentry)](https://blog.sentry.io/you-cant-rust-that/)
- [Heap Allocations -- The Rust Performance Book](https://nnethercote.github.io/perf-book/heap-allocations.html)
- [Rust Performance Pitfalls](https://llogiq.github.io/2017/06/01/perf-pitfalls.html)
- [Top 5 Tokio Runtime Mistakes](https://www.techbuddies.io/2026/03/21/top-5-tokio-runtime-mistakes-that-quietly-kill-your-async-rust/)
- [Async: What is blocking? -- Alice Ryhl](https://ryhl.io/blog/async-what-is-blocking/)
- [Learn Unsafe Rust From My Mistakes](https://geo-ant.github.io/blog/2023/unsafe-rust-exploration/)
- [Item 29: Listen to Clippy -- Effective Rust](https://effective-rust.com/clippy.html)
- [Clone to Satisfy the Borrow Checker](https://rust-unofficial.github.io/patterns/anti_patterns/borrow_clone.html)
- [Newtype Pattern -- Rust Design Patterns](https://rust-unofficial.github.io/patterns/patterns/behavioural/newtype.html)
- [The Ultimate Guide to Rust Newtypes](https://www.howtocodeit.com/guides/ultimate-guide-rust-newtypes)
- [Clippy Lint Index](https://rust-lang.github.io/rust-clippy/master/index.html)
