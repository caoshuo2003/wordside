//! Pronunciations: fetched from Cambridge into memory, decoded with Media Foundation and
//! played with XAudio2. Nothing is written to disk.
//!
//! A new audio stream drops the first fraction of a second, which is the start of the word
//! (`dictionary` came out as `tionary`), and switching readings opened a new one each time.
//! So one stream stays open while sounds are being played and each sound goes into it; the
//! first sound after it opens has silence in front, for the device to wake up during.
use crate::card::allowed_link;
use std::{
    sync::{Once, OnceLock},
    time::Duration,
};
use windows::{
    core::{Error, Interface, Result, PCWSTR},
    Win32::{
        Foundation::E_FAIL,
        Media::{
            Audio::{AudioCategory_Other, XAudio2::*, WAVEFORMATEX},
            MediaFoundation::*,
        },
        System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED},
        UI::Shell::SHCreateMemStream,
    },
};

const CLIP_LIMIT: usize = 2 * 1024 * 1024;
/// Decoded sound, about a minute of CD audio: far more than any pronunciation.
const PCM_LIMIT: usize = 12 * 1024 * 1024;
const FETCH_LIMIT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
/// Silence before the first sound in a newly opened stream: Bluetooth headsets and
/// power-saving sound cards take about this long to wake, and drop what plays meanwhile.
const WAKE_UP: Duration = Duration::from_millis(400);
const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xfffe;

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

fn start() {
    static START: Once = Once::new();
    START.call_once(|| unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let _ = MFStartup(MF_VERSION, MFSTARTUP_LITE);
    });
}

/// A sound decoded to 16-bit PCM.
pub struct Pcm {
    format: WAVEFORMATEX,
    data: Vec<u8>,
}

/// `name` only hints the format, e.g. `sound.mp3`. Must run on the window's thread.
pub fn decode(name: &str, clip: &[u8]) -> Result<Pcm> {
    start();
    unsafe {
        let stream = SHCreateMemStream(Some(clip)).ok_or_else(|| Error::from(E_FAIL))?;
        let source = MFCreateMFByteStreamOnStream(&stream)?;
        if name.ends_with(".mp3") {
            if let Ok(attributes) = source.cast::<IMFAttributes>() {
                let _ = attributes
                    .SetString(&MF_BYTESTREAM_CONTENT_TYPE, windows::core::w!("audio/mpeg"));
            }
        }
        let reader = MFCreateSourceReaderFromByteStream(&source, None)?;
        let audio = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
        let wanted = MFCreateMediaType()?;
        wanted.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        wanted.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        wanted.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        reader.SetCurrentMediaType(audio, None, &wanted)?;
        let actual = reader.GetCurrentMediaType(audio)?;
        let mut wave = std::ptr::null_mut();
        MFCreateWaveFormatExFromMFMediaType(&actual, &mut wave, None, 0)?;
        let mut format = *wave;
        CoTaskMemFree(Some(wave as *const _));
        // Plain 16-bit PCM of one or two channels needs nothing the extensible form adds.
        if format.wFormatTag == WAVE_FORMAT_EXTENSIBLE {
            format.wFormatTag = WAVE_FORMAT_PCM;
            format.cbSize = 0;
        }
        if format.wFormatTag != WAVE_FORMAT_PCM || format.nBlockAlign == 0 {
            return Err(Error::from(E_FAIL));
        }
        let mut data = Vec::new();
        loop {
            let mut flags = 0u32;
            let mut sample = None;
            reader.ReadSample(audio, 0, None, Some(&mut flags), None, Some(&mut sample))?;
            if let Some(sample) = sample {
                let buffer = sample.ConvertToContiguousBuffer()?;
                let (mut bytes, mut length) = (std::ptr::null_mut(), 0u32);
                buffer.Lock(&mut bytes, None, Some(&mut length))?;
                let fits = data.len() + length as usize <= PCM_LIMIT;
                if fits {
                    data.extend_from_slice(std::slice::from_raw_parts(bytes, length as usize));
                }
                buffer.Unlock()?;
                if !fits {
                    return Err(Error::from(E_FAIL));
                }
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
        }
        Ok(Pcm { format, data })
    }
}

/// The audio stream sounds go into, open while sounds are being played.
pub struct Speaker {
    engine: IXAudio2,
    master: IXAudio2MasteringVoice,
    /// The sound playing and the samples it reads, which must outlive it.
    voice: Option<(IXAudio2SourceVoice, Vec<u8>)>,
    /// Nothing has played since the stream opened, so the device may still be waking.
    fresh: bool,
}

impl Speaker {
    pub fn open() -> Result<Self> {
        start();
        unsafe {
            let mut engine = None;
            XAudio2CreateWithVersionInfo(&mut engine, 0, XAUDIO2_DEFAULT_PROCESSOR, 0x0a00_0000)?;
            let engine = engine.ok_or_else(|| Error::from(E_FAIL))?;
            let mut master = None;
            engine.CreateMasteringVoice(
                &mut master,
                0,
                0,
                0,
                PCWSTR::null(),
                None,
                AudioCategory_Other,
            )?;
            let master = master.ok_or_else(|| Error::from(E_FAIL))?;
            Ok(Self {
                engine,
                master,
                voice: None,
                fresh: true,
            })
        }
    }

    /// Plays `pcm` in place of whatever was playing.
    pub fn play(&mut self, pcm: &Pcm) -> Result<()> {
        self.stop();
        let format = pcm.format;
        let lead = if self.fresh {
            let frames =
                (format.nSamplesPerSec as u64 * WAKE_UP.as_millis() as u64 / 1000) as usize;
            frames * format.nBlockAlign as usize
        } else {
            0
        };
        self.fresh = false;
        let mut samples = vec![0u8; lead];
        samples.extend_from_slice(&pcm.data);
        unsafe {
            let mut voice = None;
            self.engine.CreateSourceVoice(
                &mut voice,
                &format,
                0,
                XAUDIO2_DEFAULT_FREQ_RATIO,
                None,
                None,
                None,
            )?;
            let voice = voice.ok_or_else(|| Error::from(E_FAIL))?;
            let buffer = XAUDIO2_BUFFER {
                Flags: XAUDIO2_END_OF_STREAM,
                AudioBytes: samples.len() as u32,
                pAudioData: samples.as_ptr(),
                ..Default::default()
            };
            let started = voice
                .SubmitSourceBuffer(&buffer, None)
                .and_then(|_| voice.Start(0, XAUDIO2_COMMIT_NOW));
            // Kept with its samples even on failure, so `stop` destroys it before they go.
            self.voice = Some((voice, samples));
            started
        }
    }

    /// The sound is still playing.
    pub fn playing(&self) -> bool {
        self.voice.as_ref().is_some_and(|(voice, _)| unsafe {
            let mut state = XAUDIO2_VOICE_STATE::default();
            voice.GetState(&mut state, 0);
            state.BuffersQueued > 0
        })
    }

    /// Silences the sound; the stream stays open for the next one.
    pub fn stop(&mut self) {
        if let Some((voice, samples)) = self.voice.take() {
            unsafe { voice.DestroyVoice() };
            drop(samples);
        }
    }
}

impl Drop for Speaker {
    fn drop(&mut self) {
        self.stop();
        unsafe {
            self.master.DestroyVoice();
            self.engine.StopEngine();
        }
    }
}

/// The file name in a pronunciation URL, which tells the decoder the format.
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

    /// A 16-bit mono WAV of `frames` samples of a tone.
    fn tone(rate: u32, frames: u32) -> Vec<u8> {
        let data: Vec<u8> = (0..frames)
            .flat_map(|i| {
                {
                    let t = i as f32 / rate as f32;
                    ((t * 440.0 * std::f32::consts::TAU).sin() * 8000.0) as i16 as u16
                }
                .to_le_bytes()
            })
            .collect();
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        for value in [16u32, 1 | (1 << 16), rate, rate * 2, 2 | (16 << 16)] {
            wav.extend_from_slice(&value.to_le_bytes());
        }
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        wav
    }

    /// Plays at volume 0 on the real audio device: `cargo test -- --ignored
    /// every_sample_plays_from_the_first`. Every sample of the sound, its first ones
    /// included, must come out, after the wake-up silence the first time only.
    #[test]
    #[ignore]
    fn every_sample_plays_from_the_first() {
        let (rate, frames) = (22050, 22050 * 6 / 10);
        let pcm = decode("tone.wav", &tone(rate, frames)).unwrap();
        assert_eq!({ pcm.format.nSamplesPerSec }, rate);
        assert_eq!(
            pcm.data.len(),
            frames as usize * pcm.format.nBlockAlign as usize
        );
        let mut speaker = Speaker::open().unwrap();
        unsafe { speaker.master.SetVolume(0.0, 0).unwrap() };
        let lead = rate as u64 * WAKE_UP.as_millis() as u64 / 1000;
        for expected in [lead + frames as u64, frames as u64, frames as u64] {
            speaker.play(&pcm).unwrap();
            // The count starts at the sound's first sample and is reset once it ends, so
            // the most seen before that is how far it got.
            let (started, mut most) = (std::time::Instant::now(), 0u64);
            loop {
                let mut state = XAUDIO2_VOICE_STATE::default();
                let (voice, _) = speaker.voice.as_ref().unwrap();
                unsafe { voice.GetState(&mut state, 0) };
                if state.BuffersQueued == 0 {
                    break;
                }
                most = most.max(state.SamplesPlayed);
                assert!(started.elapsed() < Duration::from_secs(3), "never ended");
                std::thread::sleep(Duration::from_millis(1));
            }
            let close = rate as u64 / 50;
            assert!(
                most <= expected && most + close >= expected,
                "{most} of {expected}"
            );
        }
        // Switching mid-sound: the new one starts from its first sample too.
        speaker.play(&pcm).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        speaker.play(&pcm).unwrap();
        let mut state = XAUDIO2_VOICE_STATE::default();
        let (voice, _) = speaker.voice.as_ref().unwrap();
        unsafe { voice.GetState(&mut state, 0) };
        let played = state.SamplesPlayed;
        assert!(played < frames as u64 / 4, "{played}");
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
