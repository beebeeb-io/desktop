//! New Word, Excel and PowerPoint documents (a later task fills this).
#![allow(dead_code)]

use rune::Tx;

use crate::State;
use crate::frames::viewers::new_file;

pub fn open(_s: &mut State, _tx: &mut Tx, _kind: Option<new_file::Kind>, _folder: Option<String>) {}
