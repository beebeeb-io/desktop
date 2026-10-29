// Portable Windows cleanup regressions. Native identity/deletion stays in signout.rs.
#[path = "../src/windows_cf/signout_cleanup.rs"]
mod cleanup;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::{Path, PathBuf},
};
#[derive(Clone)]
struct Row {
    file_id: String,
}
#[derive(Clone)]
struct Contract {
    cache_path: Option<String>,
}
struct StateDb {
    rows: RefCell<Vec<Row>>,
    contracts: HashMap<String, Contract>,
    fail_finish_once: Cell<bool>,
}
impl StateDb {
    fn windows_signout_preflight(&self) -> anyhow::Result<()> {
        Ok(())
    }
    fn list_files(&self) -> anyhow::Result<Vec<Row>> {
        Ok(self.rows.borrow().clone())
    }
    fn get_file_contract_state(&self, id: &str) -> anyhow::Result<Option<Contract>> {
        Ok(self.contracts.get(id).cloned())
    }
    fn finish_windows_signout(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.fail_finish_once.replace(false), "injected DB failure");
        self.rows.borrow_mut().clear();
        Ok(())
    }
}
fn is_disposable_cache_path(path: &Path) -> bool {
    path.is_absolute() && path.parent().and_then(Path::file_name).is_some_and(|p| p == "cache")
}
struct Fixture {
    base: PathBuf,
    root: PathBuf,
    db: StateDb,
}
impl Fixture {
    fn new(name: &str, cache_path: &str, fail_once: bool) -> Self {
        let base = std::env::temp_dir().join(format!("1639-signout-{}-{name}", std::process::id()));
        std::fs::create_dir_all(base.join("root")).unwrap();
        std::fs::create_dir_all(base.join("cache")).unwrap();
        let root = base.join("root");
        std::fs::write(root.join("hydrated.txt"), b"hydrated bytes").unwrap();
        let db = StateDb {
            rows: RefCell::new(vec![Row { file_id: "one".into() }]),
            contracts: HashMap::from([(
                "one".into(),
                Contract {
                    cache_path: Some(cache_path.into()),
                },
            )]),
            fail_finish_once: Cell::new(fail_once),
        };
        Self { base, root, db }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}
#[test]
fn windows_signout_hydrated_sentinel_is_not_external_cache() {
    let f = Fixture::new("sentinel", "", false);
    let result = purge(&f.db, Some(&f.root));
    assert!(result.is_ok(), "hydrated sentinel must sign out: {result:?}");
    assert_eq!(f.db.rows.borrow().len(), 0);
    assert_eq!(std::fs::read_dir(&f.root).unwrap().count(), 0);
}
#[test]
fn windows_signout_rejects_external_cache_before_deleting_placeholders() {
    let f = Fixture::new("invalid", "/not-disposable/valuable.txt", false);
    assert!(purge(&f.db, Some(&f.root)).is_err());
    assert!(
        f.root.join("hydrated.txt").exists(),
        "invalid external cache path must be rejected before deleting placeholders"
    );
    assert_eq!(f.db.rows.borrow().len(), 1);
}
#[test]
fn windows_signout_retries_after_partial_cleanup_with_hydrated_sentinel() {
    let mut f = Fixture::new("retry", "", true);
    let external = f.base.join("cache/content.bin");
    std::fs::write(&external, b"external plaintext").unwrap();
    f.db.rows.borrow_mut().push(Row { file_id: "two".into() });
    f.db.contracts.insert(
        "two".into(),
        Contract {
            cache_path: Some(external.to_string_lossy().into_owned()),
        },
    );
    let error = purge(&f.db, Some(&f.root)).unwrap_err();
    assert!(!f.root.join("hydrated.txt").exists());
    assert!(
        !external.exists(),
        "external plaintext must be deleted before injected final DB failure"
    );
    assert_eq!(f.db.rows.borrow().len(), 2);
    assert!(
        error.to_string().contains("injected DB failure"),
        "cleanup must reach injected final-stage failure: {error}"
    );
    let result = purge(&f.db, Some(&f.root));
    assert!(
        result.is_ok(),
        "retry must complete after partially removed placeholders: {result:?}"
    );
    assert_eq!(f.db.rows.borrow().len(), 0);
}

// Supply filesystem and DB doubles at the two native integration boundaries;
// cache validation ordering, sentinel handling, external unlink and retry policy
// execute the exact helper used by Windows production.
fn purge(db: &StateDb, root: Option<&Path>) -> anyhow::Result<()> {
    db.windows_signout_preflight()?;
    let rows = db.list_files()?;
    let cache_paths = rows
        .iter()
        .map(|row| {
            db.get_file_contract_state(&row.file_id)
                .map(|state| state.and_then(|state| state.cache_path))
        })
        .collect::<Result<Vec<_>, _>>()?;
    cleanup::purge(
        cache_paths,
        is_disposable_cache_path,
        || {
            if let Some(root) = root {
                for entry in std::fs::read_dir(root)? {
                    std::fs::remove_file(entry?.path())?;
                }
            }
            Ok(())
        },
        || db.finish_windows_signout(),
    )
}
