//! The backend seam of the `web` host pkg: one trait, two bodies. The
//! 11 crossings' TRAP laws and the registry bindings are written once
//! (generic over [`DomBackend`]); the wasm32 half implements the trait
//! over web_sys, the host-lane twin implements it over a `HashMap`-backed
//! element tree with the same trap shapes (survey §5.4 — what keeps
//! `cargo test --workspace` a real gate for a web example).

/// One DOM/timer backend. Implementors are page-lifetime values (the
/// `'static` bound: the sink closures and the wasm thread-locals hold
/// the state for the page's life). Every `Err(String)` is a TRAP
/// message carried verbatim under the `web::<fn>: ` prefix (the
/// loud-fail law — never silently swallowed). The trap shapes live in
/// the backends:
///
/// - **unknown id** — `get` on a missing element;
/// - **kind mismatch** — an element-typed op on the wrong DOM type,
///   naming both sides (`boundary: got 'div' where HtmlInputElement
///   binds`, the expect_kind convention);
/// - **DOM exception** — `create`/`attr`/`append` rejections carrying
///   the exception's message.
pub trait DomBackend: 'static {
    /// The element handle boxed across the boundary — an
    /// `OpaqueBox<Self::El>` (RFC 0023): the payload is invisible to
    /// rut, `o is opaque` holds, and its release at rc 0 is
    /// deterministic (RFC 0016 §3).
    type El: Clone + 'static;

    /// `ui_get` — missing id is a trap (`no element '#x'`), a wiring
    /// bug is loud, never a rut-side optional.
    fn get(&self, id: &str) -> Result<Self::El, String>;

    /// `ui_create` — a DOM-rejected tag traps carrying the exception's
    /// message.
    fn create(&mut self, tag: &str) -> Result<Self::El, String>;

    /// `ui_set_text` — the text content replaces the children (the DOM
    /// `textContent` setter's own law).
    fn set_text(&mut self, el: &Self::El, text: &str) -> Result<(), String>;

    /// `ui_attr` — a DOM-rejected name traps carrying its message.
    fn attr(&mut self, el: &Self::El, name: &str, value: &str) -> Result<(), String>;

    /// `ui_append` — hierarchy rejections trap (e.g. appending an
    /// ancestor into its own descendant).
    fn append(&mut self, parent: &Self::El, child: &Self::El) -> Result<(), String>;

    /// `ui_remove` — `false` when not a child: a real DOM negative, not
    /// an error; no trap.
    fn remove(&mut self, parent: &Self::El, child: &Self::El) -> Result<bool, String>;

    /// `ui_clear` — one backend call (or loop), not app logic: the
    /// re-render reset.
    fn clear(&mut self, el: &Self::El) -> Result<(), String>;

    /// The event-detail source: the element's current value when it IS
    /// an input, `None` otherwise. The host attaches it to every event
    /// row the element's listeners fire — rut reads input state as
    /// event data, never by pulling the DOM.
    fn current_value(&self, el: &Self::El) -> Option<String>;

    /// `ui_set_input_value` — a non-input element traps naming both
    /// sides.
    fn set_input_value(&mut self, el: &Self::El, value: &str) -> Result<(), String>;

    /// `ui_listen` — the backend wires its own callback for `id` (the
    /// glue allocated it; rut's dispatch key). The twin records the row;
    /// web_sys registers a `Closure` the registry owns for the page's
    /// life (detaching is out of scope, a documented limitation).
    fn listen(&mut self, el: &Self::El, event: &str, id: i64) -> Result<(), String>;

    /// `tim_after` — the simulated-latency primitive. The host owns
    /// time (RFC 0018's own law): a real `setTimeout` on wasm, a scripted
    /// deadline queue the tests fire synchronously on the twin.
    fn after(&mut self, ms: i64, tag: &str) -> Result<(), String>;
}
