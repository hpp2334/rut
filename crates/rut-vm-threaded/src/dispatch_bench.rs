//! A self-contained dispatch microbenchmark: one tiny integer loop run two
//! ways — a `match` loop and a `become`-threaded loop — with the **same** op
//! bodies and the **same** state-threading shape (code, tags, regs, pc
//! passed as arguments; no shared `self` memory).
//!
//! The point is to isolate *dispatch style* from the VM's memory layout and
//! answer "can computed goto beat the match here?" before wiring anything.
//!
//! The loop is `s = sum(i for i in 0..n)`:
//!   regs: [i, s, t, n, one]
//!   pc 0: s += i        (dst=1, a=0)
//!   pc 1: t = i < n     (dst=2, a=0, b=3)
//!   pc 2: if !t goto 5  (c=2, target=5)
//!   pc 3: i += 1        (dst=0, a=4)
//!   pc 4: goto 1
//!   pc 5: return s

use std::time::Instant;

pub const K_ADD: u8 = 0;
pub const K_LT: u8 = 1;
pub const K_JMP: u8 = 2;
pub const K_BR: u8 = 3;
pub const K_RET: u8 = 4;

pub struct Code {
    pub tags: Vec<u8>,
    pub ops: Vec<[u32; 4]>,
}

pub fn make_loop() -> Code {
    let mut tags = vec![];
    let mut ops = vec![];
    let mut push = |t: u8, o: [u32; 4]| {
        tags.push(t);
        ops.push(o);
    };
    push(K_LT, [2, 0, 3, 0]); // t = i < n
    push(K_BR, [2, 5, 0, 0]); // if !t goto 5
    push(K_ADD, [1, 0, 0, 0]); // s += i
    push(K_ADD, [0, 4, 0, 0]); // i += 1
    push(K_JMP, [0, 0, 0, 0]); // goto 0
    push(K_RET, [1, 0, 0, 0]); // return s
    Code { tags, ops }
}

pub fn init_regs(n: i64) -> [i64; 5] {
    [0, 0, 0, n, 1]
}

pub fn expected(n: i64) -> i64 {
    (0..n).fold(0i64, |s, i| s.wrapping_add(i))
}

// ---------------------------------------------------------------- match loop

pub fn run_match(code: &Code, regs: &mut [i64; 5], pc0: u32) -> i64 {
    let mut pc = pc0 as usize;
    loop {
        match code.tags[pc] {
            K_ADD => {
                let o = code.ops[pc];
                regs[o[0] as usize] = regs[o[0] as usize].wrapping_add(regs[o[1] as usize]);
                pc += 1;
            }
            K_LT => {
                let o = code.ops[pc];
                regs[o[0] as usize] = (regs[o[1] as usize] < regs[o[2] as usize]) as i64;
                pc += 1;
            }
            K_BR => {
                let o = code.ops[pc];
                pc = if regs[o[0] as usize] == 0 {
                    o[1] as usize
                } else {
                    pc + 1
                };
            }
            K_JMP => pc = code.ops[pc][0] as usize,
            _ => return regs[code.ops[pc][0] as usize],
        }
    }
}

// ------------------------------------------------------------- threaded loop

#[cfg(rut_threaded)]
mod threaded {
    use super::*;

    pub type Handler =
        extern "rust-preserve-none" fn(*const (), *const Code, *mut i64, u32) -> Result<i64, ()>;

    macro_rules! next {
        ($table:expr, $c:expr, $regs:expr, $n:expr) => {{
            let tag = unsafe { *$c.tags.get_unchecked($n as usize) } as usize;
            let h = unsafe { (*(($table) as *const [Handler; 5]))[tag] };
            become h($table, $c as *const Code, $regs, $n)
        }};
    }

    extern "rust-preserve-none" fn h_add(
        table: *const (),
        code: *const Code,
        regs: *mut i64,
        pc: u32,
    ) -> Result<i64, ()> {
        let c = unsafe { &*code };
        let o = c.ops[pc as usize];
        unsafe {
            let d = o[0] as usize;
            *regs.add(d) = (*regs.add(d)).wrapping_add(*regs.add(o[1] as usize));
        }
        next!(table, c, regs, pc + 1)
    }

    extern "rust-preserve-none" fn h_lt(
        table: *const (),
        code: *const Code,
        regs: *mut i64,
        pc: u32,
    ) -> Result<i64, ()> {
        let c = unsafe { &*code };
        let o = c.ops[pc as usize];
        unsafe {
            *regs.add(o[0] as usize) =
                ((*regs.add(o[1] as usize)) < (*regs.add(o[2] as usize))) as i64;
        }
        next!(table, c, regs, pc + 1)
    }

    extern "rust-preserve-none" fn h_jmp(
        table: *const (),
        code: *const Code,
        regs: *mut i64,
        pc: u32,
    ) -> Result<i64, ()> {
        let c = unsafe { &*code };
        let n = c.ops[pc as usize][0];
        next!(table, c, regs, n)
    }

    extern "rust-preserve-none" fn h_br(
        table: *const (),
        code: *const Code,
        regs: *mut i64,
        pc: u32,
    ) -> Result<i64, ()> {
        let c = unsafe { &*code };
        let o = c.ops[pc as usize];
        let n = if unsafe { *regs.add(o[0] as usize) } == 0 {
            o[1]
        } else {
            pc + 1
        };
        next!(table, c, regs, n)
    }

    extern "rust-preserve-none" fn h_ret(
        _table: *const (),
        code: *const Code,
        regs: *mut i64,
        pc: u32,
    ) -> Result<i64, ()> {
        let c = unsafe { &*code };
        let o = c.ops[pc as usize];
        Ok(unsafe { *regs.add(o[0] as usize) })
    }

    static TABLE: [Handler; 5] = [h_add, h_lt, h_jmp, h_br, h_ret];

    pub fn run(code: &Code, regs: &mut [i64; 5], pc0: u32) -> i64 {
        let tp = &TABLE as *const [Handler; 5] as *const ();
        let cp = code as *const Code;
        let rp = regs.as_mut_ptr();
        let tag = code.tags[pc0 as usize] as usize;
        let h = TABLE[tag];
        match h(tp, cp, rp, pc0) {
            Ok(v) => v,
            Err(()) => unreachable!(),
        }
    }
}

#[cfg(rut_threaded)]
pub fn run_threaded(code: &Code, regs: &mut [i64; 5], pc0: u32) -> i64 {
    threaded::run(code, regs, pc0)
}

#[cfg(not(rut_threaded))]
pub fn run_threaded(code: &Code, regs: &mut [i64; 5], pc0: u32) -> i64 {
    run_match(code, regs, pc0)
}

// --------------------------------------------------------------------- bench

/// Returns `(match_min, match_median)` and `(threaded_min, threaded_median)`
/// in milliseconds.
pub fn bench(iters: usize, n: i64) -> ((f64, f64), (f64, f64)) {
    let code = make_loop();
    let mut m = Vec::new();
    let mut t = Vec::new();
    for _ in 0..iters {
        let mut r = init_regs(n);
        let s = Instant::now();
        let v = run_match(&code, &mut r, 0);
        m.push(s.elapsed().as_secs_f64() * 1e3);
        assert_eq!(v, expected(n));

        let mut r = init_regs(n);
        let s = Instant::now();
        let v = run_threaded(&code, &mut r, 0);
        t.push(s.elapsed().as_secs_f64() * 1e3);
        assert_eq!(v, expected(n));
    }
    m.sort_by(|a, b| a.partial_cmp(b).unwrap());
    t.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = |v: &[f64]| (v[0], v[v.len() / 2]);
    (med(&m), med(&t))
}
