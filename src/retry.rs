use std::thread::sleep;
use std::time::Duration;

pub fn retry_with_backoff<T, E: std::fmt::Display>(
    max_retries: u32,
    base_sleep: Duration,
    operation_name: &str,
    mut f: impl FnMut() -> Result<T, E>,
) -> Result<T, String> {
    let mut last_err = String::new();
    for attempt in 0..max_retries {
        if attempt > 0 {
            let backoff = base_sleep * 2u32.pow((attempt - 1).min(5));
            // Add jitter: random between 50-100% of backoff
            let jitter_ms = backoff.as_millis() as u64 / 2;
            let sleep_time = backoff - Duration::from_millis(jitter_ms / 2);
            log::warn!(
                "{} failed, retrying in {:.1}s (attempt {}/{})...",
                operation_name,
                sleep_time.as_secs_f64(),
                attempt + 1,
                max_retries
            );
            sleep(sleep_time);
        }
        match f() {
            Ok(val) => return Ok(val),
            Err(e) => last_err = format!("{} failed: {}", operation_name, e),
        }
    }
    Err(last_err)
}
