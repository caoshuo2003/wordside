//! Windows 11 draws the frame: rounded corners, a 1px border and the main window's caption.
//! Tint them with the page palette so frame and content match.
#[cfg(windows)]
#[link(name="dwmapi")]
extern "system" { fn DwmSetWindowAttribute(window:isize,attribute:u32,value:*const std::ffi::c_void,size:u32)->i32; }
#[cfg(windows)] const IMMERSIVE_DARK_MODE:u32=20;
#[cfg(windows)] const WINDOW_CORNER_PREFERENCE:u32=33;
#[cfg(windows)] const BORDER_COLOR:u32=34;
#[cfg(windows)] const CAPTION_COLOR:u32=35;
#[cfg(windows)] const TEXT_COLOR:u32=36;
#[cfg(windows)] const ROUND:u32=2;

#[derive(Debug,PartialEq)]
pub struct Frame { pub border:u32, pub caption:Option<u32>, pub text:Option<u32>, pub dark:bool }

impl Frame {
    /// Every color given must be `#rrggbb`; the popup has no caption or title text.
    pub fn parse(border:&str,caption:Option<&str>,text:Option<&str>,dark:bool)->Option<Self>{
        let optional=|value:Option<&str>|match value { Some(value)=>colorref(value).map(Some), None=>Some(None) };
        Some(Self{border:colorref(border)?,caption:optional(caption)?,text:optional(text)?,dark})
    }

    /// Dark mode keeps the caption buttons legible on a dark caption. Returns
    /// whether Windows drew the tinted border: false before Windows 11 (build
    /// 22000), where the page keeps drawing its own.
    #[cfg(windows)]
    pub fn apply(&self,window:isize)->bool{
        let set=|attribute:u32,value:u32|unsafe{DwmSetWindowAttribute(window,attribute,&value as *const u32 as _,4)==0};
        set(IMMERSIVE_DARK_MODE,self.dark.into());
        if let Some(caption)=self.caption{set(CAPTION_COLOR,caption);}
        if let Some(text)=self.text{set(TEXT_COLOR,text);}
        set(WINDOW_CORNER_PREFERENCE,ROUND)&&set(BORDER_COLOR,self.border)
    }
}

/// `#rrggbb` → COLORREF (0x00bbggrr). Other CSS color syntaxes are refused.
fn colorref(value:&str)->Option<u32>{
    let hex=value.trim().strip_prefix('#')?;
    if hex.len()!=6 || !hex.bytes().all(|byte|byte.is_ascii_hexdigit()) {return None;}
    let rgb=u32::from_str_radix(hex,16).ok()?;
    Some((rgb>>16)|(rgb&0xff00)|((rgb&0xff)<<16))
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn css_hex_becomes_colorref(){
        assert_eq!(colorref("#e0e7df"),Some(0x00df_e7e0));
        assert_eq!(colorref(" #37433E "),Some(0x003e_4337));
        for value in ["","e0e7df","#fff","#20372818","#e0e7dg","rgb(1,2,3)"] {assert_eq!(colorref(value),None,"{value}");}
    }
    #[test]fn frame_rejects_any_invalid_color(){
        assert_eq!(Frame::parse("#e0e7df",None,None,false),Some(Frame{border:0x00df_e7e0,caption:None,text:None,dark:false}));
        assert_eq!(Frame::parse("#37433e",Some("#181c1b"),Some("#e8eeeb"),true),Some(Frame{border:0x003e_4337,caption:Some(0x001b_1c18),text:Some(0x00eb_eee8),dark:true}));
        assert_eq!(Frame::parse("#37433e",Some("transparent"),None,true),None);
        assert_eq!(Frame::parse("#37433e",None,Some(""),false),None);
        assert_eq!(Frame::parse("",None,None,false),None);
    }
}
