use gitplot::git_walk::{analyze, WalkMessage};
use std::collections::BTreeMap;
use std::path::PathBuf;
use tokio::sync::mpsc;

fn main() {
    let path = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: walk_summary <repo-path>"),
    );

    let (tx, mut rx) = mpsc::unbounded_channel();
    let walk_path = path.clone();
    std::thread::spawn(move || analyze(walk_path, tx));

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        while let Some(msg) = rx.recv().await {
            match msg {
                WalkMessage::Progress { processed, total } => {
                    if processed == total || processed % 10 == 0 {
                        println!("  progress {processed}/{total}");
                    }
                }
                WalkMessage::Failed(e) => {
                    eprintln!("FAILED: {e}");
                    return;
                }
                WalkMessage::Done(data) => {
                    println!("\nrepo: {}", path.display());
                    println!("days: {}", data.snapshots.len());
                    println!("extensions: {:?}", data.all_extensions);

                    if let Some(latest) = data.snapshots.last() {
                        let mut by_ext: BTreeMap<&str, u64> = BTreeMap::new();
                        for fs in &latest.files {
                            *by_ext.entry(fs.extension.as_str()).or_insert(0) += fs.code_lines;
                        }
                        let total: u64 = by_ext.values().sum();
                        println!("\nlatest snapshot ({}): {total} total LOC", latest.date);
                        let mut rows: Vec<_> = by_ext.into_iter().collect();
                        rows.sort_by_key(|(_, v)| std::cmp::Reverse(*v));
                        for (ext, n) in rows.iter().take(15) {
                            println!("  .{ext:<10} {n}");
                        }
                    }

                    if let (Some(first), Some(last)) =
                        (data.snapshots.first(), data.snapshots.last())
                    {
                        let first_total: u64 =
                            first.files.iter().map(|f| f.code_lines).sum();
                        let last_total: u64 = last.files.iter().map(|f| f.code_lines).sum();
                        println!(
                            "\nspan: {} → {} ({} → {} LOC)",
                            first.date, last.date, first_total, last_total
                        );
                    }
                    return;
                }
            }
        }
    });
}
