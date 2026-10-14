use super::QueryError;
use std::time::{Duration, Instant};
pub const HTML_LIMIT: usize = 4 * 1024 * 1024;
pub fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .pool_idle_timeout(Duration::from_secs(30)).pool_max_idle_per_host(4)
        .tcp_keepalive(Duration::from_secs(60)).tcp_nodelay(true).gzip(true)
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let target = attempt.url();
            if attempt.previous().len() > 5 || target.scheme() != "https" ||
                target.host_str() != Some("dictionary.cambridge.org") || target.port_or_known_default() != Some(443) ||
                !target.username().is_empty() || target.password().is_some() {
                attempt.error("Unexpected Cambridge redirect")
            } else { attempt.follow() }
        }))
}
fn transport_error(error: reqwest::Error) -> QueryError {
    if error.is_timeout() { QueryError::timeout() } else { QueryError::unavailable() }
}
pub async fn read_html(client: &reqwest::Client, url: reqwest::Url, deadline: Instant) -> Result<String, QueryError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() { return Err(QueryError::timeout()); }
    let read = async {
        let mut response = client.get(url).send().await.map_err(transport_error)?;
        match response.status().as_u16() {
            200 => {}, 404 => return Err(QueryError::not_found()),
            403 | 429 => return Err(QueryError::blocked()), _ => return Err(QueryError::unavailable()),
        }
        if response.content_length().is_some_and(|n| n > HTML_LIMIT as u64) { return Err(QueryError::invalid_page()); }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            if bytes.len() + chunk.len() > HTML_LIMIT { return Err(QueryError::invalid_page()); }
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes).map_err(|_| QueryError::invalid_page())
    };
    tokio::time::timeout(remaining, read).await.map_err(|_| QueryError::timeout())?
}
