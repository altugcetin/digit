//! GTK plays a file through a stream with no URI. With GStreamer 1.28, id3demux then
//! gives one track two different ids and decodebin3 aborts the whole process.
//! Hiding the tags at both ends of the file keeps that demuxer out of the pipeline.
use gtk::{gio, glib, glib::subclass::prelude::*, subclass::prelude::*};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

const HEADER: usize = 10;
const ID3V1: u64 = 128;
const APE_FOOTER: u64 = 32;

/// Where the audio begins: after every ID3v2 tag at the start of the file.
fn audio_start(input: &mut (impl Read + Seek)) -> io::Result<u64> {
    let length = input.seek(SeekFrom::End(0))?;
    let mut start = 0;
    loop {
        input.seek(SeekFrom::Start(start))?;
        let mut header = [0; HEADER];
        if input.read_exact(&mut header).is_err() {
            return Ok(start);
        }
        let Some(size) = tag_size(&header) else { return Ok(start) };
        if start + size >= length {
            return Ok(start);
        }
        start += size;
    }
}

fn tag_size(header: &[u8; HEADER]) -> Option<u64> {
    if &header[..3] != b"ID3" || header[3] == 0xff || header[4] == 0xff || header[6..].iter().any(|b| b & 0x80 != 0) {
        return None;
    }
    let body = header[6..].iter().fold(0, |size, b| size << 7 | u64::from(*b));
    let footer = if header[5] & 0x10 != 0 { HEADER as u64 } else { 0 };
    Some(HEADER as u64 + body + footer)
}

/// Where the audio stops: before the ID3v1 and APE tags at the end of the file.
fn audio_end(input: &mut (impl Read + Seek), start: u64) -> io::Result<u64> {
    let mut end = input.seek(SeekFrom::End(0))?;
    loop {
        let mut mark = [0; 3];
        if end >= start + ID3V1 {
            input.seek(SeekFrom::Start(end - ID3V1))?;
            input.read_exact(&mut mark)?;
            if &mark == b"TAG" {
                end -= ID3V1;
                continue;
            }
        }
        if end < start + APE_FOOTER {
            return Ok(end);
        }
        let mut footer = [0; APE_FOOTER as usize];
        input.seek(SeekFrom::Start(end - APE_FOOTER))?;
        input.read_exact(&mut footer)?;
        match ape_size(&footer).filter(|size| end >= start + size) {
            Some(size) => end -= size,
            None => return Ok(end),
        }
    }
}

fn ape_size(footer: &[u8; APE_FOOTER as usize]) -> Option<u64> {
    if &footer[..8] != b"APETAGEX" {
        return None;
    }
    let word = |at: usize| u32::from_le_bytes([footer[at], footer[at + 1], footer[at + 2], footer[at + 3]]);
    let header = if word(20) & 0x8000_0000 != 0 { APE_FOOTER } else { 0 };
    Some(u64::from(word(12)).max(APE_FOOTER) + header)
}

pub fn open(path: &Path) -> io::Result<(File, Range<u64>)> {
    let mut file = File::open(path)?;
    let start = audio_start(&mut file)?;
    let end = audio_end(&mut file, start)?;
    file.seek(SeekFrom::Start(start))?;
    Ok((file, start..end))
}

fn failed(error: impl std::fmt::Display) -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::Failed, &error.to_string())
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Tagless {
        pub source: OnceLock<(Mutex<File>, Range<u64>)>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Tagless {
        const NAME: &'static str = "DigitFilesTaglessAudio";
        type Type = super::Tagless;
        type ParentType = gio::InputStream;
        type Interfaces = (gio::Seekable,);
    }

    impl ObjectImpl for Tagless {}

    impl Tagless {
        fn file(&self) -> Result<(std::sync::MutexGuard<'_, File>, Range<u64>), glib::Error> {
            let (file, audio) = self.source.get().ok_or_else(|| failed("no file"))?;
            Ok((file.lock().map_err(failed)?, audio.clone()))
        }
    }

    impl InputStreamImpl for Tagless {
        fn read(&self, buffer: &mut [u8], _: Option<&gio::Cancellable>) -> Result<usize, glib::Error> {
            let (mut file, audio) = self.file()?;
            let left = audio.end.saturating_sub(file.stream_position().map_err(failed)?);
            let count = buffer.len().min(usize::try_from(left).unwrap_or(usize::MAX));
            file.read(&mut buffer[..count]).map_err(failed)
        }
    }

    impl SeekableImpl for Tagless {
        fn tell(&self) -> i64 {
            self.file().ok().and_then(|(mut file, audio)| Some(file.stream_position().ok()?.saturating_sub(audio.start) as i64)).unwrap_or(0)
        }

        fn can_seek(&self) -> bool {
            true
        }

        fn seek(&self, offset: i64, kind: glib::SeekType, _: Option<&gio::Cancellable>) -> Result<(), glib::Error> {
            let (mut file, audio) = self.file()?;
            let here = file.stream_position().map_err(failed)?;
            let base = match kind {
                glib::SeekType::Set => audio.start,
                glib::SeekType::End => audio.end,
                _ => here,
            };
            let target = base.checked_add_signed(offset).filter(|target| *target >= audio.start).ok_or_else(|| failed("seek before the audio"))?;
            file.seek(SeekFrom::Start(target)).map(|_| ()).map_err(failed)
        }

        fn can_truncate(&self) -> bool {
            false
        }

        fn truncate(&self, _: i64, _: Option<&gio::Cancellable>) -> Result<(), glib::Error> {
            Err(glib::Error::new(gio::IOErrorEnum::NotSupported, "read only"))
        }
    }
}

glib::wrapper! {
    pub struct Tagless(ObjectSubclass<imp::Tagless>) @extends gio::InputStream, @implements gio::Seekable;
}

impl Tagless {
    pub fn new(file: File, audio: Range<u64>) -> Self {
        let stream: Self = glib::Object::new();
        let _ = stream.imp().source.set((Mutex::new(file), audio));
        stream
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::prelude::*;
    use std::io::{Cursor, Write};

    fn tag(body: usize, flags: u8) -> Vec<u8> {
        let size = [(body >> 21) & 127, (body >> 14) & 127, (body >> 7) & 127, body & 127].map(|b| b as u8);
        let mut tag = vec![b'I', b'D', b'3', 4, 0, flags];
        tag.extend(size);
        tag.resize(HEADER + body + if flags & 0x10 != 0 { HEADER } else { 0 }, 0);
        tag
    }

    fn silence() -> Vec<u8> {
        let mut frame = vec![0; 417];
        frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0x00]);
        frame.repeat(200)
    }

    fn id3v1() -> Vec<u8> {
        let mut tag = b"TAG".to_vec();
        tag.resize(ID3V1 as usize, 0);
        tag
    }

    fn ape(items: u32, header: bool) -> Vec<u8> {
        let mut tag = vec![0; items as usize];
        tag.extend(b"APETAGEX");
        tag.extend(2000u32.to_le_bytes());
        tag.extend((items + APE_FOOTER as u32).to_le_bytes());
        tag.extend(0u32.to_le_bytes());
        tag.extend(if header { 0x8000_0000u32 } else { 0 }.to_le_bytes());
        tag.extend([0; 8]);
        if header {
            let mut front = vec![0; APE_FOOTER as usize];
            front.extend(tag);
            return front;
        }
        tag
    }

    #[test]
    fn audio_stops_before_the_tags_at_the_end() {
        let mut bytes = vec![1; 400];
        bytes.extend(ape(40, true));
        bytes.extend(id3v1());
        assert_eq!(audio_end(&mut Cursor::new(&bytes), 0).unwrap(), 400);
        assert_eq!(audio_end(&mut Cursor::new(vec![1; 400]), 0).unwrap(), 400);
        assert_eq!(audio_end(&mut Cursor::new(id3v1()), 0).unwrap(), 0);
    }

    #[test]
    fn audio_starts_after_every_tag() {
        let mut bytes = tag(300, 0);
        bytes.extend(tag(20, 0x10));
        bytes.extend(b"audio");
        assert_eq!(audio_start(&mut Cursor::new(&bytes)).unwrap(), 350);
    }

    #[test]
    fn untagged_short_and_damaged_files_start_at_zero() {
        assert_eq!(audio_start(&mut Cursor::new(b"fLaC audio that is long enough")).unwrap(), 0);
        assert_eq!(audio_start(&mut Cursor::new(b"ID3")).unwrap(), 0);
        let mut oversized = tag(4000, 0);
        oversized.truncate(100);
        assert_eq!(audio_start(&mut Cursor::new(&oversized)).unwrap(), 0);
        let mut damaged = tag(10, 0);
        damaged[6] = 0x80;
        damaged.extend(b"audio");
        assert_eq!(audio_start(&mut Cursor::new(&damaged)).unwrap(), 0);
    }

    fn sample(bytes: &[u8]) -> (tempfile_path::Temp, Tagless) {
        let temp = tempfile_path::Temp::new();
        File::create(&temp.0).unwrap().write_all(bytes).unwrap();
        let (file, audio) = open(&temp.0).unwrap();
        (temp, Tagless::new(file, audio))
    }

    mod tempfile_path {
        pub struct Temp(pub std::path::PathBuf);
        impl Temp {
            pub fn new() -> Self {
                Self(std::env::temp_dir().join(format!("digit-audio-{}-{:?}", std::process::id(), std::thread::current().id())))
            }
        }
        impl Drop for Temp {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
    }

    #[test]
    fn the_stream_hides_the_tag_from_reads_and_seeks() {
        let mut bytes = tag(30, 0);
        bytes.extend(b"abcdef");
        bytes.extend(id3v1());
        let (_temp, stream) = sample(&bytes);
        let seekable = stream.dynamic_cast_ref::<gio::Seekable>().unwrap();
        assert_eq!(seekable.tell(), 0);
        let mut read = [0; 2];
        stream.read_all(&mut read, gio::Cancellable::NONE).unwrap();
        assert_eq!(&read, b"ab");
        seekable.seek(0, glib::SeekType::End, gio::Cancellable::NONE).unwrap();
        assert_eq!(seekable.tell(), 6);
        assert!(seekable.seek(-7, glib::SeekType::Cur, gio::Cancellable::NONE).is_err());
        assert_eq!(seekable.tell(), 6);
        seekable.seek(4, glib::SeekType::Set, gio::Cancellable::NONE).unwrap();
        stream.read_all(&mut read, gio::Cancellable::NONE).unwrap();
        assert_eq!(&read, b"ef");
        assert_eq!(stream.read_all(&mut read, gio::Cancellable::NONE).unwrap().0, 0);
    }

    #[test]
    fn tagged_audio_plays_without_aborting() {
        if !crate::alone("audio::tests::tagged_audio_plays_without_aborting") || gtk::init().is_err() {
            return;
        }
        let mut bytes = tag(64, 0);
        bytes.extend(silence());
        bytes.extend(id3v1());
        let (_temp, stream) = sample(&bytes);
        let context = glib::MainContext::default();
        let media = gtk::MediaFile::for_input_stream(&stream);
        media.set_muted(true);
        media.set_playing(true);
        let until = std::time::Instant::now() + std::time::Duration::from_millis(800);
        while std::time::Instant::now() < until {
            context.iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(media.error().is_none(), "{:?}", media.error());
        media.set_playing(false);
    }
}
