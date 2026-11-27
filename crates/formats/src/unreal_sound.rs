//! Unreal Engine 4 cooked sounds: a SoundWave export's data (in the `.uexp`, found with
//! [`crate::unreal_texture::package`]), its sound inline or in the package's `.ubulk`.
//!
//! No public specification says where a cooked sound's fields lie after its tagged properties, so
//! this reads only what the texture reader already reads: the tagged properties up to `None`, the
//! object's guid flag, and bulk-data records (flags, count, size on disk, offset; 64-bit counts
//! with flag 0x2000; the bytes follow unless the payload is elsewhere). The sound is the first such
//! record whose bytes are one whole file in a container with a public specification, so it's saved
//! as it is, without re-encoding:
//! - WAV: a RIFF file of form `WAVE` whose size covers the record exactly, with a `fmt ` chunk
//!   (format tag, channels, rate) and a `data` chunk (Microsoft/IBM Multimedia Programming
//!   Interface and Data Specifications 1.0, 1991; format tags as in RFC 2361);
//! - Ogg: pages (`OggS`, version 0) back to back to the record's end, the first marked beginning
//!   and the last end of stream, numbered without a gap (RFC 3533 section 6); the first packet
//!   names the codec: Vorbis (Vorbis I specification, section 4.2.2) or Opus (RFC 7845 section
//!   5.1).
//!
//! Anything else (Bink Audio, Oodle-compressed or a game's own codec, a sound split into streamed
//! chunks) is refused rather than guessed at.

use crate::common::FormatError;
use crate::unreal_pak::Cur;
use crate::unreal_texture::skip_properties;
use serde::Serialize;

/// Bulk-data flags (as in [`crate::unreal_texture`]).
const PAYLOAD_AT_END: u32 = 0x1;
const ZLIB: u32 = 0x2;
const UNUSED: u32 = 0x20;
const SEPARATE_FILE: u32 = 0x100;
const SIZE_64: u32 = 0x2000;

/// The container a sound is stored in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Container {
    Wav,
    Ogg,
}

impl Container {
    /// The file extension for it.
    pub fn extension(self) -> &'static str {
        match self {
            Container::Wav => "wav",
            Container::Ogg => "ogg",
        }
    }
}

/// Which file a sound's bytes are in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Place {
    /// The export's own data.
    Uexp,
    /// The package's `.ubulk`.
    Ubulk,
}

/// A SoundWave export's sound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Sound {
    pub container: Container,
    /// The codec inside: `PCM`, `IMA ADPCM`, `Vorbis`, `Opus`, `format 0x…`, …
    pub codec: String,
    pub channels: u16,
    /// Samples per second, as the file's header says.
    pub rate: u32,
    pub place: Place,
    /// Where its bytes are in that file: start and length.
    pub range: (usize, usize),
}

fn le_u16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn le_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn le_i64(b: &[u8], at: usize) -> Option<i64> {
    Some(i64::from_le_bytes(b.get(at..at.checked_add(8)?)?.try_into().ok()?))
}

/// What a record's bytes turned out to be.
enum Found {
    Whole(Container, String, u16, u32),
    /// A container's start that doesn't hold a whole file (cut short, or one streamed chunk).
    Part,
}

/// WAVE format tags named here (RFC 2361 appendix A).
fn wave_format(tag: u16) -> String {
    match tag {
        0x0001 => "PCM".into(),
        0x0002 => "Microsoft ADPCM".into(),
        0x0003 => "IEEE float".into(),
        0x0011 => "IMA ADPCM".into(),
        0xFFFE => "extensible".into(),
        _ => format!("format 0x{tag:04X}"),
    }
}

fn riff(b: &[u8]) -> Option<Found> {
    if b.get(..4)? != b"RIFF" || b.get(8..12)? != b"WAVE" {
        return None;
    }
    if usize::try_from(le_u32(b, 4)?).ok()?.checked_add(8) != Some(b.len()) {
        return Some(Found::Part);
    }
    let (mut at, mut fmt, mut data) = (12usize, None, false);
    while at < b.len() {
        let size = usize::try_from(le_u32(b, at + 4).unwrap_or(u32::MAX)).ok()?;
        let body = at + 8;
        let Some(end) = body.checked_add(size).filter(|e| *e <= b.len()) else { return Some(Found::Part) };
        match b.get(at..at + 4)? {
            b"fmt " if size >= 16 => fmt = Some((le_u16(b, body)?, le_u16(b, body + 2)?, le_u32(b, body + 4)?)),
            b"data" => data = true,
            _ => {}
        }
        // Chunks are padded to an even length.
        at = end + (size & 1);
    }
    match (fmt, data) {
        (Some((tag, channels, rate)), true) => Some(Found::Whole(Container::Wav, wave_format(tag), channels, rate)),
        _ => Some(Found::Part),
    }
}

fn ogg(b: &[u8]) -> Option<Found> {
    if b.get(..5)? != b"OggS\0" {
        return None;
    }
    let (mut at, mut first, mut last_flags) = (0usize, None, 0u8);
    // The stream's serial number and the sequence number its next page must have.
    let mut stream: Option<(u32, u32)> = None;
    while at < b.len() {
        // A page: `OggS`, version 0, header flags, granule position, serial and sequence numbers,
        // checksum, the segment count and table, the body.
        let header = b.get(at..at + 27).filter(|h| h.starts_with(b"OggS\0"));
        let Some(header) = header else { return Some(Found::Part) };
        let (flags, segments) = (header[5], usize::from(header[26]));
        let (serial, sequence) = (le_u32(header, 14)?, le_u32(header, 18)?);
        // A missing page breaks the count (RFC 3533 section 6): a stream starts at 0, at a page
        // marked beginning of stream, and each page follows the one before.
        let expected = if flags & 2 != 0 { (serial, 0) } else { stream.unwrap_or((serial, u32::MAX)) };
        if (serial, sequence) != expected {
            return Some(Found::Part);
        }
        stream = Some((serial, sequence.wrapping_add(1)));
        let Some(table) = b.get(at + 27..at + 27 + segments) else { return Some(Found::Part) };
        let body = at + 27 + segments;
        let end = body + table.iter().map(|n| usize::from(*n)).sum::<usize>();
        if end > b.len() {
            return Some(Found::Part);
        }
        first.get_or_insert((flags, body, end));
        last_flags = flags;
        at = end;
    }
    let (first_flags, body, end) = first?;
    // Header flags: 2 beginning of stream, 4 end of stream.
    if first_flags & 2 == 0 || last_flags & 4 == 0 {
        return Some(Found::Part);
    }
    let packet = b.get(body..end)?;
    let (codec, channels, rate) = if packet.starts_with(b"\x01vorbis") {
        ("Vorbis", packet.get(11).copied().map(u16::from), le_u32(packet, 12))
    } else if packet.starts_with(b"OpusHead") {
        ("Opus", packet.get(9).copied().map(u16::from), le_u32(packet, 12))
    } else {
        ("unknown", Some(0), Some(0))
    };
    Some(Found::Whole(Container::Ogg, codec.into(), channels?, rate?))
}

/// A bulk-data record at `at`: its flags, size on disk, offset and header length.
fn record(data: &[u8], at: usize) -> Option<(u32, usize, i64, usize)> {
    let flags = le_u32(data, at)?;
    let (size, offset, header) = if flags & SIZE_64 != 0 {
        (le_i64(data, at + 12)?, le_i64(data, at + 20)?, 28)
    } else {
        (i64::from(i32::from_le_bytes(data.get(at + 8..at + 12)?.try_into().ok()?)), le_i64(data, at + 12)?, 20)
    };
    Some((flags, usize::try_from(size).ok()?, offset, header))
}

/// Find the sound in a SoundWave export's data (`data`: its `serial_size` bytes), with the
/// package's `.ubulk` when it has one.
pub fn sound(data: &[u8], names: &[String], ubulk: Option<&[u8]>) -> Result<Sound, FormatError> {
    let mut c = Cur::new(data);
    skip_properties(&mut c, names)?;
    if c.i32("guid flag")? != 0 {
        c.take(16, "guid")?;
    }
    let mut part = false;
    for at in c.position()..data.len() {
        let Some((flags, size, offset, header)) = record(data, at) else { continue };
        // At the end of the .uexp (rather than a separate file) isn't read, as for textures.
        let at_end = flags & PAYLOAD_AT_END != 0 && flags & SEPARATE_FILE == 0;
        if size < 12 || at_end || flags & (ZLIB | UNUSED) != 0 {
            continue;
        }
        let (place, start, bytes) = if flags & SEPARATE_FILE != 0 {
            let Some(start) = usize::try_from(offset).ok() else { continue };
            (Place::Ubulk, start, ubulk.and_then(|u| u.get(start..start.checked_add(size)?)))
        } else {
            (Place::Uexp, at + header, data.get(at + header..(at + header).saturating_add(size)))
        };
        let Some(bytes) = bytes else { continue };
        match riff(bytes).or_else(|| ogg(bytes)) {
            Some(Found::Whole(container, codec, channels, rate)) => return Ok(Sound { container, codec, channels, rate, place, range: (start, size) }),
            Some(Found::Part) => part = true,
            None => {}
        }
    }
    Err(FormatError::Unsupported(if part {
        "a sound whose stored bytes aren't one whole WAV or Ogg file (it may be streamed in parts, which aren't joined here)".into()
    } else {
        "a sound that isn't stored as a WAV or Ogg file (Bink Audio, Oodle-compressed and other codecs aren't read here)".into()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unreal_texture::package;
    use craft_anything_testkit::games::{ogg_file, unreal_sound_package, wav_file};

    fn export_data<'a>(uasset: &[u8], uexp: &'a [u8]) -> (&'a [u8], Vec<String>) {
        let p = package(uasset).unwrap();
        let e = &p.exports[0];
        assert_eq!(e.class, "SoundWave");
        let start = (e.serial_offset - p.header_size) as usize;
        (&uexp[start..start + e.serial_size as usize], p.names)
    }

    #[test]
    fn finds_a_wav_inline_and_an_ogg_in_the_ubulk() {
        let wav = wav_file(1, 2, 22050, &[0; 64]);
        let (uasset, uexp) = unreal_sound_package("S_Click", "ADPCM", &wav, false);
        let (data, names) = export_data(&uasset, &uexp);
        let s = sound(data, &names, None).unwrap();
        assert_eq!((s.container, s.codec.as_str(), s.channels, s.rate, s.place), (Container::Wav, "PCM", 2, 22050, Place::Uexp));
        assert_eq!(&data[s.range.0..s.range.0 + s.range.1], &wav[..]);

        let ogg = ogg_file(b"\x01vorbis", 1, 44100, 3);
        let (uasset, uexp) = unreal_sound_package("S_Theme", "OGG", &ogg, true);
        let (data, names) = export_data(&uasset, &uexp);
        assert!(sound(data, &names, None).is_err(), "without its .ubulk there's nothing to read");
        let s = sound(data, &names, Some(&ogg)).unwrap();
        assert_eq!((s.container, s.codec.as_str(), s.channels, s.rate, s.place, s.range), (Container::Ogg, "Vorbis", 1, 44100, Place::Ubulk, (0, ogg.len())));
        let opus = ogg_file(b"OpusHead", 2, 48000, 2);
        let (uasset, uexp) = unreal_sound_package("S_Voice", "OPUS", &opus, false);
        let (data, names) = export_data(&uasset, &uexp);
        assert_eq!(sound(data, &names, None).unwrap().codec, "Opus");
    }

    #[test]
    fn refuses_what_isnt_a_whole_wav_or_ogg() {
        let (uasset, uexp) = unreal_sound_package("S", "BINKA", b"1FCB\x01\x02\x00\x00some bink-like bytes", false);
        let (data, names) = export_data(&uasset, &uexp);
        assert!(matches!(sound(data, &names, None), Err(FormatError::Unsupported(m)) if m.contains("isn't stored as")));

        // The first of several Ogg pages, or a WAV whose RIFF size names more than is stored.
        let ogg = ogg_file(b"\x01vorbis", 1, 44100, 3);
        let segments = usize::from(ogg[26]);
        let first_page = 27 + segments + ogg[27..27 + segments].iter().map(|n| usize::from(*n)).sum::<usize>();
        let wav = wav_file(1, 1, 8000, &[0; 32]);
        // Its pages without the middle one: beginning and end of stream, but not one whole file.
        let page_end = |at: usize| {
            let segments = usize::from(ogg[at + 26]);
            at + 27 + segments + ogg[at + 27..at + 27 + segments].iter().map(|n| usize::from(*n)).sum::<usize>()
        };
        let gap = [&ogg[..first_page], &ogg[page_end(first_page)..]].concat();
        for cut in [&ogg[..first_page], &wav[..wav.len() - 4], &gap[..]] {
            let (uasset, uexp) = unreal_sound_package("S", "OGG", cut, false);
            let (data, names) = export_data(&uasset, &uexp);
            assert!(matches!(sound(data, &names, None), Err(FormatError::Unsupported(m)) if m.contains("in parts")), "{}", cut.len());
        }

        let (uasset, uexp) = unreal_sound_package("S", "ADPCM", &wav, false);
        let (data, names) = export_data(&uasset, &uexp);
        for cut in 0..data.len() {
            let _ = sound(&data[..cut], &names, None);
        }
    }
}
