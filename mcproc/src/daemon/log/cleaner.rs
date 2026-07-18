use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::warn;

/// Delete log files under a project's log directory.
/// Files listed in `exclude` are kept (logs of processes still running).
/// Returns the paths of files actually deleted.
pub fn delete_project_logs(project_log_dir: &Path, exclude: &HashSet<PathBuf>) -> Vec<PathBuf> {
    let entries = match fs::read_dir(project_log_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            warn!(
                path = %project_log_dir.display(),
                %error,
                "Failed to read project log directory"
            );
            return Vec::new();
        }
    };

    let mut candidates = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            let is_log_file = entry.file_type().is_ok_and(|file_type| file_type.is_file())
                && path.extension().is_some_and(|extension| extension == "log");

            (is_log_file && !exclude.contains(&path)).then_some(path)
        })
        .collect::<Vec<_>>();
    candidates.sort();

    let mut deleted = Vec::with_capacity(candidates.len());
    for path in candidates {
        match fs::remove_file(&path) {
            Ok(()) => deleted.push(path),
            Err(error) => warn!(
                path = %path.display(),
                %error,
                "Failed to delete log file"
            ),
        }
    }

    let directory_is_empty =
        fs::read_dir(project_log_dir).is_ok_and(|mut entries| entries.next().is_none());
    if directory_is_empty {
        let _ = fs::remove_dir(project_log_dir);
    }

    deleted
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn deletes_all_log_files_and_returns_sorted_paths() {
        let temp = tempdir().unwrap();
        let project_dir = temp.path().join("project");
        fs::create_dir(&project_dir).unwrap();
        let second = project_dir.join("second.log");
        let first = project_dir.join("first.log");
        fs::write(&second, "second").unwrap();
        fs::write(&first, "first").unwrap();

        let deleted = delete_project_logs(&project_dir, &HashSet::new());

        assert_eq!(deleted, vec![first.clone(), second.clone()]);
        assert!(!first.exists());
        assert!(!second.exists());
    }

    #[test]
    fn keeps_excluded_log_file() {
        let temp = tempdir().unwrap();
        let project_dir = temp.path().join("project");
        fs::create_dir(&project_dir).unwrap();
        let deleted_log = project_dir.join("delete.log");
        let excluded_log = project_dir.join("keep.log");
        fs::write(&deleted_log, "delete").unwrap();
        fs::write(&excluded_log, "keep").unwrap();
        let exclude = HashSet::from([excluded_log.clone()]);

        let deleted = delete_project_logs(&project_dir, &exclude);

        assert_eq!(deleted, vec![deleted_log.clone()]);
        assert!(!deleted_log.exists());
        assert!(excluded_log.exists());
    }

    #[test]
    fn returns_empty_for_missing_directory() {
        let temp = tempdir().unwrap();
        let missing = temp.path().join("missing");

        let deleted = delete_project_logs(&missing, &HashSet::new());

        assert!(deleted.is_empty());
    }

    #[test]
    fn keeps_non_log_files() {
        let temp = tempdir().unwrap();
        let project_dir = temp.path().join("project");
        fs::create_dir(&project_dir).unwrap();
        let note = project_dir.join("note.txt");
        fs::write(&note, "note").unwrap();

        let deleted = delete_project_logs(&project_dir, &HashSet::new());

        assert!(deleted.is_empty());
        assert!(note.exists());
    }

    #[test]
    fn keeps_subdirectories_and_their_contents() {
        let temp = tempdir().unwrap();
        let project_dir = temp.path().join("project");
        let subdirectory = project_dir.join("sub");
        fs::create_dir_all(&subdirectory).unwrap();
        let inner_log = subdirectory.join("inner.log");
        fs::write(&inner_log, "inner").unwrap();

        let deleted = delete_project_logs(&project_dir, &HashSet::new());

        assert!(deleted.is_empty());
        assert!(subdirectory.exists());
        assert!(inner_log.exists());
    }

    #[test]
    fn removes_directory_after_deleting_all_files() {
        let temp = tempdir().unwrap();
        let project_dir = temp.path().join("project");
        fs::create_dir(&project_dir).unwrap();
        fs::write(project_dir.join("only.log"), "log").unwrap();

        delete_project_logs(&project_dir, &HashSet::new());

        assert!(!project_dir.exists());
    }

    #[test]
    fn keeps_directory_when_excluded_file_remains() {
        let temp = tempdir().unwrap();
        let project_dir = temp.path().join("project");
        fs::create_dir(&project_dir).unwrap();
        let excluded_log = project_dir.join("keep.log");
        fs::write(&excluded_log, "keep").unwrap();
        let exclude = HashSet::from([excluded_log]);

        delete_project_logs(&project_dir, &exclude);

        assert!(project_dir.exists());
    }
}
