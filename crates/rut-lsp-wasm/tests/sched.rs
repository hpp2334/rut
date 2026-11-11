//! The clock + defer + ReloadDeps/Wake lane's gate — the walk's SCHEDULING
//! POLICY, moved from the JS `depEngine` into the actor, proven
//! through the public ABI over `spawn_native` with the native fake
//! clock of D8: the env STORES the wake source `Rc` and RECORDS every
//! arm; the TEST fires the stored `Rc` by hand. Deterministic, zero
//! timers, zero cfg — the whole discipline observable from the posts.
//!
//! The world is an in-memory file map (the walk's whole input): a
//! root is its `rut.jsonc` + `src/mod.rut`, so which manifest got
//! read names which walk ran. The machinery laws (the neutral queue,
//! the cancel door) live in `event_loop`'s own unit gate; this file
//! drives the LANE — `Cmd::ReloadDeps`'s composition of those doors.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use futures::executor::LocalPool;
use futures::FutureExt;
use rut_lsp_wasm::env::{Io, WakeFn};
use rut_lsp_wasm::RutLsp;

/// the walk lane's debounce window — `event_loop::DEBOUNCE_MS`
/// (pub(crate); the gate spells the same 200 the JS engine spelled)
const WINDOW_MS: u32 = 200;

/// the in-memory host env: a file map + dir set (the walk's whole
/// world), the clock's recordings, and the stored wake source.
/// `clock: false` spells the capability-less env — D3's degrade
/// world (an env whose `arm_wake` answers `false`).
#[derive(Default)]
struct MemIo {
    files: HashMap<String, Vec<u8>>,
    dirs: HashSet<String>,
    clock: bool,
    /// a path whose read PANICS — the case-8 world (one turn drops,
    /// the loop lives)
    panic_at: Option<String>,
    reads: RefCell<Vec<String>>,
    arms: RefCell<Vec<u32>>,
    wake: RefCell<Option<WakeFn>>,
    notes: RefCell<Vec<String>>,
}

impl MemIo {
    /// lay one file (and every ancestor dir) into the world
    fn put(&mut self, path: &str, text: &str) {
        let mut dir = path;
        while let Some(i) = dir.rfind('/') {
            dir = &dir[..i];
            if dir.is_empty() {
                break;
            }
            self.dirs.insert(dir.to_string());
        }
        self.files
            .insert(path.to_string(), text.as_bytes().to_vec());
    }

    /// fire the stored wake source by hand — the test IS the timer
    fn fire(&self) {
        let wake = self
            .wake
            .borrow()
            .clone()
            .expect("the wake source installed at set_env");
        wake();
    }

    /// which roots walked — the manifest reads, in order
    fn manifests(&self) -> Vec<String> {
        self.reads
            .borrow()
            .iter()
            .filter(|p| p.ends_with("/rut.jsonc"))
            .cloned()
            .collect()
    }
}

impl Io for MemIo {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        if self.panic_at.as_deref() == Some(path) {
            panic!("the fake clock's scripted panic at {path}");
        }
        self.reads.borrow_mut().push(path.to_string());
        self.files.get(path).cloned()
    }

    fn read_dir(&self, _path: &str) -> Option<Vec<String>> {
        None // the manifest world never asks this spelling
    }

    fn is_dir(&self, path: &str) -> bool {
        self.dirs.contains(path)
    }

    fn notify(&self, message: &str, _is_error: bool) {
        self.notes.borrow_mut().push(message.to_string());
    }

    fn set_wake(&self, wake: WakeFn) {
        *self.wake.borrow_mut() = Some(wake);
    }

    fn arm_wake(&self, ms: u32) -> bool {
        if !self.clock {
            return false; // the host object carried no wakeAfter
        }
        self.arms.borrow_mut().push(ms);
        true
    }
}

// ---- the harness ----

/// one root's world: `rut.jsonc` + `src/mod.rut`
fn root(io: &mut MemIo, path: &str, name: &str) {
    io.put(
        &format!("{path}/rut.jsonc"),
        &format!("{{\n  \"name\": \"{name}\"\n}}\n"),
    );
    io.put(
        &format!("{path}/src/mod.rut"),
        "pub fn tag() -> i32 { return 1; }\n",
    );
}

/// boot a fresh core with `io` injected through the same door the wasm
/// bridge posts its `JsEnv` — the clock rides `io.clock`
fn boot(io: MemIo) -> (RutLsp, LocalPool, Rc<MemIo>) {
    let (lsp, mut pool) = RutLsp::spawn_native();
    let io = Rc::new(io);
    pool.run_until(lsp.set_env_io(io.clone()));
    (lsp, pool, io)
}

/// the roots array as the ABI's JSON spelling
fn rjson(roots: &[&str]) -> String {
    serde_json::to_string(&roots.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
}

/// parse an export's JSON answer
fn take(json: String) -> serde_json::Value {
    serde_json::from_str(&json).expect("the envelope is valid JSON")
}

/// post a reload and let the loop take its turn: the message lands,
/// the walk DEFERS (the future stays pending), the window arms. Binds
/// `$name` (a `Pin<&mut …>`) at the CALL site — poll it with
/// `as_mut()`, resolve it with `pool.run_until($name)`.
macro_rules! reload_pending {
    ($name:ident = $lsp:expr, $pool:expr, [$($root:expr),* $(,)?]) => {
        let mut $name = std::pin::pin!($lsp.reload_deps(rjson(&[$($root),*])));
        assert!(
            $name.as_mut().now_or_never().is_none(),
            "the reload stays pending under the window"
        );
        $pool.run_until_stalled(); // the loop's turn: defer + arm, no walk
    };
}

// ---- the lane ----

#[test]
fn reload_arms_the_window_and_the_oneshot_carries_walkout() {
    let mut io = MemIo {
        clock: true,
        ..Default::default()
    };
    root(&mut io, "/ws/a", "appa");
    let (lsp, mut pool, io) = boot(io);

    reload_pending!(reload = lsp, pool, ["/ws/a"]);
    assert!(
        (&mut reload).now_or_never().is_none(),
        "the oneshot stays unanswered before the tick"
    );
    assert_eq!(*io.arms.borrow(), vec![WINDOW_MS], "the window armed");
    assert!(io.manifests().is_empty(), "NO walk ran before the tick");

    // the tick: ONE walk, and the reload's own oneshot carries the
    // WalkOut envelope (D2 — the answer rides the RPC's reply channel)
    io.fire();
    let out = take(pool.run_until(reload));
    assert_eq!(out["missing"].as_array().unwrap().len(), 0, "{out}");
    assert_eq!(io.manifests(), vec!["/ws/a/rut.jsonc".to_string()]);
}

#[test]
fn a_burst_of_reloads_coalesces_to_one_walk_over_the_freshest_roots() {
    let mut io = MemIo {
        clock: true,
        ..Default::default()
    };
    root(&mut io, "/ws/old", "olda");
    root(&mut io, "/ws/new", "newa");
    let (lsp, mut pool, io) = boot(io);

    let mut s1 = std::pin::pin!(lsp.reload_deps(rjson(&["/ws/old"])));
    let mut s2 = std::pin::pin!(lsp.reload_deps(rjson(&["/ws/new"])));
    assert!(
        s1.as_mut().now_or_never().is_none() && s2.as_mut().now_or_never().is_none(),
        "both reloads post"
    );
    pool.run_until_stalled();
    // ONE window for the whole burst, no walk yet
    assert_eq!(*io.arms.borrow(), vec![WINDOW_MS]);
    assert!(io.manifests().is_empty());
    // the FIRST reload is superseded — merged, answered Done (`null` on
    // the wire): the LAST reload's promise carries the world
    assert_eq!(
        take(s1.now_or_never().expect("the superseded reload answers")),
        serde_json::Value::Null
    );

    io.fire();
    let out = take(pool.run_until(s2));
    assert_eq!(out["missing"].as_array().unwrap().len(), 0, "{out}");
    // exactly ONE walk, over the FRESHEST roots — never the stale ones
    assert_eq!(io.manifests(), vec!["/ws/new/rut.jsonc".to_string()]);
}

#[test]
fn a_reload_after_the_window_fired_sends_no_done_and_gets_its_own_walk() {
    let mut io = MemIo {
        clock: true,
        ..Default::default()
    };
    root(&mut io, "/ws/one", "onea");
    root(&mut io, "/ws/two", "twoa");
    let (lsp, mut pool, io) = boot(io);

    // the first window fires: the walk runs, its own reload answers WalkOut
    reload_pending!(s1 = lsp, pool, ["/ws/one"]);
    io.fire();
    let out1 = take(pool.run_until(s1));
    assert_eq!(out1["missing"].as_array().unwrap().len(), 0, "{out1}");

    // the LATER reload: the lane's remembered handle is STALE (its walk
    // already fired and answered its own reload), so cancel answers None
    // and NO Done is sent — the earlier promise already carried its
    // WalkOut. The reload arms its OWN window (never starves) and walks.
    reload_pending!(s2 = lsp, pool, ["/ws/two"]);
    assert_eq!(*io.arms.borrow(), vec![WINDOW_MS, WINDOW_MS]);
    io.fire();
    let out2 = take(pool.run_until(s2));
    assert_eq!(out2["missing"].as_array().unwrap().len(), 0, "{out2}");
    assert_eq!(
        io.manifests(),
        vec![
            "/ws/one/rut.jsonc".to_string(),
            "/ws/two/rut.jsonc".to_string(),
        ]
    );
}

#[test]
fn a_clockless_env_reloads_eagerly_in_turn() {
    // D3 through the ABI: `clock: false` — arm_wake answers false, so
    // the walk runs inside the ReloadDeps turn itself (the spent-sentinel
    // path); the biz observable is identical, no window ever arms
    let mut io = MemIo::default(); // clock: false
    root(&mut io, "/ws/eager", "eagera");
    let (lsp, mut pool, io) = boot(io);

    let mut reload = std::pin::pin!(lsp.reload_deps(rjson(&["/ws/eager"])));
    assert!(reload.as_mut().now_or_never().is_none()); // only unpolled
    let out = take(pool.run_until(reload));
    assert_eq!(out["missing"].as_array().unwrap().len(), 0, "{out}");
    assert_eq!(io.manifests(), vec!["/ws/eager/rut.jsonc".to_string()]);
    assert!(io.arms.borrow().is_empty(), "no clock, no arms");
}

#[test]
fn an_envless_reload_answers_the_error_envelope_eagerly() {
    // no env at all: the walk cannot read anything — the loud error
    // envelope, in-turn (the eager degrade again, the no-env body)
    let (lsp, mut pool) = RutLsp::spawn_native();
    let out = take(pool.run_until(lsp.reload_deps(rjson(&["/ws/nowhere"]))));
    assert!(
        out["error"]
            .as_str()
            .unwrap()
            .contains("no host environment"),
        "{out}"
    );
}

#[test]
fn resolve_deps_and_reload_deps_answer_the_same_envelope() {
    // D9 through the ABI: the eager manual door and the deferred door
    // run ONE body — identical bytes for identical worlds
    let mk = || {
        let mut io = MemIo {
            clock: true,
            ..Default::default()
        };
        root(&mut io, "/ws/par", "para");
        io
    };
    let (lsp1, mut pool1, _io1) = boot(mk());
    let eager = pool1.run_until(lsp1.resolve_deps(rjson(&["/ws/par"])));

    let (lsp2, mut pool2, io2) = boot(mk());
    reload_pending!(reload = lsp2, pool2, ["/ws/par"]);
    io2.fire();
    let deferred = pool2.run_until(reload);
    assert_eq!(eager, deferred, "one walk body, two doors");
}

#[test]
fn env_reinjection_installs_the_new_wake_and_a_stale_wake_is_harmless() {
    // case 7: replace-on-reinject gives the NEW env its set_wake, and
    // a STALE timer's Wake (the old env's stored wake) drains CURRENT
    // state — harmless, exactly the design's word
    let mut io1 = MemIo {
        clock: true,
        ..Default::default()
    };
    root(&mut io1, "/ws/r1", "r1a");
    let mut io2 = MemIo {
        clock: true,
        ..Default::default()
    };
    root(&mut io2, "/ws/r2", "r2a");
    let io1 = Rc::new(io1);
    let io2 = Rc::new(io2);

    let (lsp, mut pool) = RutLsp::spawn_native();
    pool.run_until(lsp.set_env_io(io1.clone()));
    pool.run_until(lsp.set_env_io(io2.clone()));
    assert!(
        io2.wake.borrow().is_some(),
        "the NEW env's set_wake ran at reinject"
    );

    // a reload deferred under env2's clock — but the STALE env1 timer
    // fires: its wake posts the same inbox; the current queue drains
    reload_pending!(reload = lsp, pool, ["/ws/r2"]);
    assert_eq!(*io2.arms.borrow(), vec![WINDOW_MS]);
    io1.fire(); // the OLD env's stored wake
    let out = take(pool.run_until(reload));
    assert_eq!(out["missing"].as_array().unwrap().len(), 0, "{out}");
    assert_eq!(io2.manifests(), vec!["/ws/r2/rut.jsonc".to_string()]);
}

#[test]
fn two_instances_each_wake_their_own_loop() {
    // case 9: each wake closure posts into its OWN captured inbox —
    // instance 1's tick never answers instance 2's reload
    let mk = |path: &str, name: &str| {
        let mut io = MemIo {
            clock: true,
            ..Default::default()
        };
        root(&mut io, path, name);
        io
    };
    let (lsp1, mut pool1, io1) = boot(mk("/ws/i1", "i1a"));
    let (lsp2, mut pool2, io2) = boot(mk("/ws/i2", "i2a"));

    reload_pending!(s1 = lsp1, pool1, ["/ws/i1"]);
    reload_pending!(s2 = lsp2, pool2, ["/ws/i2"]);

    io1.fire(); // only instance 1's reload ran
    let out1 = take(pool1.run_until(s1));
    assert_eq!(out1["missing"].as_array().unwrap().len(), 0, "{out1}");
    assert!(
        s2.as_mut().now_or_never().is_none(),
        "the other loop's reload stays pending"
    );

    io2.fire();
    let out2 = take(pool2.run_until(s2));
    assert_eq!(out2["missing"].as_array().unwrap().len(), 0, "{out2}");
}

#[test]
fn a_panicking_walk_drops_one_turn_and_the_loop_lives() {
    // case 8: the walk panics inside the tick — catch_unwind drops ONE
    // turn (the deferred tx drops in the unwind, so the awaiting reload
    // answers the canceled-oneshot error envelope — today's
    // panicking-handler shape), and the NEXT reload re-arms and walks
    let mut io = MemIo {
        clock: true,
        ..Default::default()
    };
    root(&mut io, "/ws/boom", "booma");
    root(&mut io, "/ws/fine", "finea");
    io.panic_at = Some("/ws/boom/rut.jsonc".to_string());
    let (lsp, mut pool, io) = boot(io);

    // keep the scripted panic out of the gate's output — the hook is
    // process-global and nextest gives every test its own process
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    reload_pending!(s1 = lsp, pool, ["/ws/boom"]);
    io.fire();
    let out = take(pool.run_until(s1));
    assert!(
        out["error"].as_str().unwrap().contains("loop is gone"),
        "the dropped tx answers the bridge's dead-loop envelope: {out}"
    );

    // the loop LIVES: the next reload re-arms and walks the clean world
    reload_pending!(s2 = lsp, pool, ["/ws/fine"]);
    assert_eq!(*io.arms.borrow(), vec![WINDOW_MS, WINDOW_MS]);
    io.fire();
    let out2 = take(pool.run_until(s2));
    assert_eq!(out2["missing"].as_array().unwrap().len(), 0, "{out2}");
    assert!(io.manifests().contains(&"/ws/fine/rut.jsonc".to_string()));

    std::panic::set_hook(hook);
}

#[test]
fn resolve_deps_walks_eagerly_beside_a_pending_reload() {
    // case 10: the eager manual door walks NOW — no tick needed; the
    // later tick walks the (same, idempotent) world for the reload's own
    // oneshot — both promises carry WalkOut, the world stays whole
    let mut io = MemIo {
        clock: true,
        ..Default::default()
    };
    root(&mut io, "/ws/par", "para");
    let (lsp, mut pool, io) = boot(io);

    reload_pending!(reload = lsp, pool, ["/ws/par"]);
    let eager = take(pool.run_until(lsp.resolve_deps(rjson(&["/ws/par"]))));
    assert_eq!(eager["missing"].as_array().unwrap().len(), 0, "{eager}");
    assert_eq!(io.manifests(), vec!["/ws/par/rut.jsonc".to_string()]);

    io.fire();
    let out = take(pool.run_until(reload));
    assert_eq!(out, eager, "the idempotent re-walk answers identically");
}
