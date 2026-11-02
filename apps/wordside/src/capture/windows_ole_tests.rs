//! In-process IDataObject fixture. Never calls any clipboard/window/input API.
use super::*;
use ::windows::{
    core::{IUnknown, IUnknown_Vtbl, Interface, GUID, HRESULT},
    Win32::System::Com::IDataObject_Vtbl,
};
use std::sync::atomic::{AtomicU32, Ordering};
#[link(name = "kernel32")]
extern "system" {
    fn GlobalAlloc(flags: u32, size: usize) -> Handle;
}
#[link(name = "user32")]
extern "system" {
    fn RegisterClipboardFormatW(name: *const u16) -> u32;
}
#[link(name = "gdi32")]
extern "system" {
    fn SetEnhMetaFileBits(size: u32, bits: *const u8) -> Handle;
    fn CreateEnhMetaFileW(
        dc: Handle,
        file: *const u16,
        rect: *const c_void,
        text: *const u16,
    ) -> Handle;
    fn CloseEnhMetaFile(dc: Handle) -> Handle;
    fn Rectangle(dc: Handle, left: i32, top: i32, right: i32, bottom: i32) -> i32;
}
fn metafile_bits() -> Vec<u8> {
    unsafe {
        let dc = CreateEnhMetaFileW(0, ptr::null(), ptr::null(), ptr::null());
        Rectangle(dc, 1, 1, 20, 20);
        let metafile = CloseEnhMetaFile(dc);
        let size = GetEnhMetaFileBits(metafile, 0, ptr::null_mut());
        let mut bits = vec![0u8; size as usize];
        GetEnhMetaFileBits(metafile, size, bits.as_mut_ptr());
        DeleteEnhMetaFile(metafile);
        bits
    }
}
#[link(name = "shell32")]
extern "system" {
    fn SHCreateStdEnumFmtEtc(
        count: u32,
        formats: *const FORMATETC,
        output: *mut *mut c_void,
    ) -> HRESULT;
}
#[repr(C)]
struct Provider {
    vtable: *const IDataObject_Vtbl,
    references: AtomicU32,
    items: Vec<(FORMATETC, Vec<u8>)>,
}
unsafe extern "system" fn query(
    this: *mut c_void,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    if iid.is_null() || out.is_null() {
        return HRESULT(0x80004003u32 as i32);
    }
    *out = ptr::null_mut();
    if *iid == IDataObject::IID || *iid == IUnknown::IID {
        *out = this;
        add(this);
        HRESULT(0)
    } else {
        HRESULT(0x80004002u32 as i32)
    }
}
unsafe extern "system" fn add(this: *mut c_void) -> u32 {
    (*(this as *mut Provider))
        .references
        .fetch_add(1, Ordering::SeqCst)
        + 1
}
unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    let count = (*(this as *mut Provider))
        .references
        .fetch_sub(1, Ordering::SeqCst)
        - 1;
    if count == 0 {
        drop(Box::from_raw(this as *mut Provider));
    }
    count
}
unsafe extern "system" fn get(
    this: *mut c_void,
    format: *const FORMATETC,
    out: *mut STGMEDIUM,
) -> HRESULT {
    if format.is_null() || out.is_null() {
        return HRESULT(0x80004003u32 as i32);
    }
    let own = &*(this as *mut Provider);
    let Some((_, bytes)) = own
        .items
        .iter()
        .find(|(value, _)| value.cfFormat == (*format).cfFormat)
    else {
        return HRESULT(0x80040064u32 as i32);
    };
    // An enhanced metafile is drawn from its stored bits, "fail" is a source error,
    // and no bits means a metafile the source declares but cannot draw.
    if (*format).tymed == 64 && !bytes.is_empty() {
        if bytes == b"fail" {
            return HRESULT(0x80004005u32 as i32);
        }
        let metafile = SetEnhMetaFileBits(bytes.len() as u32, bytes.as_ptr());
        out.write(STGMEDIUM {
            tymed: 64,
            u: ::windows::Win32::System::Com::STGMEDIUM_0 {
                hEnhMetaFile: ::windows::Win32::Graphics::Gdi::HENHMETAFILE(metafile as _),
            },
            ..Default::default()
        });
        return HRESULT(0);
    }
    if (*format).tymed != 1 {
        return HRESULT(0x80040069u32 as i32);
    }
    let data = GlobalAlloc(2, bytes.len());
    if data == 0 {
        return HRESULT(0x8007000eu32 as i32);
    }
    let dest = GlobalLock(data) as *mut u8;
    if dest.is_null() {
        GlobalFree(data);
        return HRESULT(0x8007000eu32 as i32);
    }
    ptr::copy_nonoverlapping(bytes.as_ptr(), dest, bytes.len());
    GlobalUnlock(data);
    out.write(STGMEDIUM {
        tymed: 1,
        u: ::windows::Win32::System::Com::STGMEDIUM_0 {
            hGlobal: ::windows::Win32::Foundation::HGLOBAL(data as _),
        },
        ..Default::default()
    });
    HRESULT(0)
}
unsafe extern "system" fn get_here(
    _: *mut c_void,
    _: *const FORMATETC,
    _: *mut STGMEDIUM,
) -> HRESULT {
    HRESULT(0x80004001u32 as i32)
}
unsafe extern "system" fn query_data(_: *mut c_void, _: *const FORMATETC) -> HRESULT {
    HRESULT(0x80004001u32 as i32)
}
unsafe extern "system" fn canonical(
    _: *mut c_void,
    _: *const FORMATETC,
    _: *mut FORMATETC,
) -> HRESULT {
    HRESULT(0x80004001u32 as i32)
}
unsafe extern "system" fn set(
    _: *mut c_void,
    _: *const FORMATETC,
    _: *const STGMEDIUM,
    _: ::windows::core::BOOL,
) -> HRESULT {
    HRESULT(0x80004001u32 as i32)
}
unsafe extern "system" fn enumerate(
    this: *mut c_void,
    direction: u32,
    out: *mut *mut c_void,
) -> HRESULT {
    if direction != 1 {
        return HRESULT(0x80004001u32 as i32);
    }
    let own = &*(this as *mut Provider);
    let formats: Vec<_> = own.items.iter().map(|(format, _)| *format).collect();
    SHCreateStdEnumFmtEtc(formats.len() as u32, formats.as_ptr(), out)
}
unsafe extern "system" fn advise(
    _: *mut c_void,
    _: *const FORMATETC,
    _: u32,
    _: *mut c_void,
    _: *mut u32,
) -> HRESULT {
    HRESULT(0x80040003u32 as i32)
}
unsafe extern "system" fn unadvise(_: *mut c_void, _: u32) -> HRESULT {
    HRESULT(0x80040003u32 as i32)
}
unsafe extern "system" fn enum_advise(_: *mut c_void, _: *mut *mut c_void) -> HRESULT {
    HRESULT(0x80040003u32 as i32)
}
static VTABLE: IDataObject_Vtbl = IDataObject_Vtbl {
    base__: IUnknown_Vtbl {
        QueryInterface: query,
        AddRef: add,
        Release: release,
    },
    GetData: get,
    GetDataHere: get_here,
    QueryGetData: query_data,
    GetCanonicalFormatEtc: canonical,
    SetData: set,
    EnumFormatEtc: enumerate,
    DAdvise: advise,
    DUnadvise: unadvise,
    EnumDAdvise: enum_advise,
};
fn provider(items: Vec<(FORMATETC, Vec<u8>)>) -> IDataObject {
    unsafe {
        IDataObject::from_raw(Box::into_raw(Box::new(Provider {
            vtable: &VTABLE,
            references: AtomicU32::new(1),
            items,
        })) as _)
    }
}
fn format(id: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: id,
        dwAspect: 1,
        lindex: -1,
        tymed: 1,
        ..Default::default()
    }
}
fn custom(name: &str) -> u16 {
    unsafe {
        let name: Vec<_> = name.encode_utf16().chain([0]).collect();
        let id = RegisterClipboardFormatW(name.as_ptr());
        assert!(id >= 0xc000);
        id as u16
    }
}
fn saved_bytes(value: &Format, len: usize) -> Vec<u8> {
    unsafe {
        let pointer = GlobalLock(value.data) as *const u8;
        assert!(!pointer.is_null());
        let bytes = std::slice::from_raw_parts(pointer, len).to_vec();
        GlobalUnlock(value.data);
        bytes
    }
}
#[test]
fn ole_text_and_private_payloads_are_independent_after_provider_is_destroyed() {
    let text: Vec<u8> = "before 中文\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let id = custom("application/x-moz-custom-clipdata");
    let private = vec![0, 1, 2, 3, 4, 5, 0];
    let source = provider(vec![
        (format(13), text.clone()),
        (format(id), private.clone()),
    ]);
    let snapshot = materialize(&source, false, &mut 0, || true).unwrap();
    drop(source);
    assert!(snapshot.rendered.is_empty() && snapshot.empty.is_empty());
    let saved = snapshot.saved;
    assert_eq!(saved.len(), 2);
    assert_eq!(saved_bytes(&saved[0], text.len()), text);
    assert_eq!(saved_bytes(&saved[1], private.len()), private);
}
#[test]
fn stream_storage_and_metafiles_are_read_from_the_win32_rendering_not_refused() {
    let id = custom("Embed Source");
    let source = provider(vec![
        (format(13), vec![0, 0]),
        (
            FORMATETC {
                tymed: 8,
                ..format(id)
            },
            vec![],
        ),
        (
            FORMATETC {
                tymed: 4,
                ..format(1)
            },
            vec![],
        ),
        (
            FORMATETC {
                tymed: 64,
                ..format(14)
            },
            metafile_bits(),
        ),
    ]);
    let snapshot = materialize(&source, false, &mut 0, || true).unwrap();
    let ids: Vec<_> = snapshot.saved.iter().map(|value| value.id).collect();
    assert_eq!(ids, [13]);
    assert_eq!(snapshot.rendered, [u32::from(id), 1, 14]);
    assert!(snapshot.empty.is_empty());
    // As on a tester's PC: a metafile picture declared (also as global memory) that
    // the source cannot draw. Nothing can paste it, so it is noted, not refused.
    let source = provider(vec![
        (format(13), vec![0, 0]),
        (
            FORMATETC {
                tymed: 1 | 32,
                ..format(3)
            },
            vec![],
        ),
        (
            FORMATETC {
                tymed: 64,
                ..format(14)
            },
            vec![],
        ),
    ]);
    let snapshot = materialize(&source, false, &mut 0, || true).unwrap();
    assert!(snapshot.rendered.is_empty());
    assert_eq!(snapshot.empty, [3, 14]);
    assert!(undeliverable(&Error::from_hresult(HRESULT(
        0x800401d3u32 as i32
    ))));
    // Any other failure may be passing, so the backup is refused as before.
    let source = provider(vec![(
        FORMATETC {
            tymed: 64,
            ..format(14)
        },
        b"fail".to_vec(),
    )]);
    let error = materialize(&source, false, &mut 0, || true).err().unwrap();
    assert_eq!(
        (error.stage, error.format_id),
        (BackupStage::OleRead, Some(14))
    );
    // Only these representations: an unknown medium still has no Win32 copy.
    let source = provider(vec![(
        FORMATETC {
            tymed: 128,
            ..format(1)
        },
        vec![],
    )]);
    let error = materialize(&source, false, &mut 0, || true).err().unwrap();
    assert_eq!(error.stage, BackupStage::UnsupportedFormat);
}
#[test]
fn indexed_target_specific_and_conflicting_formats_refuse_before_copy() {
    let ordinary = (format(13), vec![0, 0]);
    for invalid in [
        FORMATETC {
            lindex: 0,
            tymed: 4,
            ..format(1)
        },
        FORMATETC {
            lindex: 0,
            ..format(1)
        },
        FORMATETC {
            dwAspect: 2,
            ..format(1)
        },
    ] {
        let source = provider(vec![ordinary.clone(), (invalid, vec![0])]);
        let error = materialize(&source, false, &mut 0, || true).err().unwrap();
        assert_eq!(error.stage, BackupStage::UnsupportedFormat);
    }
    let source = provider(vec![ordinary.clone(), ordinary]);
    assert_eq!(
        materialize(&source, false, &mut 0, || true)
            .err()
            .unwrap()
            .stage,
        BackupStage::UnsupportedFormat
    );
    // Two declarations of one rendered format are just as ambiguous.
    let stream = FORMATETC {
        tymed: 4,
        ..format(1)
    };
    let source = provider(vec![(stream, vec![]), (stream, vec![])]);
    assert_eq!(
        materialize(&source, false, &mut 0, || true)
            .err()
            .unwrap()
            .stage,
        BackupStage::UnsupportedFormat
    );
    assert!(!static_format(&FORMATETC {
        ptd: 1usize as _,
        ..format(1)
    }));
}
#[test]
fn source_replacement_and_size_limit_discard_the_whole_snapshot() {
    let source = provider(vec![(format(13), vec![0, 0])]);
    let calls = std::cell::Cell::new(0);
    let error = materialize(&source, false, &mut 0, || {
        let call = calls.get() + 1;
        calls.set(call);
        call < 3
    })
    .err()
    .unwrap();
    assert_eq!(error.stage, BackupStage::SourceChanged);
    assert_eq!(
        materialize(&source, false, &mut { MAX_BYTES }, || true)
            .err()
            .unwrap()
            .stage,
        BackupStage::SizeLimit
    );
}
#[test]
fn ole_transport_metadata_declared_as_real_payload_is_not_silently_dropped() {
    let id = custom("DataObject");
    let source = provider(vec![(format(id), vec![0; 8])]);
    let error = materialize(&source, false, &mut 0, || true).err().unwrap();
    assert_eq!(error.stage, BackupStage::UnsupportedFormat);
}
#[test]
fn copied_files_on_disk_keep_their_paths_without_the_virtual_contents_view() {
    let drop = (format(15), vec![1, 2, 3, 4, 5, 6]);
    let names = (format(custom("FileNameW")), vec![b'C', 0, 0, 0]);
    let descriptor = (format(custom("FileGroupDescriptorW")), vec![1, 0, 0, 0]);
    let contents = (
        FORMATETC {
            tymed: 4,
            ..format(custom("FileContents"))
        },
        vec![],
    );
    let source = provider(vec![
        drop.clone(),
        descriptor.clone(),
        contents.clone(),
        names.clone(),
    ]);
    let saved = materialize(&source, true, &mut 0, || true).unwrap().saved;
    assert_eq!(
        saved.iter().map(|value| value.id).collect::<Vec<_>>(),
        [15, u32::from(names.0.cfFormat)],
        "paths are kept and the stream pair is left out"
    );
    assert_eq!(saved_bytes(&saved[0], drop.1.len()), drop.1);
    // Virtual files without CF_HDROP have no other copy of their data.
    let source = provider(vec![descriptor, contents.clone(), names]);
    let error = materialize(&source, false, &mut 0, || true).err().unwrap();
    assert_eq!(
        (error.stage, error.format_id),
        (
            BackupStage::UnsupportedFormat,
            Some(u32::from(contents.0.cfFormat))
        )
    );
}
#[test]
fn only_the_virtual_file_pair_counts_as_a_second_view_of_copied_files() {
    for name in [
        "FileContents",
        "FileGroupDescriptor",
        "FileGroupDescriptorW",
    ] {
        assert!(file_contents_view(u32::from(custom(name))), "{name}");
    }
    for name in [
        "Shell IDList Array",
        "FileNameW",
        "Preferred DropEffect",
        "DataObject",
    ] {
        assert!(!file_contents_view(u32::from(custom(name))), "{name}");
    }
    assert!(!file_contents_view(CF_HDROP));
}
