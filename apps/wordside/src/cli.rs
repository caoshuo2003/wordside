//! Command-line entries that never open a window, read settings or touch the tray. Same
//! names and output as 0.2.0, so the release checks and troubleshooting notes still apply.
//! `--wordside-dictionary-probe [lang:]word...` queries Cambridge and prints each payload,
//! `--wordside-dictionary-parse file...` parses saved pages, and
//! `--wordside-dictionary-licenses` prints the third-party notices.
use crate::{dictionary, lookup::fetch_payload, selection_input::selected_word};
use std::io::Read;
use windows::Win32::System::Console::{
    AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE,
};

const LICENSES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/THIRD-PARTY-LICENSES.deflate"));

/// The third-party notices, inflated when someone asks for them.
pub fn licenses() -> String {
    miniz_oxide::inflate::decompress_to_vec(LICENSES)
        .ok()
        .and_then(|text| String::from_utf8(text).ok())
        .unwrap_or_default()
}
/// Saved pages are read up to the same limit the network reader enforces.
const PAGE_LIMIT: u64 = 4 * 1024 * 1024;

/// True when the arguments asked for one of these entries and it has run.
pub fn run() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(flag) = args.first() else {
        return false;
    };
    let rest = &args[1..];
    match flag.as_str() {
        "--wordside-dictionary-licenses" => {
            attach_console();
            print!("{}", licenses());
        }
        "--wordside-dictionary-parse" => {
            attach_console();
            parse(rest);
        }
        "--wordside-dictionary-probe" => {
            attach_console();
            probe(rest);
        }
        _ => return false,
    }
    true
}

/// The exe is a windowed program, so output only shows in a console it attaches to; when
/// the output is redirected to a file or pipe it already has somewhere to go.
fn attach_console() {
    unsafe {
        if GetStdHandle(STD_OUTPUT_HANDLE).is_ok_and(|handle| !handle.is_invalid()) {
            return;
        }
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

fn read_page(path: &str) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|_| "无法读取对照文件".to_string())?;
    let mut bytes = Vec::new();
    file.take(PAGE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取对照文件".to_string())?;
    if bytes.len() as u64 > PAGE_LIMIT {
        return Err("对照文件过大".into());
    }
    String::from_utf8(bytes).map_err(|_| "对照文件编码无效".to_string())
}

fn parse(paths: &[String]) {
    for path in paths {
        let payload = match read_page(path) {
            Ok(html) => match dictionary::parser::parse_page(&html) {
                Ok(entry) => serde_json::json!({"entry":entry}),
                Err(reason) => {
                    serde_json::json!({"entry":null,"apiError":reason.message,"apiStatus":reason.status,"apiCode":reason.code})
                }
            },
            Err(error) => serde_json::json!({"entry":null,"apiError":error}),
        };
        println!("{}", serde_json::json!({"file":path,"payload":payload}));
    }
}

fn probe(queries: &[String]) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    runtime.block_on(async {
        let backend = dictionary::Backend::default();
        println!(
            "{}",
            serde_json::json!({"phase":"initial","backend":backend.diagnostic()})
        );
        for query in queries {
            let (language, raw) = query.split_once(':').unwrap_or(("en-cn", query.as_str()));
            let word = selected_word(raw);
            let started = std::time::Instant::now();
            let payload = if let Some(word) = word.as_deref() {
                fetch_payload(&backend, word, language)
                    .await
                    .unwrap_or_else(|reason| serde_json::json!({"entry":null,"apiError":reason}))
            } else {
                serde_json::json!({"entry":null,"apiError":"请选择单行英文单词或短语"})
            };
            println!(
                "{}",
                serde_json::json!({"selection":raw,"word":word,"language":language,"payload":payload,"ms":started.elapsed().as_secs_f64()*1000.0,"backend":backend.diagnostic()})
            );
        }
        backend.close();
        println!(
            "{}",
            serde_json::json!({"phase":"closed","backend":backend.diagnostic()})
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_pages_are_bounded_and_must_be_text() {
        let folder = std::env::temp_dir().join(format!("wordside-cli-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let big = folder.join("big.html");
        std::fs::write(&big, vec![b'a'; PAGE_LIMIT as usize + 1]).unwrap();
        let binary = folder.join("binary.html");
        std::fs::write(&binary, [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(
            read_page(big.to_str().unwrap()).unwrap_err(),
            "对照文件过大"
        );
        assert_eq!(
            read_page(binary.to_str().unwrap()).unwrap_err(),
            "对照文件编码无效"
        );
        assert_eq!(
            read_page("no-such-file.html").unwrap_err(),
            "无法读取对照文件"
        );
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn the_notices_keep_the_parser_authors_credit() {
        // The real list comes from tools/build-licenses.mjs; a placeholder otherwise says how.
        let notices = licenses();
        assert!(notices.contains("WordSide") || notices.contains("build-licenses"));
    }
}
