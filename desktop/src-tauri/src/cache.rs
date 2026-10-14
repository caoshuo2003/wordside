use std::{collections::VecDeque,time::{Duration,Instant}};
use serde_json::Value;
const TTL:Duration=Duration::from_secs(15*60);
const LIMIT:usize=12*1024*1024;
struct Item {key:String,at:Instant,value:Value,size:usize,ticket:u64,validated:bool}
#[derive(Default)]
pub struct Cache {entries:VecDeque<Item>}
impl Cache {
 pub fn get(&mut self,key:&str)->Option<Value>{
  self.entries.retain(|item|item.at.elapsed()<TTL);
  let index=self.entries.iter().position(|item|item.key==key&&item.validated)?;
  let item=self.entries.remove(index)?;let value=item.value.clone();self.entries.push_back(item);Some(value)
 }
 pub fn insert_pending(&mut self,key:String,value:Value,ticket:u64){
  let size=serde_json::to_vec(&value).map(|v|v.len()).unwrap_or(LIMIT+1);
  self.entries.retain(|item|item.key!=key&&item.at.elapsed()<TTL);
  if size>LIMIT{return;}
  while self.entries.len()>=32||self.entries.iter().map(|item|item.size).sum::<usize>()+size>LIMIT{self.entries.pop_front();}
  self.entries.push_back(Item{key,at:Instant::now(),value,size,ticket,validated:false});
 }
 pub fn settle(&mut self,ticket:u64,accepted:bool){
  if let Some(index)=self.entries.iter().position(|item|item.ticket==ticket){if accepted{self.entries[index].validated=true;}else{self.entries.remove(index);}}
 }
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn bounded_expiring_cache(){let mut c=Cache::default();for n in 0..40{c.insert_pending(n.to_string(),serde_json::json!({"word":n}),n);c.settle(n,true);}assert_eq!(c.entries.len(),32);assert!(c.get("0").is_none());assert!(c.get("39").is_some());for item in &mut c.entries{item.at=Instant::now()-TTL;}assert!(c.get("39").is_none());}
 #[test]fn byte_limit(){let mut c=Cache::default();c.insert_pending("large".into(),Value::String("x".repeat(LIMIT+1)),1);assert!(c.entries.is_empty());}
 #[test]fn only_validated_source_is_reused(){let mut c=Cache::default();c.insert_pending("word".into(),serde_json::json!({"html":"unvalidated"}),1);assert!(c.get("word").is_none());c.settle(1,true);assert_eq!(c.get("word").unwrap()["html"],"unvalidated");c.settle(1,false);assert!(c.get("word").is_none());}
 #[test]fn stale_verdict_cannot_remove_newer_response(){let mut c=Cache::default();c.insert_pending("word".into(),Value::Null,1);c.insert_pending("word".into(),serde_json::json!({"html":"new"}),2);c.settle(1,true);assert!(c.get("word").is_none());c.settle(2,true);c.settle(1,false);assert_eq!(c.get("word").unwrap()["html"],"new");}
}
