use gitplot::git_walk::{analyze, WalkMessage};
use std::path::PathBuf;
use tokio::sync::mpsc;

fn run(path: PathBuf) -> Vec<WalkMessage> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let handle = std::thread::spawn(move || analyze(path, tx));
    let mut out = Vec::new();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        while let Some(msg) = rx.recv().await {
            out.push(msg);
        }
    });
    handle.join().unwrap();
    out
}

#[test]
fn walks_self_repo_without_error() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let messages = run(PathBuf::from(manifest));
    assert!(!messages.is_empty(), "expected at least one message");
    let last = messages.last().unwrap();
    match last {
        WalkMessage::Done(data) => {
            assert!(
                !data.snapshots.is_empty(),
                "self repo should have at least one daily snapshot"
            );
            for snap in &data.snapshots {
                let total: u64 = snap.files.iter().map(|f| f.code_lines).sum();
                assert!(
                    total > 0 || snap.files.is_empty(),
                    "non-empty snapshot must have nonzero code lines"
                );
            }
        }
        WalkMessage::Failed(e) => panic!("walk failed: {e}"),
        WalkMessage::Progress { .. } => panic!("last message must be Done or Failed"),
    }
}

#[test]
fn rejects_non_repo_directory() {
    let messages = run(PathBuf::from("/tmp"));
    let last = messages.last().expect("at least one message");
    assert!(matches!(last, WalkMessage::Failed(_)));
}
