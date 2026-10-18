//! The url-dep tests' shared harness: the ready-map remote (ex-`Table`
//! — a `BTreeMap` fixture behind `std::future::ready`: no network, no
//! runtime dep) and the std-only `block_on` driver. The layer split
//! under test everywhere it is used: the CALL SITE owns HOW bytes
//! arrive (here: the map), the LOADER owns WHAT they are — the pin is
//! rut.lock law, verified at the mount door on every load.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::task::{Context, Poll};

use rut_native::{DepRemote, RemoteError};

/// Write `<dir>/rut.lock` pinning every url in the table to the
/// sha256 of its fixture bytes — the realistic fixture: the walk
/// reads the lock from disk now, and a project that fetches is a
/// project that installed.
pub fn write_lock(dir: &Path, table: &BTreeMap<String, Vec<u8>>) {
    let rows = table
        .iter()
        .map(|(url, bytes)| rut_driver::LockRow {
            pkg: url
                .rsplit('/')
                .next()
                .unwrap_or(url)
                .trim_end_matches(".rutbundle")
                .to_string(),
            url: url.clone(),
            sha256: rut_driver::sha256_hex(bytes),
        })
        .collect();
    std::fs::write(
        dir.join("rut.lock"),
        rut_driver::render_lock(&rut_driver::LockFile::from_rows(rows)),
    )
    .unwrap();
}

/// The in-memory remote: url → bytes, answered `ready` (sync impls are
/// first-class on the trait).
pub struct Table(BTreeMap<String, Vec<u8>>);

impl Table {
    pub fn new() -> Self {
        Table(BTreeMap::new())
    }

    pub fn insert(&mut self, url: &str, bytes: Vec<u8>) {
        self.0.insert(url.to_string(), bytes);
    }
}

impl From<BTreeMap<String, Vec<u8>>> for Table {
    fn from(map: BTreeMap<String, Vec<u8>>) -> Self {
        Table(map)
    }
}

impl DepRemote for Table {
    fn fetch(
        &self,
        url: &str,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<Vec<u8>, RemoteError>> + '_>,
    > {
        let r = match self.0.get(url) {
            Some(bytes) => Ok(bytes.clone()),
            None => Err(RemoteError::new(format!("no fixture bytes for {url}"))),
        };
        Box::pin(std::future::ready(r))
    }
}

/// The std-only driver for the `_with` lanes: the fetched futures are
/// `ready`, so one noop-waker poll settles them; a Pending here would
/// spin — and there is nothing async behind these fixtures to park on.
pub fn block_on<F: Future>(fut: F) -> F::Output {
    let mut fut = std::pin::pin!(fut);
    let mut cx = Context::from_waker(std::task::Waker::noop());
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

/// The archive group a named pkg rode under — `deps/<name>@<fp16>`:
/// tests derive the identity dir from the packed entry names (the fp16
/// is the content's, never a constant here).
#[allow(dead_code)]
pub fn dep_dir(names: &[String], pkg: &str) -> String {
    let comp = names
        .iter()
        .filter_map(|n| {
            let comp = n.strip_prefix("deps/")?.split('/').next()?;
            rut_driver::bundle::parse_identity(comp)
                .filter(|(n, _)| *n == pkg)
                .map(|_| comp.to_string())
        })
        .next()
        .unwrap_or_else(|| panic!("no deps/ group for `{pkg}` in the archive"));
    format!("deps/{comp}")
}

/// The walked pkg whose mount key spells `name` — a bare root or an
/// identity-keyed group (`name@fp16`); single-instance worlds only.
#[allow(dead_code)]
pub fn pkg_named<'a>(
    loaded: &'a rut_driver::Walk,
    name: &str,
) -> &'a rut_driver::Mounted {
    loaded
        .pkgs
        .iter()
        .find(|p| rut_driver::bundle::mount_name(&p.spec) == name)
        .unwrap_or_else(|| {
            panic!(
                "no pkg named `{name}` (mounted: {:?})",
                loaded.pkgs.iter().map(|p| p.spec.clone()).collect::<Vec<_>>()
            )
        })
}
