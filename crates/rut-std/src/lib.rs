//! Host-side runtime helpers — the embedder's half of
//! native modules. `rut-vm` deliberately knows nothing about any package;
//! the implementations an embedder installs live here.

pub mod async_host;
pub mod bench_cross;
#[cfg(feature = "http")]
pub mod http;
pub mod logger;
pub mod math;
pub mod nmap;
pub mod strbuild;
