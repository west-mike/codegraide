//! Publish only wholly owned bundles. Stage before replacing the old directory.
use std::{collections::BTreeSet, fs, io, path::Path};

const MANIFEST: &str = "codegraide-dependency-report.json";

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn filename(name: &str) -> io::Result<()> {
    if name.is_empty() || name.contains(['/', '\\']) || matches!(name, "." | "..") {
        return Err(invalid("dependency manifest contains an unsafe filename"));
    }
    Ok(())
}

fn validate_existing(output: &Path) -> io::Result<bool> {
    let metadata = match fs::symlink_metadata(output) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !metadata.is_dir() {
        return Err(invalid("output must be a directory, not a symlink or file"));
    }
    let entries = fs::read_dir(output)?.collect::<Result<Vec<_>, _>>()?;
    if entries.is_empty() {
        return Ok(true);
    }
    // Check entry types before opening the manifest; never read through a link.
    for entry in &entries {
        if !entry.file_type()?.is_file() {
            return Err(invalid("report contains a symlink or non-regular entry"));
        }
    }
    let source = fs::read_to_string(output.join(MANIFEST))
        .map_err(|_| invalid("directory is nonempty and has no Codegraide dependency manifest"))?;
    let manifest: serde_json::Value =
        serde_json::from_str(&source).map_err(|_| invalid("invalid dependency manifest"))?;
    if manifest["format"] != "codegraide-dependency-html-bundle-v1" {
        return Err(invalid("unrecognized dependency bundle format"));
    }
    let names = manifest["generated_files"]
        .as_array()
        .ok_or_else(|| invalid("manifest requires generated_files"))?;
    let mut owned = BTreeSet::from([MANIFEST]);
    for name in names {
        let name = name
            .as_str()
            .ok_or_else(|| invalid("invalid generated filename"))?;
        filename(name)?;
        owned.insert(name);
    }
    for entry in entries {
        if !entry
            .file_name()
            .to_str()
            .is_some_and(|name| owned.contains(name))
        {
            return Err(invalid("report directory contains an unowned file"));
        }
    }
    Ok(true)
}

pub(crate) fn publish(output: &Path, pages: &[(String, String)]) -> io::Result<()> {
    for (name, _) in pages {
        filename(name)?;
    }
    if pages
        .iter()
        .map(|(name, _)| name)
        .collect::<BTreeSet<_>>()
        .len()
        != pages.len()
    {
        return Err(invalid("duplicate report filename"));
    }
    let existed = validate_existing(output)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let stage = tempfile::Builder::new()
        .prefix("codegraide-report-")
        .tempdir_in(parent)?;
    let next = stage.path().join("next");
    fs::create_dir(&next)?;
    for (name, contents) in pages {
        fs::write(next.join(name), contents)?;
    }
    let previous = stage.path().join("previous");
    if existed {
        fs::rename(output, &previous)?;
    }
    if let Err(error) = fs::rename(&next, output) {
        if existed {
            if let Err(restore) = fs::rename(&previous, output) {
                let recovery = stage.keep();
                return Err(io::Error::other(format!(
                    "publication failed: {error}; rollback failed: {restore}; previous bundle retained at {}",
                    recovery.join("previous").display()
                )));
            }
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pages() -> Vec<(String, String)> {
        vec![("python.html".into(), "new".into()), (MANIFEST.into(),
            r#"{"format":"codegraide-dependency-html-bundle-v1","generated_files":["python.html"]}"#.into())]
    }
    #[test]
    fn invalid_manifest_and_foreign_files_leave_old_bundle_intact() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("report");
        publish(&output, &pages()).unwrap();
        fs::write(output.join("foreign"), "keep").unwrap();
        assert!(publish(&output, &pages()).is_err());
        assert_eq!(fs::read_to_string(output.join("foreign")).unwrap(), "keep");
        fs::remove_file(output.join("foreign")).unwrap();
        fs::write(output.join(MANIFEST), r#"{"format":"codegraide-dependency-html-bundle-v1","generated_files":["python.html","../bad"]}"#).unwrap();
        assert!(publish(&output, &pages()).is_err());
        assert_eq!(
            fs::read_to_string(output.join("python.html")).unwrap(),
            "new"
        );
    }
    #[cfg(unix)]
    #[test]
    fn links_cannot_redirect_publication() {
        for name in ["python.html", MANIFEST] {
            let root = tempfile::tempdir().unwrap();
            let output = root.path().join("report");
            publish(&output, &pages()).unwrap();
            let victim = root.path().join("victim");
            fs::write(&victim, "keep").unwrap();
            fs::remove_file(output.join(name)).unwrap();
            std::os::unix::fs::symlink(&victim, output.join(name)).unwrap();
            assert!(publish(&output, &pages()).is_err());
            assert_eq!(fs::read_to_string(victim).unwrap(), "keep");
        }
    }
}
