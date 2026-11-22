//! Host-side runtime helpers — the embedder's half of
//! native modules. `rut-vm` deliberately knows nothing about any package;
//! the implementations an embedder installs live here.

pub mod async_host;
pub mod bench_cross;
pub mod http;
// the wasm fetch lane (`pkg_fetch`): the browser/node door over the
// SAME seam. Target-AND-feature gated — the JS trio is a wasm32-only
// dependency and native builds (feature on or off) never compile the
// module, so the default builds stay clean like the reqwest lane's.
#[cfg(all(feature = "http-wasm", target_arch = "wasm32"))]
pub mod http_wasm;
pub mod logger;
pub mod math;
pub mod nmap;
pub mod strbuild;
