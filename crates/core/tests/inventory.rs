use std::fs;

use codegraide_core::{
    ExtensionId, InventoryOptions, LanguageId, inventory_repository,
    inventory_repository_with_options,
};
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
    assert_eq!(inventory.num_builtin_ignored_directories, 1);
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

#[test]
fn respects_repository_gitignore_rules_and_negations() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();

    fs::write(
        root.join(".gitignore"),
        "generated/\n*.log\n!important.log\n",
    )
    .expect("Git ignore fixture should be written");
    fs::write(root.join("app.py"), "print('included')\n")
        .expect("included Python fixture should be written");
    fs::write(root.join("debug.log"), "ignored\n").expect("ignored log fixture should be written");
    fs::write(root.join("important.log"), "included\n")
        .expect("re-included log fixture should be written");

    fs::create_dir(root.join("generated")).expect("ignored directory should be created");
    fs::write(
        root.join("generated").join("output.py"),
        "print('ignored')\n",
    )
    .expect("ignored Python fixture should be written");

    let inventory = inventory_repository(root).expect("repository should be inventoried");

    assert_eq!(inventory.total_files, 3);
    assert_eq!(inventory.recognized_source_files(), 1);
    assert_eq!(inventory.unclassified_files(), 2);
    assert_eq!(
        inventory.files_by_language.get(&LanguageId::new("python")),
        Some(&1)
    );
    assert_eq!(
        inventory
            .unclassified_files_by_extension
            .get(&ExtensionId::new("log")),
        Some(&1)
    );
}

#[test]
fn scopes_nested_gitignore_rules_to_their_directory() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();
    let nested = root.join("packages").join("example");

    fs::create_dir_all(&nested).expect("nested directory should be created");
    fs::write(nested.join(".gitignore"), "*.tmp\n")
        .expect("nested Git ignore fixture should be written");
    fs::write(nested.join("ignored.tmp"), "ignored\n")
        .expect("nested ignored fixture should be written");
    fs::write(nested.join("app.py"), "print('included')\n")
        .expect("nested Python fixture should be written");
    fs::write(root.join("included.tmp"), "included\n").expect("root fixture should be written");

    let inventory = inventory_repository(root).expect("repository should be inventoried");

    assert_eq!(inventory.total_files, 3);
    assert_eq!(inventory.recognized_source_files(), 1);
    assert_eq!(
        inventory
            .unclassified_files_by_extension
            .get(&ExtensionId::new("tmp")),
        Some(&1)
    );
}

#[test]
fn includes_multiple_ignored_globs_without_counting_duplicates() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();

    fs::write(root.join(".gitignore"), "generated/\nvendor/\n")
        .expect("Git ignore fixture should be written");
    fs::create_dir(root.join("generated")).expect("generated directory should be created");
    fs::create_dir(root.join("vendor")).expect("vendor directory should be created");
    fs::write(
        root.join("generated").join("client.py"),
        "print('generated')\n",
    )
    .expect("generated Python fixture should be written");
    fs::write(
        root.join("vendor").join("library.cpp"),
        "void library() {}\n",
    )
    .expect("vendor C++ fixture should be written");

    let options = InventoryOptions {
        include_ignored: vec![
            "generated/**".to_owned(),
            "vendor/**".to_owned(),
            "**/*.py".to_owned(),
        ],
    };
    let inventory = inventory_repository_with_options(root, &options)
        .expect("repository should be inventoried");

    assert_eq!(inventory.total_files, 3);
    assert_eq!(inventory.num_included_ignored_files, 2);
    assert_eq!(inventory.recognized_source_files(), 2);
    assert_eq!(
        inventory.files_by_language.get(&LanguageId::new("python")),
        Some(&1)
    );
    assert_eq!(
        inventory.files_by_language.get(&LanguageId::new("cpp")),
        Some(&1)
    );
}

#[test]
fn built_in_ignored_directories_cannot_be_reincluded() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();

    fs::write(root.join(".gitignore"), "!target/\n!target/generated.rs\n")
        .expect("Git ignore fixture should be written");
    fs::create_dir(root.join("target")).expect("built-in ignored directory should be created");
    fs::write(
        root.join("target").join("generated.rs"),
        "fn generated() {}\n",
    )
    .expect("built-in ignored fixture should be written");

    let options = InventoryOptions {
        include_ignored: vec!["target/**".to_owned()],
    };
    let inventory = inventory_repository_with_options(root, &options)
        .expect("repository should be inventoried");

    assert_eq!(inventory.total_files, 1);
    assert_eq!(inventory.recognized_source_files(), 0);
    assert_eq!(inventory.num_builtin_ignored_directories, 1);
    assert_eq!(inventory.num_included_ignored_files, 0);
}

#[test]
fn rejects_invalid_include_ignored_globs() {
    let repository = tempdir().expect("temporary repository should be created");
    let options = InventoryOptions {
        include_ignored: vec!["[".to_owned()],
    };

    let error = inventory_repository_with_options(repository.path(), &options)
        .expect_err("invalid glob should fail inventory");

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("include-ignored"));
}

#[test]
fn does_not_apply_ripgrep_ignore_files() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();

    fs::write(root.join(".ignore"), "visible.py\n").expect("ignore fixture should be written");
    fs::write(root.join("visible.py"), "print('visible')\n")
        .expect("Python fixture should be written");

    let inventory = inventory_repository(root).expect("repository should be inventoried");

    assert_eq!(inventory.total_files, 2);
    assert_eq!(inventory.recognized_source_files(), 1);
}

#[cfg(unix)]
#[test]
fn does_not_follow_symbolic_links() {
    use std::os::unix::fs::symlink;

    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();
    let source = root.join("source.py");

    fs::write(&source, "print('source')\n").expect("Python fixture should be written");
    symlink(&source, root.join("linked.py")).expect("symbolic link should be created");

    let inventory = inventory_repository(root).expect("repository should be inventoried");

    assert_eq!(inventory.total_files, 1);
    assert_eq!(inventory.recognized_source_files(), 1);
}
