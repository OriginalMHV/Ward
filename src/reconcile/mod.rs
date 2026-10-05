pub mod access_integrations;
pub mod actions_environments;
pub mod files;
pub mod general;
pub mod security_rules;
pub mod unified;

use std::future::Future;

use crate::github::actions::ReadOutcome;
use futures_util::StreamExt;
use futures_util::stream;

/// How many repositories are processed at once. Request concurrency is still
/// bounded by the client's `--parallelism` semaphore.
const REPO_CONCURRENCY: usize = 8;

/// Downgrade a failed read of an endpoint the manifest does not ask for.
///
/// A permission or availability failure on such an endpoint is not state Ward
/// manages, so it must not make a managed category count as unknown. The reason
/// stays in the coverage record.
pub(crate) fn relax_unrequested<T>(outcome: ReadOutcome<T>, requested: bool) -> ReadOutcome<T> {
    match outcome {
        ReadOutcome::PermissionDenied(reason) | ReadOutcome::Unavailable(reason) if !requested => {
            ReadOutcome::NotApplicable(format!("not required by the manifest: {reason}"))
        }
        other => other,
    }
}

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
    use std::time::Duration;

    use tokio::sync::Barrier;

    use super::map_buffered;

    #[tokio::test]
    async fn map_buffered_runs_concurrently_and_keeps_input_order() {
        let delays = [60u64, 10, 40, 20];
        // Every item waits for all the others, so the run completes only if they overlap.
        let barrier = Barrier::new(delays.len());
        let run = map_buffered(delays, |delay| {
            let barrier = &barrier;
            async move {
                barrier.wait().await;
                delay
            }
        });

        let results = tokio::time::timeout(Duration::from_secs(10), run)
            .await
            .expect("items did not run concurrently");

        assert_eq!(results, delays);
    }
}
