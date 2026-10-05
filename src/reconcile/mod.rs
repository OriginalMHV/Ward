pub mod access_integrations;
pub mod actions_environments;
pub mod files;
pub mod general;
pub mod security_rules;
pub mod unified;

use std::future::Future;

use futures_util::StreamExt;
use futures_util::stream;

/// How many repositories are processed at once. Request concurrency is still
/// bounded by the client's `--parallelism` semaphore.
const REPO_CONCURRENCY: usize = 8;

/// Map items concurrently on the current task and keep the input order.
/// Futures here are not `Send`, so this never spawns.
pub async fn map_buffered<I, T, F, Fut>(items: I, f: F) -> Vec<T>
where
    I: IntoIterator,
    F: FnMut(I::Item) -> Fut,
    Fut: Future<Output = T>,
{
    stream::iter(items)
        .map(f)
        .buffered(REPO_CONCURRENCY)
        .collect()
        .await
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::map_buffered;

    #[tokio::test]
    async fn map_buffered_runs_concurrently_and_keeps_input_order() {
        let delays = [60u64, 10, 40, 20];
        let started = Instant::now();
        let results = map_buffered(delays, |delay| async move {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            delay
        })
        .await;

        assert_eq!(results, delays);
        assert!(
            started.elapsed() < Duration::from_millis(120),
            "sequential execution would take 130 ms"
        );
    }
}
