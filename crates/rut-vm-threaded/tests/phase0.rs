//! Phase 0/1 — validate the threaded mechanism on the real engine API.
//!
//! A synthetic `Machine` (this file is a separate crate, so it also proves
//! the generic handlers instantiate across the crate boundary). A long
//! chain runs on a deliberately tiny stack: if the backend is not doing
//! real tail calls, it overflows and the test aborts.

use rut_core::ops::Op;
use rut_vm_threaded::{Flow, Machine, ThreadOut};

const N: i64 = 5_000_000;

/// Loops on a single `Mov`; the engine's tag maps it to the threaded
/// handler, which tail-jumps forever until the counter says stop.
struct Counter {
    ops: Vec<Op>,
    acc: i64,
}

impl Machine for Counter {
    type Out = i64;
    type Err = ();

    fn pc(&self) -> u32 {
        0
    }
    fn set_pc(&mut self, _pc: u32) {}
    fn op_at(&self, pc: u32) -> &Op {
        &self.ops[pc as usize]
    }
    fn tick(&mut self, _pc: u32) -> Result<(), ()> {
        Ok(())
    }
    fn op_mov(&mut self, _pc: u32) -> Result<Flow<i64>, ()> {
        self.acc += 1;
        if self.acc >= N {
            Ok(Flow::Done(self.acc))
        } else {
            Ok(Flow::Next(0))
        }
    }
}

#[test]
fn long_chain_is_flat_and_correct() {
    // A tiny stack makes the tail-call guarantee observable: 5M ordinary
    // frames would need hundreds of MB.
    let handle = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let ops = vec![Op::Mov { dst: 0, src: 0 }];
            let mut c = Counter { ops, acc: 0 };
            let table = rut_vm_threaded::build_table::<Counter>();
            match rut_vm_threaded::run(&mut c, 0, &table).unwrap() {
                ThreadOut::Done(v) => v,
                ThreadOut::Bail => panic!("unexpected bail"),
            }
        })
        .expect("spawn");

    assert_eq!(handle.join().expect("join"), N);
}
