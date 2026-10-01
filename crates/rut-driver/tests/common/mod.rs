//! The url-dep tests' shared harness: the ready-map remote (ex-`Table`
//! — a `BTreeMap` fixture behind `std::future::ready`: no network, no
//! runtime dep) and the std-only `block_on` driver. The layer split
//! under test everywhere it is used: the CALL SITE owns HOW bytes
//! arrive (here: the map), the LOADER owns WHAT they are — the pin is
//! manifest law, verified at the mount door on every load.

use std::collections::BTreeMap;
use std::future::Future;
use std::task::{Context, Poll};

use rut_driver::{DepRemote, RemoteError};

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
