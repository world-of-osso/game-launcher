//! Transfer failures retain whether retrying a GET is appropriate and its server delay.

use std::time::{Duration, SystemTime};

#[derive(Debug)]
pub(crate) struct TransferFailure {
    pub message: String,
    pub retry_after: Option<Duration>,
}

impl From<String> for TransferFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            retry_after: None,
        }
    }
}

impl TransferFailure {
    pub fn network(path: &str, error: reqwest::Error) -> Self {
        let retryable = error.is_timeout() || error.is_connect() || error.is_body();
        Self {
            message: format!("Transfer {path}: {error}"),
            retry_after: retryable.then_some(Duration::ZERO),
        }
    }

    pub fn status(path: &str, response: &reqwest::Response) -> Self {
        let status = response.status();
        let retryable = matches!(
            status,
            reqwest::StatusCode::REQUEST_TIMEOUT | reqwest::StatusCode::TOO_MANY_REQUESTS
        ) || status.is_server_error();
        let delay = match read_server_retry_delay(path, response) {
            Ok(delay) => delay,
            Err(message) => return message.into(),
        };
        Self {
            message: format!("Download {path}: HTTP {status}"),
            retry_after: retryable.then_some(delay),
        }
    }
}

fn read_server_retry_delay(path: &str, response: &reqwest::Response) -> Result<Duration, String> {
    let Some(header) = response.headers().get(reqwest::header::RETRY_AFTER) else {
        return Ok(Duration::ZERO);
    };
    header
        .to_str()
        .ok()
        .and_then(|value| parse_retry_after(value, SystemTime::now()))
        .ok_or_else(|| format!("Invalid Retry-After for {path}"))
}

fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let date = httpdate::parse_http_date(value).ok()?;
    Some(date.duration_since(now).unwrap_or(Duration::ZERO))
}

pub(crate) fn retry_delay(attempt: u32, server_delay: Duration) -> Result<Duration, String> {
    let backoff = Duration::from_millis(250 * 2u64.pow(attempt));
    let clock = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| format!("Read retry clock: {error}"))?;
    let jitter = clock.subsec_nanos() as u64 % 100;
    Ok(server_delay.max(backoff) + Duration::from_millis(jitter))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_supports_dates_seconds_and_past_dates() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_445_412_470);
        assert_eq!(parse_retry_after("10", now), Some(Duration::from_secs(10)));
        assert_eq!(
            parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT", now),
            Some(Duration::from_secs(10))
        );
        assert_eq!(
            parse_retry_after(
                "Wed, 21 Oct 2015 07:28:00 GMT",
                now + Duration::from_secs(20)
            ),
            Some(Duration::ZERO)
        );
        assert_eq!(parse_retry_after("not a date", now), None);
    }
}
