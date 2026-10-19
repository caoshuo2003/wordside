//! The exe, installer and shortcuts keep the fixed paper icon (icons/ciban.ico). The tray icon
//! and the main window's title bar, taskbar and Alt+Tab icons follow the palette. Each palette
//! ships one ICO holding PNG images from 16 to 64 px; the size Windows asks for at the current
//! scaling is picked at runtime so the icons stay sharp.
#[cfg(windows)]
#[link(name="user32")]
extern "system" {
    fn GetDpiForWindow(window:isize)->u32;
    fn GetSystemMetrics(index:i32)->i32;
    fn GetSystemMetricsForDpi(index:i32,dpi:u32)->i32;
    fn CreateIconFromResourceEx(data:*const u8,size:u32,icon:i32,version:u32,width:i32,height:i32,flags:u32)->isize;
    fn SendMessageW(window:isize,message:u32,wparam:usize,lparam:isize)->isize;
}
#[cfg(windows)] const SM_CXICON:i32=11;
#[cfg(windows)] const SM_CXSMICON:i32=49;
#[cfg(windows)] const WM_SETICON:u32=0x0080;
#[cfg(windows)] const ICON_SMALL:usize=0;
#[cfg(windows)] const ICON_BIG:usize=1;

fn icons(palette:&str)->&'static [u8]{
    match palette {
        "lavender"=>include_bytes!("../icons/palette/lavender.ico"),
        "blue"=>include_bytes!("../icons/palette/blue.ico"),
        "stone"=>include_bytes!("../icons/palette/stone.ico"),
        _=>include_bytes!("../icons/palette/green.ico"),
    }
}

/// The ICO's image for `size` px: the smallest one at least that big, else the largest.
fn image(ico:&[u8],size:u32)->Option<&[u8]>{
    let count=u16::from_le_bytes(ico.get(4..6)?.try_into().ok()?) as usize;
    let mut best:Option<(u32,&[u8])>=None;
    for index in 0..count {
        let entry=ico.get(6+16*index..22+16*index)?;
        let width=match entry[0] {0=>256,width=>width as u32};
        let len=u32::from_le_bytes(entry[8..12].try_into().ok()?) as usize;
        let offset=u32::from_le_bytes(entry[12..16].try_into().ok()?) as usize;
        let data=ico.get(offset..offset.checked_add(len)?)?;
        let better=match best {
            None=>true,
            Some((current,_)) if current>=size=>width>=size && width<current,
            Some((current,_))=>width>current,
        };
        if better {best=Some((width,data));}
    }
    best.map(|(_,data)|data)
}

/// The tray draws small icons at the system scaling.
fn tray_size()->u32{
    #[cfg(windows)]{unsafe{GetSystemMetrics(SM_CXSMICON)}.max(16) as u32}
    #[cfg(not(windows))]{32}
}

pub fn tray(palette:&str)->Option<tauri::image::Image<'static>>{
    tauri::image::Image::from_bytes(image(icons(palette),tray_size())?).ok()
}

/// Set the window's small (title bar) and big (taskbar, Alt+Tab) icons for its current DPI.
/// Call again after the window moves to a monitor with another scaling.
#[cfg(windows)]
pub fn apply(window:isize,palette:&str){
    let dpi=unsafe{GetDpiForWindow(window)};
    if dpi==0 {return;}
    for (kind,metric) in [(ICON_SMALL,SM_CXSMICON),(ICON_BIG,SM_CXICON)] {
        let size=unsafe{GetSystemMetricsForDpi(metric,dpi)};
        if let Some(icon)=handle(palette,size.max(16) as u32) {unsafe{SendMessageW(window,WM_SETICON,kind,icon);}}
    }
}

/// Icons live as long as the process: at most one per palette and size, shared by every main
/// window the app recreates. The window does not own them, so they are never destroyed.
#[cfg(windows)]
fn handle(palette:&str,size:u32)->Option<isize>{
    static CACHE:std::sync::Mutex<Vec<(String,u32,isize)>>=std::sync::Mutex::new(Vec::new());
    let mut cache=CACHE.lock().unwrap();
    if let Some(&(_,_,icon))=cache.iter().find(|(name,known,_)|name==palette && *known==size) {return Some(icon);}
    let png=image(icons(palette),size)?;
    let icon=unsafe{CreateIconFromResourceEx(png.as_ptr(),png.len() as u32,1,0x0003_0000,size as i32,size as i32,0)};
    if icon==0 {return None;}
    cache.push((palette.into(),size,icon));
    Some(icon)
}

#[cfg(test)]mod tests{
    use super::*;
    const PNG:&[u8]=b"\x89PNG\r\n\x1a\n";
    fn width(png:&[u8])->u32{u32::from_be_bytes(png[16..20].try_into().unwrap())}
    #[test]fn every_palette_has_its_own_icons(){
        let all:Vec<&[u8]>=crate::PALETTES.iter().map(|palette|icons(palette)).collect();
        for (index,ico) in all.iter().enumerate() {
            assert!(all[..index].iter().all(|other|other.as_ptr()!=ico.as_ptr()),"{} reuses another palette's icons",crate::PALETTES[index]);
        }
    }
    #[test]fn picks_the_smallest_image_at_least_as_big(){
        for palette in crate::PALETTES {
            let ico=icons(palette);
            for (want,got) in [(16,16),(20,20),(24,24),(28,32),(32,32),(40,40),(48,48),(56,64),(64,64),(96,64),(1,16)] {
                let png=image(ico,want).unwrap();
                assert!(png.starts_with(PNG),"{palette} {want}");
                assert_eq!(width(png),got,"{palette} {want}");
            }
            assert!(tauri::image::Image::from_bytes(image(ico,24).unwrap()).is_ok(),"{palette}");
        }
    }
    #[test]fn malformed_ico_is_refused(){
        let ico=icons("green");
        assert_eq!(image(&[],16),None);
        assert_eq!(image(&ico[..10],16),None);
        assert_eq!(image(&ico[..ico.len()-1],64),None,"the last image is cut short");
    }
}
