use std::{
    env,
    process::{Child, Command},
};

use crate::Result;

pub fn is_default<T: Default + PartialEq>(t: &T) -> bool {
    *t == Default::default()
}

/// Restart the program and keep the argument.
///
/// Inherit the environment/io/working directory of current process.
pub fn restart() -> Result<Child> {
    Command::new(env::current_exe().unwrap())
        .args(env::args().skip(1))
        .spawn()
        .map_err(Into::into)
}
