//! Leave, Stop sharing and Close request ask first; Open on a link row opens its item (w17-shared-items).

use std::cell::RefCell;
use std::rc::Rc;

use super::super::dialogs::Dialog;
use super::super::parts::table::Row;
use super::super::scenes;
use super::super::source::{
    Access, BoardShared, Invitation, InviteResult, LinkShare, Member, PeopleShare, Request, RowTarget, SharedSource,
};
use super::super::tests::{click, runtime, settle};
use super::super::Tab;
use crate::State;
use crate::frames::Intent;
use rune::Runtime;

/// What the engine was asked to do.
#[derive(Default)]
struct Asked {
    left: Vec<String>,
    stopped: Vec<String>,
    closed: Vec<String>,
}

/// The board's lists, recording the calls that change something, and pointing a link at an item.
struct Spy(BoardShared, Rc<RefCell<Asked>>);

impl SharedSource for Spy {
    fn invitations(&self) -> Vec<Invitation> {
        self.0.invitations()
    }
    fn with_me(&self) -> Vec<Row> {
        self.0.with_me()
    }
    fn people_shares(&self) -> Vec<PeopleShare> {
        self.0.people_shares()
    }
    fn links(&self) -> Vec<LinkShare> {
        self.0.links()
    }
    fn requests(&self) -> Vec<Request> {
        self.0.requests()
    }
    fn accept(&mut self, id: &str) -> bool {
        self.0.accept(id)
    }
    fn decline(&mut self, id: &str) -> bool {
        self.0.decline(id)
    }
    fn keep_offline(&mut self, id: &str) -> bool {
        self.0.keep_offline(id)
    }
    fn stop_sharing(&mut self, id: &str) -> bool {
        self.1.borrow_mut().stopped.push(id.into());
        true
    }
    fn close_request(&mut self, id: &str) -> bool {
        self.1.borrow_mut().closed.push(id.into());
        true
    }
    fn remove_request(&mut self, id: &str) -> bool {
        self.0.remove_request(id)
    }
    fn leave(&mut self, id: &str) -> bool {
        self.1.borrow_mut().left.push(id.into());
        true
    }
    fn target_of(&self, id: &str) -> Option<RowTarget> {
        if self.with_me().iter().any(|r| r.id == id) {
            return Some(RowTarget::Item {
                id: "the-file".into(),
                is_folder: false,
            });
        }
        self.links().iter().any(|l| l.id == id).then(|| RowTarget::Item {
            id: "the-item".into(),
            is_folder: true,
        })
    }
    fn members(&self, share: &str) -> Vec<Member> {
        self.0.members(share)
    }
    fn set_access(&mut self, share: &str, person: &str, access: Access) -> bool {
        self.0.set_access(share, person, access)
    }
    fn remove_access(&mut self, share: &str, person: &str) -> bool {
        self.0.remove_access(share, person)
    }
    fn invite_answer(&mut self) -> Option<(String, InviteResult)> {
        None
    }
}

fn window(scene: fn(&mut State)) -> (Runtime<State>, Rc<RefCell<Asked>>) {
    let mut rt = runtime(scene);
    let asked = Rc::new(RefCell::new(Asked::default()));
    rt.state.frames.shared.source = Box::new(Spy(BoardShared::new(), asked.clone()));
    settle(&mut rt);
    (rt, asked)
}

/// Takes what the frame queued, as the shell's drain does for Shared.
fn drain(rt: &mut Runtime<State>) {
    rt.update(|s, tx| {
        for intent in std::mem::take(&mut s.frames.intents) {
            assert!(super::take(s, tx, &intent), "taken: {intent:?}");
        }
    });
    settle(rt);
}

#[test]
fn leave_sends_one_leave_only_after_the_confirmation() {
    let (mut rt, asked) = window(scenes::dw_0505);
    let id = rt.state.frames.shared.source.with_me()[1].id.clone();
    click(&mut rt, "mi-leave");
    assert_eq!(rt.state.frames.shared.dialog, Some(Dialog::ConfirmLeave { id: id.clone() }));
    drain(&mut rt);
    assert!(asked.borrow().left.is_empty(), "nothing is sent by the menu");
    // Cancel sends nothing.
    click(&mut rt, "btn-cancel");
    assert!(asked.borrow().left.is_empty() && rt.state.frames.shared.dialog.is_none());
    // Asking again and confirming sends exactly one Leave.
    rt.state.frames.shared.menu = Some(super::super::parts::menu::Open::at(430.0, 262.0));
    settle(&mut rt);
    click(&mut rt, "mi-leave");
    settle(&mut rt);
    click(&mut rt, "btn-leave");
    drain(&mut rt);
    drain(&mut rt);
    assert_eq!(asked.borrow().left, vec![id]);
    assert!(rt.state.frames.shared.dialog.is_none());
}

#[test]
fn an_answer_without_its_confirmation_does_nothing() {
    let (mut rt, asked) = window(scenes::dw_0505);
    rt.state.frames.intents.push(Intent::Do("confirm-leave", "x".into()));
    rt.state.frames.intents.push(Intent::Do("confirm-stop", "x".into()));
    rt.state.frames.intents.push(Intent::Do("confirm-close", "x".into()));
    drain(&mut rt);
    let a = asked.borrow();
    assert!(a.left.is_empty() && a.stopped.is_empty() && a.closed.is_empty());
}

#[test]
fn stop_sharing_and_close_request_ask_first() {
    let (mut rt, asked) = window(scenes::dw_0506);
    click(&mut rt, "mi-stop-sharing");
    assert!(matches!(rt.state.frames.shared.dialog, Some(Dialog::ConfirmStop { .. })));
    assert!(asked.borrow().stopped.is_empty());
    click(&mut rt, "btn-stop-sharing");
    drain(&mut rt);
    assert_eq!(asked.borrow().stopped, vec!["l-budget".to_string()]);

    let (mut rt, asked) = window(scenes::dw_0507);
    click(&mut rt, "mi-close-request");
    assert!(matches!(rt.state.frames.shared.dialog, Some(Dialog::ConfirmClose { .. })));
    assert!(asked.borrow().closed.is_empty());
    click(&mut rt, "btn-close-request");
    drain(&mut rt);
    assert_eq!(asked.borrow().closed.len(), 1);
}

#[test]
fn open_on_a_link_row_opens_the_item_not_the_share_id() {
    let (mut rt, _) = window(scenes::dw_0502);
    rt.state.frames.shared.show(Tab::ByMe);
    rt.state.frames.intents.push(Intent::OpenItem("l-budget".into()));
    drain(&mut rt);
    assert_eq!(rt.state.shell.app.route().folder.as_deref(), Some("the-item"));
}

#[test]
fn open_on_a_shared_file_goes_through_open_file() {
    // A viewer that shows no kind: only `open_file` sends the file on to the system's app (step 2).
    let (mut rt, _) = window(scenes::dw_0501);
    let calls = crate::frames::open_file::testing::wire_into(&mut rt.state, false);
    let id = rt.state.frames.shared.source.with_me()[0].id.clone();
    rt.state.frames.intents.push(Intent::OpenItem(id));
    drain(&mut rt);
    assert_eq!(
        *calls.lock().unwrap(),
        [crate::frames::files::source::Call::OpenDefault("the-file".into())]
    );
}
