//! tui-realm components. They hold no screen state: their only job is to turn events
//! into `Msg`s and to own focus. All drawing is done by `Model::draw`.

pub mod bridge;
pub mod confirm;
pub mod day;
pub mod form;
pub mod help;
pub mod month;
pub mod stats;

use tuirealm::command::{Cmd, CmdResult};
use tuirealm::component::Component;
use tuirealm::props::{AttrValue, Attribute, Props, QueryResult};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;
use tuirealm::state::State;

/// A component that draws nothing and only routes keys.
#[derive(Default)]
pub struct KeyOnly {
    props: Props,
}

impl Component for KeyOnly {
    fn view(&mut self, _f: &mut Frame, _a: Rect) {}

    fn query<'a>(&'a self, attr: Attribute) -> Option<QueryResult<'a>> {
        self.props.get_for_query(attr)
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        self.props.set(attr, value);
    }

    fn state(&self) -> State {
        State::None
    }

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        CmdResult::Invalid(cmd)
    }
}
