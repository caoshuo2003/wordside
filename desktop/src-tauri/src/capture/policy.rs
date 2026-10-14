pub use crate::selection_input::selected_word;
pub fn permits_source(source_pid:u32,own_pid:u32)->bool {
    source_pid!=0&&source_pid!=own_pid
}

pub fn mouse_selection(start:(i32,i32),end:(i32,i32),double_click:bool,shift:bool)->bool {
    (i64::from(start.0)-i64::from(end.0)).abs()>=4||(i64::from(start.1)-i64::from(end.1)).abs()>=4||double_click||shift
}
pub fn copy_chord(control:bool,c:bool,insert:bool,alt:bool,windows:bool)->bool {control&&(c||insert)&&!alt&&!windows}
