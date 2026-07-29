use std::fs;
use std::path::{Path, PathBuf};

use codegraide_core::{
    ExtensionId, FileCategory, InventoryOptions, LanguageId, inventory_repository,
    inventory_repository_with_options,
};
use tempfile::tempdir;

fn write_config(root: &Path, contents: &str) -> PathBuf {
    let path = root.join("rules.json");
    fs::write(&path, contents).expect("configuration fixture should be written");
    path
}

#[test]
fn empty_repository_has_no_files_and_all_categories_exist() {
    let repository = tempdir().expect("temporary repository should be created");

    let inventory =
        inventory_repository(repository.path()).expect("empty repository should be inventoried");

    assert_eq!(inventory.inventoried_files, 0);
    for category in FileCategory::ALL {
        assert_eq!(inventory.category_count(category), 0);
    }
}

#[test]
fn applies_embedded_categories_and_keeps_paths_relative_and_sorted() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();

    fs::create_dir(root.join("src")).expect("source directory should be created");
    fs::write(root.join("src").join("main.rs"), "fn main() {}\n")
        .expect("Rust fixture should be written");
    fs::write(root.join("app.py"), "print('hello')\n").expect("Python fixture should be written");
    fs::write(root.join("README.md"), "# Example\n").expect("Markdown fixture should be written");
    fs::write(root.join("Cargo.toml"), "[package]\n")
        .expect("configuration fixture should be written");
    fs::write(root.join("events.jsonl"), "{}\n").expect("data fixture should be written");
    fs::write(root.join("logo.PNG"), []).expect("asset fixture should be written");
    fs::write(root.join("LICENSE"), "Example license\n")
        .expect("uncategorized fixture should be written");

    fs::create_dir(root.join("target")).expect("ignored directory should be created");
    fs::write(root.join("target").join("generated.o"), [])
        .expect("ignored fixture should be written");

    let inventory = inventory_repository(root).expect("fixture repository should be inventoried");

    assert_eq!(inventory.inventoried_files, 7);
    assert_eq!(inventory.source_files(), 2);
    assert_eq!(inventory.category_count(FileCategory::Documentation), 1);
    assert_eq!(inventory.category_count(FileCategory::Configuration), 1);
    assert_eq!(inventory.category_count(FileCategory::Data), 1);
    assert_eq!(inventory.category_count(FileCategory::Assets), 1);
    assert_eq!(inventory.uncategorized_files(), 1);
    assert_eq!(
        inventory.category_files(FileCategory::Source),
        [PathBuf::from("app.py"), PathBuf::from("src/main.rs")]
    );
    assert_eq!(inventory.ignored.builtin_directory_count(), 1);
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
            .uncategorized_files_by_extension
            .get(&ExtensionId::new("[none]")),
        Some(&1)
    );
}

#[test]
fn reports_observed_ignored_files_and_pruned_directory_boundaries() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();

    fs::write(root.join(".gitignore"), "generated/\n*.log\n")
        .expect("Git ignore fixture should be written");
    fs::write(root.join("debug.log"), "ignored\n").expect("ignored file fixture should be written");
    fs::create_dir(root.join("generated")).expect("ignored directory should be created");
    fs::write(root.join("generated").join("output.py"), "ignored\n")
        .expect("ignored descendant fixture should be written");

    let inventory = inventory_repository(root).expect("repository should be inventoried");

    assert!(!inventory.ignored.exact);
    assert_eq!(inventory.ignored.files, [PathBuf::from("debug.log")]);
    assert_eq!(inventory.ignored.directories, [PathBuf::from("generated")]);
}

#[test]
fn exact_ignored_audit_enumerates_descendants_but_not_builtin_contents() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();

    fs::write(root.join(".gitignore"), "generated/\n*.log\n")
        .expect("Git ignore fixture should be written");
    fs::write(root.join("debug.log"), "ignored\n").expect("ignored file fixture should be written");
    fs::create_dir_all(root.join("generated").join("nested"))
        .expect("ignored directories should be created");
    fs::write(root.join("generated").join("output.py"), "ignored\n")
        .expect("ignored descendant fixture should be written");
    fs::write(
        root.join("generated").join("nested").join("data.json"),
        "{}\n",
    )
    .expect("nested ignored fixture should be written");
    fs::create_dir(root.join("target")).expect("built-in directory should be created");
    fs::write(root.join("target").join("secret.bin"), [])
        .expect("built-in descendant should be written");

    let options = InventoryOptions {
        audit_ignored: true,
        ..InventoryOptions::default()
    };
    let inventory =
        inventory_repository_with_options(root, &options).expect("ignored audit should complete");

    assert!(inventory.ignored.exact);
    assert_eq!(
        inventory.ignored.files,
        [
            PathBuf::from("debug.log"),
            PathBuf::from("generated/nested/data.json"),
            PathBuf::from("generated/output.py")
        ]
    );
    assert_eq!(
        inventory.ignored.directories,
        [
            PathBuf::from("generated"),
            PathBuf::from("generated/nested")
        ]
    );
    assert_eq!(
        inventory.ignored.builtin_directories,
        [PathBuf::from("target")]
    );
    assert!(
        inventory
            .ignored
            .files
            .iter()
            .all(|path| !path.starts_with("target"))
    );
}

#[test]
fn respects_gitignore_negations_and_nested_scope() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();
    let nested = root.join("packages").join("example");

    fs::write(root.join(".gitignore"), "*.log\n!important.log\n")
        .expect("root Git ignore fixture should be written");
    fs::write(root.join("debug.log"), "ignored\n").expect("ignored fixture should be written");
    fs::write(root.join("important.log"), "included\n").expect("negated fixture should be written");
    fs::create_dir_all(&nested).expect("nested directory should be created");
    fs::write(nested.join(".gitignore"), "*.tmp\n")
        .expect("nested Git ignore fixture should be written");
    fs::write(nested.join("ignored.tmp"), "ignored\n")
        .expect("nested ignored fixture should be written");
    fs::write(root.join("included.tmp"), "included\n").expect("root fixture should be written");

    let inventory = inventory_repository(root).expect("repository should be inventoried");

    assert_eq!(inventory.inventoried_files, 4);
    assert_eq!(
        inventory.ignored.files,
        [
            PathBuf::from("debug.log"),
            PathBuf::from("packages/example/ignored.tmp")
        ]
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
    fs::write(root.join("generated").join("client.py"), "generated\n")
        .expect("generated fixture should be written");
    fs::write(root.join("vendor").join("library.cpp"), "vendor\n")
        .expect("vendor fixture should be written");

    let options = InventoryOptions {
        include_ignored: vec![
            "generated/**".to_owned(),
            "vendor/**".to_owned(),
            "**/*.py".to_owned(),
        ],
        ..InventoryOptions::default()
    };
    let inventory = inventory_repository_with_options(root, &options)
        .expect("repository should be inventoried");

    assert_eq!(inventory.inventoried_files, 3);
    assert_eq!(inventory.num_included_ignored_files, 2);
    assert_eq!(inventory.source_files(), 2);
}

#[test]
fn builtins_cannot_be_reincluded_or_enumerated() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();

    fs::write(root.join(".gitignore"), "!target/\n!target/generated.rs\n")
        .expect("Git ignore fixture should be written");
    fs::create_dir(root.join("target")).expect("built-in ignored directory should be created");
    fs::write(root.join("target").join("generated.rs"), "generated\n")
        .expect("built-in ignored fixture should be written");

    let options = InventoryOptions {
        include_ignored: vec!["target/**".to_owned()],
        audit_ignored: true,
        ..InventoryOptions::default()
    };
    let inventory = inventory_repository_with_options(root, &options)
        .expect("repository should be inventoried");

    assert_eq!(inventory.inventoried_files, 1);
    assert_eq!(inventory.num_included_ignored_files, 0);
    assert_eq!(
        inventory.ignored.builtin_directories,
        [PathBuf::from("target")]
    );
    assert!(inventory.ignored.files.is_empty());
}

#[test]
fn custom_rules_extend_defaults_and_warn_on_specific_override() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();
    let config = write_config(
        root,
        r#"{
            "config_version": "0.1.0",
            "inventory": {
                "categories": {
                    "configuration": {
                        "include_filenames": ["special.md"]
                    }
                }
            }
        }"#,
    );
    fs::write(root.join("special.md"), "special\n").expect("special fixture should be written");
    fs::write(root.join("other.md"), "other\n").expect("documentation fixture should be written");

    let options = InventoryOptions {
        config_path: Some(config),
        ..InventoryOptions::default()
    };
    let inventory =
        inventory_repository_with_options(root, &options).expect("custom rules should be applied");

    assert_eq!(
        inventory.category_files(FileCategory::Configuration),
        [PathBuf::from("special.md")]
    );
    assert!(
        inventory
            .category_files(FileCategory::Documentation)
            .contains(&PathBuf::from("other.md"))
    );
    assert!(
        inventory
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "category-more-specific-rule-wins")
    );
}

#[test]
fn does_not_auto_discover_repository_local_configuration() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();
    fs::write(
        root.join("codegraide.json"),
        r#"{
            "config_version": "0.1.0",
            "inventory": {
                "ignore_defaults": true,
                "categories": {
                    "data": {"include_extensions": ["log"]}
                }
            }
        }"#,
    )
    .expect("local configuration-looking file should be written");
    fs::write(root.join("events.log"), "event\n").expect("log fixture should be written");

    let inventory = inventory_repository(root)
        .expect("repository-local config should be inventoried but not loaded");

    assert_eq!(inventory.category_count(FileCategory::Data), 0);
    assert!(
        inventory
            .category_files(FileCategory::Configuration)
            .contains(&PathBuf::from("codegraide.json"))
    );
    assert!(
        inventory
            .category_files(FileCategory::Uncategorized)
            .contains(&PathBuf::from("events.log"))
    );
}

#[test]
fn warning_collection_can_be_disabled() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();
    let config = write_config(
        root,
        r#"{
            "config_version": "0.1.0",
            "inventory": {
                "categories": {
                    "configuration": {
                        "include_filenames": ["special.md"]
                    }
                }
            }
        }"#,
    );
    fs::write(root.join("special.md"), "special\n").expect("special fixture should be written");

    let options = InventoryOptions {
        config_path: Some(config),
        emit_warnings: false,
        ..InventoryOptions::default()
    };
    let inventory =
        inventory_repository_with_options(root, &options).expect("custom rules should be applied");

    assert!(inventory.diagnostics.is_empty());
}

#[test]
fn category_replace_discards_only_that_categories_defaults() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();
    let config = write_config(
        root,
        r#"{
            "config_version": "0.1.0",
            "inventory": {
                "categories": {
                    "documentation": {
                        "mode": "replace",
                        "include_extensions": ["txt"]
                    }
                }
            }
        }"#,
    );
    fs::write(root.join("README.md"), "markdown\n").expect("Markdown fixture should be written");
    fs::write(root.join("guide.txt"), "text\n").expect("text fixture should be written");
    fs::write(root.join("main.rs"), "fn main() {}\n").expect("source fixture should be written");

    let options = InventoryOptions {
        config_path: Some(config),
        ..InventoryOptions::default()
    };
    let inventory = inventory_repository_with_options(root, &options)
        .expect("replacement rules should be applied");

    assert_eq!(
        inventory.category_files(FileCategory::Documentation),
        [PathBuf::from("guide.txt")]
    );
    assert!(
        inventory
            .category_files(FileCategory::Uncategorized)
            .contains(&PathBuf::from("README.md"))
    );
    assert_eq!(inventory.source_files(), 1);
}

#[test]
fn global_ignore_defaults_is_absolute_and_warns_about_child_extend() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();
    let config = write_config(
        root,
        r#"{
            "config_version": "0.1.0",
            "inventory": {
                "ignore_defaults": true,
                "categories": {
                    "data": {
                        "mode": "extend",
                        "include_extensions": ["log"]
                    }
                }
            }
        }"#,
    );
    fs::write(root.join("app.py"), "print('hello')\n").expect("Python fixture should be written");
    fs::write(root.join("events.log"), "event\n").expect("log fixture should be written");

    let options = InventoryOptions {
        config_path: Some(config),
        ..InventoryOptions::default()
    };
    let inventory = inventory_repository_with_options(root, &options)
        .expect("global replacement should be applied");

    assert_eq!(inventory.category_count(FileCategory::Data), 1);
    assert!(
        inventory
            .category_files(FileCategory::Uncategorized)
            .contains(&PathBuf::from("app.py"))
    );
    assert!(
        inventory
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "config-child-extend-ignored")
    );
}

#[test]
fn filename_regex_include_can_be_carved_out_by_directory_regex_exclude() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();
    let config = write_config(
        root,
        r#"{
            "config_version": "0.1.0",
            "inventory": {
                "ignore_defaults": true,
                "categories": {
                    "data": {
                        "include_filename_regexes": ["[0-9]{8}\\.json"],
                        "exclude_filename_regexes": [
                            "private/(?:.*/)?[0-9]{8}\\.json"
                        ]
                    }
                }
            }
        }"#,
    );
    fs::create_dir_all(root.join("public").join("logs"))
        .expect("public directory should be created");
    fs::create_dir_all(root.join("private").join("team"))
        .expect("private directory should be created");
    fs::write(
        root.join("public").join("logs").join("20260728.json"),
        "{}\n",
    )
    .expect("public log should be written");
    fs::write(
        root.join("private").join("team").join("20260728.json"),
        "{}\n",
    )
    .expect("private log should be written");

    let options = InventoryOptions {
        config_path: Some(config),
        ..InventoryOptions::default()
    };
    let inventory =
        inventory_repository_with_options(root, &options).expect("regex rules should be applied");

    assert_eq!(
        inventory.category_files(FileCategory::Data),
        [PathBuf::from("public/logs/20260728.json")]
    );
    assert!(
        inventory
            .category_files(FileCategory::Uncategorized)
            .contains(&PathBuf::from("private/team/20260728.json"))
    );
}

#[test]
fn regex_override_warning_recommends_a_regex_exclusion() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();
    let config = write_config(
        root,
        r#"{
            "config_version": "0.1.0",
            "inventory": {
                "categories": {
                    "data": {
                        "include_filename_regexes": ["service-.*\\.md"]
                    }
                }
            }
        }"#,
    );
    fs::write(root.join("service-users.md"), "data\n")
        .expect("regex override fixture should be written");

    let options = InventoryOptions {
        config_path: Some(config),
        ..InventoryOptions::default()
    };
    let inventory = inventory_repository_with_options(root, &options)
        .expect("regex override should be applied");

    assert!(
        inventory
            .category_files(FileCategory::Data)
            .contains(&PathBuf::from("service-users.md"))
    );
    let warning = inventory
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "category-more-specific-rule-wins")
        .expect("regex override should warn");
    assert!(warning.message.contains("exclude_filename_regexes"));
    assert!(warning.message.contains("service-.*\\\\.md"));
}

#[test]
fn rejects_invalid_and_contradictory_configuration() {
    let cases = [
        (
            "bad version",
            r#"{"config_version":"0.1","inventory":{}}"#,
            "Semantic Versioning",
        ),
        (
            "unknown category",
            r#"{
                "config_version":"0.1.0",
                "inventory":{"categories":{"infrastructure":{}}}
            }"#,
            "unknown category",
        ),
        (
            "extension conflict",
            r#"{
                "config_version":"0.1.0",
                "inventory":{
                    "ignore_defaults":true,
                    "categories":{
                        "documentation":{"include_extensions":["md"]},
                        "data":{"include_extensions":["MD"]}
                    }
                }
            }"#,
            "claimed by both",
        ),
        (
            "exact filename conflict",
            r#"{
                "config_version":"0.1.0",
                "inventory":{
                    "ignore_defaults":true,
                    "categories":{
                        "documentation":{"include_filenames":["manifest.json"]},
                        "configuration":{"include_filenames":["manifest.json"]}
                    }
                }
            }"#,
            "claimed by both",
        ),
        (
            "contradictory regex",
            r#"{
                "config_version":"0.1.0",
                "inventory":{
                    "ignore_defaults":true,
                    "categories":{
                        "data":{
                            "include_filename_regexes":["private/.*\\.json"],
                            "exclude_filename_regexes":["private/.*\\.json"]
                        }
                    }
                }
            }"#,
            "includes and excludes",
        ),
        (
            "absolute-looking regex",
            r#"{
                "config_version":"0.1.0",
                "inventory":{
                    "ignore_defaults":true,
                    "categories":{
                        "data":{"include_filename_regexes":["/private/.*\\.json"]}
                    }
                }
            }"#,
            "starts with '/'",
        ),
        (
            "extensionless exact include",
            r#"{
                "config_version":"0.1.0",
                "inventory":{
                    "ignore_defaults":true,
                    "categories":{
                        "documentation":{"include_filenames":["README"]}
                    }
                }
            }"#,
            "with an extension",
        ),
    ];

    for (name, contents, expected) in cases {
        let repository = tempdir().expect("temporary repository should be created");
        let config = write_config(repository.path(), contents);
        let options = InventoryOptions {
            config_path: Some(config),
            ..InventoryOptions::default()
        };

        let error = inventory_repository_with_options(repository.path(), &options).expect_err(name);

        assert!(
            error.to_string().contains(expected),
            "{name}: expected {expected:?}, got {error}"
        );
    }
}

#[test]
fn rejects_runtime_regex_conflicts_at_the_same_specificity() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();
    let config = write_config(
        root,
        r#"{
            "config_version": "0.1.0",
            "inventory": {
                "ignore_defaults": true,
                "categories": {
                    "documentation": {
                        "include_filename_regexes": [".*\\.json"]
                    },
                    "data": {
                        "include_filename_regexes": ["[0-9]{8}\\.json"]
                    }
                }
            }
        }"#,
    );
    fs::write(root.join("20260728.json"), "{}\n").expect("conflicting fixture should be written");

    let options = InventoryOptions {
        config_path: Some(config),
        ..InventoryOptions::default()
    };
    let error = inventory_repository_with_options(root, &options)
        .expect_err("overlapping regexes should conflict");

    assert!(error.to_string().contains("20260728.json"));
    assert!(error.to_string().contains("documentation"));
    assert!(error.to_string().contains("data"));
}

#[test]
fn rejects_invalid_include_ignored_globs() {
    let repository = tempdir().expect("temporary repository should be created");
    let options = InventoryOptions {
        include_ignored: vec!["[".to_owned()],
        ..InventoryOptions::default()
    };

    let error = inventory_repository_with_options(repository.path(), &options)
        .expect_err("invalid glob should fail inventory");

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

    assert_eq!(inventory.inventoried_files, 2);
    assert_eq!(inventory.source_files(), 1);
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn inventories_non_utf8_filenames_without_panicking() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let repository = tempdir().expect("temporary repository should be created");
    let filename = OsString::from_vec(vec![b'm', b'o', b'd', 0xFF, b'.', b'r', b's']);
    fs::write(repository.path().join(filename), "fn example() {}\n")
        .expect("non-UTF-8 fixture should be written");

    let inventory =
        inventory_repository(repository.path()).expect("non-UTF-8 filename should be inventoried");

    assert_eq!(inventory.inventoried_files, 1);
    assert_eq!(inventory.source_files(), 1);
    assert_eq!(
        inventory.files_by_language.get(&LanguageId::new("rust")),
        Some(&1)
    );
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

    assert_eq!(inventory.inventoried_files, 1);
    assert_eq!(inventory.source_files(), 1);
}
