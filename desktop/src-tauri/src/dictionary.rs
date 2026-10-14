//! Native Cambridge client. Shared connection pool; no query process, Node or local server.
#[path = "dictionary/http.rs"] mod http;
#[path = "dictionary/parser.rs"] pub mod parser;
use serde_json::{json, Value};
use std::{sync::{atomic::{AtomicBool, AtomicU64, Ordering}, Arc, Mutex}, time::{Duration, Instant}};
use tokio::sync::{Notify, Semaphore};
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const SUGGESTION_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub struct QueryError {
    pub message: String,
    pub status: Option<u64>,
    pub code: Option<String>,
    pub suggestions: Vec<String>,
}
impl QueryError {
    fn new(status: u64, code: &str) -> Self {
        let message = match (status, code) {
            (400, _) => "查询词或词库无效",
            (404, _) => "词典服务返回 HTTP 404；未找到此词条",
            (504, _) => "查询超时，请重试",
            (502, "INVALID_CAMBRIDGE_PAGE") => "词典网页内容不完整，请重试",
            (502, "CAMBRIDGE_BLOCKED") => "Cambridge 暂时拒绝访问，请稍后重试",
            _ => "词典连接失败，请检查网络或代理后重试",
        };
        Self { message:message.into(), status:Some(status), code:Some(code.into()), suggestions:Vec::new() }
    }
    fn not_found() -> Self { Self::new(404, "WORD_NOT_FOUND") }
    fn invalid_page() -> Self { Self::new(502, "INVALID_CAMBRIDGE_PAGE") }
    fn blocked() -> Self { Self::new(502, "CAMBRIDGE_BLOCKED") }
    fn unavailable() -> Self { Self::new(502, "CAMBRIDGE_UNAVAILABLE") }
    fn timeout() -> Self { Self::new(504, "CAMBRIDGE_TIMEOUT") }
    fn is_missing(&self) -> bool { self.status == Some(404) && self.code.as_deref() == Some("WORD_NOT_FOUND") }
}
impl From<String> for QueryError {
    fn from(message: String) -> Self { Self { message, status: None, code: None, suggestions: Vec::new() } }
}
impl From<&str> for QueryError { fn from(message: &str) -> Self { message.to_owned().into() } }
impl std::fmt::Display for QueryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.message.fmt(formatter) }
}

pub struct Backend {
    client: Mutex<Option<reqwest::Client>>,
    closed: AtomicBool,
    close_signal: Notify,
    slots: Arc<Semaphore>,
    requests: AtomicU64,
    timeout: Duration,
    suggestion_timeout: Duration,
    #[cfg(test)] origin: Option<reqwest::Url>,
}
impl Default for Backend {
    fn default() -> Self {
        Self { client:Mutex::new(None), closed:AtomicBool::new(false), close_signal:Notify::new(),
            slots:Arc::new(Semaphore::new(2)), requests:AtomicU64::new(0), timeout:REQUEST_TIMEOUT,
            suggestion_timeout:SUGGESTION_TIMEOUT, #[cfg(test)] origin:None }
    }
}
impl Backend {
    fn client(&self) -> Result<reqwest::Client, QueryError> {
        let mut client = self.client.lock().map_err(|_| QueryError::unavailable())?;
        if self.closed.load(Ordering::SeqCst) { return Err("词典已关闭".into()); }
        if client.is_none() { *client = Some(http::client_builder().build().map_err(|_| QueryError::unavailable())?); }
        Ok(client.as_ref().unwrap().clone())
    }
    fn transport_url(&self, url: reqwest::Url) -> reqwest::Url {
        #[cfg(test)] if let Some(origin) = &self.origin {
            let mut local = origin.clone(); local.set_path(url.path()); local.set_query(url.query()); return local;
        }
        url
    }
    async fn read(&self, client: &reqwest::Client, url: reqwest::Url, deadline: Instant) -> Result<String, QueryError> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        http::read_html(client, self.transport_url(url), deadline).await
    }
    async fn execute(&self, word: &str, language: &str, deadline: Instant) -> Result<Value, QueryError> {
        let permit = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), self.slots.clone().acquire_owned()).await
            .map_err(|_| QueryError::timeout())?.map_err(|_| QueryError::from("词典已关闭"))?;
        let client = self.client()?;
        let url = parser::dictionary_url(language, word)?;
        let exact = match self.read(&client, url, deadline).await {
            Ok(html) => {
                // CPU-only, bounded HTML work runs away from the webview/async executor.
                // Keep the slot inside the closure if its caller cancels while parsing.
                let parsing = tauri::async_runtime::spawn_blocking(move || (parser::parse_page(&html), permit));
                let (result, permit) = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), parsing).await
                    .map_err(|_| QueryError::timeout())?.map_err(|_| QueryError::invalid_page())?;
                match result { Ok(entry) => return Ok(entry), Err(error) => (error, permit) }
            }
            Err(error) => (error, permit),
        };
        let (mut error, permit) = exact;
        if !error.is_missing() { return Err(error); }
        let remaining = deadline.saturating_duration_since(Instant::now()).min(self.suggestion_timeout);
        if !remaining.is_zero() {
            let optional = async {
                let html = self.read(&client, parser::suggestion_url(language, word)?, Instant::now() + remaining).await?;
                let language = language.to_owned(); let word = word.to_owned();
                tauri::async_runtime::spawn_blocking(move || {
                    let _permit = permit;
                    parser::parse_suggestions(&html, &language, &word)
                }).await.map_err(|_| QueryError::invalid_page())?
            };
            if let Ok(Ok(suggestions)) = tokio::time::timeout(remaining, optional).await { error.suggestions = suggestions; }
        }
        Err(error)
    }
    pub async fn query(&self, word: &str, language: &str) -> Result<Value, QueryError> {
        // Validate before creating a pool or occupying a slot. UI normalization is separate.
        if !["en-cn", "en"].contains(&language) { return Err(QueryError::new(400, "UNSUPPORTED_LANGUAGE")); }
        parser::dictionary_url(language, word)?;
        let closing = self.close_signal.notified(); tokio::pin!(closing); closing.as_mut().enable();
        if self.closed.load(Ordering::SeqCst) { return Err("词典已关闭".into()); }
        let deadline = Instant::now() + self.timeout;
        tokio::select! {
            _ = closing => Err("查询已取消".into()),
            result = self.execute(word, language, deadline) => result
        }
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst); self.slots.close(); self.close_signal.notify_waiters();
        if let Ok(mut client) = self.client.lock() { *client = None; }
    }
    pub fn diagnostic(&self) -> Value {
        json!({"mode":"native-cambridge","version":env!("CARGO_PKG_VERSION"),"workerPid":null,"runtimeFolder":null,
            "clientReady":self.client.lock().unwrap().is_some(),"httpRequests":self.requests.load(Ordering::SeqCst),"closed":self.closed.load(Ordering::SeqCst)})
    }
}
impl Drop for Backend { fn drop(&mut self) { self.close(); } }

#[cfg(test)] #[path = "dictionary/tests.rs"] mod tests;
