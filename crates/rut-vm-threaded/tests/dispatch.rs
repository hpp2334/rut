//! Measures the two dispatch styles on an identical op set. Run with
//! `cargo test -p rut-vm-threaded --release -- --nocapture`.

#[test]
fn dispatch_match_vs_threaded() {
    let ((m_min, m_med), (t_min, t_med)) =
        rut_vm_threaded::dispatch_bench::bench(10, 10_000_000);
    println!("match:    min {m_min:.2} ms   median {m_med:.2} ms");
    println!("threaded: min {t_min:.2} ms   median {t_med:.2} ms");
    println!("threaded/match (median): {:.3}", t_med / m_med);
}
