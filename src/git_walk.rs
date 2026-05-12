use crate::loc;
use crate::types::{DailySnapshot, FileStats, RepoData};
use chrono::{DateTime, Local, NaiveDate, TimeZone};
use git2::{ObjectType, Oid, Repository, Sort, TreeWalkMode, TreeWalkResult};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug)]
pub enum WalkMessage {
    Progress { processed: usize, total: usize },
    Done(Box<RepoData>),
    Failed(String),
}

#[derive(Clone, Copy)]
enum BlobOutcome {
    Lines(u64),
    Binary,
}

const MAX_WORKERS: usize = 4;

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

    let total = chosen.len();
    if total == 0 {
        return Ok(RepoData {
            snapshots: Vec::new(),
            all_extensions: BTreeSet::new(),
            commits_per_day,
        });
    }
    // Workers open their own Repository handles; drop this one so we don't
    // hold extra resources during the parallel phase.
    drop(repo);

    let memo: Mutex<HashMap<(Oid, String), BlobOutcome>> = Mutex::new(HashMap::new());
    let next_idx = AtomicUsize::new(0);
    let processed = AtomicUsize::new(0);
    let results: Mutex<Vec<DailySnapshot>> = Mutex::new(Vec::with_capacity(total));
    let error: Mutex<Option<String>> = Mutex::new(None);

    let n_workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .min(MAX_WORKERS)
        .min(total)
        .max(1);

    let path_ref = &path;
    let chosen_ref = &chosen;
    let memo_ref = &memo;
    let next_ref = &next_idx;
    let proc_ref = &processed;
    let results_ref = &results;
    let error_ref = &error;

    std::thread::scope(|s| {
        for _ in 0..n_workers {
            s.spawn(move || {
                let repo = match Repository::open(path_ref) {
                    Ok(r) => r,
                    Err(e) => {
                        let mut slot = error_ref.lock().unwrap();
                        if slot.is_none() {
                            *slot = Some(e.to_string());
                        }
                        return;
                    }
                };
                loop {
                    if error_ref.lock().unwrap().is_some() {
                        return;
                    }
                    let i = next_ref.fetch_add(1, Ordering::Relaxed);
                    if i >= total {
                        return;
                    }
                    let (date, oid) = chosen_ref[i];
                    match snapshot_for_commit(&repo, date, oid, memo_ref) {
                        Ok(snap) => {
                            results_ref.lock().unwrap().push(snap);
                            let done = proc_ref.fetch_add(1, Ordering::Relaxed) + 1;
                            let _ = tx.send(WalkMessage::Progress {
                                processed: done,
                                total,
                            });
                        }
                        Err(e) => {
                            let mut slot = error_ref.lock().unwrap();
                            if slot.is_none() {
                                *slot = Some(e);
                            }
                            return;
                        }
                    }
                }
            });
        }
    });

    if let Some(e) = error.into_inner().unwrap() {
        return Err(e);
    }

    let mut snapshots = results.into_inner().unwrap();
    snapshots.sort_by_key(|s| s.date);

    let mut all_extensions: BTreeSet<String> = BTreeSet::new();
    for snap in &snapshots {
        for fs in &snap.files {
            all_extensions.insert(fs.extension.clone());
        }
    }

    Ok(RepoData {
        snapshots,
        all_extensions,
        commits_per_day,
    })
}

fn snapshot_for_commit(
    repo: &Repository,
    date: NaiveDate,
    oid: Oid,
    memo: &Mutex<HashMap<(Oid, String), BlobOutcome>>,
) -> Result<DailySnapshot, String> {
    let commit = repo.find_commit(oid).map_err(|e| e.to_string())?;
    let tree = commit.tree().map_err(|e| e.to_string())?;

    let mut totals: HashMap<String, u64> = HashMap::new();
    let mut walk_err: Option<String> = None;

    tree.walk(TreeWalkMode::PreOrder, |dir, entry| {
        if entry.kind() != Some(ObjectType::Blob) {
            return TreeWalkResult::Ok;
        }
        let name = match entry.name() {
            Some(n) => n,
            None => return TreeWalkResult::Ok,
        };
        let full = format!("{dir}{name}");
        let path = Path::new(&full);
        let ext = loc::extension_of(path);
        let blob_oid = entry.id();
        let key = (blob_oid, ext.clone());

        if let Some(&cached) = memo.lock().unwrap().get(&key) {
            if let BlobOutcome::Lines(n) = cached {
                if n > 0 {
                    *totals.entry(ext).or_insert(0) += n;
                }
            }
            return TreeWalkResult::Ok;
        }

        let blob = match repo.find_blob(blob_oid) {
            Ok(b) => b,
            Err(e) => {
                walk_err = Some(e.to_string());
                return TreeWalkResult::Abort;
            }
        };
        let outcome = if blob.is_binary() {
            BlobOutcome::Binary
        } else {
            BlobOutcome::Lines(loc::count_code_lines(path, blob.content()))
        };
        memo.lock().unwrap().insert(key, outcome);
        if let BlobOutcome::Lines(n) = outcome {
            if n > 0 {
                *totals.entry(ext).or_insert(0) += n;
            }
        }
        TreeWalkResult::Ok
    })
    .map_err(|e| e.to_string())?;

    if let Some(e) = walk_err {
        return Err(e);
    }

    let files = totals
        .into_iter()
        .map(|(extension, code_lines)| FileStats {
            extension,
            code_lines,
        })
        .collect();

    Ok(DailySnapshot { date, files })
}

fn local_date(unix_secs: i64) -> NaiveDate {
    let dt: DateTime<Local> = Local
        .timestamp_opt(unix_secs, 0)
        .single()
        .unwrap_or_else(|| Local.timestamp_opt(0, 0).unwrap());
    dt.date_naive()
}
