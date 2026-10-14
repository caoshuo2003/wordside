use serde_json::Value;
use crate::dictionary::Backend;

// The native parser requests Cambridge directly. No deployed API or server.
pub async fn fetch_payload(backend:&Backend,word:&str,language:&str)->Result<Value,String>{
    if !["en-cn","en"].contains(&language){return Err("无效词库".into());}
    Ok(match backend.query(word,language).await{
        Ok(entry)=>serde_json::json!({"entry":entry}),
        Err(reason)=>{
            let mut payload=serde_json::json!({"entry":null,"apiError":reason.message});
            if let Some(status)=reason.status { payload["apiStatus"]=serde_json::json!(status); }
            if let Some(code)=reason.code { payload["apiCode"]=serde_json::json!(code); }
            if !reason.suggestions.is_empty() { payload["suggestions"]=serde_json::json!(reason.suggestions); }
            payload
        },
    })
}
