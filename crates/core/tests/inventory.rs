use std::fs;

use codegraide_core::{ExtensionId, LanguageId, inventory_repository};
use tempfile::tempdir;

#[test]
fn empty_repository_has_no_files() {
    let repository = tempdir().expect("temporary repository should be created");

    let inventory =
        inventory_repository(repository.path()).expect("empty repository should be inventoried");

    assert_eq!(inventory.total_files, 0);
}

#[test]
fn classifies_repository_files_and_skips_generated_directories() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();

    fs::create_dir(root.join("src")).expect("source directory should be created");
    fs::write(root.join("src").join("main.rs"), "fn main() {}\n")
        .expect("Rust fixture should be written");
    fs::write(root.join("app.py"), "print('hello')\n").expect("Python fixture should be written");
    fs::write(root.join("README.md"), "# Example\n").expect("Markdown fixture should be written");
    fs::write(root.join("LICENSE"), "Example license\n")
        .expect("license fixture should be written");

    fs::create_dir(root.join("target")).expect("ignored directory should be created");
    fs::write(root.join("target").join("generated.o"), [])
        .expect("ignored fixture should be written");

    let inventory = inventory_repository(root).expect("fixture repository should be inventoried");

    assert_eq!(inventory.total_files, 4);
    assert_eq!(inventory.recognized_source_files(), 2);
    assert_eq!(inventory.unclassified_files(), 2);
    assert_eq!(inventory.num_ignored_directories, 1);
    assert_eq!(
        inventory.files_by_language.get(&LanguageId::new("rust")),
        Some(&1)
    );
    assert_eq!(
        inventory.files_by_language.get(&LanguageId::new("python")),
        Some(&1)
    );
    assert_eq!(
        inventory
            .unclassified_files_by_extension
            .get(&ExtensionId::new("md")),
        Some(&1)
    );
    assert_eq!(
        inventory
            .unclassified_files_by_extension
            .get(&ExtensionId::new("[none]")),
        Some(&1)
    );
}
