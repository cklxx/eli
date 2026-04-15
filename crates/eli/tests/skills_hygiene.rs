use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf()
}

fn skill_files() -> Vec<PathBuf> {
    let mut files = fs::read_dir(repo_root().join(".agents/skills"))
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path().join("SKILL.md")))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    files.sort();
    files
}

#[test]
fn skill_docs_do_not_reference_repo_external_paths() {
    for path in skill_files() {
        let body = fs::read_to_string(&path).unwrap();
        assert!(
            !body.contains("/Users/"),
            "skill doc contains absolute user path: {}",
            path.display()
        );
    }
}

#[test]
fn skill_docs_do_not_hardcode_python3_runner() {
    for path in skill_files() {
        let body = fs::read_to_string(&path).unwrap();
        assert!(
            !body.contains("python3 $SKILL_DIR"),
            "skill doc hardcodes python3 runner: {}",
            path.display()
        );
    }
}

#[test]
fn disabled_skill_docs_explain_why() {
    for path in skill_files() {
        let body = fs::read_to_string(&path).unwrap();
        if body.contains("enabled: false") {
            assert!(
                body.contains("disabled_reason:"),
                "disabled skill doc is missing disabled_reason: {}",
                path.display()
            );
        }
    }
}
