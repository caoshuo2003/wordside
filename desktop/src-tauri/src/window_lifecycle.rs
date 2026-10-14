//! Main-window close preference. Closing a definition never quits the app.
use serde::{Deserialize, Serialize};
use tauri::tray::{MouseButton, TrayIconEvent};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="snake_case")]
pub enum CloseBehavior { #[default] Tray, Exit }
impl CloseBehavior {
    pub fn parse(value:&str)->Option<Self>{match value {"tray"=>Some(Self::Tray),"exit"=>Some(Self::Exit),_=>None}}
}
pub fn deserialize_behavior<'de,D:serde::Deserializer<'de>>(deserializer:D)->Result<CloseBehavior,D::Error>{
    let value=serde_json::Value::deserialize(deserializer)?;
    Ok(value.as_str().and_then(CloseBehavior::parse).unwrap_or_default())
}
#[derive(Debug, PartialEq, Eq)]
pub enum CloseAction { HidePopup, CloseToTray, ExitApplication, CloseOther }
pub fn close_action(label:&str,behavior:CloseBehavior)->CloseAction {
    match label {
        "popup"=>CloseAction::HidePopup,
        "main" if behavior==CloseBehavior::Exit=>CloseAction::ExitApplication,
        "main"=>CloseAction::CloseToTray,
        _=>CloseAction::CloseOther,
    }
}
pub fn opens_main(event:&TrayIconEvent)->bool {
    matches!(event,TrayIconEvent::DoubleClick{button:MouseButton::Left,..})
}
#[cfg(test)]mod tests {
    use super::*;
    #[test]fn definition_close_never_exits_and_main_follows_preference(){
        for behavior in [CloseBehavior::Tray,CloseBehavior::Exit]{
            assert_eq!(close_action("popup",behavior),CloseAction::HidePopup);
            assert_eq!(close_action("future-window",behavior),CloseAction::CloseOther);
        }
        assert_eq!(close_action("main",CloseBehavior::Tray),CloseAction::CloseToTray);
        assert_eq!(close_action("main",CloseBehavior::Exit),CloseAction::ExitApplication);
    }
    #[test]fn only_left_double_click_opens_main(){
        let rect=tauri::Rect{position:tauri::Position::Physical(tauri::PhysicalPosition::new(0,0)),size:tauri::Size::Physical(tauri::PhysicalSize::new(32,32))};
        for button in [MouseButton::Left,MouseButton::Right,MouseButton::Middle]{
            let double=TrayIconEvent::DoubleClick{id:"wordside".into(),position:tauri::PhysicalPosition::new(0.0,0.0),rect,button};
            assert_eq!(opens_main(&double),button==MouseButton::Left);
            for state in [tauri::tray::MouseButtonState::Up,tauri::tray::MouseButtonState::Down]{
                assert!(!opens_main(&TrayIconEvent::Click{id:"wordside".into(),position:tauri::PhysicalPosition::new(0.0,0.0),rect,button,button_state:state}));
            }
        }
    }
}
