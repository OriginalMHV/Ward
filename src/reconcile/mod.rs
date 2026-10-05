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
