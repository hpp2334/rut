//! Host-side runtime helpers (RFC 0022/0026/0028) — the embedder's half of
//! native modules. `rut-vm` deliberately knows nothing about any package;
//! the implementations an embedder installs live here.

pub mod logger;
pub mod math;
pub mod nmap;
