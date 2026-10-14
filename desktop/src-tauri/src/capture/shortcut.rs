use serde::{Serialize,Deserialize};
pub const DEFAULT_HOTKEY:&str="Ctrl+Alt+A";
#[derive(Clone,Debug,PartialEq,Eq)]pub struct Hotkey{pub text:String,pub modifiers:u32,pub key:u32}
pub fn parse(value:&str)->Result<Option<Hotkey>,String>{
 let value=value.trim();if value.is_empty(){return Ok(None);}
 let mut modifiers=0;let mut key=None;let mut key_name=String::new();
 for part in value.split('+').map(str::trim){
  let bit=match part.to_ascii_lowercase().as_str(){"ctrl"|"control"=>Some(2),"alt"=>Some(1),"shift"=>Some(4),_=>None};
  if let Some(bit)=bit{if modifiers&bit!=0{return Err("快捷键修饰键重复。".into());}modifiers|=bit;continue;}
  if key.is_some(){return Err("快捷键只能包含一个字母、数字或功能键。".into());}
  let upper=part.to_ascii_uppercase();let code=if upper.len()==1&&upper.as_bytes()[0].is_ascii_alphanumeric(){u32::from(upper.as_bytes()[0])}
   else if let Some(number)=upper.strip_prefix('F').and_then(|v|v.parse::<u32>().ok()).filter(|n|(1..=24).contains(n)&&*n!=12){key_name=format!("F{number}");0x6f+number}
   else{return Err("请使用 Ctrl / Alt / Shift 加字母、数字或 F1–F24；F12 和 Win 组合为系统保留。".into());};
  if key_name.is_empty(){key_name=upper;}key=Some(code);
 }
 if modifiers&3==0||key.is_none(){return Err("快捷键需包含 Ctrl 或 Alt，以及一个字母、数字或功能键。".into());}
 let mut parts=Vec::new();if modifiers&2!=0{parts.push("Ctrl");}if modifiers&1!=0{parts.push("Alt");}if modifiers&4!=0{parts.push("Shift");}parts.push(&key_name);
 Ok(Some(Hotkey{text:parts.join("+"),modifiers,key:key.unwrap()}))
}
#[derive(Clone,Debug,Default,Serialize,Deserialize)]pub struct HotkeyStatus{pub active:bool,pub shortcut:String,pub error:Option<String>}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn canonical_shortcuts_and_disable(){assert_eq!(parse(DEFAULT_HOTKEY).unwrap().unwrap(),Hotkey{text:DEFAULT_HOTKEY.into(),modifiers:3,key:65});assert_eq!(parse("alt + d + Control").unwrap().unwrap(),Hotkey{text:"Ctrl+Alt+D".into(),modifiers:3,key:68});assert_eq!(parse(" Ctrl+Shift+F24 ").unwrap().unwrap().key,0x87);assert_eq!(parse("Alt+7").unwrap().unwrap().text,"Alt+7");assert!(parse("").unwrap().is_none());}
 #[test]fn ambiguous_reserved_and_unmodified_keys_are_rejected(){for value in ["D","Shift+D","Ctrl+Alt","Ctrl+Ctrl+D","Ctrl+A+B","Win+D","Ctrl+F12","Alt+F25","Ctrl+Escape","Ctrl++D"]{assert!(parse(value).is_err(),"{value}");}}
}
