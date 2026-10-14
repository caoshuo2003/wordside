use serde::{Serialize,Deserialize};
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(tag="kind",content="detail",rename_all="snake_case")]
pub enum Selection{Word(String),NoSelection,Unsupported,Protected,SourceMismatch,InvalidWord,AccessDenied,ReadError(String),Timeout}
impl Selection{
 pub fn failure(&self)->Option<(&'static str,&'static str)>{Some(match self{
  Self::Word(_)=>return None,
  Self::NoSelection=>("no_selection","应用没有返回选中文字。若已选中，请尝试同一应用的其他文本区域；也可以直接输入查询。"),
  Self::Unsupported=>("unsupported","此位置没有提供可读取的文字选区接口，可尝试其他文本区域或在 WordSide 中手动输入。"),
  Self::Protected=>("protected","密码或受保护的文本不会读取。"),
  Self::SourceMismatch=>("source_changed","原窗口或焦点已变化，请在目标应用重新选词后查询。"),
  Self::InvalidWord=>("invalid_selection","请选择单行英文单词或短语，最多 80 字符；不支持多段选区、日志、路径或命令。"),
  Self::AccessDenied=>("access_denied","无法访问此应用的文字选区，请检查源应用与 WordSide 的权限是否一致。"),
  Self::ReadError(_)=>("read_error","读取选区失败，请重新选择后再试；也可以在 WordSide 中手动输入。"),
  Self::Timeout=>("timeout","读取选区超时，取词进程已回收；可以重新选词再试。"),
 })}
}
#[cfg(test)]mod tests{use super::*;#[test]fn distinct_reading_failures(){let values=[Selection::NoSelection,Selection::Unsupported,Selection::Protected,Selection::SourceMismatch,Selection::InvalidWord,Selection::AccessDenied,Selection::ReadError("GetSelection".into()),Selection::Timeout];let codes:std::collections::HashSet<_>=values.iter().map(|s|s.failure().unwrap().0).collect();assert_eq!(codes.len(),values.len());assert!(Selection::Word("terminal".into()).failure().is_none());for value in values{assert_eq!(serde_json::from_str::<Selection>(&serde_json::to_string(&value).unwrap()).unwrap(),value);}}}
