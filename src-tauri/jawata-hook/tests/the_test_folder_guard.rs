//! The guard every test makes its temporary folder with: the folder goes with it, also when the test panics.

#[path = "../src/testdir.rs"]
mod testdir;

use testdir::TestDir;

#[test]
fn the_folder_and_what_it_holds_go_with_the_guard() {
    let kept;
    {
        let dir = TestDir::new("jawata-testdir-drop");
        std::fs::write(dir.join("file"), "x").unwrap();
        kept = dir.to_path_buf();
        assert!(kept.join("file").is_file());
    }
    assert!(!kept.exists(), "{} is left behind", kept.display());
}

#[test]
fn a_test_that_panics_still_removes_its_folder() {
    let seen = std::sync::Mutex::new(None);
    let outcome = std::panic::catch_unwind(|| {
        let dir = TestDir::new("jawata-testdir-panic");
        *seen.lock().unwrap() = Some(dir.to_path_buf());
        panic!("the test fails");
    });
    assert!(outcome.is_err());
    let path = seen.lock().unwrap_or_else(|e| e.into_inner()).clone().expect("the folder was made");
    assert!(!path.exists(), "{} is left behind", path.display());
}

#[test]
fn a_child_keeps_the_folder_alive_and_the_whole_folder_goes_with_it() {
    let root;
    {
        let inner = TestDir::new("jawata-testdir-child").child("seats");
        root = inner.parent().unwrap().to_path_buf();
        assert!(root.is_dir(), "the folder lives as long as the guard of its child");
        assert!(!inner.exists(), "a child is a path, not a folder made");
    }
    assert!(!root.exists());
}

#[test]
fn two_folders_of_one_label_are_two_folders() {
    let (a, b) = (TestDir::new("jawata-testdir-two"), TestDir::new("jawata-testdir-two"));
    assert_ne!(*a, *b);
}
