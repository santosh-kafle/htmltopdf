//! Loading the resources a document refers to, such as images.
//!
//! The engine never reads files or the network by itself: it asks a
//! [`Loader`], which decides what may be loaded. `data:` URLs carry their
//! content inline and are always safe to load; [`DataUrls`] loads those and
//! nothing else.

use std::borrow::Cow;
use std::fmt;

use data_url::DataUrl;

/// Fetches resources by URL on behalf of the engine.
pub trait Loader {
    /// The contents of the resource at `url`, exactly as written in the
    /// document (a `src` attribute, say). Resolving relative URLs is up to
    /// the loader.
    fn load(&self, url: &str) -> Result<Vec<u8>, LoadError>;
}

/// Why a resource was not loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// Loading it would need network access, which presswork does not do.
    Network,
    /// The loader's policy does not allow it; the message says why.
    Blocked(String),
    /// Reading the resource failed, or its contents could not be used.
    Failed(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Network => f.write_str("network resources are not loaded"),
            LoadError::Blocked(why) | LoadError::Failed(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for LoadError {}

/// Loads `data:` URLs only; refuses everything else.
#[derive(Debug, Clone, Copy, Default)]
pub struct DataUrls;

impl Loader for DataUrls {
    fn load(&self, url: &str) -> Result<Vec<u8>, LoadError> {
        decode_data_url(url)
            .unwrap_or_else(|| Err(LoadError::Blocked("only data: URLs are allowed".to_owned())))
    }
}

/// Decodes a `data:` URL, or returns `None` if `url` is not one.
pub fn decode_data_url(url: &str) -> Option<Result<Vec<u8>, LoadError>> {
    let parsed = match DataUrl::process(url.trim()) {
        Ok(parsed) => parsed,
        Err(data_url::DataUrlError::NotADataUrl) => return None,
        Err(_) => return Some(Err(LoadError::Failed("malformed data: URL".to_owned()))),
    };
    Some(
        parsed
            .decode_to_vec()
            .map(|(bytes, _fragment)| bytes)
            .map_err(|_| LoadError::Failed("invalid base64 in data: URL".to_owned())),
    )
}

/// `url` shortened for messages: `data:` URLs can be megabytes long.
pub fn display_url(url: &str) -> Cow<'_, str> {
    const MAX: usize = 60;
    let url = url.trim();
    match url.char_indices().nth(MAX) {
        Some((end, _)) => Cow::Owned(format!("{}…", &url[..end])),
        None => Cow::Borrowed(url),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_base64_and_percent_encoded_data_urls() {
        // Attribute values often wrap long base64 over several lines.
        let url = "data:image/png;base64,iVBO\n  Rw0K";
        assert_eq!(DataUrls.load(url).unwrap(), b"\x89PNG\r\n".to_vec());
        assert_eq!(DataUrls.load("data:,a%20b").unwrap(), b"a b");
    }

    #[test]
    fn refuses_everything_else() {
        assert!(matches!(
            DataUrls.load("file:///etc/passwd"),
            Err(LoadError::Blocked(_))
        ));
        assert!(matches!(
            DataUrls.load("logo.png"),
            Err(LoadError::Blocked(_))
        ));
        assert!(matches!(
            DataUrls.load("data:image/png;base64,%%%"),
            Err(LoadError::Failed(_))
        ));
    }

    #[test]
    fn shortens_long_urls_for_messages() {
        let long = format!("data:image/png;base64,{}", "A".repeat(1000));
        let shown = display_url(&long);
        assert!(shown.chars().count() == 61 && shown.ends_with('…'));
        assert_eq!(display_url(" logo.png "), "logo.png");
    }
}
