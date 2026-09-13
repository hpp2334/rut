//! Host-side runtime helpers (RFC 0022/0026/0028) — the embedder's half of
//! native modules. `rut-vm` deliberately knows nothing about any `std:`
//! module; the implementations an embedder installs live here.

pub mod logger;
pub mod math;
