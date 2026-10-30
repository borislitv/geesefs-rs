//! Go MIME policy adapter, not a new MIME syntax parser (that belongs to `mime`).
//! Default association data: public Go standard library, copyright The Go Authors,
//! BSD-3-Clause, https://go.dev/src/mime/type.go and type_unix.go.
use crate::error::{Error, Result};
use std::{collections::HashMap, io::Read};

#[derive(Debug)]
pub struct ContentTypes {
    exact: HashMap<String, String>,
    lower: HashMap<String, String>,
}
impl Default for ContentTypes {
    fn default() -> Self {
        let mut result = Self {
            exact: HashMap::new(),
            lower: HashMap::new(),
        };
        for (extension, kind) in [
            ("avif", "image/avif"),
            ("css", "text/css"),
            ("gif", "image/gif"),
            ("htm", "text/html"),
            ("html", "text/html"),
            ("jpeg", "image/jpeg"),
            ("jpg", "image/jpeg"),
            ("js", "text/javascript"),
            ("json", "application/json"),
            ("mjs", "text/javascript"),
            ("pdf", "application/pdf"),
            ("png", "image/png"),
            ("svg", "image/svg+xml"),
            ("wasm", "application/wasm"),
            ("webp", "image/webp"),
            ("xml", "text/xml"),
        ] {
            result.insert(extension, kind);
        }
        result
    }
}
impl ContentTypes {
    pub fn system() -> Result<Self> {
        let mut types = Self::default();
        for path in ["/usr/local/share/mime/globs2", "/usr/share/mime/globs2"] {
            if let Ok(file) = std::fs::File::open(path) {
                types.add_globs(file)?;
                return Ok(types);
            }
        }
        for path in [
            "/etc/mime.types",
            "/etc/apache2/mime.types",
            "/etc/apache/mime.types",
            "/etc/httpd/conf/mime.types",
        ] {
            if let Ok(file) = std::fs::File::open(path) {
                types.add_types(file)?;
            }
        }
        Ok(types)
    }
    fn insert(&mut self, extension: &str, kind: &str) {
        self.exact.insert(extension.into(), kind.into());
        self.lower.insert(extension.to_lowercase(), kind.into());
    }
    fn text(reader: impl Read) -> Result<String> {
        const MAX: u64 = 8 * 1024 * 1024;
        let mut text = String::new();
        reader
            .take(MAX + 1)
            .read_to_string(&mut text)
            .map_err(|_| Error::Config("MIME database read/UTF-8 failure".into()))?;
        if text.len() as u64 > MAX || text.lines().any(|line| line.len() > 65536) {
            return Err(Error::Config(
                "MIME database exceeds byte/line limit".into(),
            ));
        }
        Ok(text)
    }
    pub fn add_globs(&mut self, reader: impl Read) -> Result<()> {
        for line in Self::text(reader)?.lines() {
            let mut fields = line.split(':');
            let weight = fields.next().unwrap_or("");
            let kind = fields.next().unwrap_or("");
            let pattern = fields.next().unwrap_or("");
            if weight.is_empty() || weight.starts_with('#') {
                continue;
            }
            let Some(extension) = pattern
                .strip_prefix("*.")
                .filter(|s| !s.is_empty() && !s.contains(['?', '*', '[']))
            else {
                continue;
            };
            if self.exact.contains_key(extension) {
                continue;
            }
            if let Ok(kind) = kind.parse::<mime::Mime>() {
                self.insert(extension, kind.essence_str());
            }
        }
        Ok(())
    }
    pub fn add_types(&mut self, reader: impl Read) -> Result<()> {
        for line in Self::text(reader)?.lines() {
            let mut fields = line.split_whitespace();
            let Some(kind) = fields.next().filter(|s| !s.starts_with('#')) else {
                continue;
            };
            let Ok(kind) = kind.parse::<mime::Mime>() else {
                continue;
            };
            for extension in fields.take_while(|s| !s.starts_with('#')) {
                self.insert(extension, kind.essence_str());
            }
        }
        Ok(())
    }
    pub fn guess(&self, path: &str) -> Option<&str> {
        let extension = path.rsplit_once('.')?.1;
        self.exact
            .get(extension)
            .or_else(|| self.lower.get(&extension.to_lowercase()))
            .map(String::as_str)
    }
}
