//! Loading the resources a document refers to: images, stylesheets and
//! fonts.
//!
//! The engine never reads files or the network by itself. A [`Fetcher`]
//! resolves each URL, decodes `data:` URLs (which carry their content
//! inline and are always safe), and asks a [`Loader`] for everything else;
//! the loader decides what may be read. The fetcher also remembers what
//! failed, so it can be reported once the document is done.

use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt;

use data_url::DataUrl;
pub use url::Url;

/// Reads resources on behalf of the engine, deciding what is allowed.
pub trait Loader {
    /// The contents of the resource at `url`, an absolute URL other than
    /// `data:`.
    fn load(&self, url: &Url) -> Result<Vec<u8>, LoadError>;
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

/// A resource that could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadFailure {
    /// The URL as written in the document, shortened if very long.
    pub url: String,
    pub error: LoadError,
}

/// A loader that refuses everything, so only `data:` URLs load.
#[derive(Debug, Clone, Copy, Default)]
pub struct DataUrls;

impl Loader for DataUrls {
    fn load(&self, _url: &Url) -> Result<Vec<u8>, LoadError> {
        Err(LoadError::Blocked("only data: URLs are allowed".to_owned()))
    }
}

/// Resolves and loads the URLs of one document, and collects failures.
pub struct Fetcher<'a> {
    loader: &'a dyn Loader,
    /// What relative URLs in the document resolve against.
    base: Option<Url>,
    failures: RefCell<Vec<LoadFailure>>,
}

impl fmt::Debug for Fetcher<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fetcher")
            .field("base", &self.base)
            .field("failures", &self.failures.borrow().len())
            .finish_non_exhaustive()
    }
}

impl<'a> Fetcher<'a> {
    /// A fetcher for a document at `base` (`None` if it has no location,
    /// in which case relative URLs fail to load).
    pub fn new(loader: &'a dyn Loader, base: Option<Url>) -> Self {
        Self {
            loader,
            base,
            failures: RefCell::new(Vec::new()),
        }
    }

    /// The document's location, if it has one.
    pub fn base(&self) -> Option<&Url> {
        self.base.as_ref()
    }

    /// A fetcher that only loads `data:` URLs.
    pub fn data_only() -> Fetcher<'static> {
        Fetcher::new(&DataUrls, None)
    }

    /// Resolves `url` against `base` (the stylesheet it appears in), or
    /// against the document if `base` is `None`.
    pub fn resolve(&self, url: &str, base: Option<&Url>) -> Result<Url, LoadError> {
        let url = url.trim();
        match base.or(self.base.as_ref()) {
            Some(base) => base.join(url),
            None => Url::parse(url),
        }
        .map_err(|error| match error {
            url::ParseError::RelativeUrlWithoutBase => {
                LoadError::Failed("relative URL, but the document has no location".to_owned())
            }
            error => LoadError::Failed(format!("invalid URL: {error}")),
        })
    }

    /// Loads `url`, relative to `base` as in [`Fetcher::resolve`], without
    /// recording a failure. Use it to try alternatives.
    pub fn try_fetch(&self, url: &str, base: Option<&Url>) -> Result<Vec<u8>, LoadError> {
        if let Some(data) = decode_data_url(url) {
            return data;
        }
        let url = self.resolve(url, base)?;
        if url.scheme() == "data" {
            // A relative reference cannot become a data: URL, but a base
            // could be one.
            return decode_data_url(url.as_str()).expect("a data: URL");
        }
        self.loader.load(&url)
    }

    /// Loads `url`, recording a failure if it does not load.
    pub fn fetch(&self, url: &str, base: Option<&Url>) -> Option<Vec<u8>> {
        self.try_fetch(url, base)
            .map_err(|error| self.record(url, error))
            .ok()
    }

    /// Records that `url` could not be loaded or used. Each URL is
    /// recorded once.
    pub fn record(&self, url: &str, error: LoadError) {
        let url = display_url(url).into_owned();
        let mut failures = self.failures.borrow_mut();
        if !failures.iter().any(|failure| failure.url == url) {
            failures.push(LoadFailure { url, error });
        }
    }

    /// The failures so far, in the order they happened.
    pub fn failures(&self) -> Vec<LoadFailure> {
        self.failures.borrow().clone()
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

    /// Serves `text` for every URL, echoing the URL it was asked for.
    struct Echo;

    impl Loader for Echo {
        fn load(&self, url: &Url) -> Result<Vec<u8>, LoadError> {
            Ok(url.as_str().as_bytes().to_vec())
        }
    }

    #[test]
    fn decodes_base64_and_percent_encoded_data_urls() {
        let fetcher = Fetcher::data_only();
        // Attribute values often wrap long base64 over several lines.
        let url = "data:image/png;base64,iVBO\n  Rw0K";
        assert_eq!(fetcher.fetch(url, None).unwrap(), b"\x89PNG\r\n".to_vec());
        assert_eq!(fetcher.fetch("data:,a%20b", None).unwrap(), b"a b");
        assert!(fetcher.failures().is_empty());
    }

    #[test]
    fn refuses_everything_else_and_records_it_once() {
        let fetcher = Fetcher::data_only();
        assert!(fetcher.fetch("file:///etc/passwd", None).is_none());
        assert!(fetcher.fetch("logo.png", None).is_none());
        assert!(fetcher.fetch(" logo.png", None).is_none());
        assert!(fetcher.fetch("data:image/png;base64,%%%", None).is_none());
        let failures = fetcher.failures();
        let urls: Vec<&str> = failures.iter().map(|f| f.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "file:///etc/passwd",
                "logo.png",
                "data:image/png;base64,%%%"
            ]
        );
        assert!(matches!(failures[0].error, LoadError::Blocked(_)));
        assert!(matches!(failures[1].error, LoadError::Failed(_)));
    }

    #[test]
    fn resolves_against_the_stylesheet_then_the_document() {
        let document = Url::parse("file:///site/invoice.html").unwrap();
        let fetcher = Fetcher::new(&Echo, Some(document));
        let fetch = |url, base: Option<&Url>| {
            String::from_utf8(fetcher.try_fetch(url, base).unwrap()).unwrap()
        };
        assert_eq!(fetch("logo.png", None), "file:///site/logo.png");
        let sheet = Url::parse("file:///site/css/main.css").unwrap();
        assert_eq!(
            fetch("../fonts/a.ttf", Some(&sheet)),
            "file:///site/fonts/a.ttf"
        );
        // try_fetch records nothing.
        assert!(fetcher.failures().is_empty());
    }

    #[test]
    fn shortens_long_urls_for_messages() {
        let long = format!("data:image/png;base64,{}", "A".repeat(1000));
        let shown = display_url(&long);
        assert!(shown.chars().count() == 61 && shown.ends_with('…'));
        assert_eq!(display_url(" logo.png "), "logo.png");
    }
}
