//! One bounded pool shared by merge jobs; nested work does not create extra pools.
use rayon::ThreadPool;
use std::sync::OnceLock;
use std::sync::{Condvar, Mutex};

pub fn workers() -> usize {
    std::env::var("LOCUS_MERGE_WORKERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map_or(8, usize::from)
                .saturating_mul(2)
                .max(8)
        })
        .clamp(1, 32)
}

pub fn pool() -> &'static ThreadPool {
    static POOL: OnceLock<ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers())
            .thread_name(|index| format!("merge-io-{index}"))
            .build()
            .expect("merge worker pool")
    })
}

/// Bound simultaneous whole-file allocations to roughly 256 MiB. One oversized
/// file can proceed alone instead of deadlocking while asking for extra permits.
pub fn with_bytes<T>(bytes: u64, work: impl FnOnce() -> T) -> T {
    static MEMORY: (Mutex<usize>, Condvar) = (Mutex::new(256), Condvar::new());
    let units = bytes.div_ceil(1024 * 1024).clamp(1, 256) as usize;
    let mut available = MEMORY.0.lock().unwrap_or_else(|e| e.into_inner());
    while *available < units {
        available = MEMORY.1.wait(available).unwrap_or_else(|e| e.into_inner());
    }
    *available -= units;
    drop(available);
    struct Permit(usize);
    impl Drop for Permit {
        fn drop(&mut self) {
            *MEMORY.0.lock().unwrap_or_else(|e| e.into_inner()) += self.0;
            MEMORY.1.notify_all();
        }
    }
    let _permit = Permit(units);
    work()
}
