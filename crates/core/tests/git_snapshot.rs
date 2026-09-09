use codegraide_core::git_snapshot::{GitLimits, GitRepository, SnapshotError};
use std::{fs, path::Path, process::Command, time::Duration};

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-q"]);
    git(root.path(), &["checkout", "-qb", "codex/snapshot-test"]);
    fs::write(root.path().join("empty.py"), "").unwrap();
    fs::write(root.path().join("é space.py"), "pass\n").unwrap();
    fs::write(root.path().join("binary.py"), b"a\0b").unwrap();
    fs::write(root.path().join("unsupported.txt"), "text").unwrap();
    git(root.path(), &["add", "."]);
    git(
        root.path(),
        &["commit", "-qm", "test(snapshot): add source fixtures"],
    );
    root
}
#[test]
fn batch_reads_empty_unicode_and_binary_blobs_without_worktree_reads() {
    let root = fixture();
    fs::write(root.path().join("é space.py"), "dirty").unwrap();
    let repo = GitRepository::open(root.path()).unwrap();
    let snapshot = repo
        .snapshot("HEAD", 8, |p| p.extension().is_some_and(|e| e == "py"))
        .unwrap();
    assert_eq!(snapshot.files.len(), 2);
    assert_eq!(snapshot.files[Path::new("é space.py")].source, "pass\n");
    assert!(snapshot.files[Path::new("empty.py")].source.is_empty());
    assert_eq!(snapshot.entries.len(), 4);
    assert_eq!(snapshot.excluded.len(), 2);
}
#[test]
fn source_and_metadata_have_independent_limits() {
    let root = fixture();
    let repo = GitRepository::open(root.path()).unwrap();
    assert!(matches!(
        repo.snapshot("HEAD", 1, |_| true),
        Err(SnapshotError::Limit {
            resource: "--max-input-bytes",
            ..
        })
    ));
    let repo = GitRepository::open_with_limits(
        root.path(),
        GitLimits {
            entries: 1,
            ..GitLimits::default()
        },
    )
    .unwrap();
    assert!(matches!(
        repo.snapshot("HEAD", 100, |_| false),
        Err(SnapshotError::Limit {
            resource: "tree entries",
            ..
        })
    ));
    let repo = GitRepository::open_with_limits(
        root.path(),
        GitLimits {
            metadata_bytes: 64,
            ..GitLimits::default()
        },
    )
    .unwrap();
    assert!(repo.snapshot("HEAD", 100, |_| false).is_err());
}
#[test]
fn deadline_errors_retain_the_io_cause() {
    use std::error::Error;
    let root = fixture();
    let error = match GitRepository::open_with_limits(
        root.path(),
        GitLimits {
            timeout: Duration::ZERO,
            ..GitLimits::default()
        },
    ) {
        Ok(_) => panic!("expired deadline must fail"),
        Err(error) => error,
    };
    assert_eq!(
        error
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::TimedOut
    );
}
