//! Native clipboard integration in a disposable, noninteractive window station.
//! No clipboard API runs until both process station and thread desktop are isolated.
use super::*;
use std::os::windows::process::CommandExt;
#[link(name="user32")]
extern "system" {
    fn CreateWindowStationW(name:*const u16,flags:u32,access:u32,security:*const c_void)->Handle;
    fn SetProcessWindowStation(station:Handle)->i32;
    fn GetProcessWindowStation()->Handle;
    fn CloseWindowStation(station:Handle)->i32;
    fn GetThreadDesktop(thread:u32)->Handle;
    fn CreateDesktopW(name:*const u16,device:*const u16,mode:*const c_void,flags:u32,access:u32,security:*const c_void)->Handle;
    fn SetThreadDesktop(desktop:Handle)->i32;
    fn CloseDesktop(desktop:Handle)->i32;
    fn GetUserObjectInformationW(object:Handle,index:i32,data:*mut c_void,length:u32,needed:*mut u32)->i32;
}
#[link(name="kernel32")]
extern "system" {fn GetCurrentThreadId()->u32;fn GlobalAlloc(flags:u32,bytes:usize)->Handle;}
#[repr(C)]#[derive(Default)]struct UserFlags{inherit:i32,reserved:i32,flags:u32}
struct Isolated { old_station:Handle,old_desktop:Handle,station:Handle,desktop:Handle }
impl Isolated {
    fn new()->Self{unsafe{
        let old_station=GetProcessWindowStation();let old_desktop=GetThreadDesktop(GetCurrentThreadId());
        // CREATE_ONLY: never join another existing station, named or unnamed.
        let station_name:Vec<_>=format!("WordSideClipboardTest-{}\0",std::process::id()).encode_utf16().collect();
        let station=CreateWindowStationW(station_name.as_ptr(),1,0x1000_0000,ptr::null());
        assert_ne!(station,0,"create private station, error {}",GetLastError());
        let mut own=Self{old_station,old_desktop,station,desktop:0};
        assert_ne!(SetProcessWindowStation(station),0,"assign station, error {}",GetLastError());
        let name:Vec<_>="WordSideTest\0".encode_utf16().collect();
        own.desktop=CreateDesktopW(name.as_ptr(),ptr::null(),ptr::null(),0,0x1000_0000,ptr::null());
        assert_ne!(own.desktop,0,"create private desktop, error {}",GetLastError());
        assert_ne!(SetThreadDesktop(own.desktop),0,"assign private desktop, error {}",GetLastError());
        let mut flags=UserFlags::default();let mut needed=0;
        assert_ne!(GetUserObjectInformationW(GetProcessWindowStation(),1,&mut flags as *mut _ as _,std::mem::size_of::<UserFlags>() as u32,&mut needed),0);
        assert_eq!(flags.flags&1,0,"private station must not be visible");
        let mut name=[0u16;256];
        assert_ne!(GetUserObjectInformationW(GetProcessWindowStation(),2,name.as_mut_ptr() as _,512,&mut needed),0);
        let name=String::from_utf16_lossy(&name[..name.iter().position(|v|*v==0).unwrap()]);
        assert_ne!(name.to_ascii_lowercase(),"winsta0","refuse interactive clipboard");
        assert_eq!(GetThreadDesktop(GetCurrentThreadId()),own.desktop);
        println!("private station verified: noninteractive, separate clipboard, no input or desktop switch");
        own
    }}
}
impl Drop for Isolated{fn drop(&mut self){unsafe{
    if self.desktop!=0{let _=SetThreadDesktop(self.old_desktop);let _=CloseDesktop(self.desktop);}
    let _=SetProcessWindowStation(self.old_station);let _=CloseWindowStation(self.station);
}}}
fn data(id:u32,contents:&[u8])->Format{unsafe{
    let memory=GlobalAlloc(2,contents.len());assert_ne!(memory,0);
    let target=GlobalLock(memory) as *mut u8;assert!(!target.is_null());
    ptr::copy_nonoverlapping(contents.as_ptr(),target,contents.len());GlobalUnlock(memory);
    Format{id,data:memory,bytes:contents.len()}
}}
fn place(owner:&Owner,values:Vec<Format>){
    let _lock=ClipboardLock::open(owner.0,Duration::from_millis(100)).unwrap();
    assert_ne!(unsafe{EmptyClipboard()},0);
    for mut value in values {assert_ne!(unsafe{SetClipboardData(value.id,value.data)},0);value.data=0;}
}
fn formats()->Vec<(u32,usize)>{
    let _lock=ClipboardLock::open(0,Duration::from_millis(100)).unwrap();let mut out=vec![];let mut id=0;
    loop{unsafe{SetLastError(0);id=EnumClipboardFormats(id);}if id==0{assert_eq!(unsafe{GetLastError()},0);break;}
        let handle=unsafe{GetClipboardData(id)};
        out.push((id,if matches!(id,2|9){0}else{unsafe{GlobalSize(handle)}}));
    }out
}
fn read(id:u32,length:usize)->Vec<u8>{
    let _lock=ClipboardLock::open(0,Duration::from_millis(100)).unwrap();let memory=unsafe{GetClipboardData(id)};assert_ne!(memory,0);
    let source=unsafe{GlobalLock(memory)} as *const u8;assert!(!source.is_null());assert!(unsafe{GlobalSize(memory)}>=length);
    let bytes=unsafe{std::slice::from_raw_parts(source,length)}.to_vec();unsafe{GlobalUnlock(memory);}bytes
}
fn unicode(text:&str)->Vec<u8>{text.encode_utf16().flat_map(u16::to_le_bytes).collect()}
fn dib(bits:u16)->Vec<u8>{
    let palette=if bits==8{256*4}else{0};let mut bytes=vec![0u8;40+palette+4];
    bytes[..4].copy_from_slice(&40u32.to_le_bytes());bytes[4..8].copy_from_slice(&1i32.to_le_bytes());bytes[8..12].copy_from_slice(&1i32.to_le_bytes());
    bytes[12..14].copy_from_slice(&1u16.to_le_bytes());bytes[14..16].copy_from_slice(&bits.to_le_bytes());
    bytes[20..24].copy_from_slice(&4u32.to_le_bytes());
    if bits==8{bytes[32..36].copy_from_slice(&256u32.to_le_bytes());for i in 0..256{bytes[40+i*4..43+i*4].fill(i as u8);}}
    bytes
}
#[test]#[ignore="private station child entry; invoke through parent integration test"]
fn private_clipboard_process(){
    assert_eq!(std::env::var("WORDSIDE_ISOLATED_CLIPBOARD_CHILD").as_deref(),Ok("1"));
    let _isolated=Isolated::new(); // Must precede Owner creation and every clipboard API.
    let owner=Owner::new().unwrap();
    let cases=[("empty",0,vec![]),("unicode",13,unicode("previous 中文 🧪\0")),("ansi",1,b"previous ascii\0".to_vec()),("dib32",8,dib(32)),("dib8",8,dib(8))];
    let mut failures=vec![];
    for (name,id,bytes) in cases{
        place(&owner,if id==0{vec![]}else{vec![data(id,&bytes)]});
        println!("case {name}, enumerated {:?}",formats());
        let at=Instant::now();let mut backup=match Backup::take(){Ok(backup)=>backup,Err(error)=>{failures.push(name);println!("BACKUP FAILED: {name}, {error:?}, elapsed {:?}",at.elapsed());continue;}};
        place(&owner,vec![data(13,&unicode("temporary runtime\0"))]);
        let copied=unsafe{GetClipboardSequenceNumber()};
        let request=TextRequest{pid:std::process::id(),window:owner.0,focus:0,origin:super::super::windows_text::TextOrigin::Focused};
        assert!(backup.restore(Some(copied),&request).is_ok(),"restore {name}");
        if id==0{assert!(formats().is_empty());}else{assert_eq!(read(id,bytes.len()),bytes,"restored {name}");}
        println!("case {name}: backup/replace/restore passed in {:?}",at.elapsed());
    }
    assert!(failures.is_empty(),"native backup failed for {failures:?}");
}
#[test]#[ignore="requires permission to create a new noninteractive window station"]
fn native_backup_round_trip_uses_private_window_station(){
    let mut child=std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored","--exact","capture::windows_copy::isolated_tests::private_clipboard_process","--nocapture"])
        .env("WORDSIDE_ISOLATED_CLIPBOARD_CHILD","1").creation_flags(0x08000000)
        .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    let deadline=Instant::now()+Duration::from_secs(20);
    loop{if child.try_wait().unwrap().is_some(){break;}if Instant::now()>=deadline{let _=child.kill();let _=child.wait();panic!("private clipboard test timed out");}thread::sleep(Duration::from_millis(20));}
    let result=child.wait_with_output().unwrap();
    let stdout=String::from_utf8_lossy(&result.stdout);let stderr=String::from_utf8_lossy(&result.stderr);
    println!("{stdout}");assert!(result.status.success(),"isolated native clipboard failed: {stderr}");
    assert!(stdout.contains("private station verified"));
}
