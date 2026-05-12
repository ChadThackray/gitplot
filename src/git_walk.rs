use crate::loc;
use crate::types::{DailySnapshot, FileStats, RepoData};
use chrono::{DateTime, Local, NaiveDate, TimeZone};
use git2::{ObjectType, Oid, Repository, Sort, TreeWalkMode, TreeWalkResult};
use gix::prelude::Find;
use gix::ObjectId;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use tokei::LanguageType;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug)]
pub enum WalkMessage {
    Progress { processed: usize, total: usize },
    Done(Box<RepoData>),
    Failed(String),
}

/// Upper bound on parallel parse workers. `available_parallelism()`
/// already caps this to the machine's core count, so the constant only
/// matters on very-large hosts. With rayon's pool sized to nproc, phase 2
/// scales almost linearly up to nproc on this workload.
const MAX_WORKERS: usize = 32;

/// Coalesce progress messages: don't send more than ~PROGRESS_STEPS updates total.
const PROGRESS_STEPS: usize = 200;

/// Extensions for which we won't even open the blob — assume binary, count 0 lines.
/// Curated so it never overlaps anything tokei recognises as code/markup.
/// IMPORTANT: do NOT add svg/json/xml/yaml/ipynb/csv/toml/md — those are text
/// and tokei does classify them.
const BINARY_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "ico", "bmp", "tiff", "tif", "webp", "psd",
    "zip", "gz", "tgz", "bz2", "xz", "7z", "rar", "tar",
    "so", "dll", "dylib", "exe", "a", "lib", "o", "obj",
    "pyc", "pyd", "pyo", "class", "jar", "war", "wasm",
    "ttf", "otf", "woff", "woff2", "eot",
    "mp4", "mov", "webm", "mkv", "avi", "flv",
    "mp3", "wav", "flac", "ogg", "m4a", "aac",
    "pdf", "sqlite", "sqlite3", "bin", "dat",
];

fn is_binary_ext(ext: &str) -> bool {
    BINARY_EXTS.contains(&ext)
}

/// Treat a blob as binary if a NUL byte appears in the first 8 KB.
/// Matches libgit2's heuristic closely enough that LOC totals don't shift.
fn is_binary_bytes(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(8000)];
    head.contains(&0u8)
}

/// Small interner for extension strings — most trees see <50 distinct extensions
/// but emit hundreds of thousands of references to them.
#[derive(Default)]
struct ExtInterner {
    by_id: Vec<String>,
    by_str: HashMap<String, u32>,
}

impl ExtInterner {
    fn intern(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.by_str.get(s) {
            return id;
        }
        let id = self.by_id.len() as u32;
        self.by_id.push(s.to_string());
        self.by_str.insert(s.to_string(), id);
        id
    }

    fn lookup(&self, id: u32) -> &str {
        &self.by_id[id as usize]
    }
}

pub fn analyze(path: PathBuf, tx: UnboundedSender<WalkMessage>) {
    match analyze_inner(path, &tx) {
        Ok(data) => {
            let _ = tx.send(WalkMessage::Done(Box::new(data)));
        }
        Err(e) => {
            let _ = tx.send(WalkMessage::Failed(e));
        }
    }
}

fn analyze_inner(path: PathBuf, tx: &UnboundedSender<WalkMessage>) -> Result<RepoData, String> {
    let repo = Repository::open(&path).map_err(|e| format!("not a git repo: {e}"))?;

    let head_oid = {
        let head = repo.head().map_err(|e| format!("no HEAD: {e}"))?;
        head.target()
            .ok_or_else(|| "HEAD is not a direct reference".to_string())?
    };

    // Pick the last commit per local-calendar day from the full revwalk.
    let mut last_per_day: BTreeMap<NaiveDate, (i64, Oid)> = BTreeMap::new();
    let mut commits_count: BTreeMap<NaiveDate, u32> = BTreeMap::new();
    {
        let mut walk = repo.revwalk().map_err(|e| e.to_string())?;
        walk.set_sorting(Sort::TIME).map_err(|e| e.to_string())?;
        walk.push(head_oid).map_err(|e| e.to_string())?;
        for oid_res in walk {
            let oid = oid_res.map_err(|e| e.to_string())?;
            let commit = repo.find_commit(oid).map_err(|e| e.to_string())?;
            let secs = commit.time().seconds();
            let date = local_date(secs);
            *commits_count.entry(date).or_insert(0) += 1;
            last_per_day
                .entry(date)
                .and_modify(|cur| {
                    if secs > cur.0 {
                        *cur = (secs, oid);
                    }
                })
                .or_insert((secs, oid));
        }
    }

    let commits_per_day: Vec<(NaiveDate, u32)> = commits_count.into_iter().collect();

    let chosen: Vec<(NaiveDate, Oid)> = last_per_day
        .into_iter()
        .map(|(d, (_t, oid))| (d, oid))
        .collect();

    if chosen.is_empty() {
        return Ok(RepoData {
            snapshots: Vec::new(),
            all_extensions: BTreeSet::new(),
            commits_per_day,
        });
    }

    // ---- Phase 1: walk each chosen tree once; collect blob references ----
    // Extensions are interned to u32 ids so per-day lists store
    // (ObjectId, u32) — no allocation per push, no String clone per file.
    // ObjectIds use gix's hash type so phase 2 can hand them straight to
    // the gix object database without conversion.
    let mut ext_tab = ExtInterner::default();
    let mut per_day: Vec<Vec<(ObjectId, u32)>> = Vec::with_capacity(chosen.len());
    let mut unique: HashMap<(ObjectId, u32), Option<LanguageType>> = HashMap::new();
    let mut lang_by_name: HashMap<String, Option<LanguageType>> = HashMap::new();

    for (_date, oid) in &chosen {
        let mut day_blobs: Vec<(ObjectId, u32)> = Vec::new();
        let commit = repo.find_commit(*oid).map_err(|e| e.to_string())?;
        let tree = commit.tree().map_err(|e| e.to_string())?;
        tree.walk(TreeWalkMode::PreOrder, |_dir, entry| {
            if entry.kind() != Some(ObjectType::Blob) {
                return TreeWalkResult::Ok;
            }
            let name = match entry.name() {
                Some(n) => n,
                None => return TreeWalkResult::Ok,
            };
            let ext = loc::extension_of(Path::new(name));
            if is_binary_ext(&ext) {
                return TreeWalkResult::Ok;
            }
            let ext_id = ext_tab.intern(&ext);
            let blob_oid = ObjectId::try_from(entry.id().as_bytes())
                .expect("git2 oid must be valid sha");
            day_blobs.push((blob_oid, ext_id));
            unique.entry((blob_oid, ext_id)).or_insert_with(|| {
                if let Some(&lang) = lang_by_name.get(name) {
                    lang
                } else {
                    let lang = loc::language_for(Path::new(name));
                    lang_by_name.insert(name.to_string(), lang);
                    lang
                }
            });
            TreeWalkResult::Ok
        })
        .map_err(|e| e.to_string())?;
        per_day.push(day_blobs);
    }
    drop(repo);

    let keys: Vec<((ObjectId, u32), Option<LanguageType>)> = unique.into_iter().collect();
    let total = keys.len();

    if total == 0 {
        let snapshots: Vec<DailySnapshot> = chosen
            .iter()
            .map(|(d, _)| DailySnapshot {
                date: *d,
                files: Vec::new(),
            })
            .collect();
        return Ok(RepoData {
            snapshots,
            all_extensions: BTreeSet::new(),
            commits_per_day,
        });
    }

    // ---- Phase 2: parse each unique blob exactly once, in parallel ----
    // Blob reads go through gitoxide (`gix`) instead of libgit2:
    //   * zlib-ng decompression is ~30 % faster than libgit2's bundled zlib
    //     on a single thread,
    //   * gix object handles share an mmap'd pack but read into a
    //     per-thread buffer, so contention drops at higher worker counts.
    let parsed: Mutex<HashMap<(ObjectId, u32), u64>> =
        Mutex::new(HashMap::with_capacity(total));
    let next_idx = AtomicUsize::new(0);
    let processed = AtomicUsize::new(0);
    let error: Mutex<Option<String>> = Mutex::new(None);

    let n_workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .min(MAX_WORKERS)
        .min(total)
        .max(1);

    // Throttle progress emissions: at most PROGRESS_STEPS updates over the run.
    let progress_chunk = total.div_ceil(PROGRESS_STEPS).max(1);

    let gix_repo = gix::open(&path).map_err(|e| e.to_string())?;
    let keys_ref = &keys;
    let parsed_ref = &parsed;
    let next_ref = &next_idx;
    let proc_ref = &processed;
    let error_ref = &error;

    std::thread::scope(|s| {
        for _ in 0..n_workers {
            let repo = gix_repo.clone();
            s.spawn(move || {
                let mut local: Vec<((ObjectId, u32), u64)> = Vec::with_capacity(64);
                let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
                loop {
                    if error_ref.lock().unwrap().is_some() {
                        return;
                    }
                    let i = next_ref.fetch_add(1, Ordering::Relaxed);
                    if i >= total {
                        break;
                    }
                    let ((oid, ext_id), lang) = &keys_ref[i];
                    buf.clear();
                    let lines = match repo.objects.try_find(oid, &mut buf) {
                        Ok(Some(obj)) => {
                            if is_binary_bytes(obj.data) {
                                0
                            } else {
                                loc::count_code_lines_with_lang(*lang, obj.data)
                            }
                        }
                        Ok(None) => 0,
                        Err(e) => {
                            let mut slot = error_ref.lock().unwrap();
                            if slot.is_none() {
                                *slot = Some(e.to_string());
                            }
                            return;
                        }
                    };
                    local.push(((*oid, *ext_id), lines));
                    if local.len() >= 64 {
                        let mut g = parsed_ref.lock().unwrap();
                        for (k, v) in local.drain(..) {
                            g.insert(k, v);
                        }
                    }
                    let done = proc_ref.fetch_add(1, Ordering::Relaxed) + 1;
                    if done == total || done % progress_chunk == 0 {
                        let _ = tx.send(WalkMessage::Progress {
                            processed: done,
                            total,
                        });
                    }
                }
                if !local.is_empty() {
                    let mut g = parsed_ref.lock().unwrap();
                    for (k, v) in local.drain(..) {
                        g.insert(k, v);
                    }
                }
            });
        }
    });

    if let Some(e) = error.into_inner().unwrap() {
        return Err(e);
    }

    let parsed = parsed.into_inner().unwrap();

    // ---- Phase 3: assemble per-day totals from parsed table ----
    // Roll up by ext_id (fixed u32) then materialise Strings once at the end.
    let n_exts = ext_tab.by_id.len();
    let mut snapshots: Vec<DailySnapshot> = Vec::with_capacity(chosen.len());
    let mut seen_ext: Vec<bool> = vec![false; n_exts];
    let mut totals_buf: Vec<u64> = vec![0u64; n_exts];

    for ((date, _oid), blobs) in chosen.iter().zip(per_day.iter()) {
        for slot in totals_buf.iter_mut() {
            *slot = 0;
        }
        for (oid, ext_id) in blobs {
            if let Some(&lines) = parsed.get(&(*oid, *ext_id)) {
                if lines > 0 {
                    totals_buf[*ext_id as usize] += lines;
                    seen_ext[*ext_id as usize] = true;
                }
            }
        }
        let mut files: Vec<FileStats> = Vec::new();
        for (i, &lines) in totals_buf.iter().enumerate() {
            if lines > 0 {
                files.push(FileStats {
                    extension: ext_tab.lookup(i as u32).to_string(),
                    code_lines: lines,
                });
            }
        }
        snapshots.push(DailySnapshot { date: *date, files });
    }

    let mut all_extensions: BTreeSet<String> = BTreeSet::new();
    for (i, &seen) in seen_ext.iter().enumerate() {
        if seen {
            all_extensions.insert(ext_tab.lookup(i as u32).to_string());
        }
    }

    Ok(RepoData {
        snapshots,
        all_extensions,
        commits_per_day,
    })
}

fn local_date(unix_secs: i64) -> NaiveDate {
    let dt: DateTime<Local> = Local
        .timestamp_opt(unix_secs, 0)
        .single()
        .unwrap_or_else(|| Local.timestamp_opt(0, 0).unwrap());
    dt.date_naive()
}
