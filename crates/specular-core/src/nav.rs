//! Where the user is: a stack of screens, each remembering its focus.

use crate::model::{RowId, ScreenId};

/// One screen on the stack. `params` says what it shows (e.g. which system's games).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenState {
    pub id: ScreenId,
    pub params: Option<String>,
    /// The focused row, by identity, so it survives the screen's rows changing.
    pub focus: Option<RowId>,
}

impl ScreenState {
    pub fn new(id: ScreenId, params: Option<String>) -> Self { Self { id, params, focus: None } }
}

/// How the UI should animate a change of screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition { Push, Pop }

/// The screens the user went through; the bottom one (home) is never popped.
#[derive(Clone, Debug)]
pub struct NavStack(Vec<ScreenState>);

impl NavStack {
    pub fn new(home: ScreenState) -> Self { Self(vec![home]) }

    pub fn top(&self) -> &ScreenState { self.0.last().unwrap() }
    pub fn top_mut(&mut self) -> &mut ScreenState { self.0.last_mut().unwrap() }
    pub fn depth(&self) -> usize { self.0.len() }
    /// The screen below the top one, if any.
    pub fn parent(&self) -> Option<&ScreenState> { self.0.iter().rev().nth(1) }

    /// Whether a screen of this kind is anywhere on the stack.
    pub fn contains(&self, id: &ScreenId) -> bool { self.0.iter().any(|s| &s.id == id) }

    pub fn push(&mut self, s: ScreenState) -> Transition { self.0.push(s); Transition::Push }

    /// Goes back one screen; None at home.
    pub fn pop(&mut self) -> Option<Transition> {
        (self.0.len() > 1).then(|| { self.0.pop(); Transition::Pop })
    }

    /// Back to home; None if already there.
    pub fn home(&mut self) -> Option<Transition> {
        (self.0.len() > 1).then(|| { self.0.truncate(1); Transition::Pop })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(id: &'static str) -> ScreenState { ScreenState::new(ScreenId(id.into()), None) }

    #[test]
    fn home_stays() {
        let mut nav = NavStack::new(screen("home"));
        assert_eq!(nav.pop(), None);
        assert_eq!(nav.home(), None);
        nav.push(screen("games"));
        nav.push(screen("settings"));
        assert_eq!(nav.depth(), 3);
        assert_eq!(nav.pop(), Some(Transition::Pop));
        assert_eq!(nav.top().id, ScreenId("games".into()));
        nav.push(screen("settings"));
        assert_eq!(nav.home(), Some(Transition::Pop));
        assert_eq!(nav.top().id, ScreenId("home".into()));
    }

    #[test]
    fn focus_is_kept_per_screen() {
        let mut nav = NavStack::new(screen("home"));
        nav.top_mut().focus = Some(RowId("gba".into()));
        nav.push(screen("games"));
        nav.pop();
        assert_eq!(nav.top().focus, Some(RowId("gba".into())));
    }
}
