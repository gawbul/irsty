//! Split windows: the scrollback area divided into panes, each showing a
//! buffer, as a tree of splits. One pane has focus; the input, the topic and
//! the nick list belong to it.
//!
//! A layout is saved as text (`ui.layout`), such as
//! `v(libera/#rust,h(libera/#irsty,oftc))`: `v(a,b)` puts `a` above `b`,
//! `h(a,b)` puts them side by side, and a pane is a network name, optionally
//! followed by `/` and a channel or nick (a network alone is its server
//! buffer). `%`, `(`, `)` and `,` in names are written `%25`, `%28`, `%29`
//! and `%2C`.

use ratatui::layout::{Constraint, Layout, Rect};

use irsty_core::state::BufferId;

/// Most panes at once.
pub const MAX_PANES: usize = 8;

/// How a split divides its space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// One above the other.
    Stacked,
    /// Side by side.
    SideBySide,
}

/// A layout: a pane, or a split of two layouts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node<T> {
    /// A pane.
    Pane(T),
    /// Two layouts, first above or left of second.
    Split(Direction, Box<Node<T>>, Box<Node<T>>),
}

impl<T> Node<T> {
    /// The panes, top-left first.
    pub fn leaves(&self) -> Vec<&T> {
        match self {
            Self::Pane(t) => vec![t],
            Self::Split(_, a, b) => {
                let mut out = a.leaves();
                out.extend(b.leaves());
                out
            }
        }
    }

    /// Each pane's area within `area`.
    pub fn areas(&self, area: Rect) -> Vec<(&T, Rect)> {
        match self {
            Self::Pane(t) => vec![(t, area)],
            Self::Split(dir, a, b) => {
                let halves = [Constraint::Fill(1), Constraint::Fill(1)];
                let [first, second] = match dir {
                    Direction::Stacked => Layout::vertical(halves).areas(area),
                    Direction::SideBySide => Layout::horizontal(halves).areas(area),
                };
                let mut out = a.areas(first);
                out.extend(b.areas(second));
                out
            }
        }
    }

    fn map<U>(self, f: &mut impl FnMut(T) -> U) -> Node<U> {
        match self {
            Self::Pane(t) => Node::Pane(f(t)),
            Self::Split(d, a, b) => Node::Split(d, Box::new(a.map(f)), Box::new(b.map(f))),
        }
    }
}

/// Where a pane wants to be, by name, until that buffer exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    /// The network's name.
    pub network: String,
    /// The channel or nick; `None` for the server buffer.
    pub buffer: Option<String>,
}

/// One pane.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pane {
    /// The buffer it shows. For the focused pane this is kept in
    /// [`crate::ui::Ui::active`] instead and may be stale here.
    pub buffer: Option<BufferId>,
    /// A buffer from a saved layout that doesn't exist yet.
    pub want: Option<Place>,
}

/// The panes and which has focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Panes {
    root: Node<usize>,
    slots: Vec<Option<Pane>>,
    focus: usize,
}

impl Default for Panes {
    fn default() -> Self {
        Self {
            root: Node::Pane(0),
            slots: vec![Some(Pane::default())],
            focus: 0,
        }
    }
}

impl Panes {
    /// The focused pane's id.
    pub fn focus(&self) -> usize {
        self.focus
    }

    /// How many panes there are.
    pub fn len(&self) -> usize {
        self.root.leaves().len()
    }

    /// Whether there is only one pane, as there always is at least one.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Pane ids, top-left first.
    pub fn ids(&self) -> Vec<usize> {
        self.root.leaves().into_iter().copied().collect()
    }

    /// A pane.
    pub fn get(&self, id: usize) -> Option<&Pane> {
        self.slots.get(id)?.as_ref()
    }

    /// A pane, to change.
    pub fn get_mut(&mut self, id: usize) -> Option<&mut Pane> {
        self.slots.get_mut(id)?.as_mut()
    }

    /// Each pane's area.
    pub fn areas(&self, area: Rect) -> Vec<(usize, Rect)> {
        self.root
            .areas(area)
            .into_iter()
            .map(|(&id, r)| (id, r))
            .collect()
    }

    /// Splits the focused pane; the new pane shows `buffer` and takes focus.
    /// Returns the new pane's id, or `None` at [`MAX_PANES`].
    pub fn split(&mut self, dir: Direction, buffer: Option<BufferId>) -> Option<usize> {
        if self.len() >= MAX_PANES {
            return None;
        }
        let id = self.slots.len();
        self.slots.push(Some(Pane { buffer, want: None }));
        let focus = self.focus;
        replace(&mut self.root, focus, &mut |old| {
            Node::Split(dir, Box::new(old), Box::new(Node::Pane(id)))
        });
        self.focus = id;
        Some(id)
    }

    /// Closes the focused pane; its neighbour takes the space and focus.
    /// Returns false if it is the only one.
    pub fn close(&mut self) -> bool {
        let focus = self.focus;
        let Some(next) = remove(&mut self.root, focus) else {
            return false;
        };
        if let Some(slot) = self.slots.get_mut(focus) {
            *slot = None;
        }
        self.focus = next;
        true
    }

    /// Keeps only the focused pane.
    pub fn only(&mut self) {
        let focus = self.focus;
        for (id, slot) in self.slots.iter_mut().enumerate() {
            if id != focus {
                *slot = None;
            }
        }
        self.root = Node::Pane(focus);
    }

    /// Moves focus to the next pane (or the previous), wrapping round.
    /// Returns the newly focused pane's id.
    pub fn cycle(&mut self, forward: bool) -> usize {
        let ids = self.ids();
        let at = ids.iter().position(|&i| i == self.focus).unwrap_or(0);
        let next = if forward {
            (at + 1) % ids.len()
        } else {
            (at + ids.len() - 1) % ids.len()
        };
        self.focus = ids.get(next).copied().unwrap_or(self.focus);
        self.focus
    }

    /// Focuses pane `id`, if it exists.
    pub fn set_focus(&mut self, id: usize) -> bool {
        let exists = self.ids().contains(&id);
        if exists {
            self.focus = id;
        }
        exists
    }

    /// The layout as text, naming each pane's buffer with `name`, which
    /// gives the network and, unless it is a server buffer, the buffer name.
    pub fn to_spec(&self, mut name: impl FnMut(&Pane) -> Option<Place>) -> String {
        let named = self.root.clone().map(&mut |id| {
            self.get(id)
                .and_then(|p| p.want.clone().or_else(|| name(p)))
        });
        spec(&named)
    }

    /// Panes from a saved layout; they show their buffers once they exist.
    /// The first pane has focus.
    pub fn from_spec(text: &str) -> Result<Self, String> {
        let mut parser = Parser { text, at: 0 };
        let node = parser.node()?;
        parser.skip_space();
        if parser.at != text.len() {
            return Err(format!("unexpected {:?} in the layout", parser.rest()));
        }
        let mut slots = Vec::new();
        let root = node.map(&mut |want| {
            slots.push(Some(Pane { buffer: None, want }));
            slots.len() - 1
        });
        if slots.len() > MAX_PANES {
            return Err(format!("a layout has at most {MAX_PANES} panes"));
        }
        Ok(Self {
            root,
            slots,
            focus: 0,
        })
    }
}

/// Replaces leaf `id` with `f(leaf)`.
fn replace(node: &mut Node<usize>, id: usize, f: &mut impl FnMut(Node<usize>) -> Node<usize>) {
    match node {
        Node::Pane(i) if *i == id => {
            let old = std::mem::replace(node, Node::Pane(id));
            *node = f(old);
        }
        Node::Pane(_) => {}
        Node::Split(_, a, b) => {
            replace(a, id, f);
            replace(b, id, f);
        }
    }
}

/// Removes leaf `id`, its sibling taking the parent's place. Returns the pane
/// that should take focus (the sibling's first), or `None` if `id` is the
/// root.
fn remove(node: &mut Node<usize>, id: usize) -> Option<usize> {
    let Node::Split(_, a, b) = node else {
        return None;
    };
    let sibling = match (&**a, &**b) {
        (Node::Pane(i), _) if *i == id => Some((**b).clone()),
        (_, Node::Pane(i)) if *i == id => Some((**a).clone()),
        _ => None,
    };
    if let Some(sibling) = sibling {
        let focus = sibling.leaves().first().map(|&&i| i);
        *node = sibling;
        return focus;
    }
    remove(a, id).or_else(|| remove(b, id))
}

fn escape(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            '%' => out.push_str("%25"),
            '(' => out.push_str("%28"),
            ')' => out.push_str("%29"),
            ',' => out.push_str("%2C"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(text: &str) -> String {
    text.replace("%28", "(")
        .replace("%29", ")")
        .replace("%2C", ",")
        .replace("%2c", ",")
        .replace("%25", "%")
}

fn spec(node: &Node<Option<Place>>) -> String {
    match node {
        Node::Pane(None) => "-".into(),
        Node::Pane(Some(place)) => match &place.buffer {
            Some(buffer) => format!("{}/{}", escape(&place.network), escape(buffer)),
            None => escape(&place.network),
        },
        Node::Split(dir, a, b) => {
            let letter = match dir {
                Direction::Stacked => 'v',
                Direction::SideBySide => 'h',
            };
            format!("{letter}({},{})", spec(a), spec(b))
        }
    }
}

struct Parser<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        self.text.get(self.at..).unwrap_or_default()
    }

    fn skip_space(&mut self) {
        let rest = self.rest();
        self.at += rest.len() - rest.trim_start().len();
    }

    fn eat(&mut self, c: char) -> bool {
        self.skip_space();
        if self.rest().starts_with(c) {
            self.at += c.len_utf8();
            true
        } else {
            false
        }
    }

    fn node(&mut self) -> Result<Node<Option<Place>>, String> {
        self.skip_space();
        let rest = self.rest();
        for (prefix, dir) in [("v(", Direction::Stacked), ("h(", Direction::SideBySide)] {
            if rest.starts_with(prefix) {
                self.at += prefix.len();
                let a = self.node()?;
                if !self.eat(',') {
                    return Err("expected \",\" between the two halves of a split".into());
                }
                let b = self.node()?;
                if !self.eat(')') {
                    return Err("expected \")\" closing a split".into());
                }
                return Ok(Node::Split(dir, Box::new(a), Box::new(b)));
            }
        }
        let end = rest.find([',', ')', '(']).unwrap_or(rest.len());
        let token = rest.get(..end).unwrap_or_default().trim();
        self.at += end;
        if token.is_empty() {
            return Err("expected a pane (network/buffer) or a split".into());
        }
        if token == "-" {
            return Ok(Node::Pane(None));
        }
        // Network names may contain "/" (a bouncer's networks); the last "/"
        // starts the buffer, and the client tries each split when resolving.
        Ok(Node::Pane(Some(match token.rsplit_once('/') {
            Some((network, buffer)) => Place {
                network: unescape(network),
                buffer: Some(unescape(buffer)),
            },
            None => Place {
                network: unescape(token),
                buffer: None,
            },
        })))
    }
}

impl Place {
    /// The ways to read this place as network and buffer: as parsed, and with
    /// the buffer taken as part of the network's name (a bouncer network's
    /// server buffer, such as `soju/libera`).
    pub fn readings(&self) -> Vec<(String, Option<String>)> {
        let mut out = vec![(self.network.clone(), self.buffer.clone())];
        if let Some(buffer) = &self.buffer {
            out.push((format!("{}/{buffer}", self.network), None));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(network: &str, buffer: Option<&str>) -> Place {
        Place {
            network: network.into(),
            buffer: buffer.map(str::to_owned),
        }
    }

    #[test]
    fn split_close_and_cycle() {
        let mut p = Panes::default();
        assert_eq!(p.len(), 1);
        assert!(!p.close(), "the last pane stays");
        let right = p.split(Direction::SideBySide, Some(BufferId(2))).unwrap();
        assert_eq!(p.focus(), right);
        let below = p.split(Direction::Stacked, Some(BufferId(3))).unwrap();
        assert_eq!(p.ids(), [0, right, below]);
        let area = Rect::new(0, 0, 80, 20);
        let areas = p.areas(area);
        assert_eq!(areas[0].1, Rect::new(0, 0, 40, 20));
        assert_eq!(areas[1].1, Rect::new(40, 0, 40, 10));
        assert_eq!(areas[2].1, Rect::new(40, 10, 40, 10));

        assert_eq!(p.cycle(true), 0, "wraps round");
        assert_eq!(p.cycle(false), below);
        // Closing the bottom-right gives its space and focus to its sibling.
        assert!(p.close());
        assert_eq!(p.ids(), [0, right]);
        assert_eq!(p.focus(), right);
        assert_eq!(p.areas(area)[1].1, Rect::new(40, 0, 40, 20));
        assert!(p.get(below).is_none());
        p.split(Direction::Stacked, None);
        p.only();
        assert_eq!(p.ids().len(), 1);
        assert!(p.set_focus(p.ids()[0]));
        assert!(!p.set_focus(99));
    }

    #[test]
    fn at_most_max_panes() {
        let mut p = Panes::default();
        for _ in 1..MAX_PANES {
            assert!(p.split(Direction::Stacked, None).is_some());
        }
        assert!(p.split(Direction::Stacked, None).is_none());
    }

    #[test]
    fn layouts_round_trip() {
        let mut p = Panes::default();
        p.split(Direction::Stacked, None);
        p.split(Direction::SideBySide, None);
        let names = [
            place("libera", Some("#rust")),
            place("Libera, (old)", Some("#a(b)")),
            place("soju/oftc", None),
        ];
        let mut i = 0;
        let text = p.to_spec(|_| {
            i += 1;
            names.get(i - 1).cloned()
        });
        assert_eq!(
            text,
            "v(libera/#rust,h(Libera%2C %28old%29/#a%28b%29,soju/oftc))"
        );
        let back = Panes::from_spec(&text).unwrap();
        let wants: Vec<Place> = back
            .ids()
            .iter()
            .map(|&id| back.get(id).unwrap().want.clone().unwrap())
            .collect();
        assert_eq!(wants[0], names[0]);
        assert_eq!(wants[1], names[1]);
        // "soju/oftc" reads as network "soju", buffer "oftc", or as the
        // server buffer of network "soju/oftc".
        assert_eq!(
            wants[2].readings(),
            [
                ("soju".to_owned(), Some("oftc".to_owned())),
                ("soju/oftc".to_owned(), None)
            ]
        );
        assert_eq!(back.focus(), 0);
        assert_eq!(Panes::from_spec(" h( a/#x , - ) ").unwrap().len(), 2);
    }

    #[test]
    fn bad_layouts_are_refused() {
        for bad in ["", "v(a)", "h(a,b", "a b(", "v(a,b)c", "h(,b)"] {
            assert!(Panes::from_spec(bad).is_err(), "{bad:?}");
        }
        let nine = (0..MAX_PANES).fold("x".to_owned(), |acc, _| format!("v({acc},x)"));
        assert!(Panes::from_spec(&nine).is_err());
    }
}
