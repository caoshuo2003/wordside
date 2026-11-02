//! Pronunciations: fetched from Cambridge into memory and played with Media Foundation.
//! Nothing is written to disk, and the engine goes away as soon as a sound stops.
use crate::card::allowed_link;
use std::{
    sync::{Once, OnceLock},
    time::Duration,
};
use windows::{
    core::{implement, Error, Interface, Result, BSTR},
    Win32::{
        Foundation::{E_OUTOFMEMORY, HWND, LPARAM, WPARAM},
        Media::MediaFoundation::*,
        System::Com::{
            CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
        },
        UI::{Shell::SHCreateMemStream, WindowsAndMessaging::PostMessageW},
    },
};

const CLIP_LIMIT: usize = 2 * 1024 * 1024;
const FETCH_LIMIT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// Only Cambridge over HTTPS, redirects included, and at most `CLIP_LIMIT` bytes.
pub async fn fetch(url: &str) -> std::result::Result<Vec<u8>, ()> {
    if !allowed_link(url) {
        return Err(());
    }
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    let client = CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(4))
            .user_agent(USER_AGENT)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() > 5 || !allowed_link(attempt.url().as_str()) {
                    attempt.error("Unexpected pronunciation redirect")
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .unwrap_or_default()
    });
    let read = async {
        let mut response = client.get(url).send().await.map_err(|_| ())?;
        if !response.status().is_success() {
            return Err(());
        }
        let mut clip = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
            if clip.len() + chunk.len() > CLIP_LIMIT {
                return Err(());
            }
            clip.extend_from_slice(&chunk);
        }
        Ok(clip)
    };
    tokio::time::timeout(FETCH_LIMIT, read)
        .await
        .map_err(|_| ())?
}

/// Posts `message` with the ticket in `wparam`; `lparam` is 0 when the sound ended, 1 on error.
#[implement(IMFMediaEngineNotify)]
struct Notify {
    hwnd: isize,
    message: u32,
    ticket: usize,
}

impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, _: usize, _: u32) -> Result<()> {
        let failed = match MF_MEDIA_ENGINE_EVENT(event as i32) {
            MF_MEDIA_ENGINE_EVENT_ENDED => 0,
            MF_MEDIA_ENGINE_EVENT_ERROR => 1,
            _ => return Ok(()),
        };
        unsafe {
            PostMessageW(
                Some(HWND(self.hwnd as *mut _)),
                self.message,
                WPARAM(self.ticket),
                LPARAM(failed),
            )
        }
    }
}

pub struct Player(IMFMediaEngine);

impl Player {
    /// `name` only hints the format, e.g. `sound.mp3`. Must run on the window's thread.
    pub fn play(hwnd: HWND, message: u32, ticket: usize, name: &str, clip: &[u8]) -> Result<Self> {
        static START: Once = Once::new();
        unsafe {
            START.call_once(|| {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                let _ = MFStartup(MF_VERSION, MFSTARTUP_LITE);
            });
            let factory: IMFMediaEngineClassFactory =
                CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)?;
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 1)?;
            let attributes = attributes.ok_or_else(|| Error::from(E_OUTOFMEMORY))?;
            let notify: IMFMediaEngineNotify = Notify {
                hwnd: hwnd.0 as isize,
                message,
                ticket,
            }
            .into();
            attributes.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
            let engine = factory.CreateInstance(MF_MEDIA_ENGINE_AUDIOONLY.0 as u32, &attributes)?;
            let player = Self(engine);
            let stream = SHCreateMemStream(Some(clip)).ok_or_else(|| Error::from(E_OUTOFMEMORY))?;
            let source = MFCreateMFByteStreamOnStream(&stream)?;
            player
                .0
                .cast::<IMFMediaEngineEx>()?
                .SetSourceFromByteStream(&source, &BSTR::from(name))?;
            player.0.Play()?;
            Ok(player)
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // Shutdown also breaks the engine's hold on the callback.
        unsafe {
            let _ = self.0.Shutdown();
        }
    }
}

/// The file name in a pronunciation URL, which tells the engine the format.
pub fn clip_name(url: &str) -> &str {
    url.rsplit('/')
        .next()
        .filter(|name| name.contains('.'))
        .unwrap_or("sound.mp3")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_format_hint_comes_from_the_file_name() {
        assert_eq!(
            clip_name("https://dictionary.cambridge.org/media/uk_pron/u/ukr/ukrec/ukrecor014.mp3"),
            "ukrecor014.mp3"
        );
        assert_eq!(
            clip_name("https://dictionary.cambridge.org/x/"),
            "sound.mp3"
        );
    }

    #[test]
    fn only_cambridge_sounds_are_fetched() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        for url in [
            "http://dictionary.cambridge.org/a.mp3",
            "https://example.com/a.mp3",
            "",
        ] {
            assert!(runtime.block_on(fetch(url)).is_err(), "{url}");
        }
    }
}
