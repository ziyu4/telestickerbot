//! Retry logic with exponential backoff for Telegram API calls

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// Maximum number of retry attempts for Telegram API calls
const MAX_RETRIES: u32 = 3;

/// Initial backoff delay in milliseconds
const INITIAL_BACKOFF_MS: u64 = 1000;

/// Executes an async operation with exponential backoff retry logic.
///
/// This function will retry a failing operation up to MAX_RETRIES times,
/// with the delay between retries doubling each time (exponential backoff).
///
/// # Arguments
///
/// * `f` - A closure that returns a pinned future representing the async operation
///
/// # Returns
///
/// * `Ok(T)` - The successful result of the operation
/// * `Err(E)` - The final error after all retries are exhausted
///
/// # Example
///
/// ```ignore
/// use crate::bot::retry::with_retry;
///
/// async fn call_telegram_api() -> Result<String, ApiError> {
///     // Some API call
/// }
///
/// let result = with_retry(|| Box::pin(call_telegram_api())).await?;
/// ```
///
/// # Retry Behavior
///
/// - Attempt 1: Immediate
/// - Attempt 2: After 1000ms (1 second)
/// - Attempt 3: After 2000ms (2 seconds)
///
/// Total maximum delay before final failure: ~3 seconds
pub async fn with_retry<F, T, E>(f: F) -> Result<T, E>
where
    F: Fn() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>,
    E: std::fmt::Debug,
{
    let mut delay = INITIAL_BACKOFF_MS;

    for attempt in 0..MAX_RETRIES {
        match f().await {
            Ok(result) => return Ok(result),
            Err(e) if attempt < MAX_RETRIES - 1 => {
                tracing::warn!(
                    attempt = attempt + 1,
                    max_retries = MAX_RETRIES,
                    backoff_ms = delay,
                    error = ?e,
                    "Operation failed, retrying with exponential backoff"
                );
                tokio::time::sleep(Duration::from_millis(delay)).await;
                delay *= 2;
            }
            Err(e) => {
                tracing::error!(
                    attempts = MAX_RETRIES,
                    error = ?e,
                    "Operation failed after all retry attempts"
                );
                return Err(e);
            }
        }
    }

    unreachable!("Loop always returns before exhausting all iterations")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[derive(Debug, Clone, PartialEq)]
    struct TestError;

    #[tokio::test]
    async fn test_retry_succeeds_on_first_attempt() {
        let call_count = Arc::new(AtomicU32::new(0));
        let count_clone = call_count.clone();

        let result = with_retry(|| {
            let count = count_clone.clone();
            Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
                Ok::<_, TestError>("success")
            })
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_succeeds_after_failures() {
        let call_count = Arc::new(AtomicU32::new(0));
        let count_clone = call_count.clone();

        let result = with_retry(|| {
            let count = count_clone.clone();
            Box::pin(async move {
                let current = count.fetch_add(1, Ordering::SeqCst);
                if current < 2 {
                    Err(TestError)
                } else {
                    Ok::<_, TestError>("success")
                }
            })
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
        assert_eq!(call_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_retry_fails_after_max_retries() {
        let call_count = Arc::new(AtomicU32::new(0));
        let count_clone = call_count.clone();

        let result = with_retry(|| {
            let count = count_clone.clone();
            Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(TestError)
            })
        })
        .await;

        assert!(result.is_err());
        assert_eq!(call_count.load(Ordering::SeqCst), MAX_RETRIES);
    }

    #[tokio::test]
    async fn test_exponential_backoff_timing() {
        use tokio::time::{Instant, Duration};

        let call_count = Arc::new(AtomicU32::new(0));
        let count_clone = call_count.clone();

        let start = Instant::now();

        let result = with_retry(|| {
            let count = count_clone.clone();
            Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(TestError)
            })
        })
        .await;

        let elapsed = start.elapsed();

        assert!(result.is_err());
        // Should have waited: 1000ms + 2000ms = 3000ms between retries
        // Allow some tolerance for test timing
        let expected_min = Duration::from_millis(INITIAL_BACKOFF_MS + INITIAL_BACKOFF_MS * 2);
        assert!(
            elapsed >= expected_min.saturating_sub(Duration::from_millis(100)),
            "Elapsed time {:?} should be at least {:?}",
            elapsed,
            expected_min
        );
    }
}