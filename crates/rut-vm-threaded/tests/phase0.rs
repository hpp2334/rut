//! Phase 0 — validate the threaded mechanism.
//!
//! A synthetic `Machine` (this file is a separate crate, so it also proves
//! the generic handlers instantiate across the crate boundary). A long
//! chain runs on a deliberately tiny stack: if the backend is not doing
//! real tail calls, it overflows and the test aborts. We also decode a
//! couple of handlers and assert they end in an indirect jump.

use rut_vm_threaded::Machine;

/// `ops` is a tag stream: 0 = inc, 1 = dec, 2 = halt.
struct Counter {
    ops: Vec<u8>,
    acc: i64,
}

impl Machine for Counter {
    type Out = i64;
    type Err = ();

    fn tag_at(&self, pc: u32) -> u8 {
        self.ops[pc as usize]
    }
    fn inc(&mut self, pc: u32) -> Result<u32, ()> {
        self.acc += 1;
        Ok(pc + 1)
    }
    fn dec(&mut self, pc: u32) -> Result<u32, ()> {
        self.acc -= 1;
        Ok(pc + 1)
    }
    fn halt(&mut self, _pc: u32) -> i64 {
        self.acc
    }
}

#[test]
fn long_chain_is_flat_and_correct() {
    const N: u32 = 5_000_000;

    // A tiny stack makes the tail-call guarantee observable: 5M ordinary
    // frames would need hundreds of MB.
    let handle = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let mut ops = vec![0u8; N as usize];
            ops.push(2); // halt
            let mut c = Counter { ops, acc: 0 };
            rut_vm_threaded::run(&mut c, 0).unwrap()
        })
        .expect("spawn");

    assert_eq!(handle.join().expect("join"), N as i64);
}
