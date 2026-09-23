//! The StackTrace natives (RFC 0036, err-channel phase 2): capture is a
//! RAW frame walk (§2 — no names, no source, no symbolication); the
//! members symbolicate LAZILY, per index, against the loaded program
//! (§3's in-VM path — the interner for names, the position table for
//! line/col), degrading to `0`/pc-only text when stripped.
use super::*;
use crate::heap::TraceFrame;

impl Vm {
    /// `capture_stacktrace()` — the frame walk. The ACTIVE frame is
    /// innermost; each saved frame follows outward. O(depth) pushes,
    /// one heap cell: pay-per-capture, opt-in at the raise site.
    pub(super) fn nat_capture_trace(&mut self, dst: Option<Reg>) -> Result<(), Trap> {
        let mut frames = Vec::with_capacity(self.frames.len() + 1);
        // frame 0: the function executing the capture — its call site is
        // this very CallNat op (both engines keep cur_pc pinned to it)
        frames.push(TraceFrame { func: self.cur_func, pc: self.cur_pc });
        // every saved frame's pc is the RESUME point (call op + 1), so the
        // call site is one before it
        for f in self.frames.iter().rev() {
            frames.push(TraceFrame { func: f.func, pc: f.pc.saturating_sub(1) });
        }
        let s = self.heap.alloc_trace(frames)?;
        self.store_result(dst, s)?;
        Ok(())
    }

    /// The trace cell behind a member's receiver — every member checks
    /// loud: a non-trace receiver is wiring drift, not data.
    fn trace_cell(&self, recv: Option<Reg>) -> Result<&Vec<TraceFrame>, Trap> {
        let cell = cell_of(self.reg(recv.unwrap()));
        match &cell.data {
            CellData::Trace { frames } => Ok(frames),
            _ => Err(Trap::new(TrapKind::Invalid, "StackTrace member on a non-trace value")),
        }
    }

    /// Bounds-checked frame access — the index is a bug, not data: the
    /// loud-boundary convention (out-of-range traps, never nil).
    fn trace_frame(&self, recv: Option<Reg>, args: &[Reg]) -> Result<TraceFrame, Trap> {
        let frames = self.trace_cell(recv)?;
        let i = unsafe { self.reg(args[0]).i };
        if i < 0 || i as usize >= frames.len() {
            return Err(Trap::new(
                TrapKind::IndexOutOfBounds,
                format!("StackTrace index {i} out of range — len is {}", frames.len()),
            ));
        }
        Ok(frames[i as usize])
    }

    pub(super) fn nat_trace_len(&mut self, recv: Option<Reg>, dst: Option<Reg>) -> Result<(), Trap> {
        let n = self.trace_cell(recv)?.len() as i64;
        if let Some(d) = dst {
            self.cur_regs[d as usize] = Slot::int(n);
        }
        Ok(())
    }

    pub(super) fn nat_trace_name(&mut self, recv: Option<Reg>, args: &[Reg], dst: Option<Reg>) -> Result<(), Trap> {
        let f = self.trace_frame(recv, args)?;
        let name = self.prog.funcs[f.func as usize].name;
        let s = self.prog.interner.name(name).to_string();
        let cell = self.heap.alloc_str(s)?;
        self.store_result(dst, cell)?;
        Ok(())
    }

    /// The frame's call-site position — RFC 0036 §4: the position table
    /// is parallel to the span table (pc == entry index), so this is one
    /// direct read. `(0, 0)` when stripped (the driver never filled it).
    fn trace_frame_pos(&self, f: TraceFrame) -> (u32, u32) {
        let fc = &self.prog.funcs[f.func as usize];
        match fc.pos.get(f.pc as usize) {
            Some(&(line, col)) => (line, col),
            None => (0, 0),
        }
    }

    pub(super) fn nat_trace_line(&mut self, recv: Option<Reg>, args: &[Reg], dst: Option<Reg>) -> Result<(), Trap> {
        let f = self.trace_frame(recv, args)?;
        if let Some(d) = dst {
            self.cur_regs[d as usize] = Slot::int(self.trace_frame_pos(f).0 as i64);
        }
        Ok(())
    }

    pub(super) fn nat_trace_col(&mut self, recv: Option<Reg>, args: &[Reg], dst: Option<Reg>) -> Result<(), Trap> {
        let f = self.trace_frame(recv, args)?;
        if let Some(d) = dst {
            self.cur_regs[d as usize] = Slot::int(self.trace_frame_pos(f).1 as i64);
        }
        Ok(())
    }

    /// `render()` — the RFC 0036 symbolication string, one whole-trace
    /// pass, innermost first:
    ///   `at c (app_main:12:9)`          — names + positions restored
    ///   `at c (app_main #2 @ pc 41)`    — stripped: pc-only degradation
    pub(super) fn nat_trace_render(&mut self, recv: Option<Reg>, dst: Option<Reg>) -> Result<(), Trap> {
        let frames: Vec<TraceFrame> = self.trace_cell(recv)?.to_vec();
        let prog_name = self.prog.name.clone();
        let mut out = String::new();
        for (i, f) in frames.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            let name = self.prog.interner.name(self.prog.funcs[f.func as usize].name);
            let (line, col) = self.trace_frame_pos(*f);
            if line == 0 && col == 0 {
                out.push_str(&format!("at {name} ({prog_name} #{} @ pc {})", f.func, f.pc));
            } else {
                out.push_str(&format!("at {name} ({prog_name}:{line}:{col})"));
            }
        }
        let cell = self.heap.alloc_str(out)?;
        self.store_result(dst, cell)?;
        Ok(())
    }
}
