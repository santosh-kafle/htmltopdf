//! `@font-face` fonts.
//!
//! A face's font is loaded the first time text asks for its family, as
//! browsers do, so unused faces cost nothing. Each family is registered in
//! the font collection under a name private to the current layout, so one
//! document's fonts never leak into another laid out by the same engine,
//! and are removed again when layout ends.
//!
//! TrueType, OpenType and WOFF 1 fonts are supported. WOFF2 is not yet;
//! sources declared as WOFF2 are skipped in favor of later ones.

use std::collections::HashMap;

use fontique::{Blob, FamilyId, FontInfoOverride, FontStyle, FontWeight, FontWidth};
use parley::FontContext;
use presswork_dom::resource::{Fetcher, LoadError};
use presswork_style::stylesheet::{FontFace, FontSource};

/// The `@font-face` fonts of one document.
pub(crate) struct WebFonts<'s> {
    /// Faces by family name, lowercase.
    faces: HashMap<String, Vec<&'s FontFace>>,
    /// Families asked for so far, lowercase, with the name their fonts are
    /// registered under, or `None` if none loaded.
    loaded: HashMap<String, Option<String>>,
    /// Makes registered family names unique to this layout.
    prefix: String,
    /// What to unregister when layout ends.
    registered: Vec<(FamilyId, FontWidth, FontStyle, FontWeight)>,
}

/// `format()` hints of sources that can be read.
const SUPPORTED_FORMATS: [&str; 6] = [
    "truetype",
    "opentype",
    "woff",
    "collection",
    "truetype-variations",
    "opentype-variations",
];

impl<'s> WebFonts<'s> {
    /// The faces of a document; `id` distinguishes this layout from others
    /// sharing the font collection.
    pub fn new(faces: &'s [FontFace], id: u64) -> Self {
        let mut by_family: HashMap<String, Vec<&FontFace>> = HashMap::new();
        for face in faces {
            by_family
                .entry(face.family.to_lowercase())
                .or_default()
                .push(face);
        }
        Self {
            faces: by_family,
            loaded: HashMap::new(),
            prefix: format!("presswork {id} "),
            registered: Vec::new(),
        }
    }

    /// The name to ask the font collection for to get `family`: its
    /// registered name if it is an `@font-face` family that loaded, or
    /// `family` itself.
    pub fn name<'a>(&'a self, family: &'a str) -> &'a str {
        match self.loaded.get(&family.to_lowercase()) {
            Some(Some(registered)) => registered,
            _ => family,
        }
    }

    /// Loads the faces of `family` if it has any and this is the first time
    /// it is asked for.
    pub fn load(&mut self, family: &str, fonts: &mut FontContext, fetcher: &Fetcher<'_>) {
        if self.faces.is_empty() {
            return;
        }
        let key = family.to_lowercase();
        if self.loaded.contains_key(&key) {
            return;
        }
        let Some(faces) = self.faces.get(&key) else {
            self.loaded.insert(key, None);
            return;
        };
        let registered = format!("{}{family}", self.prefix);
        let mut any = false;
        for face in faces {
            let Some(bytes) = load_face(face, fetcher) else {
                continue;
            };
            let info = FontInfoOverride {
                family_name: Some(&registered),
                weight: face.weight.map(FontWeight::new),
                style: Some(if face.italic {
                    FontStyle::Italic
                } else {
                    FontStyle::Normal
                }),
                ..FontInfoOverride::default()
            };
            for (id, infos) in fonts
                .collection
                .register_fonts(Blob::from(bytes), Some(info))
            {
                any = true;
                self.registered.extend(
                    infos
                        .iter()
                        .map(|font| (id, font.width(), font.style(), font.weight())),
                );
            }
        }
        self.loaded.insert(key, any.then_some(registered));
    }

    /// Removes the fonts this layout registered.
    pub fn unregister(self, fonts: &mut FontContext) {
        for (family, width, style, weight) in self.registered {
            fonts
                .collection
                .unregister_font(family, width, style, weight);
        }
    }
}

/// Loads the first usable source of `face`, as an sfnt (TrueType or
/// OpenType) font file. Records a failure if none loads.
fn load_face(face: &FontFace, fetcher: &Fetcher<'_>) -> Option<Vec<u8>> {
    let mut first_error: Option<(&str, LoadError)> = None;
    let mut unsupported: Option<&str> = None;
    for source in &face.sources {
        // `local()` would name an installed font; system fonts are
        // reachable by family name anyway.
        let FontSource::Url { url, format } = source else {
            continue;
        };
        if let Some(format) = format
            && !SUPPORTED_FORMATS.contains(&format.as_str())
        {
            unsupported.get_or_insert(url);
            continue;
        }
        let result = fetcher
            .try_fetch(url, face.base.as_ref())
            .and_then(|bytes| sfnt(bytes).map_err(LoadError::Failed));
        match result {
            Ok(bytes) => return Some(bytes),
            Err(error) => {
                first_error.get_or_insert((url, error));
            }
        }
    }
    match (first_error, unsupported) {
        (Some((url, error)), _) => fetcher.record(url, error),
        (None, Some(url)) => fetcher.record(
            url,
            LoadError::Failed(format!(
                "font format not supported yet for `{}`; add a TrueType, OpenType or WOFF \
                 source",
                face.family
            )),
        ),
        (None, None) => {}
    }
    None
}

/// A font file as TrueType or OpenType data: as is, or decoded from WOFF.
fn sfnt(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    match bytes.get(..4) {
        Some([0, 1, 0, 0] | b"OTTO" | b"true" | b"ttcf") => Ok(bytes),
        Some(b"wOFF") => decode_woff(&bytes),
        Some(b"wOF2") => Err("WOFF2 fonts are not supported yet; use TTF, OTF or WOFF".into()),
        _ => Err("not a font file".into()),
    }
}

/// Fonts larger than this are refused, so a malicious file cannot exhaust
/// memory when decompressed.
const MAX_FONT_SIZE: usize = 64 << 20;

/// Decodes a WOFF 1 file into the sfnt it wraps.
///
/// Each table is stored zlib-compressed (or as is, when that is not
/// smaller); the sfnt is the offset table, the table directory, and the
/// tables, each padded to four bytes.
fn decode_woff(woff: &[u8]) -> Result<Vec<u8>, String> {
    let bad = || "malformed WOFF font".to_owned();
    let u16_at = |at: usize| -> Result<u16, String> {
        let bytes = woff.get(at..at + 2).ok_or_else(bad)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    };
    let u32_at = |at: usize| -> Result<u32, String> {
        let bytes = woff.get(at..at + 4).ok_or_else(bad)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };

    let flavor = u32_at(4)?;
    let count = usize::from(u16_at(12)?);
    if count == 0 {
        return Err(bad());
    }
    let mut tables = Vec::with_capacity(count);
    let mut total = 0;
    for index in 0..count {
        let entry = 44 + index * 20;
        let tag = u32_at(entry)?;
        let offset = u32_at(entry + 4)? as usize;
        let compressed = u32_at(entry + 8)? as usize;
        let length = u32_at(entry + 12)? as usize;
        let checksum = u32_at(entry + 16)?;
        total += length;
        if total > MAX_FONT_SIZE {
            return Err("WOFF font is too large".into());
        }
        let stored = woff
            .get(offset..offset.checked_add(compressed).ok_or_else(bad)?)
            .ok_or_else(bad)?;
        let data = if compressed < length {
            miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(stored, length)
                .map_err(|_| bad())?
        } else if compressed == length {
            stored.to_vec()
        } else {
            return Err(bad());
        };
        if data.len() != length {
            return Err(bad());
        }
        tables.push((tag, checksum, data));
    }

    // The offset table's binary search hints.
    let power = 1_usize << (usize::BITS - 1 - count.leading_zeros());
    let search_range = (power * 16) as u16;
    let entry_selector = power.trailing_zeros() as u16;
    let range_shift = (count * 16) as u16 - search_range;

    let header = 12 + 16 * count;
    let mut out = Vec::with_capacity(header + total + 3 * count);
    out.extend(flavor.to_be_bytes());
    for value in [count as u16, search_range, entry_selector, range_shift] {
        out.extend(value.to_be_bytes());
    }
    let mut offset = header;
    for (tag, checksum, data) in &tables {
        out.extend(tag.to_be_bytes());
        out.extend(checksum.to_be_bytes());
        out.extend((offset as u32).to_be_bytes());
        out.extend((data.len() as u32).to_be_bytes());
        offset += data.len().next_multiple_of(4);
    }
    for (_, _, data) in &tables {
        out.extend(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Encodes an sfnt as WOFF 1, compressing every table.
    pub(crate) fn encode_woff(sfnt: &[u8]) -> Vec<u8> {
        let u16_at = |at: usize| u16::from_be_bytes([sfnt[at], sfnt[at + 1]]);
        let u32_at =
            |at: usize| u32::from_be_bytes([sfnt[at], sfnt[at + 1], sfnt[at + 2], sfnt[at + 3]]);
        let count = usize::from(u16_at(4));
        let mut directory = Vec::new();
        let mut data = Vec::new();
        let start = 44 + 20 * count;
        for index in 0..count {
            let record = 12 + 16 * index;
            let (tag, checksum) = (u32_at(record), u32_at(record + 4));
            let (offset, length) = (u32_at(record + 8) as usize, u32_at(record + 12) as usize);
            let table = &sfnt[offset..offset + length];
            let compressed = miniz_oxide::deflate::compress_to_vec_zlib(table, 6);
            let stored = if compressed.len() < length {
                compressed
            } else {
                table.to_vec()
            };
            for value in [
                tag,
                (start + data.len()) as u32,
                stored.len() as u32,
                length as u32,
                checksum,
            ] {
                directory.extend(value.to_be_bytes());
            }
            data.extend(&stored);
            data.resize(data.len().next_multiple_of(4), 0);
        }
        let mut woff = b"wOFF".to_vec();
        woff.extend(u32_at(0).to_be_bytes());
        woff.extend(((start + data.len()) as u32).to_be_bytes());
        woff.extend((count as u16).to_be_bytes());
        woff.extend([0; 2]);
        woff.extend((sfnt.len() as u32).to_be_bytes());
        woff.extend([0; 24]);
        woff.extend(directory);
        woff.extend(data);
        woff
    }

    #[test]
    fn recognizes_font_files() {
        assert!(sfnt(b"OTTO....".to_vec()).is_ok());
        assert!(sfnt(vec![0, 1, 0, 0, 9]).is_ok());
        assert!(sfnt(b"wOF2....".to_vec()).unwrap_err().contains("WOFF2"));
        assert!(sfnt(b"<html>".to_vec()).is_err());
        assert!(sfnt(Vec::new()).is_err());
    }

    #[test]
    fn rejects_malformed_woff() {
        let mut woff = b"wOFF".to_vec();
        woff.extend([0, 1, 0, 0, 0, 0, 0, 0, 0, 1]);
        assert!(decode_woff(&woff).is_err(), "truncated directory");
        woff.resize(44, 0);
        // One table claiming data far past the end of the file.
        woff.extend([
            b'h', b'e', b'a', b'd', 0, 0, 1, 0, 0, 0, 0, 9, 0, 0, 0, 9, 0, 0, 0, 0,
        ]);
        assert!(decode_woff(&woff).is_err(), "table out of bounds");
    }
}
