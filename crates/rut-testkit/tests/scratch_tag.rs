//! THE LAW AS A TEST: a scratch owns its path's shape.
//!
//! A tag is ONE path segment — a `/` (or `\`) is flattened by the guard,
//! never worked around at the call site. So a `Scratch` is always one
//! flat directory directly under the temp root, and it is gone when the
//! guard drops — no stray parent it never owned.

use rut_testkit::{temp_root, Scratch};

#[test]
fn a_slash_bearing_tag_is_one_flat_dir_dead_at_drop() {
    let root = temp_root();
    let pid = std::process::id();
    let tag = format!("pin-{pid}-slash/tail");
    let path = {
        let scratch = Scratch::new(&tag);
        let path = scratch.to_path_buf();
        assert!(path.is_dir(), "the scratch exists: {}", path.display());
        assert_eq!(
            path.parent(),
            Some(root.as_path()),
            "a tag is ONE path segment — a `/` is flattened, so the scratch \
             sits directly under the temp root (got nested at {})",
            path.display()
        );
        let name = path
            .file_name()
            .expect("a scratch names its own directory")
            .to_string_lossy()
            .into_owned();
        assert!(
            name.starts_with(&format!("rut-pin-{pid}-slash-tail-")),
            "both halves of the tag ride the one flat name: {name}"
        );
        path
    };
    assert!(
        !path.exists(),
        "the scratch dies with the guard: {}",
        path.display()
    );
    // the guard leaves nothing of its own behind: no stray parent exists.
    assert!(
        !root.join(format!("rut-pin-{pid}-slash")).exists(),
        "the drop removed the leaf only — a stray parent survives"
    );
}

#[test]
fn a_plain_tag_keeps_its_shape() {
    let root = temp_root();
    let pid = std::process::id();
    let path = {
        let scratch = Scratch::new("plain-tag");
        let path = scratch.to_path_buf();
        assert_eq!(
            path.parent(),
            Some(root.as_path()),
            "a plain tag is one flat directory directly under the temp root"
        );
        let name = path
            .file_name()
            .expect("a scratch names its own directory")
            .to_string_lossy()
            .into_owned();
        assert!(
            name.starts_with(&format!("rut-plain-tag-{pid}-")),
            "the guard's own name shape is unchanged: {name}"
        );
        path
    };
    assert!(!path.exists(), "the scratch dies with the guard");
}
