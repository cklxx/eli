//! Control plane: turn-scoped context and budget infrastructure.
//!
//! The framework sets a [`TurnContext`] via task-local before each turn.
//! Agent internals read it transparently — no parameter threading needed.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use nexil::{CancellationToken, Tool};

/// Closure type for tool wrapping.
pub type WrapToolsFn = Arc<dyn Fn(Vec<Tool>) -> Vec<Tool> + Send + Sync>;

// ---------------------------------------------------------------------------
// Task-local turn context
// ---------------------------------------------------------------------------

tokio::task_local! {
    static TURN_CTX: TurnContext;
}

/// Per-turn usage accumulator (input, output tokens).
#[derive(Clone, Default)]
pub struct TurnUsage {
    input: Arc<AtomicU64>,
    output: Arc<AtomicU64>,
}

impl TurnUsage {
    pub fn record(&self, input_tokens: u64, output_tokens: u64) {
        self.input.fetch_add(input_tokens, Ordering::Relaxed);
        self.output.fetch_add(output_tokens, Ordering::Relaxed);
    }

    pub fn input_tokens(&self) -> u64 {
        self.input.load(Ordering::Relaxed)
    }

    pub fn output_tokens(&self) -> u64 {
        self.output.load(Ordering::Relaxed)
    }

    pub fn total_tokens(&self) -> u64 {
        self.input_tokens() + self.output_tokens()
    }
}

/// Per-turn context set by the framework, read by agent internals.
pub struct TurnContext {
    pub cancellation: CancellationToken,
    pub wrap_tools: Option<WrapToolsFn>,
    pub usage: TurnUsage,
}

/// Run `fut` with the given [`TurnContext`] bound to the current task.
pub async fn with_turn_context<F, T>(ctx: TurnContext, fut: F) -> T
where
    F: Future<Output = T>,
{
    TURN_CTX.scope(ctx, fut).await
}

/// Read the cancellation token from the current turn, if any.
pub fn turn_cancellation() -> Option<CancellationToken> {
    TURN_CTX.try_with(|ctx| ctx.cancellation.clone()).ok()
}

/// Read the usage accumulator from the current turn, if any.
pub fn turn_usage() -> Option<TurnUsage> {
    TURN_CTX.try_with(|ctx| ctx.usage.clone()).ok()
}

/// Record token usage into the current turn context.
pub fn record_turn_usage(input_tokens: u64, output_tokens: u64) {
    let _ = TURN_CTX.try_with(|ctx| ctx.usage.record(input_tokens, output_tokens));
}

/// Read the tool-wrapping function from the current turn, if any.
pub fn turn_wrap_tools() -> Option<WrapToolsFn> {
    TURN_CTX
        .try_with(|ctx| ctx.wrap_tools.clone())
        .ok()
        .flatten()
}

// ---------------------------------------------------------------------------
// Budget ledger
// ---------------------------------------------------------------------------

/// Atomic token budget. Concurrent-safe, independent of State HashMap.
pub struct BudgetLedger {
    remaining: AtomicU64,
    consumed: AtomicU64,
}

impl BudgetLedger {
    /// Unlimited budget (default).
    pub fn new() -> Self {
        Self {
            remaining: AtomicU64::new(u64::MAX),
            consumed: AtomicU64::new(0),
        }
    }

    /// Fixed token budget.
    pub fn with_budget(max_tokens: u64) -> Self {
        Self {
            remaining: AtomicU64::new(max_tokens),
            consumed: AtomicU64::new(0),
        }
    }

    /// Atomically spend `amount` tokens. Returns `true` if budget allowed it.
    pub fn try_spend(&self, amount: u64) -> bool {
        loop {
            let current = self.remaining.load(Ordering::Acquire);
            if current < amount {
                return false;
            }
            if self
                .remaining
                .compare_exchange_weak(
                    current,
                    current - amount,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                self.consumed.fetch_add(amount, Ordering::Relaxed);
                return true;
            }
        }
    }

    pub fn remaining(&self) -> u64 {
        self.remaining.load(Ordering::Relaxed)
    }

    pub fn consumed(&self) -> u64 {
        self.consumed.load(Ordering::Relaxed)
    }
}

impl Default for BudgetLedger {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_unlimited_by_default() {
        let b = BudgetLedger::new();
        assert!(b.try_spend(1_000_000));
        assert_eq!(b.consumed(), 1_000_000);
    }

    #[test]
    fn budget_rejects_overspend() {
        let b = BudgetLedger::with_budget(100);
        assert!(b.try_spend(60));
        assert!(!b.try_spend(60));
        assert_eq!(b.remaining(), 40);
        assert_eq!(b.consumed(), 60);
    }

    #[test]
    fn budget_concurrent_spend() {
        use std::sync::Arc;
        let b = Arc::new(BudgetLedger::with_budget(1000));
        let handles: Vec<_> = (0..100)
            .map(|_| {
                let b = Arc::clone(&b);
                std::thread::spawn(move || b.try_spend(10))
            })
            .collect();
        let successes: usize = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|&ok| ok)
            .count();
        assert_eq!(successes, 100);
        assert_eq!(b.remaining(), 0);
        assert_eq!(b.consumed(), 1000);
    }

    #[tokio::test]
    async fn turn_context_propagates() {
        let token = CancellationToken::new();
        let token2 = token.clone();
        let ctx = TurnContext {
            cancellation: token,
            wrap_tools: None,
            usage: Default::default(),
        };
        let result = with_turn_context(ctx, async {
            let t = turn_cancellation().unwrap();
            assert!(!t.is_cancelled());
            token2.cancel();
            assert!(t.is_cancelled());
            true
        })
        .await;
        assert!(result);
    }

    #[tokio::test]
    async fn turn_context_absent_returns_none() {
        assert!(turn_cancellation().is_none());
        assert!(turn_wrap_tools().is_none());
    }
}
