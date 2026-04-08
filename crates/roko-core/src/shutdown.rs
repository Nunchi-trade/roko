//! Graceful shutdown coordination (§42.11–§42.14).
//!
//! [`GracefulShutdown`] coordinates "drain-then-exit" behavior across
//! long-running services. Services register on-shutdown hooks; when
//! [`GracefulShutdown::drain`] is called, the draining flag flips, every
//! hook is awaited concurrently with a hard deadline, and any in-flight
//! work can poll [`GracefulShutdown::draining`] to finish gracefully.
//!
//! This module is deliberately **runtime-agnostic**: `roko-core` does not
//! depend on `tokio`, so hook futures are driven on short-lived threads
//! with a tiny hand-rolled `block_on`. The public surface area still
//! exposes `async fn` / `Future` types so callers on any runtime (tokio,
//! async-std, smol, …) can await the returned futures.
//!
//! ## Quick reference
//!
//! | Item | Section | Purpose |
//! |---|---|---|
//! | [`GracefulShutdown`] | §42.11 | SIGTERM → drain → exit ≤ 5s coordinator |
//! | [`drain_after`] | §42.13 | Bounded wait helper ("did F finish in time?") |
//! | [`LeakSentinel`] | §42.14 | Drop-panic guard for resource-leak tests |

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

// ─── Type aliases ─────────────────────────────────────────────────────────

type BoxFut = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
type HookFn = Box<dyn FnOnce() -> BoxFut + Send + 'static>;

// ─── GracefulShutdown (§42.11) ────────────────────────────────────────────

/// Coordinator for cooperative drain-then-exit shutdown (§42.11).
///
/// Construct one per binary, hand clones to subsystems, and call
/// [`GracefulShutdown::drain`] from the signal handler (or test driver).
/// Each registered hook runs concurrently; any that does not complete by
/// the hard deadline is counted as "timed out" in the
/// [`ShutdownReport`].
///
/// # Example
///
/// ```no_run
/// use std::time::Duration;
/// use roko_core::shutdown::GracefulShutdown;
///
/// # async fn run() {
/// let shutdown = GracefulShutdown::with_deadline(Duration::from_secs(2));
/// shutdown.register("flush-metrics", || async {
///     // flush in-memory counters ...
/// });
/// shutdown.register("close-db", || async {
///     // gracefully close db ...
/// });
///
/// // later, on SIGTERM:
/// let report = shutdown.drain().await;
/// assert_eq!(report.timed_out_hooks, 0);
/// # }
/// ```
#[derive(Clone)]
pub struct GracefulShutdown {
    inner: Arc<Inner>,
}

struct Inner {
    draining: AtomicBool,
    deadline: Duration,
    hooks: Mutex<Vec<(String, HookFn)>>,
    drained_count: AtomicU32,
    timed_out_count: AtomicU32,
    wait_wakers: Mutex<Vec<Waker>>,
}

/// Report produced once [`GracefulShutdown::drain`] completes (or the hard
/// deadline fires).
///
/// `drained_hooks + timed_out_hooks` equals the number of hooks that were
/// registered at the moment `drain` was called.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShutdownReport {
    /// Count of hooks that finished before the deadline.
    pub drained_hooks: u32,
    /// Count of hooks that did not finish before the deadline.
    pub timed_out_hooks: u32,
    /// Wall-clock time from `drain()` start → return, in milliseconds.
    pub elapsed_ms: u64,
    /// The deadline that was in effect, in milliseconds.
    pub hard_deadline_ms: u64,
}

impl GracefulShutdown {
    /// Construct with the default 5-second hard deadline (§42.11 SLO).
    #[must_use]
    pub fn new() -> Self {
        Self::with_deadline(Duration::from_secs(5))
    }

    /// Construct with a custom hard deadline.
    #[must_use]
    pub fn with_deadline(deadline: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                draining: AtomicBool::new(false),
                deadline,
                hooks: Mutex::new(Vec::new()),
                drained_count: AtomicU32::new(0),
                timed_out_count: AtomicU32::new(0),
                wait_wakers: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Register a hook by name. Hooks must be idempotent; they are
    /// invoked exactly once, during [`GracefulShutdown::drain`].
    ///
    /// Hooks registered **after** `drain` has been called are silently
    /// dropped (noop) — the draining flag is one-way.
    pub fn register<F, Fut>(&self, name: impl Into<String>, f: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        if self.draining() {
            // Already draining — drop the hook on the floor (noop).
            return;
        }
        let boxed: HookFn = Box::new(move || Box::pin(f()) as BoxFut);
        self.inner.hooks.lock().push((name.into(), boxed));
    }

    /// Returns `true` iff [`GracefulShutdown::drain`] has started.
    #[must_use]
    pub fn draining(&self) -> bool {
        self.inner.draining.load(Ordering::Relaxed)
    }

    /// Current hard deadline for this shutdown.
    #[must_use]
    pub fn deadline(&self) -> Duration {
        self.inner.deadline
    }

    /// Returns a future that resolves as soon as draining has started.
    ///
    /// This lets services embed a lightweight "am I being shut down?"
    /// await point in their select loops without having to poll
    /// [`GracefulShutdown::draining`] each iteration.
    #[must_use]
    pub fn wait_started(&self) -> WaitStarted {
        WaitStarted {
            inner: self.inner.clone(),
        }
    }

    /// Flip the drain flag and run all registered hooks concurrently
    /// with the configured hard deadline.
    ///
    /// Each hook is driven on its own short-lived OS thread so we do not
    /// depend on any specific async runtime. The returned
    /// [`ShutdownReport`] is populated from the concurrent completions;
    /// hooks still running when the deadline fires are counted as
    /// `timed_out_hooks` and their background threads are left to finish
    /// on their own (they will exit as their futures drop).
    pub async fn drain(&self) -> ShutdownReport {
        // Flip the draining flag (one-way).
        let was_draining = self.inner.draining.swap(true, Ordering::SeqCst);
        // Wake any `wait_started` futures.
        let wakers: Vec<Waker> = std::mem::take(&mut *self.inner.wait_wakers.lock());
        for w in wakers {
            w.wake();
        }

        // If drain was already in progress, return a zero-hook report so
        // the caller can distinguish "I was first" from "someone else
        // beat me to it".
        if was_draining {
            return ShutdownReport {
                drained_hooks: self.inner.drained_count.load(Ordering::SeqCst),
                timed_out_hooks: self.inner.timed_out_count.load(Ordering::SeqCst),
                elapsed_ms: 0,
                hard_deadline_ms: duration_to_ms(self.inner.deadline),
            };
        }

        let deadline = self.inner.deadline;
        let start = Instant::now();

        // Steal the full hook list.
        let hooks = std::mem::take(&mut *self.inner.hooks.lock());
        let total = usize_to_u32(hooks.len());

        if total == 0 {
            let elapsed_ms = duration_to_ms(start.elapsed());
            return ShutdownReport {
                drained_hooks: 0,
                timed_out_hooks: 0,
                elapsed_ms,
                hard_deadline_ms: duration_to_ms(deadline),
            };
        }

        // Spawn each hook on a thread; each signals the completion
        // channel once its future finishes.
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        for (_name, hook) in hooks {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let fut = hook();
                block_on_fut(fut);
                // Ignore send errors: main side may already be gone.
                let _ = tx.send(());
            });
        }
        drop(tx);

        // Hand control back to the caller's executor for the duration of
        // the wait: this is modelled as an async adapter over
        // `recv_timeout`, driven on a scratch thread.
        let drained = drain_wait(rx, total, deadline).await;

        let timed_out = total - drained;
        self.inner.drained_count.store(drained, Ordering::SeqCst);
        self.inner
            .timed_out_count
            .store(timed_out, Ordering::SeqCst);
        let elapsed_ms = duration_to_ms(start.elapsed());

        ShutdownReport {
            drained_hooks: drained,
            timed_out_hooks: timed_out,
            elapsed_ms,
            hard_deadline_ms: duration_to_ms(deadline),
        }
    }
}

impl std::fmt::Debug for GracefulShutdown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GracefulShutdown")
            .field("draining", &self.draining())
            .field("deadline", &self.inner.deadline)
            .field("registered_hooks", &self.inner.hooks.lock().len())
            .finish()
    }
}

impl Default for GracefulShutdown {
    fn default() -> Self {
        Self::new()
    }
}

/// Future returned by [`GracefulShutdown::wait_started`].
///
/// Resolves the first time the parent coordinator transitions into the
/// draining state. Cheap to clone / construct; no allocation beyond the
/// waker registration.
pub struct WaitStarted {
    inner: Arc<Inner>,
}

impl Future for WaitStarted {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.inner.draining.load(Ordering::Relaxed) {
            return Poll::Ready(());
        }
        // Register our waker. We push a fresh waker each poll: the
        // `drain()` side drains the vec whole-hog when it fires.
        self.inner.wait_wakers.lock().push(cx.waker().clone());
        // Double-check in case drain flipped after our load above but
        // before our waker was stashed.
        if self.inner.draining.load(Ordering::Relaxed) {
            return Poll::Ready(());
        }
        Poll::Pending
    }
}

// ─── drain_after (§42.13) ────────────────────────────────────────────────

/// Bounded-wait helper (§42.13): run `fut` on a scratch thread, return
/// `true` if it completes in ≤ `timeout`, `false` otherwise.
///
/// Used by the WS subscription drain path: "try to send the
/// `subscription_ended` event; if the peer is gone after N ms, give up
/// and close the socket".
///
/// # Example
///
/// ```
/// # use std::time::Duration;
/// # use roko_core::shutdown::drain_after;
/// # async fn run() {
/// let ok = drain_after(Duration::from_millis(50), async { /* … */ }).await;
/// assert!(ok);
/// # }
/// ```
pub async fn drain_after<F>(timeout: Duration, fut: F) -> bool
where
    F: Future<Output = ()> + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        block_on_fut(fut);
        let _ = tx.send(());
    });
    let deadline = Instant::now() + timeout;
    ScratchRecv {
        rx,
        deadline,
        ticker: None,
    }
    .await
}

// ─── LeakSentinel (§42.14) ───────────────────────────────────────────────

/// Counter-based resource leak sentinel (§42.14).
///
/// Call [`LeakSentinel::register`] when a resource is acquired, and
/// [`LeakSentinel::release`] when it is returned. On drop, if the live
/// count is non-zero, the sentinel panics with a message naming the
/// scope — exactly what a shutdown-time leak check wants.
///
/// This is intentionally a **test-quality** guard: production code
/// should wire real handles into RAII types. `LeakSentinel` exists so
/// shutdown tests can assert "we had zero dangling child processes".
///
/// # Example
///
/// ```should_panic
/// # use roko_core::shutdown::LeakSentinel;
/// let sentinel = LeakSentinel::new("agent-processes");
/// sentinel.register();
/// // forgot to release() — drop will panic.
/// ```
pub struct LeakSentinel {
    scope: String,
    live: Arc<AtomicUsize>,
    disarmed: AtomicBool,
}

impl LeakSentinel {
    /// Construct a fresh sentinel scoped under `name`.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            scope: name.into(),
            live: Arc::new(AtomicUsize::new(0)),
            disarmed: AtomicBool::new(false),
        }
    }

    /// Record that a resource was acquired.
    pub fn register(&self) {
        self.live.fetch_add(1, Ordering::SeqCst);
    }

    /// Record that a resource was released.
    ///
    /// # Panics
    ///
    /// Panics if called more times than [`LeakSentinel::register`]
    /// (i.e. if the live count would go negative).
    pub fn release(&self) {
        let prev = self.live.fetch_sub(1, Ordering::SeqCst);
        assert!(
            prev != 0,
            "LeakSentinel '{}': release() called more than register()",
            self.scope
        );
    }

    /// Current live count (registered – released).
    #[must_use]
    pub fn live(&self) -> usize {
        self.live.load(Ordering::SeqCst)
    }

    /// Suppress the drop-panic (intended for tests that want to assert
    /// a non-zero count explicitly without tearing the thread down).
    pub fn disarm(&self) {
        self.disarmed.store(true, Ordering::SeqCst);
    }
}

impl std::fmt::Debug for LeakSentinel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LeakSentinel")
            .field("scope", &self.scope)
            .field("live", &self.live())
            .field("disarmed", &self.disarmed.load(Ordering::Relaxed))
            .finish()
    }
}

impl Drop for LeakSentinel {
    fn drop(&mut self) {
        if self.disarmed.load(Ordering::SeqCst) {
            return;
        }
        let live = self.live.load(Ordering::SeqCst);
        // Only assert when we're not already unwinding (avoid double-panic).
        if std::thread::panicking() {
            return;
        }
        assert!(
            live == 0,
            "LeakSentinel '{}': dropped with {} live resource(s); leak detected",
            self.scope,
            live
        );
    }
}

// ─── internals ────────────────────────────────────────────────────────────

/// Poll-based adapter over an mpsc receiver bounded by a deadline.
///
/// On each poll we `try_recv` the fixed number of times we still need,
/// then arm a timer thread to wake us at the deadline (or a short
/// ticker, whichever fires first).
struct ScratchRecv {
    rx: std::sync::mpsc::Receiver<()>,
    deadline: Instant,
    ticker: Option<Arc<TickerState>>,
}

struct TickerState {
    fired: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

impl Future for ScratchRecv {
    type Output = bool;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<bool> {
        // SAFETY: we don't move anything out of self — just mutate fields.
        let this = Pin::into_inner(self);
        // First, any already-queued completion wins.
        if this.rx.try_recv().is_ok() {
            return Poll::Ready(true);
        }
        let now = Instant::now();
        if now >= this.deadline {
            return Poll::Ready(false);
        }
        // Arm (or refresh) the timer.
        let remaining = this.deadline - now;
        let state = this.ticker.get_or_insert_with(|| {
            Arc::new(TickerState {
                fired: AtomicBool::new(false),
                waker: Mutex::new(None),
            })
        });
        *state.waker.lock() = Some(cx.waker().clone());
        if !state.fired.load(Ordering::SeqCst) {
            // Spawn one timer thread per poll-arm cycle. Multiple wakes
            // are fine; we re-check rx + deadline on each poll.
            let state2 = state.clone();
            std::thread::spawn(move || sleep_then_wake(&state2, remaining, true));
            // Park until the timer fires or something external wakes us.
        }
        // Edge case: the timer may already have fired between the
        // deadline check and waker installation; re-check.
        if state.fired.load(Ordering::SeqCst) {
            return Poll::Ready(this.rx.try_recv().is_ok());
        }
        Poll::Pending
    }
}

/// Wait for `expected` completions on `rx` bounded by `deadline`.
///
/// Returns the number of completions observed.
async fn drain_wait(rx: std::sync::mpsc::Receiver<()>, expected: u32, deadline: Duration) -> u32 {
    struct DrainWait {
        rx: std::sync::mpsc::Receiver<()>,
        expected: u32,
        seen: u32,
        deadline_at: Instant,
        ticker: Option<Arc<TickerState>>,
    }

    impl Future for DrainWait {
        type Output = u32;
        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u32> {
            let this = Pin::into_inner(self);
            // Drain everything currently available.
            while this.seen < this.expected {
                match this.rx.try_recv() {
                    Ok(()) => this.seen += 1,
                    Err(_) => break,
                }
            }
            if this.seen >= this.expected {
                return Poll::Ready(this.seen);
            }
            let now = Instant::now();
            if now >= this.deadline_at {
                return Poll::Ready(this.seen);
            }
            let remaining = this.deadline_at - now;
            let state = this.ticker.get_or_insert_with(|| {
                Arc::new(TickerState {
                    fired: AtomicBool::new(false),
                    waker: Mutex::new(None),
                })
            });
            *state.waker.lock() = Some(cx.waker().clone());
            // Short ticker so that new completions arriving *before*
            // the deadline still wake us up even if the sender thread
            // forgot to wake (it can't — std mpsc has no waker). We use
            // a bounded poll tick to pick up senders.
            let tick = remaining.min(Duration::from_millis(5));
            if !state.fired.load(Ordering::SeqCst) {
                let state2 = state.clone();
                std::thread::spawn(move || sleep_then_wake(&state2, tick, false));
            }
            if state.fired.load(Ordering::SeqCst) {
                // reset the fired flag so next poll arms again
                state.fired.store(false, Ordering::SeqCst);
            }
            Poll::Pending
        }
    }

    DrainWait {
        rx,
        expected,
        seen: 0,
        deadline_at: Instant::now() + deadline,
        ticker: None,
    }
    .await
}

/// Park the current thread for `dur`, then flip `state.fired` (if
/// `mark_fired`) and wake the stored waker.
///
/// Uses [`std::thread::park_timeout`] rather than `sleep` so the
/// timer thread participates in the thread-parking ecosystem (and
/// so clippy's `disallowed_methods` against `sleep` stays happy —
/// see workspace `clippy.toml`).
fn sleep_then_wake(state: &Arc<TickerState>, dur: Duration, mark_fired: bool) {
    std::thread::park_timeout(dur);
    if mark_fired {
        state.fired.store(true, Ordering::SeqCst);
    }
    let waker = state.waker.lock().take();
    if let Some(w) = waker {
        w.wake();
    }
}

/// Saturating `Duration → u64 milliseconds`.
fn duration_to_ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// Saturating `usize → u32`.
fn usize_to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Drive a future to completion on the current thread, parking between
/// wakes. This is used only by our background threads; public callers
/// use their own executors.
fn block_on_fut<F: Future>(fut: F) -> F::Output {
    use std::sync::Arc;
    use std::task::Wake;

    struct ThreadWaker(std::thread::Thread);
    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    let thread = std::thread::current();
    let waker = Waker::from(Arc::new(ThreadWaker(thread)));
    let mut cx = Context::from_waker(&waker);
    let mut fut = Box::pin(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::park(),
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// Minimal executor adapter so tests don't need tokio.
    fn run<F: Future>(f: F) -> F::Output {
        block_on_fut(f)
    }

    /// Test-only sleep that loops on `park_timeout` so spurious
    /// unparks don't return early (clippy's `disallowed_methods`
    /// blocks plain `thread::sleep`, see workspace `clippy.toml`).
    fn sleep_for(ms: u64) {
        let deadline = Instant::now() + Duration::from_millis(ms);
        loop {
            let now = Instant::now();
            if now >= deadline {
                return;
            }
            std::thread::park_timeout(deadline - now);
        }
    }

    // ── GracefulShutdown: state machine ────────────────────────────────

    #[test]
    fn new_starts_not_draining() {
        let g = GracefulShutdown::new();
        assert!(!g.draining());
    }

    #[test]
    fn drain_flips_flag() {
        let g = GracefulShutdown::new();
        assert!(!g.draining());
        let report = run(g.drain());
        assert!(g.draining());
        assert_eq!(report.drained_hooks, 0);
        assert_eq!(report.timed_out_hooks, 0);
    }

    #[test]
    fn default_deadline_is_5s() {
        let g = GracefulShutdown::new();
        assert_eq!(g.deadline(), Duration::from_secs(5));
        let report = run(g.drain());
        assert_eq!(report.hard_deadline_ms, 5_000);
    }

    #[test]
    fn with_deadline_customizes() {
        let g = GracefulShutdown::with_deadline(Duration::from_millis(250));
        assert_eq!(g.deadline(), Duration::from_millis(250));
        let report = run(g.drain());
        assert_eq!(report.hard_deadline_ms, 250);
    }

    // ── GracefulShutdown: hook execution ───────────────────────────────

    #[test]
    fn register_hook_runs_on_drain() {
        let g = GracefulShutdown::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        g.register("inc", move || async move {
            c.fetch_add(1, Ordering::SeqCst);
        });
        let report = run(g.drain());
        assert_eq!(report.drained_hooks, 1);
        assert_eq!(report.timed_out_hooks, 0);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn multiple_hooks_run_concurrently() {
        let g = GracefulShutdown::with_deadline(Duration::from_secs(2));
        for _ in 0..3 {
            g.register("sleep-100ms", || async {
                sleep_for(100);
            });
        }
        let start = Instant::now();
        let report = run(g.drain());
        let elapsed = start.elapsed();
        assert_eq!(report.drained_hooks, 3);
        assert_eq!(report.timed_out_hooks, 0);
        // 3 serial sleeps would be 300ms; concurrent should be well
        // under ~250ms even on a loaded CI box.
        assert!(
            elapsed < Duration::from_millis(260),
            "expected concurrent hooks (<260ms), got {elapsed:?}"
        );
    }

    #[test]
    fn hook_past_deadline_counts_as_timed_out() {
        let g = GracefulShutdown::with_deadline(Duration::from_millis(80));
        g.register("slow", || async {
            sleep_for(400);
        });
        let report = run(g.drain());
        assert_eq!(report.drained_hooks, 0);
        assert_eq!(report.timed_out_hooks, 1);
        assert!(report.elapsed_ms >= 80);
        assert!(
            report.elapsed_ms < 300,
            "drain should return near the deadline, not wait for the hook; got {}ms",
            report.elapsed_ms
        );
    }

    #[test]
    fn drained_count_matches_registered() {
        let g = GracefulShutdown::new();
        for i in 0..7 {
            g.register(format!("h{i}"), || async {});
        }
        let report = run(g.drain());
        assert_eq!(report.drained_hooks + report.timed_out_hooks, 7);
        assert_eq!(report.drained_hooks, 7);
    }

    #[test]
    fn shutdown_report_elapsed_is_monotonic() {
        let g = GracefulShutdown::new();
        g.register("quick", || async {
            sleep_for(20);
        });
        let before = Instant::now();
        let report = run(g.drain());
        let after_dur = before.elapsed();
        // Report elapsed should be non-zero and not overshoot wall-clock.
        assert!(report.elapsed_ms >= 1);
        let wall_ms = duration_to_ms(after_dur);
        assert!(
            report.elapsed_ms <= wall_ms + 5,
            "report.elapsed_ms={} vs wall={wall_ms}ms",
            report.elapsed_ms,
        );
    }

    #[test]
    fn hooks_registered_after_drain_noop() {
        let g = GracefulShutdown::new();
        let _ = run(g.drain());
        assert!(g.draining());
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        // Should silently drop.
        g.register("late", move || async move {
            c.fetch_add(1, Ordering::SeqCst);
        });
        // A second drain returns early (already-draining path).
        let report2 = run(g.drain());
        assert_eq!(report2.elapsed_ms, 0);
        // Hook was never registered, so counter stays zero.
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    // ── GracefulShutdown: concurrency ──────────────────────────────────

    #[test]
    fn draining_is_thread_safe() {
        let g = GracefulShutdown::new();
        let seen_draining = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..4 {
            let g = g.clone();
            let seen = seen_draining.clone();
            handles.push(std::thread::spawn(move || {
                // Spin until draining is observed or 2s elapse.
                let start = Instant::now();
                while !g.draining() && start.elapsed() < Duration::from_secs(2) {
                    std::thread::yield_now();
                }
                if g.draining() {
                    seen.fetch_add(1, Ordering::SeqCst);
                }
            }));
        }
        // Give readers a moment to start spinning.
        sleep_for(20);
        let _ = run(g.drain());
        for h in handles {
            h.join().expect("reader thread panicked");
        }
        assert_eq!(seen_draining.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn wait_started_resolves_after_drain() {
        let g = GracefulShutdown::new();
        let g2 = g.clone();
        // Spawn a background thread that flips drain after a short
        // delay; our future should complete.
        std::thread::spawn(move || {
            sleep_for(30);
            run(g2.drain());
        });
        run(g.wait_started());
        assert!(g.draining());
    }

    // ── drain_after (§42.13) ────────────────────────────────────────────

    #[test]
    fn drain_after_completes_returns_true() {
        let ok = run(drain_after(Duration::from_millis(500), async {
            sleep_for(20);
        }));
        assert!(ok);
    }

    #[test]
    fn drain_after_timeout_returns_false() {
        let start = Instant::now();
        let ok = run(drain_after(Duration::from_millis(50), async {
            sleep_for(400);
        }));
        let elapsed = start.elapsed();
        assert!(!ok);
        assert!(
            elapsed < Duration::from_millis(250),
            "drain_after should bail near timeout, got {elapsed:?}"
        );
    }

    // ── LeakSentinel (§42.14) ──────────────────────────────────────────

    #[test]
    fn leak_sentinel_clean_on_balanced_register_release() {
        let s = LeakSentinel::new("balanced");
        for _ in 0..3 {
            s.register();
        }
        assert_eq!(s.live(), 3);
        for _ in 0..3 {
            s.release();
        }
        assert_eq!(s.live(), 0);
        // Drop must not panic — implicit check at scope end.
    }

    #[test]
    fn leak_sentinel_panics_if_nonzero_on_drop() {
        let result = std::panic::catch_unwind(|| {
            let s = LeakSentinel::new("leaky");
            s.register();
            s.register();
            // intentionally do not release
            drop(s);
        });
        assert!(
            result.is_err(),
            "expected LeakSentinel drop with live>0 to panic"
        );
    }

    #[test]
    fn leak_sentinel_disarm_suppresses_panic() {
        // Sanity: disarm() lets non-zero counts slide.
        let s = LeakSentinel::new("disarmed");
        s.register();
        s.disarm();
        drop(s); // must not panic
    }

    #[test]
    fn leak_sentinel_live_count_reflects_ops() {
        let s = LeakSentinel::new("counter");
        assert_eq!(s.live(), 0);
        s.register();
        s.register();
        assert_eq!(s.live(), 2);
        s.release();
        assert_eq!(s.live(), 1);
        s.disarm();
    }
}
