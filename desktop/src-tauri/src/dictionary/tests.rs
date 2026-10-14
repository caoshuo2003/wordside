use super::*;
use std::sync::atomic::AtomicUsize;
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpListener, task::{JoinHandle, JoinSet}};
#[derive(Clone)]
struct Reply { status:u16, headers:String, body:Vec<u8>, delay:Duration }
impl Reply {
    fn page(body: &str) -> Self { Self { status:200, headers:String::new(), body:body.as_bytes().to_vec(), delay:Duration::ZERO } }
    fn status(status:u16) -> Self { Self { status, ..Self::page("") } }
    fn slow(mut self, ms:u64) -> Self { self.delay=Duration::from_millis(ms); self }
}
struct Server { url:reqwest::Url, connections:Arc<AtomicUsize>, requests:Arc<Mutex<Vec<String>>>, task:JoinHandle<()> }
impl Drop for Server { fn drop(&mut self) { self.task.abort(); } }
async fn server(reply:impl Fn(&str)->Reply+Send+Sync+'static) -> Server {
    let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url=reqwest::Url::parse(&format!("http://{}/",listener.local_addr().unwrap())).unwrap();
    let connections=Arc::new(AtomicUsize::new(0)); let requests=Arc::new(Mutex::new(Vec::new()));
    let counter=connections.clone(); let recorded=requests.clone(); let reply=Arc::new(reply);
    let task=tokio::spawn(async move {
        let mut handlers=JoinSet::new();
        loop {
            tokio::select! {
                stream=listener.accept()=>{
                    let (mut stream,_)=stream.unwrap(); counter.fetch_add(1,Ordering::SeqCst);
                    let recorded=recorded.clone(); let reply=reply.clone();
                    handlers.spawn(async move {
                        loop {
                            let mut request=Vec::new(); let mut byte=[0];
                            while !request.ends_with(b"\r\n\r\n") {
                                if stream.read_exact(&mut byte).await.is_err() { return; }
                                request.push(byte[0]); if request.len()>16384 { return; }
                            }
                            let request=String::from_utf8(request).unwrap();
                            let path=request.split_whitespace().nth(1).unwrap().to_string();
                            recorded.lock().unwrap().push(path.clone());
                            let response=reply(&path); tokio::time::sleep(response.delay).await;
                            let head=format!("HTTP/1.1 {} OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n{}\r\n",response.status,response.body.len(),response.headers);
                            if stream.write_all(head.as_bytes()).await.is_err() || stream.write_all(&response.body).await.is_err() { return; }
                        }
                    });
                },
                _=handlers.join_next(),if !handlers.is_empty()=>{},
            }
        }
    });
    Server { url,connections,requests,task }
}
fn fixtures()->Value { serde_json::from_str(include_str!("../../tests/fixtures/native-cambridge.json")).unwrap() }
fn entry()->String { fixtures()["pages"][0]["html"].as_str().unwrap().to_owned() }
fn landing()->String { fixtures()["pages"].as_array().unwrap().iter().find(|x|x["name"]=="landing").unwrap()["html"].as_str().unwrap().to_owned() }
fn candidates()->String { fixtures()["suggestions"][0]["html"].as_str().unwrap().to_owned() }
fn backend(server:&Server,timeout:u64)->Backend {
    let mut b=Backend::default();b.origin=Some(server.url.clone());b.timeout=Duration::from_millis(timeout);
    *b.client.lock().unwrap()=Some(http::client_builder().no_proxy().build().unwrap());b
}
#[tokio::test] async fn native_pool_reuses_connection_without_worker_or_staging() {
    let html=entry();let s=server(move |_|Reply::page(&html)).await;let b=backend(&s,2000);
    for _ in 0..2 { assert_eq!(b.query("example","en-cn").await.unwrap(),fixtures()["pages"][0]["entry"]); }
    assert_eq!(s.connections.load(Ordering::SeqCst),1);assert_eq!(s.requests.lock().unwrap().len(),2);
    assert!(b.diagnostic()["workerPid"].is_null());assert!(b.diagnostic()["runtimeFolder"].is_null());
    b.close();assert_eq!(b.diagnostic()["clientReady"],false);assert!(b.query("example","en").await.is_err());
}
#[tokio::test] async fn confirmed_missing_retains_candidates_in_the_desktop_payload() {
    let landing=landing();let suggestions=candidates();let s=server(move |path|if path.contains("spellcheck"){Reply::page(&suggestions)}else{Reply::page(&landing)}).await;
    let b=backend(&s,2000);let payload=crate::lookup::fetch_payload(&b,"termnal","en").await.unwrap();
    assert!(payload["entry"].is_null());assert_eq!(payload["apiStatus"],404);assert_eq!(payload["apiCode"],"WORD_NOT_FOUND");assert_eq!(payload["suggestions"],json!(["terminal","terminals"]));
    assert_eq!(s.requests.lock().unwrap().len(),2);assert_eq!(s.connections.load(Ordering::SeqCst),1);
}
#[tokio::test] async fn primary_failures_do_not_request_suggestions_and_next_request_recovers() {
    for (reply,status,code) in [(Reply::status(403),502,"CAMBRIDGE_BLOCKED"),(Reply::status(429),502,"CAMBRIDGE_BLOCKED"),(Reply::status(503),502,"CAMBRIDGE_UNAVAILABLE"),(Reply::page("<html></html>"),502,"INVALID_CAMBRIDGE_PAGE"),(Reply::page("<html><title>Just a moment</title></html>"),502,"CAMBRIDGE_BLOCKED")] {
        let html=entry();let s=server(move |p|if p.ends_with("/failure"){reply.clone()}else{Reply::page(&html)}).await;let b=backend(&s,2000);
        let error=b.query("failure","en").await.unwrap_err();assert_eq!(error.status,Some(status));assert_eq!(error.code.as_deref(),Some(code));assert!(error.suggestions.is_empty());assert_eq!(s.requests.lock().unwrap().len(),1);
        assert_eq!(b.query("example","en-cn").await.unwrap()["word"],"example");
    }
}
#[tokio::test] async fn request_404_may_have_candidates_but_optional_failure_preserves_missing() {
    for optional in [Reply::status(503),Reply::page("<html></html>"),Reply::page("<html><title>Just a moment</title></html>"),Reply::page(&candidates()).slow(250)] {
        let s=server(move |p|if p.contains("spellcheck"){optional.clone()}else{Reply::status(404)}).await;let mut b=backend(&s,2000);b.suggestion_timeout=Duration::from_millis(80);
        let error=b.query("termnal","en").await.unwrap_err();assert!(error.is_missing());assert!(error.suggestions.is_empty());assert_eq!(s.requests.lock().unwrap().len(),2);
    }
}
#[tokio::test] async fn optional_deadline_retains_404_within_the_remaining_total_budget() {
    let landing=landing();let suggestion=candidates();let s=server(move |p|if p.contains("spellcheck"){Reply::page(&suggestion).slow(500)}else{Reply::page(&landing).slow(140)}).await;
    let b=backend(&s,220);let started=Instant::now();let error=b.query("termnal","en").await.unwrap_err();assert!(error.is_missing());assert!(started.elapsed()<Duration::from_millis(450));assert_eq!(s.requests.lock().unwrap().len(),2);
}
#[tokio::test] async fn timeout_is_typed_and_does_not_block_the_next_word() {
    let html=entry();let s=server(move |p|Reply::page(&html).slow(if p.ends_with("/hang"){500}else{0})).await;let b=backend(&s,100);
    let error=b.query("hang","en").await.unwrap_err();assert_eq!(error.status,Some(504));assert_eq!(error.code.as_deref(),Some("CAMBRIDGE_TIMEOUT"));
    assert_eq!(b.query("example","en").await.unwrap()["word"],"example");
}
#[tokio::test] async fn cancelling_network_future_releases_a_slot_and_next_word_recovers() {
    let html=entry();let s=server(move |p|Reply::page(&html).slow(if p.ends_with("/hang"){1000}else{0})).await;let b=Arc::new(backend(&s,2000));
    let copy=b.clone();let task=tokio::spawn(async move{copy.query("hang","en").await});
    wait_requests(&s,1).await;task.abort();let _=task.await;
    let started=Instant::now();assert_eq!(b.query("example","en-cn").await.unwrap()["word"],"example");assert!(started.elapsed()<Duration::from_millis(500));
    assert_eq!(b.slots.available_permits(),2);
}
async fn wait_requests(s:&Server,count:usize) {
    tokio::time::timeout(Duration::from_secs(1),async{while s.requests.lock().unwrap().len()<count{tokio::time::sleep(Duration::from_millis(2)).await;}}).await.unwrap();
}
#[tokio::test] async fn close_cancels_pending_io_and_is_bounded() {
    let html=entry();let s=server(move |_|Reply::page(&html).slow(2000)).await;let b=Arc::new(backend(&s,3000));let copy=b.clone();let task=tokio::spawn(async move{copy.query("example","en").await});
    wait_requests(&s,1).await;let started=Instant::now();b.close();assert!(task.await.unwrap().unwrap_err().message.contains("取消"));assert!(started.elapsed()<Duration::from_millis(200));assert_eq!(b.diagnostic()["clientReady"],false);
}
#[tokio::test] async fn different_languages_and_concurrent_windows_do_not_mix_results() {
    let html=entry();let s=server(move |p|Reply::page(&html.replace("example</span>",if p.contains("english-chinese-simplified"){"chinese</span>"}else{"english</span>"}))).await;let b=backend(&s,2000);
    let (one,two)=tokio::join!(b.query("example","en-cn"),b.query("example","en"));assert_eq!(one.unwrap()["word"],"chinese");assert_eq!(two.unwrap()["word"],"english");
}
#[tokio::test] async fn bad_input_does_not_create_a_client_or_issue_a_request() {
    let b=Backend::default();for (word,language)in [("", "en"),("x/y","en"),("a\nb","en"),("word","bad")] {assert_eq!(b.query(word,language).await.unwrap_err().status,Some(400));}
    assert_eq!(b.diagnostic()["clientReady"],false);assert_eq!(b.diagnostic()["httpRequests"],0);
}
#[tokio::test] async fn gzip_is_decoded_and_the_limit_applies_to_decoded_content() {
    for (body,valid)in [(include_bytes!("../../tests/fixtures/entry.html.gz").to_vec(),true),(include_bytes!("../../tests/fixtures/oversize.html.gz").to_vec(),false)] {
        let s=server(move |_|Reply{headers:"Content-Encoding: gzip\r\n".into(),body:body.clone(),..Reply::status(200)}).await;let b=backend(&s,2000);let result=b.query("example","en").await;
        if valid{assert_eq!(result.unwrap()["word"],"example");}else{assert_eq!(result.unwrap_err().code.as_deref(),Some("INVALID_CAMBRIDGE_PAGE"));}assert_eq!(s.requests.lock().unwrap().len(),1);
    }
}
#[tokio::test] async fn invalid_utf8_and_large_html_never_become_entries() {
    for body in [vec![0xff,0xfe],vec![b'x';http::HTML_LIMIT+1]] {
        let s=server(move |_|Reply{body:body.clone(),..Reply::status(200)}).await;let b=backend(&s,2000);assert_eq!(b.query("example","en").await.unwrap_err().code.as_deref(),Some("INVALID_CAMBRIDGE_PAGE"));
    }
}
#[tokio::test] async fn unsafe_redirect_is_rejected_without_following_it() {
    for target in ["https://example.com/","http://dictionary.cambridge.org/","https://dictionary.cambridge.org:444/","https://user@dictionary.cambridge.org/"] {
        let s=server(move |_|Reply{headers:format!("Location: {target}\r\n"),..Reply::status(302)}).await;let b=backend(&s,2000);assert_eq!(b.query("example","en").await.unwrap_err().code.as_deref(),Some("CAMBRIDGE_UNAVAILABLE"));assert_eq!(s.requests.lock().unwrap().len(),1);
    }
}
#[tokio::test] async fn an_explicit_proxy_receives_the_cambridge_connect_request() {
    let s=server(|_|Reply::status(403)).await;let b=Backend::default();*b.client.lock().unwrap()=Some(http::client_builder().proxy(reqwest::Proxy::all(s.url.as_str()).unwrap()).build().unwrap());
    let error=b.query("example","en").await.unwrap_err();assert_eq!(error.code.as_deref(),Some("CAMBRIDGE_UNAVAILABLE"));
    assert_eq!(s.requests.lock().unwrap().as_slice(),["dictionary.cambridge.org:443"]);
}
