use std::path::Path;
use tokei::{Config, LanguageType};

/// Resolve a tokei `LanguageType` for a path (filename-based, mostly).
/// Returned `None` means tokei doesn't classify it; fall back to newline count.
pub fn language_for(path: &Path) -> Option<LanguageType> {
    let config = Config::default();
    LanguageType::from_path(path, &config)
}

/// Count "code" lines, using a pre-resolved language when available.
pub fn count_code_lines_with_lang(lang: Option<LanguageType>, bytes: &[u8]) -> u64 {
    if let Some(lang) = lang {
        let config = Config::default();
        return lang.parse_from_slice(bytes, &config).code as u64;
    }
    bytes.iter().filter(|&&b| b == b'\n').count() as u64
}

pub fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_else(|| "(no ext)".to_string())
}
