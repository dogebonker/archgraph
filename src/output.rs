//! Request-local output capture, shared by the CLI and daemon.
use std::{
    cell::RefCell,
    fmt::{self, Write as _},
    io::{self, Write as _},
};
#[derive(Default, Debug, serde::Serialize, serde::Deserialize)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub code: u8,
}
thread_local! { static CAPTURE: RefCell<Vec<Output>> = const { RefCell::new(Vec::new()) }; }
pub struct Capture;
impl Capture {
    pub fn begin() -> Self {
        CAPTURE.with(|c| c.borrow_mut().push(Output::default()));
        Self
    }
    pub fn finish(self, code: u8) -> Output {
        let mut out = CAPTURE.with(|c| c.borrow_mut().pop().unwrap());
        out.code = code;
        std::mem::forget(self);
        out
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        CAPTURE.with(|c| {
            c.borrow_mut().pop();
        });
    }
}
fn emit(args: fmt::Arguments<'_>, error: bool) {
    CAPTURE.with(|c| {
        if let Some(out) = c.borrow_mut().last_mut() {
            let target = if error {
                &mut out.stderr
            } else {
                &mut out.stdout
            };
            target.write_fmt(args).expect("formatting output");
        } else if error {
            io::stderr().lock().write_fmt(args).expect("writing stderr");
        } else {
            io::stdout().lock().write_fmt(args).expect("writing stdout");
        }
    });
}
pub fn stdout(args: fmt::Arguments<'_>) {
    emit(args, false);
}
pub fn stderr(args: fmt::Arguments<'_>) {
    emit(args, true);
}
