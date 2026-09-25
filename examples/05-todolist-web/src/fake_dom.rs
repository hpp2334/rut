//! The fake-DOM host-lane twin (survey §5.4): the SAME `web.d.rut`
//! surface bound to a `HashMap`-backed element tree implementing the
//! §4.1 semantics — parent/child, attributes, input value, a scripted
//! timer queue the tests fire synchronously, and the same trap shapes
//! (unknown id, kind mismatch, DOM exception carried). This is what
//! keeps `cargo test --workspace` a real gate for a web example, and
//! what phase 2's end-to-end CRUD session runs on.
//!
//! Honesty notes: elements live for the page's life (no GC of detached
//! nodes); `textContent` replaces children; `append` moves an element
//! that already has a parent, as the DOM does; timers are a virtual
//! clock — `advance(ms)` fires due tags in (deadline, seq) order, no
//! sleeping, fully deterministic.

use std::collections::HashMap;

use rut_vm::{OpaqueRef, Trap};

use crate::backend::DomBackend;
use crate::state::EvSink;

/// The boxed element handle rut holds: the twin's node id plus the tag
/// the kind-mismatch traps name.
#[derive(Clone)]
pub struct El {
    pub id: u32,
    pub tag: String,
}

/// One element in the twin's tree.
pub struct FakeNode {
    pub tag: String,
    pub attrs: HashMap<String, String>,
    pub text: Option<String>,
    pub children: Vec<u32>,
    pub parent: Option<u32>,
}

impl FakeNode {
    fn new(tag: String) -> FakeNode {
        FakeNode { tag, attrs: HashMap::new(), text: None, children: Vec::new(), parent: None }
    }

    fn id_attr(&self) -> Option<&str> {
        self.attrs.get("id").map(String::as_str)
    }
}

struct TimerRec {
    at: i64,
    seq: u64,
    tag: String,
}

impl Clone for TimerRec {
    fn clone(&self) -> TimerRec {
        TimerRec { at: self.at, seq: self.seq, tag: self.tag.clone() }
    }
}

/// The twin backend.
pub struct FakeDom {
    sink: EvSink,
    now: i64,
    next_id: u32,
    next_seq: u64,
    nodes: HashMap<u32, FakeNode>,
    roots: Vec<u32>,
    timers: Vec<TimerRec>,
}

impl FakeDom {
    pub fn new(sink: EvSink) -> FakeDom {
        FakeDom {
            sink,
            now: 0,
            next_id: 1,
            next_seq: 1,
            nodes: HashMap::new(),
            roots: Vec::new(),
            timers: Vec::new(),
        }
    }

    /// The static page: seed one element under the virtual document —
    /// the twin's `index.html`. The page's ids are the host's own
    /// contract; `ui_get` searches from the seeded roots.
    pub fn seed_page(&mut self, tag: &str, id: &str) -> El {
        let el = self.fresh_node(tag);
        self.nodes.get_mut(&el.id).unwrap().attrs.insert("id".to_string(), id.to_string());
        self.roots.push(el.id);
        el
    }

    fn fresh_node(&mut self, tag: &str) -> El {
        let id = self.next_id;
        self.next_id += 1;
        self.nodes.insert(id, FakeNode::new(tag.to_string()));
        El { id, tag: tag.to_string() }
    }

    // ---- the test drive (the twin's Closure stand-ins) ----

    /// Fire a DOM listener now: one event row through the sink, the same
    /// entry a web_sys `Closure` takes on the page.
    pub fn fire(&self, listener: i64) {
        (self.sink.clone())(crate::state::Ev::Dom(listener));
    }

    /// Advance the virtual clock; every due timer is collected in
    /// (deadline, sequence) order — the `setTimeout` stand-in. The
    /// events come back to the caller instead of firing through the
    /// sink: the caller holds the state borrow, and the sink needs it.
    pub fn advance(&mut self, ms: i64) -> Vec<crate::state::Ev> {
        self.now += ms;
        let mut due: Vec<TimerRec> =
            self.timers.iter().filter(|t| t.at <= self.now).cloned().collect();
        self.timers.retain(|t| t.at > self.now);
        due.sort_by_key(|t| (t.at, t.seq));
        due.into_iter().map(|t| crate::state::Ev::Timer(t.tag)).collect()
    }

    /// `(deadline, tag)` rows still pending — test assertions only.
    pub fn pending_timers(&self) -> Vec<(i64, String)> {
        let mut v: Vec<(i64, String)> =
            self.timers.iter().map(|t| (t.at, t.tag.clone())).collect();
        v.sort();
        v
    }

    // ---- reads the tests make through real handles ----

    fn view<T>(&self, el: &OpaqueRef, f: impl FnOnce(&FakeNode) -> T) -> Result<T, Trap> {
        let b = rut_vm::Opaque::<El>::from_handle(el)?;
        match b.with(|el| self.nodes.get(&el.id)) {
            Ok(Some(node)) => Ok(f(node)),
            Ok(None) => Err(rut_vm::Trap::new(rut_vm::TrapKind::Invalid, "no such node")),
            Err(e) => Err(e),
        }
    }    /// The element's text content, through a real rut-held handle.
    pub fn text_of(&self, el: &OpaqueRef) -> Result<String, Trap> {
        self.view(el, |n| n.text.clone().unwrap_or_default())
    }

    /// The element's tag, through a real rut-held handle.
    pub fn tag_of(&self, el: &OpaqueRef) -> Result<String, Trap> {
        self.view(el, |n| n.tag.clone())
    }

    /// One attribute's value.
    pub fn attr_of(&self, el: &OpaqueRef, name: &str) -> Result<String, Trap> {
        self.view(el, |n| n.attrs.get(name).cloned().unwrap_or_default())
    }

    /// The children's tags, in order.
    pub fn children_of(&self, el: &OpaqueRef) -> Result<Vec<String>, Trap> {
        self.view(el, |n| {
            n.children
                .iter()
                .filter_map(|id| self.nodes.get(id).map(|c| c.tag.clone()))
                .collect()
        })
    }

    /// The children's text contents, in order (the log rows).
    pub fn child_texts_of(&self, el: &OpaqueRef) -> Result<Vec<String>, Trap> {
        self.view(el, |n| {
            n.children
                .iter()
                .filter_map(|id| self.nodes.get(id).and_then(|c| c.text.clone()))
                .collect()
        })
    }

    // ---- reads the phase-2 tests make by ELEMENT ID -------------------
    // The app program is clean (no probe entries), so the twin's
    // test-drive readers work by the ids the page itself set — the same
    // contract `ui_get` searches under. The snapshot is a clone: no
    // rut handle is involved at all.

    /// The tree search `get` runs, yielding a node id.
    fn find_id(&self, id: &str) -> Option<u32> {
        let mut stack: Vec<u32> = self.roots.clone();
        while let Some(id_num) = stack.pop() {
            let node = self.nodes.get(&id_num)?;
            if node.id_attr() == Some(id) {
                return Some(id_num);
            }
            stack.extend(node.children.iter().rev().copied());
        }
        None
    }

    /// The user typing: the field's value changes; the next `fire` of
    /// its listener carries it as the event detail (the event-carried
    /// law — the twin never hands rut a read).
    pub fn set_value_by_id(&mut self, id: &str, value: &str) -> bool {
        match self.find_id(id) {
            Some(n) => {
                self.nodes
                    .get_mut(&n)
                    .expect("find_id yielded a live node")
                    .attrs
                    .insert("value".to_string(), value.to_string());
                true
            }
            None => false,
        }
    }

    /// One element's snapshot: tag, text, attributes, and the direct
    /// children's tags and texts, in order.
    pub fn snapshot_by_id(&self, id: &str) -> Option<Snapshot> {
        let n = self.nodes.get(&self.find_id(id)?)?;
        Some(Snapshot {
            tag: n.tag.clone(),
            text: n.text.clone(),
            attrs: n.attrs.clone(),
            child_tags: n.children.iter().filter_map(|c| self.nodes.get(c).map(|c| c.tag.clone())).collect(),
            child_texts: n
                .children
                .iter()
                .filter_map(|c| self.nodes.get(c).and_then(|c| c.text.clone()))
                .collect(),
        })
    }
}

/// The phase-2 test reader's answer — one element's shape, cloned.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub tag: String,
    pub text: Option<String>,
    pub attrs: HashMap<String, String>,
    pub child_tags: Vec<String>,
    pub child_texts: Vec<String>,
}

fn valid_tag(tag: &str) -> bool {
    let mut chars = tag.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn valid_attr_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
}

fn is_ancestor(nodes: &HashMap<u32, FakeNode>, ancestor: u32, of: u32) -> bool {
    let mut cur = nodes.get(&of).and_then(|n| n.parent);
    while let Some(p) = cur {
        if p == ancestor {
            return true;
        }
        cur = nodes.get(&p).and_then(|n| n.parent);
    }
    false
}

impl DomBackend for FakeDom {
    type El = El;

    fn get(&self, id: &str) -> Result<El, String> {
        // getElementById's law: the tree, not the bag — search the
        // seeded document from the roots down.
        let mut stack: Vec<u32> = self.roots.clone();
        while let Some(id_num) = stack.pop() {
            let node = self.nodes.get(&id_num).ok_or("node table drift")?;
            if node.id_attr() == Some(id) {
                return Ok(El { id: id_num, tag: node.tag.clone() });
            }
            stack.extend(node.children.iter().rev().copied());
        }
        Err(format!("no element '#{id}'"))
    }

    fn create(&mut self, tag: &str) -> Result<El, String> {
        if !valid_tag(tag) {
            return Err(format!("InvalidCharacterError: '{tag}' is not a valid element name"));
        }
        Ok(self.fresh_node(tag))
    }

    fn set_text(&mut self, el: &El, text: &str) -> Result<(), String> {
        let node = self.nodes.get_mut(&el.id).ok_or("node table drift")?;
        node.text = Some(text.to_string());
        node.children.clear(); // the DOM textContent setter's own law
        Ok(())
    }

    fn attr(&mut self, el: &El, name: &str, value: &str) -> Result<(), String> {
        if !valid_attr_name(name) {
            return Err(format!(
                "InvalidCharacterError: '{name}' is not a valid attribute name"
            ));
        }
        self.nodes
            .get_mut(&el.id)
            .ok_or("node table drift")?
            .attrs
            .insert(name.to_string(), value.to_string());
        Ok(())
    }

    fn append(&mut self, parent: &El, child: &El) -> Result<(), String> {
        if parent.id == child.id {
            return Err(
                "HierarchyRequestError: the node to be inserted is the parent itself".to_string()
            );
        }
        if is_ancestor(&self.nodes, child.id, parent.id) {
            return Err(
                "HierarchyRequestError: the node to be inserted is an ancestor of the parent"
                    .to_string(),
            );
        }
        // appendChild moves an element that already has a parent
        if let Some(old) = self.nodes.get(&child.id).and_then(|n| n.parent) {
            if let Some(old_node) = self.nodes.get_mut(&old) {
                old_node.children.retain(|&c| c != child.id);
            }
        }
        self.nodes.get_mut(&child.id).ok_or("node table drift")?.parent = Some(parent.id);
        self.nodes
            .get_mut(&parent.id)
            .ok_or("node table drift")?
            .children
            .push(child.id);
        Ok(())
    }

    fn remove(&mut self, parent: &El, child: &El) -> Result<bool, String> {
        let p = self.nodes.get_mut(&parent.id).ok_or("node table drift")?;
        if !p.children.contains(&child.id) {
            return Ok(false); // the DOM negative — not an error, no trap
        }
        p.children.retain(|&c| c != child.id);
        if let Some(c) = self.nodes.get_mut(&child.id) {
            c.parent = None;
        }
        Ok(true)
    }

    fn clear(&mut self, el: &El) -> Result<(), String> {
        let children = self.nodes.get(&el.id).ok_or("node table drift")?.children.clone();
        for c in children {
            if let Some(node) = self.nodes.get_mut(&c) {
                node.parent = None;
            }
        }
        self.nodes.get_mut(&el.id).ok_or("node table drift")?.children.clear();
        Ok(())
    }

    fn current_value(&self, el: &El) -> Option<String> {
        if el.tag != "input" {
            return None;
        }
        self.nodes
            .get(&el.id)
            .ok_or("node table drift")
            .ok()
            .and_then(|n| n.attrs.get("value").cloned())
    }

    fn set_input_value(&mut self, el: &El, value: &str) -> Result<(), String> {
        if el.tag != "input" {
            return Err(format!(
                "boundary: got `{}` where `HtmlInputElement` binds",
                el.tag
            ));
        }
        self.nodes
            .get_mut(&el.id)
            .ok_or("node table drift")?
            .attrs
            .insert("value".to_string(), value.to_string());
        Ok(())
    }

    fn listen(&mut self, _el: &El, event: &str, id: i64) -> Result<(), String> {
        // addEventListener accepts any event name; the twin's callback
        // IS the test's `fire(id)` — nothing to wire.
        let _ = (event, id);
        Ok(())
    }

    fn after(&mut self, ms: i64, tag: &str) -> Result<(), String> {
        // setTimeout clamps negatives to 0 — the same law here
        let seq = self.next_seq;
        self.next_seq += 1;
        self.timers.push(TimerRec { at: self.now + ms.max(0), seq, tag: tag.to_string() });
        Ok(())
    }
}
