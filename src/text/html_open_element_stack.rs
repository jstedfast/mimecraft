// Adapted from MimeKit's Text/HtmlOpenElementStack.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{borrow::Cow, collections::HashMap};

use super::{HtmlNamespace, HtmlTagId, html_tree_construction::HtmlClosedElement};

pub(super) const SPECIAL: u16 = 1 << 0;
pub(super) const SCOPE: u16 = 1 << 1;
pub(super) const LIST_ITEM_SCOPE: u16 = 1 << 2;
pub(super) const BUTTON_SCOPE: u16 = 1 << 3;
pub(super) const TABLE_SCOPE: u16 = 1 << 4;
pub(super) const HEADING: u16 = 1 << 5;
// Special elements other than address, div and p stop the li/dd/dt start-tag walk.
pub(super) const LIST_ITEM_STOP: u16 = 1 << 6;
pub(super) const MATH_INTEGRATION: u16 = 1 << 7;
pub(super) const HTML_INTEGRATION: u16 = 1 << 8;

#[derive(Clone, Copy)]
pub(super) enum Predicate {
    Special,
    Scope,
    ListItemScope,
    ButtonScope,
    TableScope,
    Heading,
    ListItemStop,
    Html,
    HtmlOrIntegrationPoint,
}

#[cfg(test)]
mod tests {
    // Adapted from MimeKit's UnitTests/Text/HtmlOpenElementStackTests.cs.
    use super::*;
    use HtmlNamespace::{Html, MathMl, Svg};
    use HtmlTagId::{B, Div, Form, I, Unknown};

    fn push(stack: &mut OpenElementStack, name: &str, ns: HtmlNamespace, flags: u16) {
        let id = if ns == Html {
            HtmlTagId::from_name(name)
        } else {
            Unknown
        };
        stack.push(name, id, ns, flags);
    }

    #[test]
    fn remove_entry_rebuilds_scope_indexes() {
        let mut stack = OpenElementStack::new(32);
        push(&mut stack, "b", Html, 0);
        push(&mut stack, "form", Html, SPECIAL);
        push(&mut stack, "svg", Svg, 0);
        push(&mut stack, "mi", MathMl, SCOPE | MATH_INTEGRATION);
        assert_eq!(stack.nearest(Predicate::Special), Some(1));
        assert_eq!(stack.nearest(Predicate::Scope), Some(3));
        assert_eq!(stack.nearest(Predicate::Html), Some(1));
        assert_eq!(stack.nearest(Predicate::HtmlOrIntegrationPoint), Some(3));
        stack.remove_entry(1);
        assert_eq!(stack.entries.len(), 3);
        assert_eq!(stack.index_of(Form), None);
        assert_eq!(stack.index_of_name(Svg, Unknown, "svg"), Some(1));
        assert_eq!(stack.nearest(Predicate::Special), None);
        assert_eq!(stack.nearest(Predicate::Scope), Some(2));
        assert_eq!(stack.nearest(Predicate::Html), Some(0));
        assert_eq!(stack.nearest(Predicate::HtmlOrIntegrationPoint), Some(2));
        stack.pop_through(2);
        assert_eq!(stack.nearest(Predicate::Scope), None);
        assert_eq!(stack.nearest(Predicate::HtmlOrIntegrationPoint), Some(0));
    }

    #[test]
    fn remove_entry_folds_adjacent_runs() {
        let mut stack = OpenElementStack::new(32);
        for name in ["b", "form", "b", "b"] {
            push(
                &mut stack,
                name,
                Html,
                if name == "form" { SPECIAL } else { 0 },
            );
        }
        stack.remove_entry(1);
        assert_eq!(stack.entries.len(), 1);
        assert_eq!(stack.entries[0].count, 3);
        assert_eq!(stack.index_of(B), Some(0));
        assert_eq!(stack.nearest(Predicate::Special), None);
        for _ in 0..3 {
            stack.pop_through(0);
        }
        assert!(stack.entries.is_empty());
        assert_eq!(stack.index_of(B), None);
    }

    fn folded_stack(maximum: usize) -> OpenElementStack {
        let mut stack = OpenElementStack::new(maximum);
        for name in ["b", "b", "div", "div"] {
            push(
                &mut stack,
                name,
                Html,
                if name == "div" { SPECIAL } else { 0 },
            );
        }
        push(&mut stack, "svg", Svg, 0);
        stack
    }

    fn split_runs(stack: &mut OpenElementStack) {
        let mut formatting = stack.entries[0].clone();
        let mut block = stack.entries[1].clone();
        formatting.count = 1;
        block.count = 1;
        stack.replace_range(
            0,
            2,
            &[formatting.clone(), block.clone(), formatting, block],
        );
    }

    #[test]
    fn replace_range_rebuilds_indexes() {
        let mut stack = folded_stack(32);
        stack.closed = Some(Vec::new());
        push(&mut stack, "mi", MathMl, SCOPE | MATH_INTEGRATION);
        split_runs(&mut stack);
        assert_eq!(stack.entries.len(), 6);
        assert_eq!(stack.index_of(B), Some(2));
        assert_eq!(stack.entries[2].previous_same_name, Some(0));
        assert_eq!(stack.index_of(Div), Some(3));
        assert_eq!(stack.entries[3].previous_same_name, Some(1));
        assert_eq!(stack.index_of_name(Svg, Unknown, "svg"), Some(4));
        assert_eq!(stack.nearest(Predicate::Special), Some(3));
        assert_eq!(stack.nearest(Predicate::Scope), Some(5));
        assert_eq!(stack.nearest(Predicate::Html), Some(3));
        assert_eq!(stack.nearest(Predicate::HtmlOrIntegrationPoint), Some(5));
        assert!(stack.closed.as_ref().unwrap().is_empty());
        stack.pop_through(2);
        assert_eq!(stack.entries.len(), 2);
        assert_eq!(stack.index_of(B), Some(0));
        assert_eq!(stack.nearest(Predicate::Special), Some(1));
        assert_eq!(stack.nearest(Predicate::Scope), None);
        assert_eq!(
            stack
                .closed
                .as_ref()
                .unwrap()
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["mi", "svg", "div"]
        );
    }

    #[test]
    fn replace_range_folds_adjacent_runs() {
        let mut stack = OpenElementStack::new(32);
        for name in ["div", "b", "div", "div"] {
            push(
                &mut stack,
                name,
                Html,
                if name == "div" { SPECIAL } else { 0 },
            );
        }
        push(&mut stack, "svg", Svg, 0);
        let replacement = [stack.entries[2].clone(), stack.entries[1].clone()];
        stack.replace_range(1, 2, &replacement);
        assert_eq!(stack.entries.len(), 3);
        assert_eq!(stack.entries[0].count, 3);
        assert_eq!(stack.nearest(Predicate::Special), Some(0));
        assert_eq!(stack.index_of(B), Some(1));
        assert_eq!(stack.index_of_name(Svg, Unknown, "svg"), Some(2));
    }

    #[test]
    fn replace_range_depth_limit() {
        let mut stack = folded_stack(3);
        split_runs(&mut stack);
        assert!(stack.depth_exceeded);
        assert_eq!(stack.entries.len(), 3);
        assert_eq!(stack.entries[2].count, 1);
        assert_eq!(stack.index_of_name(Svg, Unknown, "svg"), None);
    }

    #[test]
    fn depth_limit_and_folding() {
        let mut stack = OpenElementStack::new(1);
        push(&mut stack, "b", Html, 0);
        push(&mut stack, "b", Html, 0);
        assert!(!stack.depth_exceeded);
        assert_eq!(stack.entries.len(), 1);
        assert_eq!(stack.entries[0].count, 2);
        push(&mut stack, "i", Html, 0);
        assert!(stack.depth_exceeded);
        assert_eq!(stack.index_of(I), None);
        assert_eq!(stack.entries.len(), 1);
    }

    #[test]
    fn indexes_across_vector_growth() {
        let mut stack = OpenElementStack::new(512);
        for i in 0..300 {
            push(&mut stack, if i % 2 == 0 { "b" } else { "i" }, Html, 0);
        }
        assert_eq!(stack.entries.len(), 300);
        assert_eq!(stack.index_of(B), Some(298));
        assert_eq!(stack.index_of(I), Some(299));
        stack.pop_through(63);
        assert_eq!(stack.entries.len(), 63);
        assert_eq!(stack.index_of(B), Some(62));
        assert_eq!(stack.index_of(I), Some(61));
        stack.pop_above(None);
        assert!(stack.entries.is_empty());
        assert_eq!(stack.nearest(Predicate::Html), None);
        assert_eq!(stack.index_of(B), None);
        assert_eq!(stack.index_of(I), None);
    }

    #[test]
    fn unknown_names_and_foreign_namespace_lookup() {
        let mut stack = OpenElementStack::new(8);
        push(&mut stack, "custom", Html, 0);
        push(&mut stack, "CUSTOM", Svg, 0);
        push(&mut stack, "custom", MathMl, 0);
        assert_eq!(stack.entries.len(), 3);
        assert_eq!(stack.index_of_name(Html, Unknown, "CuStOm"), Some(0));
        assert_eq!(stack.index_of_name(Svg, Unknown, "CuStOm"), Some(2));
        assert_eq!(stack.entries[2].previous_same_name, Some(1));
        stack.pop_through(2);
        assert_eq!(stack.index_of_name(MathMl, Unknown, "custom"), Some(1));
        stack.pop_through(1);
        assert_eq!(stack.index_of_name(Svg, Unknown, "custom"), None);
        push(&mut stack, "CUSTOM", Html, 0);
        assert_eq!(stack.entries.len(), 1);
        assert_eq!(stack.entries[0].count, 2);
    }

    #[test]
    fn saturating_runs_and_single_instance_removal() {
        let mut stack = OpenElementStack::new(8);
        push(&mut stack, "b", Html, 0);
        stack.entries[0].count = usize::MAX;
        push(&mut stack, "B", Html, 0);
        assert_eq!(stack.entries[0].count, usize::MAX);
        push(&mut stack, "div", Html, SPECIAL);
        stack.remove_entry(0);
        assert_eq!(stack.entries.len(), 2);
        assert_eq!(stack.entries[0].count, usize::MAX - 1);
        assert_eq!(stack.index_of(Div), Some(1));
        push(&mut stack, "b", Html, 0);
        push(&mut stack, "b", Html, 0);
        stack.remove_entry(1);
        assert_eq!(stack.entries[0].count, usize::MAX);
        assert_eq!(stack.entries.len(), 1);
    }

    #[test]
    fn cached_indexes_match_linear_search_after_mutations() {
        let mut stack = OpenElementStack::new(128);
        let cases = [
            ("b", Html, 0),
            ("div", Html, SPECIAL),
            ("table", Html, SCOPE | TABLE_SCOPE),
            ("button", Html, BUTTON_SCOPE),
            ("ul", Html, LIST_ITEM_SCOPE),
            ("h1", Html, HEADING | LIST_ITEM_STOP),
            ("svg", Svg, 0),
            ("mi", MathMl, MATH_INTEGRATION | SCOPE),
            ("title", Svg, HTML_INTEGRATION),
            ("custom", Html, 0),
            ("custom", Svg, 0),
        ];
        let mut random = 0x1234_5678_u32;
        for _ in 0..4000 {
            random = random.wrapping_mul(1664525).wrapping_add(1013904223);
            let n = random as usize;
            if stack.entries.len() < 2 || n % 5 < 2 {
                let (name, ns, flags) = cases[n % cases.len()];
                push(&mut stack, name, ns, flags);
            } else {
                let index = n % stack.entries.len();
                match n % 5 {
                    2 => stack.pop_through(index),
                    3 => stack.remove_entry(index),
                    _ => {
                        let entry = stack.entries[(n / 7) % stack.entries.len()].clone();
                        stack.replace_range(index, 1, &[entry]);
                    }
                }
            }
            assert!(!stack.depth_exceeded);
            for (slot, mask) in [
                SPECIAL,
                SCOPE,
                SCOPE | LIST_ITEM_SCOPE,
                SCOPE | BUTTON_SCOPE,
                TABLE_SCOPE,
                HEADING,
                LIST_ITEM_STOP,
            ]
            .into_iter()
            .enumerate()
            {
                let expected = stack.entries.iter().rposition(|e| e.flags & mask != 0);
                assert_eq!(stack.entries.last().and_then(|e| e.nearest[slot]), expected);
            }
            assert_eq!(
                stack.nearest(Predicate::Html),
                stack.entries.iter().rposition(|e| e.namespace == Html)
            );
            assert_eq!(
                stack.nearest(Predicate::HtmlOrIntegrationPoint),
                stack
                    .entries
                    .iter()
                    .rposition(|e| e.namespace == Html
                        || e.flags & (HTML_INTEGRATION | MATH_INTEGRATION) != 0)
            );
            for (name, ns, _) in cases {
                let id = if ns == Html {
                    HtmlTagId::from_name(name)
                } else {
                    Unknown
                };
                let expected = stack.entries.iter().rposition(|e| {
                    (e.namespace == Html) == (ns == Html) && e.name.eq_ignore_ascii_case(name)
                });
                assert_eq!(stack.index_of_name(ns, id, name), expected, "{ns:?}:{name}");
            }
            for (i, entry) in stack.entries.iter().enumerate() {
                let previous = stack.entries[..i].iter().rposition(|e| {
                    (e.namespace == Html) == (entry.namespace == Html)
                        && e.name.eq_ignore_ascii_case(&entry.name)
                });
                assert_eq!(entry.previous_same_name, previous);
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Entry {
    pub name: Cow<'static, str>,
    pub id: HtmlTagId,
    pub namespace: HtmlNamespace,
    pub flags: u16,
    // Consecutive identical elements share an entry and a repeat count.
    pub count: usize,
    previous_same_name: Option<usize>,
    // Nearest matching entries at or below this one; no stack walk is needed.
    nearest: [Option<usize>; 9],
}

#[derive(Debug)]
pub(super) struct OpenElementStack {
    pub entries: Vec<Entry>,
    html_ids: [Option<usize>; HtmlTagId::Xmp as usize + 1],
    html_names: HashMap<String, usize>,
    foreign_names: HashMap<String, usize>,
    pub max_depth: usize,
    pub depth_exceeded: bool,
    pub closed: Option<Vec<HtmlClosedElement>>,
}

impl OpenElementStack {
    pub fn new(max_depth: usize) -> Self {
        Self {
            entries: Vec::new(),
            html_ids: [None; HtmlTagId::Xmp as usize + 1],
            html_names: HashMap::new(),
            foreign_names: HashMap::new(),
            max_depth,
            depth_exceeded: false,
            closed: None,
        }
    }

    pub fn top(&self) -> Option<usize> {
        self.entries.len().checked_sub(1)
    }

    pub fn nearest(&self, predicate: Predicate) -> Option<usize> {
        self.entries
            .last()
            .and_then(|e| e.nearest[predicate as usize])
    }

    pub fn is_top_html(&self, id: HtmlTagId) -> bool {
        self.entries
            .last()
            .is_some_and(|e| e.namespace == HtmlNamespace::Html && e.id == id)
    }

    pub fn index_of(&self, id: HtmlTagId) -> Option<usize> {
        self.html_ids[id as usize]
    }

    pub fn index_of_name(&self, ns: HtmlNamespace, id: HtmlTagId, name: &str) -> Option<usize> {
        if ns == HtmlNamespace::Html && id != HtmlTagId::Unknown {
            return self.index_of(id);
        }
        let table = if ns == HtmlNamespace::Html {
            &self.html_names
        } else {
            &self.foreign_names
        };
        table.get(&name.to_ascii_lowercase()).copied()
    }

    fn set_index(&mut self, ns: HtmlNamespace, id: HtmlTagId, name: &str, index: Option<usize>) {
        if ns == HtmlNamespace::Html && id != HtmlTagId::Unknown {
            self.html_ids[id as usize] = index;
        } else {
            let table = if ns == HtmlNamespace::Html {
                &mut self.html_names
            } else {
                &mut self.foreign_names
            };
            let key = name.to_ascii_lowercase();
            if let Some(index) = index {
                table.insert(key, index);
            } else {
                table.remove(&key);
            }
        }
    }

    pub fn push(&mut self, name: &str, id: HtmlTagId, namespace: HtmlNamespace, flags: u16) {
        if let Some(top) = self.entries.last_mut()
            && top.namespace == namespace
            && top.id == id
            && top.flags == flags
            && (id != HtmlTagId::Unknown || top.name.eq_ignore_ascii_case(name))
        {
            // Fold consecutive identical elements before applying the entry limit.
            top.count = top.count.saturating_add(1);
            return;
        }
        if self.entries.len() >= self.max_depth {
            self.depth_exceeded = true;
            return;
        }
        let index = self.entries.len();
        let mut nearest = self.entries.last().map_or([None; 9], |e| e.nearest);
        for (slot, mask) in [
            SPECIAL,
            SCOPE,
            SCOPE | LIST_ITEM_SCOPE,
            SCOPE | BUTTON_SCOPE,
            TABLE_SCOPE,
            HEADING,
            LIST_ITEM_STOP,
        ]
        .into_iter()
        .enumerate()
        {
            if flags & mask != 0 {
                nearest[slot] = Some(index);
            }
        }
        if namespace == HtmlNamespace::Html {
            nearest[Predicate::Html as usize] = Some(index);
            nearest[Predicate::HtmlOrIntegrationPoint as usize] = Some(index);
        } else if flags & (MATH_INTEGRATION | HTML_INTEGRATION) != 0 {
            nearest[Predicate::HtmlOrIntegrationPoint as usize] = Some(index);
        }
        let previous_same_name = self.index_of_name(namespace, id, name);
        self.set_index(namespace, id, name, Some(index));
        // Rust's Vec grows amortized; cached indexes remain valid when it moves.
        self.entries.push(Entry {
            name: if id != HtmlTagId::Unknown && name == id.as_str() {
                Cow::Borrowed(id.as_str())
            } else {
                Cow::Owned(name.into())
            },
            id,
            namespace,
            flags,
            count: 1,
            previous_same_name,
            nearest,
        });
    }

    fn remove_top(&mut self) {
        let entry = self.entries.pop().expect("nonempty element stack");
        self.set_index(
            entry.namespace,
            entry.id,
            &entry.name,
            entry.previous_same_name,
        );
    }

    // Pop until index is the current node; None empties the stack.
    pub fn pop_above(&mut self, index: Option<usize>) {
        while self.top() > index {
            if let Some(closed) = &mut self.closed {
                let entry = self.entries.last().unwrap();
                closed.push(HtmlClosedElement {
                    name: entry.name.to_string(),
                    count: entry.count,
                    is_foreign: entry.namespace != HtmlNamespace::Html,
                });
            }
            self.remove_top();
        }
    }

    // Pop through one physical instance, excluding it from implicit closures.
    pub fn pop_through(&mut self, index: usize) {
        self.pop_above(Some(index));
        if self.entries[index].count > 1 {
            self.entries[index].count -= 1;
        } else {
            self.remove_top();
        }
    }

    pub fn remove_entry(&mut self, index: usize) {
        if self.entries[index].count > 1 {
            self.entries[index].count -= 1;
            return;
        }
        self.replace_range(index, 1, &[]);
    }

    // Retained suffixes cache indexes below them, so rebuild them after removal
    // or replacement. This is not an implicit closure of the retained nodes.
    pub fn replace_range(&mut self, index: usize, length: usize, replacement: &[Entry]) {
        let above = self.entries[index + length..].to_vec();
        while self.entries.len() > index {
            self.remove_top();
        }
        for entry in replacement.iter().chain(&above) {
            if self.depth_exceeded {
                break;
            }
            self.push(&entry.name, entry.id, entry.namespace, entry.flags);
            if !self.depth_exceeded {
                // Rebuilding can fold formerly separated runs, e.g. b/form/b.
                let top = self.entries.last_mut().unwrap();
                top.count = top.count.saturating_add(entry.count - 1);
            }
        }
    }
}
