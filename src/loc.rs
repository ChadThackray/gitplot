use std::path::Path;
use tokei::{Config, LanguageType};

pub fn count_code_lines(path: &Path, bytes: &[u8]) -> u64 {
    let config = Config::default();
    if let Some(lang) = LanguageType::from_path(path, &config) {
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
