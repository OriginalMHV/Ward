use std::time::Duration;

pub(crate) const REST_API_VERSION: &str = "2022-11-28";
pub(crate) const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
pub(crate) const MAX_RETRY_ATTEMPTS: usize = 4;
pub(crate) const RETRY_BACKOFF_SCHEDULE: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];
