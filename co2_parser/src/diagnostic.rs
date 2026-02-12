use std::sync::Mutex;

use chumsky::{error::Rich, span::SimpleSpan};

use crate::lexer::Token;

static ERRORS: Mutex<Vec<Rich<'static, Token, SimpleSpan>>> = Mutex::new(Vec::new());

pub struct RaiseErrorPanicPayload;

#[track_caller]
pub fn todo_error(span: SimpleSpan) -> ! {
    report_error(Rich::custom(
        span,
        format!("{}", std::panic::Location::caller()),
    ));
    std::panic::resume_unwind(Box::new(RaiseErrorPanicPayload));
}

pub fn raise_error(span: SimpleSpan, msg: &str) -> ! {
    report_error(Rich::custom(span, msg));
    unwind_stack_after_report();
}

pub fn unwind_stack_after_report() -> ! {
    std::panic::resume_unwind(Box::new(RaiseErrorPanicPayload));
}

pub fn report_error(err: Rich<'static, Token, SimpleSpan>) {
    ERRORS.lock().unwrap().push(err);
}

pub fn take_errors() -> Vec<Rich<'static, Token, SimpleSpan>> {
    let mut guard = ERRORS.lock().unwrap();
    std::mem::take(&mut *guard)
}
