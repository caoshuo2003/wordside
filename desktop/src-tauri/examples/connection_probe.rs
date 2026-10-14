// Developer-only timings: no Tauri, UIA, tray or real settings.
#[path = "../src/dictionary.rs"] mod dictionary;
#[path = "../src/lookup.rs"] mod lookup;
#[path = "../src/cache.rs"] mod cache;
use serde_json::{Value,json};
use std::time::Instant;
fn percentiles(values:&mut [f64])->Value{values.sort_by(f64::total_cmp);json!({"samples":values.len(),"medianMs":values[values.len()/2],"p95Ms":values[(values.len()*95).div_ceil(100)-1]})}
#[tokio::main]async fn main(){
 let backend=dictionary::Backend::default();
 let initial_language=std::env::var("WORDSIDE_PROBE_LANGUAGE").unwrap_or_else(|_|"en-cn".into());
 let workflow=std::env::var("WORDSIDE_PROBE_WORKFLOW").ok().as_deref()==Some("1");
 let words=std::env::args().skip(1).collect::<Vec<_>>();let words=if words.is_empty(){vec!["terminal".into();3]}else{words};
 for(index,word)in words.iter().enumerate(){
  let started=Instant::now();let stage=Instant::now();
  let mut payload=lookup::fetch_payload(&backend,word,&initial_language).await.unwrap_or_else(|reason|json!({"entry":null,"apiError":reason}));
  let mut language=initial_language.clone();let mut stages=vec![json!({"language":language,"ms":stage.elapsed().as_secs_f64()*1000.0,"payload":payload})];
  let typed=payload.get("apiStatus").is_some()||payload.get("apiCode").is_some();
  let missing=if typed {payload["apiStatus"]==404&&payload["apiCode"]=="WORD_NOT_FOUND"}else{payload["apiError"].as_str().is_some_and(|s|s.starts_with("词典服务返回 HTTP 404"))||(payload["entry"]["error"].as_str().is_some_and(|s|s.trim().eq_ignore_ascii_case("word not found"))&&(payload["entry"]["code"].is_null()||payload["entry"]["code"]=="WORD_NOT_FOUND"))};
  if workflow&&language=="en-cn"&&missing{language="en".into();let stage=Instant::now();payload=lookup::fetch_payload(&backend,word,&language).await.unwrap_or_else(|reason|json!({"entry":null,"apiError":reason}));stages.push(json!({"language":language,"ms":stage.elapsed().as_secs_f64()*1000.0,"payload":payload}));}
  // Capture elapsed before serializing/writing records or timing the cache primitive.
  let network_ms=started.elapsed().as_secs_f64()*1000.0;
  let mut cache_metrics=Value::Null;
  if payload["entry"]["definition"].as_array().is_some_and(|a|!a.is_empty()){
   let mut store=cache::Cache::default();let key=word.to_string();let at=Instant::now();store.insert_pending(key.clone(),payload.clone(),1);store.settle(1,true);let insert_ms=at.elapsed().as_secs_f64()*1000.0;
   let mut values=Vec::with_capacity(500);for _ in 0..500{let at=Instant::now();let value=store.get(&key).expect("approved isolated cache missing");std::hint::black_box(value);values.push(at.elapsed().as_secs_f64()*1000.0);}
   cache_metrics=percentiles(&mut values);cache_metrics["insertAndApproveMs"]=json!(insert_ms);
  }
  let record=json!({"word":word,"language":language,"payload":payload,"stages":stages,"networkMs":network_ms,"cachePrimitive":cache_metrics});
  if let Ok(folder)=std::env::var("WORDSIDE_PROBE_DIR"){let folder=std::path::PathBuf::from(folder);std::fs::create_dir_all(&folder).unwrap();std::fs::write(folder.join(format!("{}.json",index+1)),serde_json::to_vec(&record).unwrap()).unwrap();}
  println!("{}",json!({"round":index+1,"word":word,"language":language,"networkMs":network_ms,"stages":stages.iter().map(|s|json!({"language":s["language"],"ms":s["ms"],"apiError":s["payload"]["apiError"]})).collect::<Vec<_>>(),"definitions":payload["entry"]["definition"].as_array().map(Vec::len),"apiError":payload["apiError"],"cachePrimitive":cache_metrics}));
 }
 backend.close();
}
