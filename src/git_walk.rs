use crate::loc;
use crate::types::{DailySnapshot, FileStats, RepoData};
use chrono::{DateTime, Local, NaiveDate, TimeZone};
use git2::{ObjectType, Oid, Repository, Sort, TreeWalkMode, TreeWalkResult};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug)]
pub enum WalkMessage {
    Progress { processed: usize, total: usize },
    Done(Box<RepoData>),
    Failed(String),
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

    let head = repo.head().map_err(|e| format!("no HEAD: {e}"))?;
    let head_oid = head
        .target()
        .ok_or_else(|| "HEAD is not a direct reference".to_string())?;

    let mut walk = repo.revwalk().map_err(|e| e.to_string())?;
    walk.set_sorting(Sort::TIME).map_err(|e| e.to_string())?;
    walk.push(head_oid).map_err(|e| e.to_string())?;

    let mut last_per_day: BTreeMap<NaiveDate, (i64, Oid)> = BTreeMap::new();
    for oid_res in walk {
        let oid = oid_res.map_err(|e| e.to_string())?;
        let commit = repo.find_commit(oid).map_err(|e| e.to_string())?;
        let secs = commit.time().seconds();
        let date = local_date(secs);
        last_per_day
            .entry(date)
            .and_modify(|cur| {
                if secs > cur.0 {
                    *cur = (secs, oid);
                }
            })
            .or_insert((secs, oid));
    }

    let chosen: Vec<(NaiveDate, Oid)> = last_per_day
        .into_iter()
        .map(|(d, (_t, oid))| (d, oid))
        .collect();

    let total = chosen.len();
    if total == 0 {
        return Ok(RepoData {
            snapshots: Vec::new(),
            all_extensions: BTreeSet::new(),
        });
    }

    let mut snapshots = Vec::with_capacity(total);
    let mut all_extensions: BTreeSet<String> = BTreeSet::new();

    for (i, (date, oid)) in chosen.into_iter().enumerate() {
        let snapshot = snapshot_for_commit(&repo, date, oid)?;
        for fs in &snapshot.files {
            all_extensions.insert(fs.extension.clone());
        }
        snapshots.push(snapshot);
        let _ = tx.send(WalkMessage::Progress {
            processed: i + 1,
            total,
        });
    }

    Ok(RepoData {
        snapshots,
        all_extensions,
    })
}

fn snapshot_for_commit(
    repo: &Repository,
    date: NaiveDate,
    oid: Oid,
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
        let blob = match repo.find_blob(entry.id()) {
            Ok(b) => b,
            Err(e) => {
                walk_err = Some(e.to_string());
                return TreeWalkResult::Abort;
            }
        };
        if blob.is_binary() {
            return TreeWalkResult::Ok;
        }
        let lines = loc::count_code_lines(path, blob.content());
        if lines == 0 {
            return TreeWalkResult::Ok;
        }
        let ext = loc::extension_of(path);
        *totals.entry(ext).or_insert(0) += lines;
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
