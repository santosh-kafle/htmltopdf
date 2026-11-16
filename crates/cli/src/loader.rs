//! Loading the images a document refers to, under wkhtmltopdf 0.12.6's
//! rules: local files only with `--enable-local-file-access` or inside an
//! `--allow`ed path. presswork never uses the network.

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Args, ValueEnum};
use presswork_dom::resource::{LoadError, Loader, decode_data_url};
use presswork_layout::LoadFailure;
use url::Url;

/// The resource options of the command line.
#[derive(Debug, Args)]
pub(crate) struct ResourceArgs {
    /// Allow the document to load any local file
    #[arg(long, overrides_with = "disable_local_file_access")]
    pub enable_local_file_access: bool,

    /// Only allow local files under --allow paths [default]
    #[arg(long, overrides_with = "enable_local_file_access")]
    pub disable_local_file_access: bool,

    /// Allow the document to load this file, or the files in this
    /// directory. Repeatable
    #[arg(long, value_name = "PATH")]
    pub allow: Vec<PathBuf>,

    /// Do not load or print images
    #[arg(long, overrides_with = "images")]
    pub no_images: bool,

    /// Load and print images [default]
    #[arg(long, overrides_with = "no_images")]
    pub images: bool,

    /// What to do when an image fails to load
    #[arg(long, value_name = "HANDLER", value_enum, default_value_t)]
    pub load_media_error_handling: ErrorHandling,
}

/// wkhtmltopdf's `--load-*-error-handling` values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub(crate) enum ErrorHandling {
    /// Stop without writing the PDF
    Abort,
    /// Warn and leave the image out
    #[default]
    Ignore,
    /// The same as ignore
    Skip,
}

impl ResourceArgs {
    /// The loader for a document whose relative URLs resolve against
    /// `base`.
    pub fn loader(&self, base: Url) -> FileLoader {
        FileLoader {
            base,
            images: !self.no_images,
            all_files: self.enable_local_file_access,
            // Canonical, so neither `..` nor a symlink can lead outside an
            // allowed path. A path that does not exist allows nothing.
            allowed: self
                .allow
                .iter()
                .filter_map(|path| path.canonicalize().ok())
                .collect(),
        }
    }

    /// Warnings for resources that did not load, or an error when the
    /// options say to stop. Network resources are always reported, even
    /// under `quiet`, because wkhtmltopdf would have loaded them; `strict`
    /// makes them an error.
    pub fn report(
        &self,
        failures: &[LoadFailure],
        quiet: bool,
        strict: bool,
    ) -> Result<Vec<String>> {
        if self.no_images {
            // Asked for: every image was refused on purpose.
            return Ok(Vec::new());
        }
        let abort = self.load_media_error_handling == ErrorHandling::Abort;
        let mut warnings = Vec::new();
        for LoadFailure { url, error } in failures {
            match error {
                LoadError::Network => {
                    if abort || strict {
                        bail!("{url} is a network resource, which presswork does not load");
                    }
                    warnings.push(format!(
                        "warning: not loading {url}: presswork does not load network \
                         resources; use a local file or a data: URL instead"
                    ));
                }
                _ if abort => bail!("failed to load {url}: {error}"),
                _ if !quiet => warnings.push(format!("warning: failed to load {url}: {error}")),
                _ => {}
            }
        }
        Ok(warnings)
    }
}

/// Loads `data:` URLs, and local files where the options allow.
#[derive(Debug)]
pub(crate) struct FileLoader {
    /// What relative URLs resolve against: the input document, or the
    /// current directory for standard input.
    base: Url,
    /// Images may be loaded at all (no `--no-images`).
    images: bool,
    /// `--enable-local-file-access`.
    all_files: bool,
    /// `--allow` paths, canonicalized.
    allowed: Vec<PathBuf>,
}

impl FileLoader {
    fn read_file(&self, url: &Url) -> Result<Vec<u8>, LoadError> {
        // A protocol-relative `//host/...` against a file base, meant for
        // a page served over the network.
        if url
            .host_str()
            .is_some_and(|host| !host.is_empty() && host != "localhost")
        {
            return Err(LoadError::Network);
        }
        let path = url
            .to_file_path()
            .map_err(|()| LoadError::Failed("not a local file".to_owned()))?;
        if !self.all_files {
            let permitted = path
                .canonicalize()
                .is_ok_and(|real| self.allowed.iter().any(|allowed| real.starts_with(allowed)));
            if !permitted {
                return Err(LoadError::Blocked(
                    "local file access is disabled; pass --enable-local-file-access, \
                     or --allow with the file's directory"
                        .to_owned(),
                ));
            }
        }
        std::fs::read(&path).map_err(|e| LoadError::Failed(format!("{}: {e}", path.display())))
    }
}

impl Loader for FileLoader {
    fn load(&self, url: &str) -> Result<Vec<u8>, LoadError> {
        if !self.images {
            return Err(LoadError::Blocked("--no-images was given".to_owned()));
        }
        if let Some(data) = decode_data_url(url) {
            return data;
        }
        let url = self
            .base
            .join(url.trim())
            .map_err(|e| LoadError::Failed(format!("invalid URL: {e}")))?;
        match url.scheme() {
            "file" => self.read_file(&url),
            "http" | "https" | "ftp" => Err(LoadError::Network),
            scheme => Err(LoadError::Failed(format!(
                "unsupported URL scheme `{scheme}:`"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// A directory with `assets/logo.png` and `secret.txt`, removed on drop.
    struct Fixture(PathBuf);

    impl Fixture {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("presswork-loader-{name}-{}", std::process::id()));
            fs::create_dir_all(dir.join("assets")).unwrap();
            fs::write(dir.join("assets/logo.png"), b"logo").unwrap();
            fs::write(dir.join("secret.txt"), b"secret").unwrap();
            Self(dir)
        }

        fn base(&self) -> Url {
            Url::from_file_path(self.0.join("invoice.html")).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn args(all_files: bool, allow: &[&Path]) -> ResourceArgs {
        ResourceArgs {
            enable_local_file_access: all_files,
            disable_local_file_access: false,
            allow: allow.iter().map(|p| p.to_path_buf()).collect(),
            no_images: false,
            images: false,
            load_media_error_handling: ErrorHandling::Ignore,
        }
    }

    #[test]
    fn local_files_are_blocked_by_default() {
        let fixture = Fixture::new("default");
        let loader = args(false, &[]).loader(fixture.base());
        assert!(matches!(
            loader.load("assets/logo.png"),
            Err(LoadError::Blocked(_))
        ));
        // data: URLs always load.
        assert_eq!(loader.load("data:,hi").unwrap(), b"hi");
    }

    #[test]
    fn enable_local_file_access_allows_relative_and_absolute_paths() {
        let fixture = Fixture::new("enable");
        let loader = args(true, &[]).loader(fixture.base());
        assert_eq!(loader.load("assets/logo.png").unwrap(), b"logo");
        let absolute = fixture.0.join("assets/logo.png");
        assert_eq!(loader.load(absolute.to_str().unwrap()).unwrap(), b"logo");
        let url = Url::from_file_path(&absolute).unwrap();
        assert_eq!(loader.load(url.as_str()).unwrap(), b"logo");
        assert!(matches!(
            loader.load("missing.png"),
            Err(LoadError::Failed(_))
        ));
    }

    #[test]
    fn allow_admits_only_its_own_directory() {
        let fixture = Fixture::new("allow");
        let assets = fixture.0.join("assets");
        let loader = args(false, &[&assets]).loader(fixture.base());
        assert_eq!(loader.load("assets/logo.png").unwrap(), b"logo");
        assert!(matches!(
            loader.load("secret.txt"),
            Err(LoadError::Blocked(_))
        ));
        assert!(matches!(
            loader.load("assets/../secret.txt"),
            Err(LoadError::Blocked(_))
        ));
        // `url` normalizes `..` away, but a literal path can still try.
        let sneaky = format!("{}/../secret.txt", assets.display());
        assert!(matches!(loader.load(&sneaky), Err(LoadError::Blocked(_))));
    }

    #[test]
    fn network_urls_are_never_loaded() {
        let fixture = Fixture::new("network");
        let loader = args(true, &[]).loader(fixture.base());
        for url in ["https://example.com/logo.png", "//cdn.example.com/x.png"] {
            assert_eq!(loader.load(url), Err(LoadError::Network), "{url}");
        }
    }

    #[test]
    fn reports_network_failures_even_when_quiet() {
        let failures = [
            LoadFailure {
                url: "https://example.com/logo.png".into(),
                error: LoadError::Network,
            },
            LoadFailure {
                url: "logo.png".into(),
                error: LoadError::Blocked("disabled".into()),
            },
        ];
        let mut resources = args(false, &[]);
        assert_eq!(resources.report(&failures, false, false).unwrap().len(), 2);
        let quiet = resources.report(&failures, true, false).unwrap();
        assert_eq!(quiet.len(), 1);
        assert!(quiet[0].contains("https://example.com/logo.png"));
        assert!(resources.report(&failures, true, true).is_err(), "strict");

        resources.load_media_error_handling = ErrorHandling::Abort;
        let error = resources.report(&failures[1..], true, false).unwrap_err();
        assert!(error.to_string().contains("logo.png"), "{error}");

        resources.no_images = true;
        assert!(resources.report(&failures, false, true).unwrap().is_empty());
    }
}
