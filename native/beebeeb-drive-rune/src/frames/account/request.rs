//! The fixture's side of approving a sign-in (DW-A11 to DW-A17): the answers its "server" and its
//! "Windows Hello" give, by request. Every answer names the request it is for; the screen applies
//! one only to the request it shows.

use super::approve::{Approval, Denial, SignInRequest, board_request};

/// What the fixture holds between `begin_approval` and the answer.
#[derive(Default)]
pub struct BoardApprovals {
    /// The answers on their way, oldest first: `(request id, answer)`.
    answers: Vec<(String, Approval)>,
    /// Windows Hello's prompt stays up: no answer comes (the scene of DW-A14).
    hold: bool,
    /// A cancel comes too late to stop an answer that is on its way.
    late: bool,
}

impl BoardApprovals {
    pub fn hold(&mut self) {
        self.hold = true;
    }

    pub fn late(&mut self) {
        self.late = true;
    }

    /// The requests the fixture's notifications announced: the board's, and one from a place
    /// the account has not been seen in.
    pub fn waiting(request: &str) -> Option<SignInRequest> {
        match request {
            "req-iphone" => Some(board_request(request, "Germany", false)),
            "req-unusual" => Some(board_request(request, "Brazil", true)),
            _ => None,
        }
    }

    pub fn begin(&mut self, request: &str) {
        let answers: &[Approval] = match request {
            "req-expired" => &[Approval::Expired],
            "req-handled" => &[Approval::AlreadyHandled],
            "req-cancelled" => &[Approval::Cancelled],
            // The connection is gone before anything is sent; gone after the claim was sent;
            // and gone after it, with a later check that finds the request approved.
            "req-dropped" => &[Approval::Offline],
            "req-unknown" => &[Approval::Unknown, Approval::AlreadyHandled],
            "req-found-later" => &[Approval::Unknown, Approval::Approved],
            _ => &[Approval::Approved],
        };
        for answer in answers {
            self.answers.push((request.to_owned(), *answer));
        }
    }

    pub fn answer(&mut self) -> Option<(String, Approval)> {
        if self.hold || self.answers.is_empty() {
            None
        } else {
            Some(self.answers.remove(0))
        }
    }

    pub fn cancel(&mut self, request: &str) {
        if !self.late {
            self.answers.retain(|(id, _)| id != request);
        }
    }

    pub fn deny(&mut self, request: &str) -> Denial {
        // A claim that could not be recalled got there first.
        let before = self.answers.len();
        self.answers
            .retain(|(id, answer)| !(id == request && *answer == Approval::Approved));
        if self.answers.len() != before {
            return Denial::ApprovedHere;
        }
        match request {
            "req-dropped" => Denial::Offline,
            "req-expired" => Denial::Expired,
            "req-handled" => Denial::AlreadyHandled,
            _ => Denial::Denied,
        }
    }
}
