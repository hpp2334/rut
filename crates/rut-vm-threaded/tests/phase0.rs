//! Validate the threaded mechanism on the real engine API.
//!
//! A synthetic `Machine` (this file is a separate crate, so it also proves
//! the generic handlers instantiate across the crate boundary). A long
//! chain runs on a deliberately tiny stack: if the backend is not doing
//! real tail calls, it overflows and the test aborts.

use rut_core::ops::Op;
use rut_vm_threaded::{Flow, Machine, ThreadOut, ThreadState, T_MOV};

const N: i64 = 5_000_000;

/// Loops on a single `Mov`; the engine's tag maps it to the threaded
/// handler, which tail-jumps forever until the counter says stop.
struct Counter {
    ops: Vec<Op>,
    tags: Vec<u8>,
    regs: Vec<i64>,
    acc: i64,
}

impl Machine for Counter {
    type Out = i64;
    type Err = ();
    type Word = i64;

    fn thread_state(&mut self) -> ThreadState<i64> {
        ThreadState {
            code: self.ops.as_ptr(),
            tags: self.tags.as_ptr(),
            regs: self.regs.as_mut_ptr(),
            pc: 0,
            fuel: -1,
            fuel_used: 0,
        }
    }
    fn sync(&mut self, _pc: u32, _fuel: i64, _used: u64) {}
    fn sync_fuel(&mut self, _fuel: i64, _used: u64) {} // root-`Ret` writeback
    fn park(&mut self, _pc: u32, _used: u64) {}

    fn op_mov(&mut self, _op: &Op, regs: *mut i64, pc: u32) -> Result<Flow<i64>, ()> {
        self.acc += 1;
        unsafe { *regs.add(0) = self.acc };
        if self.acc >= N {
            Ok(Flow::Done(self.acc))
        } else {
            Ok(Flow::Next(pc))
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
            let mut c = Counter {
                ops: vec![Op::Mov { dst: 0, src: 0 }],
                tags: vec![T_MOV],
                regs: vec![0],
                acc: 0,
            };
            let table = rut_vm_threaded::build_table::<Counter>();
            match rut_vm_threaded::run(&mut c, 0, &table).unwrap() {
                ThreadOut::Done(v) => v,
                ThreadOut::Bail => panic!("unexpected bail"),
            }
        })
        .expect("spawn");

    assert_eq!(handle.join().expect("join"), N);
}
