use chrono::NaiveDate;
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub struct FileStats {
    pub extension: String,
    pub code_lines: u64,
}

#[derive(Debug, Clone)]
pub struct DailySnapshot {
    pub date: NaiveDate,
    pub files: Vec<FileStats>,
}

#[derive(Debug, Clone)]
pub struct RepoData {
    pub snapshots: Vec<DailySnapshot>,
    pub all_extensions: BTreeSet<String>,
}
