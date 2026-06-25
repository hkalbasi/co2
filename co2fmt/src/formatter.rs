use crate::config::{
    AlignCmtStyle, BraceStyle, Config, ExternCBrace, IndentStyle, PointerAlign, SpaceOption,
};
use crate::error::FunkyError;
use crate::token::{Span, Token, TokenKind};

// ── Brace-injection pre-pass ──────────────────────────────────────────────────
//
// When add_braces_to_if / _while / _for are enabled, we do a single pre-pass
// over the token list and inject synthetic `{` / `}` tokens around braceless
// single-statement bodies.  The resulting token slice is then fed to the main
// formatter, which sees only already-braced code.

fn inj_synthetic(kind: TokenKind, lexeme: &'static str) -> Token<'static> {
    Token {
        kind,
        lexeme,
        span: Span {
            start_byte: 0,
            end_byte: 0,
            line: 0,
            col: 0,
        },
    }
}

fn inj_copy_ws<'src>(tokens: &[Token<'src>], i: &mut usize, out: &mut Vec<Token<'src>>) {
    while *i < tokens.len() && matches!(tokens[*i].kind, TokenKind::Whitespace | TokenKind::Newline)
    {
        out.push(tokens[*i].clone());
        *i += 1;
    }
}

/// Returns true when the unbraced statement body starting at `from` contains a
/// `PreprocLine` token at brace-depth 0 before the terminating `;`.  Used to
/// suppress brace injection when the body of an `if`/`for`/`while`/`else`
/// spans a `#ifdef`/`#else`/`#endif` block — injecting braces in that case
/// would only wrap one preprocessor branch and corrupt the other.
fn inj_stmt_has_preproc(tokens: &[Token<'_>], from: usize) -> bool {
    let mut i = from;
    // skip leading whitespace/comments
    while i < tokens.len()
        && matches!(
            tokens[i].kind,
            TokenKind::Whitespace
                | TokenKind::Newline
                | TokenKind::CommentBlock
                | TokenKind::CommentLine
        )
    {
        i += 1;
    }
    // If the body is already braced, brace injection won't happen anyway.
    if i < tokens.len() && tokens[i].kind == TokenKind::LBrace {
        return false;
    }
    let mut depth = 0u32;
    while i < tokens.len() {
        match tokens[i].kind {
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            TokenKind::Semi if depth == 0 => return false,
            TokenKind::PreprocLine if depth == 0 => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

fn inj_peek_non_ws(tokens: &[Token<'_>], from: usize) -> usize {
    let mut j = from;
    while j < tokens.len() && matches!(tokens[j].kind, TokenKind::Whitespace | TokenKind::Newline) {
        j += 1;
    }
    j
}

/// Like `inj_peek_non_ws` but also skips block/line comments, so a `/* note */ {`
/// pattern is correctly recognized as an already-braced body.
fn inj_peek_non_ws_or_cmt(tokens: &[Token<'_>], from: usize) -> usize {
    let mut j = from;
    while j < tokens.len()
        && matches!(
            tokens[j].kind,
            TokenKind::Whitespace
                | TokenKind::Newline
                | TokenKind::CommentBlock
                | TokenKind::CommentLine
        )
    {
        j += 1;
    }
    j
}

/// Like `inj_copy_ws` but also copies block/line comments verbatim.
fn inj_copy_ws_or_cmt<'src>(tokens: &[Token<'src>], i: &mut usize, out: &mut Vec<Token<'src>>) {
    while *i < tokens.len()
        && matches!(
            tokens[*i].kind,
            TokenKind::Whitespace
                | TokenKind::Newline
                | TokenKind::CommentBlock
                | TokenKind::CommentLine
        )
    {
        out.push(tokens[*i].clone());
        *i += 1;
    }
}

/// Copy a balanced `(…)` group.  `tokens[*i]` must be `(`.
fn inj_copy_paren<'src>(tokens: &[Token<'src>], i: &mut usize, out: &mut Vec<Token<'src>>) {
    out.push(tokens[*i].clone());
    *i += 1;
    let mut depth = 1u32;
    while *i < tokens.len() {
        let t = tokens[*i].clone();
        out.push(t.clone());
        *i += 1;
        match t.kind {
            TokenKind::LParen => depth += 1,
            TokenKind::RParen => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }
}

/// Copy a balanced `{ … }` block, recursively applying brace injection inside.
fn inj_copy_block<'src>(
    tokens: &[Token<'src>],
    i: &mut usize,
    out: &mut Vec<Token<'src>>,
    config: &crate::config::Config,
) {
    out.push(tokens[*i].clone()); // `{`
    *i += 1;
    while *i < tokens.len() {
        if tokens[*i].kind == TokenKind::RBrace {
            out.push(tokens[*i].clone());
            *i += 1;
            break;
        }
        inj_item(tokens, i, out, config);
    }
}

/// Copy exactly one statement: a `{ }` block, a control-flow statement, or
/// tokens through the next `;` at brace-depth 0.
fn inj_copy_stmt<'src>(
    tokens: &[Token<'src>],
    i: &mut usize,
    out: &mut Vec<Token<'src>>,
    config: &crate::config::Config,
) {
    inj_copy_ws(tokens, i, out);
    if *i >= tokens.len() {
        return;
    }
    match tokens[*i].kind {
        TokenKind::LBrace => inj_copy_block(tokens, i, out, config),
        TokenKind::KwIf | TokenKind::KwFor | TokenKind::KwWhile | TokenKind::KwElse => {
            inj_item(tokens, i, out, config);
        }
        TokenKind::KwSwitch => {
            // `switch(cond) { body }` is a block-terminated statement.  Copy
            // keyword + condition parens + body block, then return.  The
            // default scanning arm would continue past the closing `}` and
            // absorb a following `else`, corrupting the source.
            out.push(tokens[*i].clone());
            *i += 1;
            inj_copy_ws(tokens, i, out);
            if *i < tokens.len() && tokens[*i].kind == TokenKind::LParen {
                inj_copy_paren(tokens, i, out);
            }
            inj_copy_ws_or_cmt(tokens, i, out);
            if *i < tokens.len() && tokens[*i].kind == TokenKind::LBrace {
                inj_copy_block(tokens, i, out, config);
            }
        }
        _ => {
            let mut depth = 0u32;
            while *i < tokens.len() {
                let t = tokens[*i].clone();
                match t.kind {
                    TokenKind::LBrace => depth += 1,
                    TokenKind::RBrace => {
                        if depth == 0 {
                            break;
                        }
                        depth -= 1;
                    }
                    TokenKind::Semi if depth == 0 => {
                        out.push(t);
                        *i += 1;
                        // Carry a trailing inline comment along with the statement
                        // so that `return -1; /* note */` stays together when
                        // braces are injected around the statement.
                        let j = {
                            let mut k = *i;
                            while k < tokens.len() && tokens[k].kind == TokenKind::Whitespace {
                                k += 1;
                            }
                            k
                        };
                        if j < tokens.len()
                            && matches!(
                                tokens[j].kind,
                                TokenKind::CommentBlock | TokenKind::CommentLine
                            )
                        {
                            while *i <= j {
                                out.push(tokens[*i].clone());
                                *i += 1;
                            }
                        }
                        return;
                    }
                    _ => {}
                }
                out.push(t);
                *i += 1;
            }
        }
    }
}

fn inj_handle_if<'src>(
    tokens: &[Token<'src>],
    i: &mut usize,
    out: &mut Vec<Token<'src>>,
    config: &crate::config::Config,
) {
    out.push(tokens[*i].clone()); // `if`
    *i += 1;
    inj_copy_ws(tokens, i, out);
    // Valid C/C++ `if` always has a `(` condition. If no `(` follows (e.g. `if`
    // used as a macro argument), don't inject braces.
    if *i >= tokens.len() || tokens[*i].kind != TokenKind::LParen {
        return;
    }
    inj_copy_paren(tokens, i, out);
    let j = inj_peek_non_ws_or_cmt(tokens, *i);
    if j < tokens.len() && tokens[j].kind == TokenKind::LBrace {
        // Already braced — still recurse inside so nested bodies are also handled.
        // Use comment-aware copy so trailing comments before `{` are preserved.
        inj_copy_ws_or_cmt(tokens, i, out);
        inj_copy_block(tokens, i, out, config);
    } else if j >= tokens.len() || tokens[j].kind == TokenKind::Semi {
        // Degenerate `if (cond);` — copy as-is.
        inj_copy_ws(tokens, i, out);
        if *i < tokens.len() {
            out.push(tokens[*i].clone());
            *i += 1;
        }
    } else if j < tokens.len() && tokens[j].kind == TokenKind::PreprocLine
        || inj_stmt_has_preproc(tokens, *i)
    {
        // Preprocessor directive between condition and body, or inside the body
        // — don't inject braces, since the added `}` would be unbalanced after
        // preprocessing.
        inj_copy_ws(tokens, i, out);
        inj_copy_stmt(tokens, i, out, config);
    } else {
        inj_copy_ws(tokens, i, out);
        out.push(inj_synthetic(TokenKind::LBrace, "{"));
        inj_copy_stmt(tokens, i, out, config);
        out.push(inj_synthetic(TokenKind::RBrace, "}"));
    }
    // Handle optional else.
    let j = inj_peek_non_ws(tokens, *i);
    if j < tokens.len() && tokens[j].kind == TokenKind::KwElse {
        inj_copy_ws(tokens, i, out);
        inj_handle_else(tokens, i, out, config);
    }
}

fn inj_handle_else<'src>(
    tokens: &[Token<'src>],
    i: &mut usize,
    out: &mut Vec<Token<'src>>,
    config: &crate::config::Config,
) {
    out.push(tokens[*i].clone()); // `else`
    *i += 1;
    inj_copy_ws(tokens, i, out);
    if *i >= tokens.len() {
        return;
    }
    match tokens[*i].kind {
        TokenKind::KwIf => inj_handle_if(tokens, i, out, config),
        TokenKind::LBrace => inj_copy_block(tokens, i, out, config),
        TokenKind::Semi => {
            out.push(tokens[*i].clone());
            *i += 1;
        }
        TokenKind::PreprocLine => {
            // Preprocessor directive between else and body — don't inject braces,
            // since the added `}` would be unbalanced after preprocessing.
            inj_copy_stmt(tokens, i, out, config);
        }
        _ if inj_stmt_has_preproc(tokens, *i - 1) => {
            // Body contains a preprocessor directive — skip brace injection.
            inj_copy_stmt(tokens, i, out, config);
        }
        _ => {
            out.push(inj_synthetic(TokenKind::LBrace, "{"));
            inj_copy_stmt(tokens, i, out, config);
            out.push(inj_synthetic(TokenKind::RBrace, "}"));
        }
    }
}

/// Handle `for (…) body` or `while (…) body` (not do-while terminator).
fn inj_handle_ctrl<'src>(
    tokens: &[Token<'src>],
    i: &mut usize,
    out: &mut Vec<Token<'src>>,
    config: &crate::config::Config,
) {
    out.push(tokens[*i].clone()); // keyword
    *i += 1;
    inj_copy_ws(tokens, i, out);
    if *i < tokens.len() && tokens[*i].kind == TokenKind::LParen {
        inj_copy_paren(tokens, i, out);
    }
    let j = inj_peek_non_ws_or_cmt(tokens, *i);
    if j >= tokens.len() || tokens[j].kind == TokenKind::LBrace || tokens[j].kind == TokenKind::Semi
    {
        // Already braced, or `;` (do-while terminator / empty loop) — copy as-is.
        inj_copy_ws_or_cmt(tokens, i, out);
        if *i < tokens.len() {
            inj_item(tokens, i, out, config);
        }
    } else if (j < tokens.len() && tokens[j].kind == TokenKind::PreprocLine)
        || inj_stmt_has_preproc(tokens, *i)
    {
        // Preprocessor directive between condition and body, or inside the body
        // — don't inject braces.
        inj_copy_ws(tokens, i, out);
        inj_copy_stmt(tokens, i, out, config);
    } else {
        inj_copy_ws(tokens, i, out);
        out.push(inj_synthetic(TokenKind::LBrace, "{"));
        inj_copy_stmt(tokens, i, out, config);
        out.push(inj_synthetic(TokenKind::RBrace, "}"));
    }
}

/// Dispatch one logical item from the token stream.
fn inj_item<'src>(
    tokens: &[Token<'src>],
    i: &mut usize,
    out: &mut Vec<Token<'src>>,
    config: &crate::config::Config,
) {
    if *i >= tokens.len() {
        return;
    }
    match tokens[*i].kind {
        TokenKind::KwIf if config.braces.add_braces_to_if => inj_handle_if(tokens, i, out, config),
        TokenKind::KwFor if config.braces.add_braces_to_for => {
            inj_handle_ctrl(tokens, i, out, config)
        }
        TokenKind::KwWhile if config.braces.add_braces_to_while => {
            inj_handle_ctrl(tokens, i, out, config)
        }
        TokenKind::KwElse if config.braces.add_braces_to_if => {
            inj_handle_else(tokens, i, out, config)
        }
        TokenKind::LBrace => inj_copy_block(tokens, i, out, config),
        _ => {
            out.push(tokens[*i].clone());
            *i += 1;
        }
    }
}

fn inject_braces_pass<'src>(
    tokens: &[Token<'src>],
    config: &crate::config::Config,
) -> Vec<Token<'src>> {
    let mut out = Vec::with_capacity(tokens.len() + 32);
    let mut i = 0;
    while i < tokens.len() {
        inj_item(tokens, &mut i, &mut out, config);
    }
    out
}

// ── Context ───────────────────────────────────────────────────────────────────

/// What opened the most recent `{`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BraceCtx {
    Block, // if/for/while/do/else/try/catch
    Type,  // struct/class/union/enum
    Namespace,
    Function, // function definition body
    Switch,   // switch statement body
    ExternC,  // extern "C" { } — no extra indentation
    Other,    // initializer list, lambda capture, etc.
}

struct Fmt<'src> {
    config: &'src Config,
    tokens: &'src [Token<'src>],
    pos: usize,

    output: String,
    /// True when the last character written was a newline.
    at_line_start: bool,
    indent_level: u32,
    /// Stack tracking what each `{` opened.
    brace_stack: Vec<BraceCtx>,
    /// Parallel to `brace_stack`: true when the corresponding `{` opened a
    /// flat large initializer that should be expanded one element per line.
    large_init_stack: Vec<bool>,
    /// Depth inside `(…)` — used to suppress newlines after `;` in for-headers.
    paren_depth: u32,
    /// Depth inside `[…]`.
    bracket_depth: u32,
    /// Pending blank lines to emit before the next meaningful token.
    blank_lines: u32,
    /// When true, the next Newline token seen in skip_ws was already emitted
    /// by the formatter (e.g. after `;` or `}`), so it must not be re-counted.
    skip_next_newline: bool,
    /// Set by `skip_ws()` to true when at least one `Whitespace` token was
    /// consumed between the previous token and the upcoming one.  Used by
    /// `SpaceOption::Preserve` to reproduce the source spacing.
    src_had_inline_ws: bool,
    /// The last non-whitespace, non-newline token kind we emitted.
    prev: Option<TokenKind>,
    /// Number of switch bodies we are currently inside — used to dedent case/default.
    switch_depth: u32,
    /// Per-switch stack tracking whether the current case body has added an
    /// extra indent level (only used when config.indent.indent_switch_case).
    case_body_stack: Vec<bool>,
    /// Current preprocessor #if nesting depth (used for pp_indent).
    pp_depth: u32,
    /// Stack of saved `indent_level` values at each `#if`/`#ifdef`/`#ifndef` entry.
    /// On `#else`/`#elif` we restore to the saved level so both branches start from
    /// the same code-brace depth; on `#endif` we pop without restoring (the last
    /// branch's accumulated depth is correct).
    pp_brace_stack: Vec<u32>,
    /// Number of open ternary `?` operators; used to detect ternary `:`.
    ternary_depth: u32,
    /// Set when `operator` keyword is emitted; cleared on the next non-ws token
    /// so that the overloaded operator symbol gets no surrounding space.
    after_operator_kw: bool,
    /// Set when an operator-overload symbol is emitted (i.e. `after_operator_kw`
    /// was true), so the following `(` is treated as a call paren (no space).
    last_was_operator_overload: bool,
    /// Number of class/struct/union bodies we are currently inside — used to dedent access specifiers.
    class_depth: u32,
    /// Set when `switch` keyword is emitted; cleared once its `{` is consumed.
    pending_switch: bool,
    /// Set when a type keyword (class/struct/union/enum) is emitted; cleared
    /// once its `{` is consumed. Needed because the `{` is often preceded by
    /// the type's name (an Ident), not the keyword itself.
    pending_type: bool,
    /// Set when `extern` is seen, kept through the following `LitStr` (`"C"`),
    /// consumed when `{` is reached to classify the block as `ExternC`.
    pending_extern_c: bool,
    /// Set when `case`/`default` keyword is emitted so the following `:` gets
    /// a newline after it instead of continuing on the same line.
    in_case_label: bool,
    /// Set after emitting a case/default label colon (`case X:` or `default:`).
    /// Used so that a `{` on the next line is treated as a block, not an
    /// initializer list.
    last_was_case_colon: bool,
    /// Set when `public`/`private`/`protected` is emitted so the following `:`
    /// gets a newline after it.
    in_access_label: bool,
    /// Set when a goto label identifier is emitted at column 0; causes the
    /// following `:` to emit a newline without applying normal spacing rules.
    in_goto_label: bool,
    /// When true, the next call to `space()` is suppressed and the flag is
    /// cleared. Used to suppress the space between a pointer `*`/`&` and the
    /// following identifier in `name` pointer-alignment mode.
    suppress_next_space: bool,
    /// Nesting depth inside template angle brackets `<…>`. Zero outside any
    /// template argument list.
    template_depth: u32,
    /// Set when the last non-whitespace token emitted was a template-closing `>`.
    /// Used to treat `>` like an identifier for call-paren spacing purposes.
    last_was_template_close: bool,
    /// Stack parallel to paren_depth: `true` if the corresponding `(` opened a
    /// C-style cast (next non-whitespace token inside was a type keyword).
    cast_paren_stack: Vec<bool>,
    /// Set when the last `)` closed a cast paren. Cleared by `set_prev`.
    last_was_cast_close: bool,
    /// Current output column (chars since last newline). Used to record
    /// opening-paren column so continuation params can be aligned.
    current_col: usize,
    /// Stack parallel to paren_depth: the column to align continuation lines
    /// to (i.e. the column right after the `(` was written).
    paren_col_stack: Vec<usize>,
    /// Parallel to paren_col_stack: when the `(` was the last non-whitespace
    /// on its line, stores `(true, col)` where `col` is the precomputed column
    /// that all continuation lines should start at.  `(false, 0)` means the
    /// `(` was not at EOL and column-alignment is used instead.
    paren_eol_stack: Vec<(bool, usize)>,
    /// Indentation column of the first non-whitespace token on the current
    /// line (set at the end of every `indent()` call, reset to 0 by `nl()`).
    line_indent_col: usize,
    /// Value of `paren_depth` at the time `indent()` was last called — used
    /// to count how many `(`s were opened on the current line.
    line_start_paren_depth: u32,
    /// Column to align continuation lines to after an `=` assignment operator.
    /// None when not inside an assignment RHS at statement level.
    assign_col: Option<usize>,
    /// True when the `=` that opened `assign_col` was the last non-whitespace
    /// on its line, meaning continuations use a regular indent.
    assign_eol: bool,

    // ── blank_line_after_var_decl_block state ─────────────────────────────────
    /// True while we are in the leading declaration run of a function body.
    in_var_decl_block: bool,
    /// True after a `;` at function scope; cleared when the next statement's
    /// first token is processed.
    at_func_stmt_start: bool,
    /// True once we have seen at least one declaration in the current function's
    /// leading block (prevents a spurious blank when the function opens with
    /// statements rather than declarations).
    saw_func_decl: bool,
    /// Set when the declaration run ends; causes flush_blank_lines to inject a
    /// blank line before the first non-declaration statement.
    force_blank_after_decls: bool,
    /// Set when `->` was emitted with `)` immediately before it (return type
    /// arrow).  Consumed by `needs_space` to add a space after return-type `->`.
    after_rparen_arrow: bool,
}

impl<'src> Fmt<'src> {
    fn new(config: &'src Config, tokens: &'src [Token<'src>]) -> Self {
        Self {
            config,
            tokens,
            pos: 0,
            output: String::with_capacity(4096),
            at_line_start: true,
            indent_level: 0,
            brace_stack: Vec::new(),
            large_init_stack: Vec::new(),
            paren_depth: 0,
            bracket_depth: 0,
            blank_lines: 0,
            skip_next_newline: false,
            src_had_inline_ws: false,
            prev: None,
            switch_depth: 0,
            case_body_stack: Vec::new(),
            pp_depth: 0,
            pp_brace_stack: Vec::new(),
            ternary_depth: 0,
            after_operator_kw: false,
            last_was_operator_overload: false,
            class_depth: 0,
            pending_switch: false,
            pending_type: false,
            pending_extern_c: false,
            in_case_label: false,
            last_was_case_colon: false,
            in_access_label: false,
            in_goto_label: false,
            suppress_next_space: false,
            template_depth: 0,
            last_was_template_close: false,
            cast_paren_stack: Vec::new(),
            last_was_cast_close: false,
            current_col: 0,
            paren_col_stack: Vec::new(),
            paren_eol_stack: Vec::new(),
            line_indent_col: 0,
            line_start_paren_depth: 0,
            assign_col: None,
            assign_eol: false,
            in_var_decl_block: false,
            at_func_stmt_start: false,
            saw_func_decl: false,
            force_blank_after_decls: false,
            after_rparen_arrow: false,
        }
    }

    // ── Navigation ───────────────────────────────────────────────────────────

    fn advance(&mut self) -> Option<&'src Token<'src>> {
        let t = self.tokens.get(self.pos)?;
        self.pos += 1;
        Some(t)
    }

    /// Skip whitespace/newline tokens, counting blank lines.
    ///
    /// When `skip_next_newline` is set, the first Newline token is silently
    /// dropped because the formatter already emitted a newline for it (e.g.
    /// the `\n` the formatter writes after `;` or `}`).
    fn skip_ws(&mut self) {
        self.src_had_inline_ws = false;
        let mut synthetic_consumed = false;
        while let Some(t) = self.tokens.get(self.pos) {
            match t.kind {
                TokenKind::Whitespace => {
                    self.src_had_inline_ws = true;
                    self.pos += 1;
                }
                TokenKind::Newline => {
                    self.pos += 1;
                    if self.skip_next_newline && !synthetic_consumed {
                        synthetic_consumed = true;
                    } else {
                        self.blank_lines += 1;
                    }
                }
                _ => break,
            }
        }
        self.skip_next_newline = false;
    }

    /// Return the kind of the next non-whitespace/newline token without consuming it.
    fn peek_non_ws_kind(&self) -> Option<TokenKind> {
        let mut j = self.pos;
        while j < self.tokens.len() {
            match self.tokens[j].kind {
                TokenKind::Whitespace | TokenKind::Newline => j += 1,
                k => return Some(k),
            }
        }
        None
    }

    // ── Output helpers ────────────────────────────────────────────────────────

    fn write(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.output.push_str(s);
        self.at_line_start = s.ends_with('\n');
        if let Some(pos) = s.rfind('\n') {
            self.current_col = s[pos + 1..].chars().count();
        } else {
            self.current_col += s.chars().count();
        }
    }

    fn nl(&mut self) {
        // Strip trailing spaces from the current line before emitting the newline.
        while self.output.ends_with(' ') {
            self.output.pop();
        }
        self.output.push_str(self.config.newline_str());
        self.at_line_start = true;
        self.suppress_next_space = false;
        self.current_col = 0;
        self.line_indent_col = 0;
    }

    fn indent(&mut self) {
        self.line_start_paren_depth = self.paren_depth;
        if self.paren_depth > 0 {
            self.align_to_paren();
        } else if self.assign_col.is_some() {
            self.align_to_assign();
        } else {
            let unit = self.config.indent_str();
            for _ in 0..self.indent_level {
                self.output.push_str(&unit);
                self.current_col += unit.len();
            }
            if self.indent_level > 0 {
                self.at_line_start = false;
            }
        }
        self.line_indent_col = self.current_col;
    }

    fn align_to_paren(&mut self) {
        // When `(` was the last non-whitespace before the newline, aligning to
        // its column would push continuation far right.  Instead we use the
        // precomputed `eol_col` stored in `paren_eol_stack`: that column is
        // `line_indent_col + parens_opened_on_that_line * indent_width`, which
        // matches uncrustify's behaviour for both simple and nested call sites.
        if let Some((eol, eol_col)) = self.paren_eol_stack.last_mut() {
            if !*eol && self.output.trim_end().ends_with('(') {
                *eol = true;
            }
            if *eol {
                let col = *eol_col;
                for _ in 0..col {
                    self.output.push(' ');
                }
                self.current_col = col;
                self.at_line_start = false;
                return;
            }
        }
        if let Some(&col) = self.paren_col_stack.last() {
            for _ in 0..col {
                self.output.push(' ');
            }
            self.current_col = col;
            if col > 0 {
                self.at_line_start = false;
            }
        }
    }

    fn align_to_assign(&mut self) {
        // When `=` was the last non-whitespace before the newline, aligning to
        // its column would leave no room for content. Use a regular indent instead.
        if !self.assign_eol && self.output.trim_end().ends_with('=') {
            self.assign_eol = true;
        }
        if self.assign_eol {
            let unit = self.config.indent_str();
            for _ in 0..=self.indent_level {
                self.output.push_str(&unit);
                self.current_col += unit.len();
            }
            self.at_line_start = false;
            return;
        }
        if let Some(col) = self.assign_col {
            for _ in 0..col {
                self.output.push(' ');
            }
            self.current_col = col;
            if col > 0 {
                self.at_line_start = false;
            }
        }
    }

    /// Update `self.prev` and clear the template-close flag in one step.
    /// Template-close arms must NOT use this — they set the two fields directly.
    fn set_prev(&mut self, kind: TokenKind) {
        self.prev = Some(kind);
        self.last_was_template_close = false;
        self.last_was_cast_close = false;
        // When `after_operator_kw` is true, we just emitted the overload symbol;
        // record that so the following `(` gets call-paren spacing.
        // For `operator[]` and `operator()`, the closing `]`/`)` must keep the
        // flag alive so that the parameter-list `(` is also treated as a call paren.
        self.last_was_operator_overload = self.after_operator_kw
            || (self.last_was_operator_overload
                && matches!(kind, TokenKind::RBracket | TokenKind::RParen));
        self.after_operator_kw = false;
        self.last_was_case_colon = false;
    }

    fn space(&mut self) {
        if self.suppress_next_space {
            self.suppress_next_space = false;
            return;
        }
        if !self.at_line_start && !self.output.ends_with(' ') {
            self.output.push(' ');
            self.current_col += 1;
        }
    }

    /// Emit pending blank lines, capped to `max_blank_lines`.
    fn flush_blank_lines(&mut self) {
        if self.force_blank_after_decls {
            self.force_blank_after_decls = false;
            // When not at line start (e.g. a trailing /* comment */ follows the
            // last decl), one nl() only terminates the current line; we need a
            // second nl() to produce an actual blank line.
            let min_lines = if self.at_line_start { 1 } else { 2 };
            if self.blank_lines < min_lines {
                self.blank_lines = min_lines;
            }
        }
        let max = self.config.newlines.max_blank_lines as u32;
        if max > 0 {
            let emit = self.blank_lines.min(max);
            for _ in 0..emit {
                self.nl();
            }
        }
        self.blank_lines = 0;
    }

    /// True for token kinds that can open a variable/type declaration at
    /// function scope. Used by blank_line_after_var_decl_block.
    fn is_decl_start(kind: TokenKind) -> bool {
        matches!(
            kind,
            TokenKind::Keyword
                | TokenKind::KwStruct
                | TokenKind::KwClass
                | TokenKind::KwUnion
                | TokenKind::KwEnum
                | TokenKind::KwTypename
                | TokenKind::KwTypedef
        )
    }

    /// True when we're at statement start and the current token is an Ident that
    /// begins a user-defined-type declaration (`TypeName varName;`).  In C/C++,
    /// `ident ident` at statement scope is always a declaration — there is no
    /// non-declaration statement in that form.  Scans ahead skipping WS and any
    /// leading `*`/`&` (pointer/reference declarators) to find the variable name.
    fn ident_starts_decl(&self) -> bool {
        // Scan forward to decide if this Ident is the start of a declaration.
        // Patterns we accept:
        //   TypeName varName         — user typedef: Ident Ident
        //   ATTR_MACRO const T* var  — attribute macro + qualifiers: Ident Keyword+ Ident
        // We stop at newlines because declarations are always on one line.
        let mut i = self.pos;
        loop {
            let Some(tk) = self.tokens.get(i) else {
                return false;
            };
            match tk.kind {
                // Stop at newlines: `TypeName varName` declarations are always
                // on one line. Skipping newlines would misidentify adjacent macro
                // calls (e.g. `EXPECT_ABORT_BEGIN\n    TEST_ASSERT(...)`) as decls.
                TokenKind::Newline => return false,
                TokenKind::Whitespace | TokenKind::Star => i += 1,
                // A bare Ident following is the variable name — declaration confirmed.
                TokenKind::Ident => return true,
                // A Keyword (const, volatile, unsigned, etc.) after the leading Ident
                // means this is an attribute-macro + qualifier pattern like
                // `UNITY_PTR_ATTRIBUTE const float* p` — keep scanning.
                TokenKind::Keyword
                | TokenKind::KwStruct
                | TokenKind::KwClass
                | TokenKind::KwUnion
                | TokenKind::KwEnum
                | TokenKind::KwTypename => i += 1,
                _ => return false,
            }
        }
    }

    /// Emit the newline/space after a `}` based on what follows.
    /// Called from both the RBrace arm and the LBrace empty-body collapse path.
    fn emit_post_brace_spacing(
        &mut self,
        ctx: BraceCtx,
        next_kind: Option<TokenKind>,
        source_line: u32,
    ) {
        let semi_follows = next_kind == Some(TokenKind::Semi);
        let typedef_name =
            matches!(ctx, BraceCtx::Type) && matches!(next_kind, Some(TokenKind::Ident));
        let cuddle = match next_kind {
            Some(TokenKind::KwElse) => {
                self.config.braces.cuddle_else && !self.config.newlines.nl_brace_else
            }
            Some(TokenKind::KwCatch) => self.config.braces.cuddle_catch,
            Some(TokenKind::KwWhile) => matches!(ctx, BraceCtx::Block),
            _ => false,
        };

        if semi_follows {
            // `;` will be written by the Semi arm directly.
        } else if typedef_name || (cuddle && matches!(self.config.braces.style, BraceStyle::Kr)) {
            self.space();
            self.skip_next_newline = true;
        } else if cuddle
            && matches!(self.config.braces.style, BraceStyle::Stroustrup)
            && next_kind == Some(TokenKind::KwElse)
        {
            self.nl();
            self.skip_next_newline = true;
        } else if self.peek_inline_comment(source_line) {
            // inline comment — let CommentLine close the line
        } else {
            self.nl();
            self.skip_next_newline = true;
        }
    }

    /// Called once per token, before the token is written, to advance the
    /// blank_line_after_var_decl_block state machine.
    fn check_var_decl_transition(&mut self, kind: TokenKind) {
        if !self.config.newlines.blank_line_after_var_decl_block {
            return;
        }
        if !self.at_func_stmt_start || !self.in_var_decl_block {
            return;
        }
        // Comments are handled by two rules:
        //   1. Inline trailing comments (not at line start) are always transparent.
        //   2. Standalone comments (at line start) are transparent when they are
        //      *between* declarations — i.e. more declarations follow — so that
        //      section comments like `/* WHEN ... */` don't split the block.
        //      A standalone comment ends the block only when it precedes code.
        if matches!(kind, TokenKind::CommentLine | TokenKind::CommentBlock) {
            if !self.at_line_start {
                return; // inline trailing comment — never ends the block
            }
            if !self.saw_func_decl {
                return; // before any declaration — preamble comment is transparent
            }
            if self.comment_precedes_decl() {
                return; // comment sits between declarations — transparent
            }
            // Otherwise fall through: standalone comment before code ends the block.
        }
        self.at_func_stmt_start = false;
        let is_decl =
            Self::is_decl_start(kind) || (kind == TokenKind::Ident && self.ident_starts_decl());
        if is_decl {
            self.saw_func_decl = true;
        } else {
            self.in_var_decl_block = false;
            if self.saw_func_decl {
                self.force_blank_after_decls = true;
            }
        }
    }

    /// Ensure we're at the start of a fresh line (emit newline + indent if not).
    fn ensure_own_line(&mut self) {
        if !self.at_line_start {
            self.nl();
        }
        self.indent();
    }

    /// For KR/Stroustrup enforcement: trim all trailing whitespace from the
    /// output so the next token can be appended to the end of the previous
    /// content line (e.g. `if (cond)\n    {` → `if (cond) {`).
    fn trim_to_prev_line_end(&mut self) {
        let new_len = self
            .output
            .trim_end_matches(|c: char| c.is_ascii_whitespace())
            .len();
        self.output.truncate(new_len);
        if let Some(pos) = self.output.rfind('\n') {
            self.current_col = self.output[pos + 1..].chars().count();
        } else {
            self.current_col = self.output.chars().count();
        }
        self.at_line_start = false;
    }

    // ── Cast detection ────────────────────────────────────────────────────────

    /// True when the token immediately before the current `(` is a sizeof-like
    /// operator keyword — meaning the `(` is NOT a cast paren.
    fn prev_is_sizeof_like(&self) -> bool {
        // self.pos is one past `(`; self.pos-1 is `(` itself; scan from self.pos-2.
        if self.pos < 2 {
            return false;
        }
        let mut i = self.pos - 2;
        loop {
            match self.tokens[i].kind {
                TokenKind::Whitespace | TokenKind::Newline => {
                    if i == 0 {
                        return false;
                    }
                    i -= 1;
                }
                _ => {
                    let tok = &self.tokens[i];
                    // Any identifier before `(` is a function call, never a cast.
                    if tok.kind == TokenKind::Ident {
                        return true;
                    }
                    return matches!(
                        tok.lexeme,
                        "sizeof" | "alignof" | "alignas" | "__alignof__" | "decltype" | "typeid"
                    );
                }
            }
        }
    }

    /// True if the tokens from `self.pos` up to and including a matching `)`
    /// look like a C-style cast type: optional cv/elaborated-type keywords,
    /// followed by exactly one type keyword or identifier, followed by zero or
    /// more `*`/`&`, followed by `)`.  Also accepts user-defined type names
    /// (bare `Ident`), not just built-in keywords.
    fn next_is_type_kw(&self) -> bool {
        let mut i = self.pos;
        let skip_ws = |mut j: usize| -> usize {
            while j < self.tokens.len()
                && matches!(
                    self.tokens[j].kind,
                    TokenKind::Whitespace | TokenKind::Newline
                )
            {
                j += 1;
            }
            j
        };
        i = skip_ws(i);

        // Accept any number of qualifier/struct/class/… keywords before the
        // core type name — but we need at least one type-like token overall.
        let mut saw_type = false;
        while i < self.tokens.len() {
            let k = self.tokens[i].kind;
            if Self::is_decl_start(k) {
                saw_type = true;
                i += 1;
                i = skip_ws(i);
            } else if k == TokenKind::Ident {
                // User-defined type name (e.g. MyStruct, size_t, uint32_t).
                saw_type = true;
                i += 1;
                i = skip_ws(i);
                break; // only one ident in a cast type
            } else {
                break;
            }
        }

        if !saw_type {
            return false;
        }

        // Optional pointer / reference decorators
        while i < self.tokens.len()
            && matches!(self.tokens[i].kind, TokenKind::Star | TokenKind::Amp)
        {
            i += 1;
            i = skip_ws(i);
        }

        // Must end with `)`
        matches!(self.tokens.get(i).map(|t| t.kind), Some(TokenKind::RParen))
    }

    // ── Inline-comment detection ──────────────────────────────────────────────

    /// True if the tokens from `self.pos` match the function-pointer declarator
    /// pattern `(*Name)` or `(&Name)` — i.e. `*`/`&` then an identifier then `)`.
    ///
    /// This distinguishes `void (*Fn)(int)` from `memset(&data, 0, n)` where the
    /// `(` is merely followed by an address-of expression, not a declarator.
    fn next_is_fn_ptr_declarator(&self) -> bool {
        let skip_ws = |mut j: usize| -> usize {
            while j < self.tokens.len()
                && matches!(
                    self.tokens[j].kind,
                    TokenKind::Whitespace | TokenKind::Newline
                )
            {
                j += 1;
            }
            j
        };
        let mut i = skip_ws(self.pos);
        // Must start with * or & (function-pointer or C++ reference-to-function).
        if !matches!(
            self.tokens.get(i).map(|t| t.kind),
            Some(TokenKind::Star | TokenKind::Amp)
        ) {
            return false;
        }
        i += 1;
        i = skip_ws(i);
        // Then an identifier (the function-pointer/reference name)
        if !matches!(self.tokens.get(i).map(|t| t.kind), Some(TokenKind::Ident)) {
            return false;
        }
        i += 1;
        i = skip_ws(i);
        // Then `)` …
        if !matches!(self.tokens.get(i).map(|t| t.kind), Some(TokenKind::RParen)) {
            return false;
        }
        i += 1;
        i = skip_ws(i);
        // … immediately followed by `(` (the parameter list).
        // This distinguishes `void (*fp)(int)` from a call like `foo(&x)` where
        // `)` closes the argument list and is followed by `;`, `,`, `)`, etc.
        matches!(self.tokens.get(i).map(|t| t.kind), Some(TokenKind::LParen))
    }

    /// True if the next token (skipping only `Whitespace`, not `Newline`) is a
    /// `CommentLine` or `CommentBlock` whose source line matches `source_line`.
    fn peek_inline_comment(&self, source_line: u32) -> bool {
        let mut i = self.pos;
        while i < self.tokens.len() && self.tokens[i].kind == TokenKind::Whitespace {
            i += 1;
        }
        matches!(
            self.tokens.get(i),
            Some(t) if matches!(t.kind, TokenKind::CommentLine | TokenKind::CommentBlock)
                && t.span.line == source_line
        )
    }

    /// True when the next non-whitespace token is a `CommentLine` on `source_line`.
    fn peek_inline_line_comment(&self, source_line: u32) -> bool {
        let mut i = self.pos;
        while i < self.tokens.len() && self.tokens[i].kind == TokenKind::Whitespace {
            i += 1;
        }
        matches!(
            self.tokens.get(i),
            Some(t) if t.kind == TokenKind::CommentLine && t.span.line == source_line
        )
    }

    /// Scans forward from `self.pos` skipping whitespace, newlines, and any
    /// comments, then returns true when the first real token is a declaration
    /// start.  Used to decide whether a standalone comment between declarations
    /// is transparent (followed by another declaration) or ends the var-decl
    /// block (followed by code).
    fn comment_precedes_decl(&self) -> bool {
        let mut i = self.pos;
        loop {
            let Some(tk) = self.tokens.get(i) else {
                return false;
            };
            match tk.kind {
                TokenKind::Whitespace | TokenKind::Newline => i += 1,
                TokenKind::CommentLine | TokenKind::CommentBlock => i += 1,
                kind => {
                    return Self::is_decl_start(kind)
                        || (kind == TokenKind::Ident && {
                            // Temporarily advance past this ident to run ident_starts_decl
                            // logic inline (we can't call self.ident_starts_decl() since it
                            // reads from self.pos).
                            let mut j = i + 1;
                            loop {
                                let Some(t2) = self.tokens.get(j) else {
                                    break false;
                                };
                                match t2.kind {
                                    TokenKind::Newline => break false,
                                    TokenKind::Whitespace | TokenKind::Star => j += 1,
                                    TokenKind::Ident => break true,
                                    TokenKind::Keyword
                                    | TokenKind::KwStruct
                                    | TokenKind::KwClass
                                    | TokenKind::KwUnion
                                    | TokenKind::KwEnum
                                    | TokenKind::KwTypename => j += 1,
                                    _ => break false,
                                }
                            }
                        });
                }
            }
        }
    }

    // ── Small initializer detection ───────────────────────────────────────────

    /// Scans forward from `self.pos` (the token immediately after `{`) looking
    /// for the matching `}`.  Returns `Some(rbrace_index)` when the initializer
    /// has no nested braces and contains at most 16 non-whitespace tokens, so
    /// it can safely be kept on a single line.  Returns `None` otherwise.
    fn small_initializer_end(&self) -> Option<usize> {
        const MAX_TOKENS: usize = 16;
        let mut count = 0;
        for (offset, tk) in self.tokens[self.pos..].iter().enumerate() {
            match tk.kind {
                // Nested brace or source newline: not a small single-line init.
                TokenKind::LBrace | TokenKind::Newline => return None,
                TokenKind::RBrace => return Some(self.pos + offset),
                TokenKind::Whitespace => {}
                _ => {
                    count += 1;
                    if count > MAX_TOKENS {
                        return None;
                    }
                }
            }
        }
        None
    }

    /// Scans forward from `self.pos` looking for the matching `}`.
    /// Returns `Some(rbrace_index)` only when the initializer is flat (no
    /// nested `{`) and written on a single source line (no `Newline` tokens).
    /// Returns `None` for multi-line or nested initializers so that the source
    /// grouping is preserved instead of being blown out one-element-per-line.
    fn large_flat_initializer_end(&self) -> Option<usize> {
        for (offset, tk) in self.tokens[self.pos..].iter().enumerate() {
            match tk.kind {
                TokenKind::LBrace | TokenKind::Newline => return None,
                TokenKind::RBrace => return Some(self.pos + offset),
                _ => {}
            }
        }
        None
    }

    // ── Brace context inference ───────────────────────────────────────────────

    /// Returns the effective previous token kind for brace-context inference,
    /// looking through any trailing comments so that `if (cond) /* note */ {`
    /// is classified the same as `if (cond) {`.
    fn prev_through_comments(&self) -> Option<TokenKind> {
        if !matches!(
            self.prev,
            Some(TokenKind::CommentBlock | TokenKind::CommentLine | TokenKind::PreprocLine)
        ) {
            return self.prev;
        }
        // self.pos is one past the LBrace; self.pos-1 is LBrace.  Scan backward.
        if self.pos < 2 {
            return None;
        }
        let mut i = self.pos - 2;
        loop {
            match self.tokens[i].kind {
                TokenKind::Whitespace
                | TokenKind::Newline
                | TokenKind::CommentBlock
                | TokenKind::CommentLine
                | TokenKind::PreprocLine => {
                    if i == 0 {
                        return None;
                    }
                    i -= 1;
                }
                k => return Some(k),
            }
        }
    }

    /// Returns `true` when the `RParen` immediately before `{` looks like it
    /// closes a macro/function *call* at statement level rather than a function
    /// *definition* parameter list.
    ///
    /// Heuristic: find the `(` that matches the `)`, then the identifier before
    /// it, then look one step further.  If what precedes the identifier is a
    /// statement boundary (`{`, `}`, `;`, preprocline) we are at the start of a
    /// statement with no return-type — this is a macro call, not a definition.
    /// If it is a type keyword, another identifier (return type), `*`, `&`, or
    /// `>` (template close), it is a function definition.
    ///
    /// The check is skipped when we are inside a class body (`class_depth > 0`)
    /// because constructors and inline methods have no return type yet still
    /// qualify as definitions.
    fn rparen_looks_like_call(&self) -> bool {
        if self.class_depth > 0 {
            return false; // inside a class: always a definition
        }
        if self.pos < 2 {
            return false;
        }
        // Start one token before the current LBrace.
        let mut i = self.pos - 2;
        // Skip whitespace / comments between ) and {.
        while matches!(
            self.tokens[i].kind,
            TokenKind::Whitespace
                | TokenKind::Newline
                | TokenKind::CommentBlock
                | TokenKind::CommentLine
        ) {
            if i == 0 {
                return false;
            }
            i -= 1;
        }
        if self.tokens[i].kind != TokenKind::RParen {
            return false;
        }
        // Find the matching `(`.
        let mut depth = 1usize;
        loop {
            if i == 0 {
                return false;
            }
            i -= 1;
            match self.tokens[i].kind {
                TokenKind::RParen => depth += 1,
                TokenKind::LParen => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        // i = the `(`.  Find the name token immediately before it.
        while i > 0 {
            i -= 1;
            if !matches!(
                self.tokens[i].kind,
                TokenKind::Whitespace | TokenKind::Newline
            ) {
                break;
            }
        }
        // i = the function/macro name token.
        // Now scan backward past the qualified name (ColonColon, Tilde) to find
        // the token that precedes the entire name.
        loop {
            if i == 0 {
                return false; // start of file without return type: treat as fn def
            }
            i -= 1;
            match self.tokens[i].kind {
                TokenKind::Whitespace | TokenKind::Newline => continue,
                // Parts of a qualified/scoped name — keep scanning.
                TokenKind::ColonColon | TokenKind::Tilde => continue,
                // A return type — this is a function definition.
                TokenKind::Ident
                | TokenKind::Keyword
                | TokenKind::KwStruct
                | TokenKind::KwClass
                | TokenKind::KwUnion
                | TokenKind::KwEnum
                | TokenKind::KwTypename
                | TokenKind::Star
                | TokenKind::Amp
                | TokenKind::Gt
                | TokenKind::RParen => return false,
                // Statement boundary with no preceding return type — macro call.
                TokenKind::Semi
                | TokenKind::LBrace
                | TokenKind::RBrace
                | TokenKind::PreprocLine => {
                    return true;
                }
                _ => return false, // conservative: treat unknown context as fn def
            }
        }
    }

    /// Returns `true` when the `RParen` immediately before `{` closes a
    /// control-flow construct (`if`, `while`, `for`, `switch`) rather than a
    /// function parameter list.  Scans backward past the `)…(` group and checks
    /// the keyword before the `(`.
    fn rparen_closes_ctrl_flow(&self) -> bool {
        if self.pos < 2 {
            return false;
        }
        let mut i = self.pos - 2; // token before LBrace
        // skip whitespace / comments between ) and {
        while matches!(
            self.tokens[i].kind,
            TokenKind::Whitespace
                | TokenKind::Newline
                | TokenKind::CommentBlock
                | TokenKind::CommentLine
        ) {
            if i == 0 {
                return false;
            }
            i -= 1;
        }
        if self.tokens[i].kind != TokenKind::RParen {
            return false;
        }
        // find the matching '('
        let mut depth = 1usize;
        loop {
            if i == 0 {
                return false;
            }
            i -= 1;
            match self.tokens[i].kind {
                TokenKind::RParen => depth += 1,
                TokenKind::LParen => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        // skip whitespace before '('
        while i > 0 {
            i -= 1;
            if !matches!(
                self.tokens[i].kind,
                TokenKind::Whitespace | TokenKind::Newline
            ) {
                break;
            }
        }
        matches!(
            self.tokens[i].kind,
            TokenKind::KwIf | TokenKind::KwWhile | TokenKind::KwFor | TokenKind::KwSwitch
        )
    }

    fn infer_brace_ctx(&self) -> BraceCtx {
        let prev = match self.prev_through_comments() {
            Some(k) => k,
            None => return BraceCtx::Other,
        };
        match prev {
            TokenKind::LitStr if self.pending_extern_c => BraceCtx::ExternC,
            TokenKind::KwNamespace => BraceCtx::Namespace,
            TokenKind::KwStruct | TokenKind::KwClass | TokenKind::KwUnion | TokenKind::KwEnum => {
                BraceCtx::Type
            }
            TokenKind::RParen => {
                if self.pending_switch {
                    BraceCtx::Switch
                } else if self.rparen_closes_ctrl_flow() || self.rparen_looks_like_call() {
                    BraceCtx::Block
                } else {
                    BraceCtx::Function
                }
            }
            TokenKind::KwElse | TokenKind::KwDo | TokenKind::KwTry => BraceCtx::Block,
            // A `{` that immediately follows a preprocessor directive is a block
            // brace (else body, function body guarded by #if, etc.), not an initializer.
            TokenKind::PreprocLine => BraceCtx::Block,
            TokenKind::Ident | TokenKind::Gt => {
                // Ident: could be a named type `class Foo {` or a function body.
                // Gt: template specialization `class Foo<T> {`.
                if self.pending_type {
                    BraceCtx::Type
                } else {
                    BraceCtx::Function
                }
            }
            TokenKind::Colon => {
                // `class Foo : public Bar {` — colon ends the base-class list.
                if self.pending_type {
                    BraceCtx::Type
                } else if self.last_was_case_colon {
                    // `case X: {` or `default: {` — block following a case label.
                    BraceCtx::Block
                } else {
                    BraceCtx::Other
                }
            }
            // After `=`, `(`, `,`, `{` → initializer-list style
            TokenKind::Eq
            | TokenKind::PlusEq
            | TokenKind::MinusEq
            | TokenKind::LParen
            | TokenKind::LBracket
            | TokenKind::LBrace
            | TokenKind::Comma => BraceCtx::Other,
            _ => BraceCtx::Other,
        }
    }

    // ── Pointer/reference declarator detection ────────────────────────────────

    /// Heuristic: a `*` or `&` is a declarator (not multiplication/address-of)
    /// when the preceding non-whitespace token is a definite type-introducing
    /// token: a type keyword, another `*`/`&` (chained pointers), `)` (cast or
    /// function-pointer return type), `>` (template instantiation), or
    /// `typename`/`struct`/`class`/`union`/`enum`.
    fn is_ptr_decl_context(&self) -> bool {
        if matches!(
            self.prev,
            Some(
                TokenKind::Keyword
                    | TokenKind::KwStruct
                    | TokenKind::KwClass
                    | TokenKind::KwUnion
                    | TokenKind::KwEnum
                    | TokenKind::KwTypename
                    | TokenKind::Star
                    | TokenKind::Amp
                    | TokenKind::Gt
            )
        ) {
            return true;
        }
        // `)` followed by `*`/`&`: never a pointer declarator after a cast.
        // `(type)*ptr` is cast + dereference; `(type*)` has the `*` inside.
        // Only non-cast `)` (e.g. function return type in a fn-ptr context)
        // could introduce a declarator, but those are handled by the
        // fn-ptr-declarator check in the LParen arm, not here.
        if self.prev == Some(TokenKind::RParen) {
            return false;
        }
        // An identifier (user-defined type) followed by `*`/`&` is a pointer
        // declarator only when the tokens after the operator look like a name,
        // not an expression.  Heuristic: skip consecutive `*`/`&`/whitespace,
        // then require an identifier or keyword followed by a declaration-ending
        // token (`;`, `,`, `)`, `=`, `[`, `{`).
        if self.prev == Some(TokenKind::Ident) {
            return self.star_after_ident_is_decl();
        }
        false
    }

    fn star_after_ident_is_decl(&self) -> bool {
        // ── backward check ───────────────────────────────────────────────────
        // Scan back past the `*` and the preceding Ident to find the token
        // that appeared before the type-name.  If that token unambiguously
        // belongs to an expression (assignment, arithmetic, comparison, …)
        // then `*` is multiplication, not a pointer declarator.
        //
        // self.pos is one past the `*` token.
        if self.pos >= 1 {
            let mut b = self.pos - 1; // index of the `*`
            // skip the `*` itself and any whitespace/newlines before the Ident
            while b > 0
                && matches!(
                    self.tokens[b].kind,
                    TokenKind::Whitespace | TokenKind::Newline | TokenKind::Star | TokenKind::Amp
                )
            {
                b -= 1;
            }
            // b should now be the Ident; step past it
            if b > 0 && self.tokens[b].kind == TokenKind::Ident {
                b -= 1;
            }
            // skip whitespace before the Ident
            while b > 0
                && matches!(
                    self.tokens[b].kind,
                    TokenKind::Whitespace | TokenKind::Newline
                )
            {
                b -= 1;
            }
            // If the token before the Ident is an expression operator, this is
            // multiplication, not a declaration.
            let before = self.tokens[b].kind;
            let is_expr_op = matches!(
                before,
                TokenKind::Eq          // assignment: r = a * b
                    | TokenKind::Plus
                    | TokenKind::Minus
                    | TokenKind::Slash
                    | TokenKind::Percent
                    | TokenKind::Pipe
                    | TokenKind::Caret
                    | TokenKind::LtLt
                    | TokenKind::GtGt
                    | TokenKind::EqEq
                    | TokenKind::BangEq
                    | TokenKind::Lt
                    | TokenKind::LtEq
                    | TokenKind::GtEq
                    | TokenKind::AmpAmp
                    | TokenKind::PipePipe
                    | TokenKind::Question
                    | TokenKind::PlusEq   // compound assignments
                    | TokenKind::MinusEq
                    | TokenKind::StarEq
                    | TokenKind::SlashEq
                    | TokenKind::PercentEq
                    | TokenKind::AmpEq
                    | TokenKind::PipeEq
                    | TokenKind::CaretEq
                    | TokenKind::LtLtEq
                    | TokenKind::GtGtEq
                    | TokenKind::Arrow    // member access: p->field & MASK
                    | TokenKind::Dot      // member access: s.field & MASK
                    | TokenKind::ArrowStar
                    | TokenKind::DotStar
            ) || matches!(
                before,
                TokenKind::KwReturn | TokenKind::KwCase | TokenKind::KwThrow
            )
                // `(` preceded by an expression op means we're in an expression subgroup
                || (before == TokenKind::LParen && b > 0 && {
                    let mut bb = b - 1;
                    while bb > 0
                        && matches!(
                            self.tokens[bb].kind,
                            TokenKind::Whitespace | TokenKind::Newline
                        )
                    {
                        bb -= 1;
                    }
                    let outer = self.tokens[bb].kind;
                    matches!(
                        outer,
                        TokenKind::Eq
                            | TokenKind::Plus
                            | TokenKind::Minus
                            | TokenKind::Slash
                            | TokenKind::Percent
                            | TokenKind::Pipe
                            | TokenKind::Caret
                            | TokenKind::LtLt
                            | TokenKind::GtGt
                            | TokenKind::EqEq
                            | TokenKind::BangEq
                            | TokenKind::Lt
                            | TokenKind::LtEq
                            | TokenKind::GtEq
                            | TokenKind::AmpAmp
                            | TokenKind::PipePipe
                            | TokenKind::PlusEq
                            | TokenKind::MinusEq
                            | TokenKind::StarEq
                            | TokenKind::SlashEq
                            | TokenKind::PercentEq
                            | TokenKind::AmpEq
                            | TokenKind::PipeEq
                            | TokenKind::CaretEq
                            | TokenKind::LtLtEq
                            | TokenKind::GtGtEq
                            | TokenKind::Comma
                            | TokenKind::LParen
                            | TokenKind::Bang   // !(key_flag & MASK)
                            | TokenKind::Tilde  // ~(key_flag & MASK)
                            | TokenKind::KwIf
                            | TokenKind::KwWhile
                            | TokenKind::KwFor
                            | TokenKind::KwSwitch
                            | TokenKind::KwReturn
                            | TokenKind::KwCase
                            | TokenKind::KwThrow
                    )
                });
            if is_expr_op {
                return false;
            }
        }

        // ── forward check ────────────────────────────────────────────────────
        // After the `*`, the tokens must look like a declarator name, not an
        // expression operand.
        let mut i = self.pos;
        // skip additional pointer/ref operators and whitespace
        while i < self.tokens.len() {
            match self.tokens[i].kind {
                TokenKind::Star | TokenKind::Amp | TokenKind::Whitespace | TokenKind::Newline => {
                    i += 1;
                }
                _ => break,
            }
        }
        // skip optional `const`/`volatile`/`restrict` after the stars
        while i < self.tokens.len()
            && matches!(self.tokens[i].kind, TokenKind::Keyword)
            && matches!(
                self.tokens[i].lexeme,
                "const" | "volatile" | "restrict" | "__restrict" | "__restrict__"
            )
        {
            i += 1;
            while i < self.tokens.len()
                && matches!(
                    self.tokens[i].kind,
                    TokenKind::Whitespace | TokenKind::Newline
                )
            {
                i += 1;
            }
        }
        // skip a qualified name (Ident :: Ident ...)
        let mut found_name = false;
        let mut last_was_operator_kw = false;
        while i < self.tokens.len() {
            match self.tokens[i].kind {
                TokenKind::Ident => {
                    last_was_operator_kw = false;
                    found_name = true;
                    i += 1;
                }
                TokenKind::Keyword => {
                    // Only `operator` can appear as (part of) a declarator name;
                    // all other keywords (sizeof, return, …) indicate an expression.
                    if self.tokens[i].lexeme != "operator" {
                        break;
                    }
                    last_was_operator_kw = true;
                    found_name = true;
                    i += 1;
                }
                TokenKind::ColonColon => {
                    last_was_operator_kw = false;
                    i += 1;
                }
                TokenKind::Whitespace | TokenKind::Newline | TokenKind::PreprocLine => {
                    i += 1;
                }
                _ => break,
            }
        }
        if !found_name {
            return false;
        }
        // If the name ended with `operator`, skip the overloaded symbol so the
        // following `(` is seen as the declaration terminator.
        if last_was_operator_kw {
            while i < self.tokens.len()
                && matches!(
                    self.tokens[i].kind,
                    TokenKind::Whitespace | TokenKind::Newline
                )
            {
                i += 1;
            }
            // Skip one operator-symbol token (=, +=, ==, [], (), etc.)
            let op_kind = self.tokens.get(i).map(|t| t.kind);
            if op_kind.is_some_and(|k| {
                k.is_binary_op()
                    || matches!(
                        k,
                        TokenKind::PlusPlus
                            | TokenKind::MinusMinus
                            | TokenKind::Bang
                            | TokenKind::Tilde
                            | TokenKind::LParen
                            | TokenKind::LBracket
                    )
            }) {
                i += 1;
                // For `operator()` and `operator[]` also skip the closing bracket.
                let closing = self.tokens.get(i).map(|t| t.kind);
                if matches!(closing, Some(TokenKind::RParen | TokenKind::RBracket)) {
                    i += 1;
                }
            }
        }
        while i < self.tokens.len()
            && matches!(
                self.tokens[i].kind,
                TokenKind::Whitespace
                    | TokenKind::Newline
                    | TokenKind::PreprocLine
                    | TokenKind::CommentBlock
                    | TokenKind::CommentLine
            )
        {
            i += 1;
        }
        // declaration-terminating tokens
        matches!(
            self.tokens.get(i).map(|t| t.kind),
            Some(
                TokenKind::Semi
                    | TokenKind::Comma
                    | TokenKind::RParen
                    | TokenKind::Eq
                    | TokenKind::LBracket
                    | TokenKind::LBrace
                    | TokenKind::LParen // function-pointer: Type (*fn)(...)
            )
        )
    }

    // ── Template angle-bracket detection ─────────────────────────────────────

    /// Returns true when the `<` just consumed looks like the opening of a
    /// template argument list rather than a less-than comparison.
    ///
    /// Scans forward from `self.pos` (the token immediately after `<`).
    /// Only tokens that can legally appear in a template argument list are
    /// permitted; the first unexpected token causes an early `false` return.
    fn looks_like_template_open(&self) -> bool {
        let mut i = self.pos;
        let mut depth: u32 = 1;
        let mut scanned = 0u32;
        while i < self.tokens.len() && scanned < 256 {
            scanned += 1;
            match self.tokens[i].kind {
                // Whitespace is irrelevant to the heuristic.
                TokenKind::Whitespace | TokenKind::Newline => {}
                // Type-like content: names, scoping, pointer/ref modifiers,
                // separators, and non-type literal parameters.
                TokenKind::Ident
                | TokenKind::Keyword
                | TokenKind::KwStruct
                | TokenKind::KwClass
                | TokenKind::KwUnion
                | TokenKind::KwEnum
                | TokenKind::KwTemplate
                | TokenKind::KwTypename
                | TokenKind::KwUsing
                | TokenKind::ColonColon
                | TokenKind::Star
                | TokenKind::Amp
                | TokenKind::Comma
                | TokenKind::LitInt
                | TokenKind::LitFloat
                | TokenKind::DotDotDot => {}
                // Nested `<`: bump depth.
                TokenKind::Lt => {
                    depth += 1;
                }
                // `>`: pop one level; if we've returned to zero it's the match.
                TokenKind::Gt => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                    if depth == 0 {
                        return true;
                    }
                }
                // `>>` closes two nesting levels (C++11 `vector<vector<int>>`).
                // When depth == 1 the second `>` belongs to the outer context,
                // but this `<` is still a valid template open.
                TokenKind::GtGt => {
                    if depth <= 2 {
                        return true;
                    }
                    depth -= 2;
                }
                // Anything else (operators, parens, braces, …) means this is
                // an expression context, not a template argument list.
                _ => return false,
            }
            i += 1;
        }
        false
    }

    // ── Spacing decision ──────────────────────────────────────────────────────

    /// Should a space be emitted before `next`, given the last emitted token `prev`?
    fn needs_space(&mut self, next: TokenKind) -> bool {
        // The token immediately following `operator` is the overloaded symbol —
        // never add space between `operator` and `=`, `+=`, `==`, `[]`, etc.
        // Exception: `operator new` and `operator delete` are keyword operators
        // that do need a space.
        if self.after_operator_kw && !matches!(next, TokenKind::KwNew | TokenKind::KwDelete) {
            return false;
        }

        let prev = match self.prev {
            Some(k) => k,
            None => return false,
        };

        // Inside a template argument list: spacing after `<` and before `>`
        // is controlled solely by space_inside_angle_brackets.
        if prev == TokenKind::Lt && self.template_depth > 0 {
            return self.config.spacing.space_inside_angle_brackets;
        }

        // Never space before these closers / punctuation
        if matches!(
            next,
            TokenKind::Semi
                | TokenKind::Comma
                | TokenKind::RParen
                | TokenKind::RBracket
                | TokenKind::RBrace
                | TokenKind::DotDotDot
                | TokenKind::PlusPlus
                | TokenKind::MinusMinus
        ) {
            // RBrace handled separately; RParen/RBracket respect space_inside_* config
            if next == TokenKind::RParen {
                return match self.config.spacing.space_inside_parens {
                    SpaceOption::Add => true,
                    SpaceOption::Remove => false,
                    SpaceOption::Preserve => self.src_had_inline_ws,
                };
            }
            if next == TokenKind::RBracket {
                return match self.config.spacing.space_inside_brackets {
                    SpaceOption::Add => true,
                    SpaceOption::Remove => false,
                    SpaceOption::Preserve => self.src_had_inline_ws,
                };
            }
            if next == TokenKind::RBrace {
                return false; // newline handled by the RBrace arm
            }
            // Space before prefix ++/--, but not before postfix ++/--.
            // Postfix: prev ends an expression (e.g. `x++`).
            // Prefix: prev is operator/punctuation (e.g. `= ++x`, `; ++i`, `, ++x`).
            // Exception: no space after `(` or `[` (e.g. `(++x)`).
            if matches!(next, TokenKind::PlusPlus | TokenKind::MinusMinus)
                && !prev.ends_expr()
                && !matches!(prev, TokenKind::LParen | TokenKind::LBracket)
            {
                return true;
            }
            // `foo(int x, ...)` — space after comma applies before `...`.
            if next == TokenKind::DotDotDot && prev == TokenKind::Comma {
                return self.config.spacing.space_after_comma;
            }
            return false;
        }

        // Never space after these openers
        if matches!(
            prev,
            TokenKind::LParen | TokenKind::LBracket | TokenKind::Tilde | TokenKind::Bang
        ) {
            if prev == TokenKind::LParen {
                return match self.config.spacing.space_inside_parens {
                    SpaceOption::Add => true,
                    SpaceOption::Remove => false,
                    SpaceOption::Preserve => self.src_had_inline_ws,
                };
            }
            if prev == TokenKind::LBracket {
                return match self.config.spacing.space_inside_brackets {
                    SpaceOption::Add => true,
                    SpaceOption::Remove => false,
                    SpaceOption::Preserve => self.src_had_inline_ws,
                };
            }
            return false;
        }

        // `->` as return-type arrow (after `)`) gets spaces; member access does not.
        if next == TokenKind::Arrow {
            // Only space before `->` when it's a return-type arrow:
            // followed by a type keyword.
            if prev != TokenKind::RParen {
                return false;
            }
            let after = {
                let mut i = self.pos;
                while i < self.tokens.len()
                    && matches!(
                        self.tokens[i].kind,
                        TokenKind::Whitespace | TokenKind::Newline
                    )
                {
                    i += 1;
                }
                self.tokens.get(i).map(|t| t.kind)
            };
            return after == Some(TokenKind::Keyword);
        }
        if prev == TokenKind::Arrow {
            let ret =
                self.after_rparen_arrow && matches!(next, TokenKind::Ident | TokenKind::Keyword);
            self.after_rparen_arrow = false;
            return ret;
        }

        // No space around other member access / scope operators.
        if matches!(
            prev,
            TokenKind::Dot | TokenKind::DotStar | TokenKind::ArrowStar | TokenKind::ColonColon
        ) {
            return false;
        }
        if matches!(
            next,
            TokenKind::Dot | TokenKind::DotStar | TokenKind::ArrowStar | TokenKind::ColonColon
        ) {
            return false;
        }

        // No space between unary prefix op and its operand (e.g. `++i`).
        // But if next is a binary op (e.g. `*p++ = x`), fall through to
        // the binary-op spacing rules below.
        if matches!(prev, TokenKind::PlusPlus | TokenKind::MinusMinus) && !next.is_binary_op() {
            return false;
        }

        // Space before `(` depends on context
        if next == TokenKind::LParen {
            // After an operator overload symbol (`operator=`, `operator+=`, `operator[]`,
            // `operator new`, etc.) the `(` opens the parameter list — treat it as a
            // call paren. Check before is_control_kw so `operator new(` doesn't get
            // keyword-paren spacing.
            if self.last_was_operator_overload {
                return self.config.spacing.space_before_call_paren;
            }
            // `new`/`delete` followed by `(` is placement-new or a qualified
            // method call — use call-paren spacing, not keyword-paren spacing.
            if matches!(prev, TokenKind::KwNew | TokenKind::KwDelete) {
                return self.config.spacing.space_before_call_paren;
            }
            if prev.is_control_kw() {
                return self.config.spacing.space_before_keyword_paren;
            }
            // Template close `>` behaves like an identifier: `vector<int>()`
            // uses call-paren spacing, not the default "always space" path.
            if matches!(prev, TokenKind::Ident | TokenKind::Keyword) || self.last_was_template_close
            {
                return self.config.spacing.space_before_call_paren;
            }
            if matches!(prev, TokenKind::RParen) {
                // Cast: honour space_after_cast; function-pointer call: no space.
                if self.last_was_cast_close {
                    return match self.config.spacing.space_after_cast {
                        SpaceOption::Add => true,
                        SpaceOption::Remove => false,
                        SpaceOption::Preserve => self.src_had_inline_ws,
                    };
                }
                return false;
            }
            return true;
        }

        // Space inside `[`
        if next == TokenKind::LBracket {
            return false;
        }

        // Unary operators: if the previous token cannot end an expression,
        // the next `+`, `-`, `*`, `&`, `!`, `~` is unary → no space before operand.
        // We handle the "before" part: no space inserted when op is unary.
        // (The op itself was already emitted with whatever spacing it got.)

        // After `;` inside a for-loop header, always emit a space regardless of
        // what follows (unary *, &, +, -, !, ~, ++, --).
        if prev == TokenKind::Semi && self.paren_depth > 0 {
            return true;
        }

        // After comma
        if prev == TokenKind::Comma {
            return self.config.spacing.space_after_comma;
        }

        // After a cast-close `)`, `*` and `&` are always unary (dereference /
        // address-of). Check this before the binary-op path which would otherwise
        // see RParen.ends_expr() == true and add a space.
        if self.last_was_cast_close
            && prev == TokenKind::RParen
            && matches!(next, TokenKind::Star | TokenKind::Amp)
        {
            return match self.config.spacing.space_after_cast {
                SpaceOption::Add => true,
                SpaceOption::Remove => false,
                SpaceOption::Preserve => self.src_had_inline_ws,
            };
        }

        // Binary operators — space on both sides if configured
        if next.is_binary_op() {
            if prev.ends_expr() {
                return self.config.spacing.space_around_binary_ops;
            }
            // next is in unary context, but prev (a binary op or keyword like
            // `return`/`throw`) still needs a trailing space: `= -1`, `return &x`.
            if prev.is_binary_op() || prev.is_any_kw() {
                return self.config.spacing.space_around_binary_ops;
            }
            // Space after `:` in Rust struct field / type annotation (`a: *mut`).
            if prev == TokenKind::Colon {
                return true;
            }
            // purely unary context (e.g. after `(`) — no space
            return false;
        }
        if prev.is_binary_op() {
            return self.config.spacing.space_around_binary_ops;
        }

        // Colon: ternary gets space on both sides; case/label/base-class does not.
        // Bitfield colons are handled directly in the Colon arm, not here.
        if next == TokenKind::Colon {
            return self.ternary_depth > 0;
        }
        if prev == TokenKind::Colon {
            return true;
        }

        // After keywords that aren't followed by `(`
        if prev.is_any_kw() {
            return true;
        }
        // Before a keyword
        if next.is_any_kw() {
            return true;
        }

        // After a cast-closing `)`, honour space_after_cast config.
        // The `next == LParen && prev == RParen` case already returned false above,
        // so this only fires when the next token is not `(`.
        if prev == TokenKind::RParen && self.last_was_cast_close {
            return match self.config.spacing.space_after_cast {
                SpaceOption::Add => true,
                SpaceOption::Remove => false,
                SpaceOption::Preserve => self.src_had_inline_ws,
            };
        }

        // A numeric literal immediately followed by an identifier with no
        // intervening whitespace is a preprocessor pp-number token (e.g.
        // `4WAY_HANDSHAKE_TIMEOUT` in a macro argument).  The lexer splits it
        // into LitInt + Ident; inserting a space would change the macro argument
        // and break token-pasting.  Preserve the original spacing.
        if matches!(prev, TokenKind::LitInt | TokenKind::LitFloat) && next == TokenKind::Ident {
            return self.src_had_inline_ws;
        }

        // Default: space between two identifier-like tokens
        true
    }

    // ── Main format loop ──────────────────────────────────────────────────────

    fn format(mut self) -> Result<String, FunkyError> {
        loop {
            self.skip_ws();

            let tok = match self.advance() {
                None => break,
                Some(t) => t.clone(),
            };

            self.check_var_decl_transition(tok.kind);

            match tok.kind {
                TokenKind::Eof => {
                    if self.config.newlines.final_newline && !self.at_line_start {
                        self.nl();
                    }
                    break;
                }

                // ── Line continuation outside preprocessor ────────────────────
                // `\` immediately before a newline in regular C/C++ code (phase-1
                // splice).  Write it verbatim; the following Newline token handles
                // the line break as usual.  Don't call set_prev so the token that
                // precedes `\` remains as `prev` for the continuation line.
                TokenKind::LineContinuation => {
                    self.flush_blank_lines();
                    if self.at_line_start {
                        self.indent();
                    } else if self.needs_space(tok.kind) {
                        self.space();
                    }
                    self.write(tok.lexeme);
                    // set_prev intentionally omitted — backslash is transparent.
                }

                // ── Preprocessor — pass through verbatim, normalized newlines ─
                TokenKind::PreprocLine => {
                    self.flush_blank_lines();
                    if !self.at_line_start {
                        self.nl();
                    }
                    // Normalize line endings in the directive.
                    let normalized = tok.lexeme.replace("\r\n", "\n").replace('\r', "\n");
                    let nl = self.config.newline_str();
                    let normalized = normalized.replace('\n', nl);

                    // Classify the directive (always, not just when pp_indent is on).
                    let directive: String = normalized
                        .trim_start()
                        .strip_prefix('#')
                        .and_then(|s| s.split_whitespace().next())
                        .unwrap_or("")
                        .to_string();
                    let directive = directive.as_str();
                    let is_open = matches!(directive, "if" | "ifdef" | "ifndef");
                    let is_close = directive == "endif";
                    let is_reopen = matches!(directive, "elif" | "else");

                    // Track code brace depth across #ifdef/#else/#endif so both
                    // branches start indenting from the same level.
                    if is_open {
                        self.pp_brace_stack.push(self.indent_level);
                    } else if is_reopen {
                        // Restore to the depth recorded at the opening #if so the
                        // else-branch starts with the same baseline.
                        if let Some(&saved) = self.pp_brace_stack.last() {
                            self.indent_level = saved;
                        }
                    } else if is_close {
                        // Pop without restoring — the last branch's accumulated
                        // depth is the correct post-#endif depth.
                        self.pp_brace_stack.pop();
                    }

                    // Normalize the number of spaces between `#endif` and a
                    // trailing `/*` comment.
                    let normalized = if is_close {
                        normalize_endif_spacing(
                            &normalized,
                            self.config.preprocessor.endif_comment_space,
                        )
                    } else {
                        normalized
                    };

                    // Apply space_around_binary_ops to #if / #elif conditions.
                    let normalized = if self.config.spacing.space_around_binary_ops
                        && (is_open || is_reopen)
                        && !matches!(directive, "ifdef" | "ifndef")
                    {
                        format_preproc_if_condition(&normalized, nl)
                    } else {
                        normalized
                    };

                    // Apply space_after_comma to #define bodies.
                    let normalized =
                        if self.config.spacing.space_after_comma && directive == "define" {
                            add_space_after_comma(&normalized)
                        } else {
                            normalized
                        };

                    if self.config.preprocessor.pp_indent {
                        // #endif and #elif/#else dedent before emit.
                        if is_close || is_reopen {
                            self.pp_depth = self.pp_depth.saturating_sub(1);
                        }
                        let indent_str = self.config.indent_str().repeat(self.pp_depth as usize);
                        // Write depth-prefix before the `#`.
                        self.write(&indent_str);
                        self.write(normalized.trim_start());
                        // #if and #elif/#else increase depth after emit.
                        if is_open || is_reopen {
                            self.pp_depth += 1;
                        }
                    } else if self.config.preprocessor.pp_indent_at_level {
                        let leading = (tok.span.col as usize).saturating_sub(1);
                        if leading > 0 {
                            self.write(&" ".repeat(leading));
                        }
                        self.write(normalized.trim_start());
                    } else {
                        self.write(&normalized);
                    }

                    if !self.at_line_start {
                        self.nl();
                    }
                    self.set_prev(TokenKind::PreprocLine);
                }

                // ── Line comment ──────────────────────────────────────────────
                TokenKind::CommentLine => {
                    let body = tok.lexeme.trim_end_matches(['\n', '\r']);
                    let is_annotation = body
                        .strip_prefix("//")
                        .map_or(false, |rest| rest.trim_start().starts_with('^'));

                    if is_annotation {
                        // Preserve annotation comments (`// ^^^`) at their
                        // original source column — do not re-indent or modify.
                        self.flush_blank_lines();
                        if !self.at_line_start {
                            self.nl();
                        }
                        let col = (tok.span.col as usize).saturating_sub(1);
                        for _ in 0..col {
                            self.write(" ");
                        }
                        self.write(body);
                        self.nl();
                        self.set_prev(TokenKind::CommentLine);
                        continue;
                    }

                    // Merge: a standalone comment with no blank lines before it
                    // can be hoisted to the end of the preceding brace/statement
                    // line when the config flag is set.
                    let can_merge = self.config.newlines.merge_line_comment
                        && self.at_line_start
                        && self.blank_lines == 0
                        && matches!(
                            self.prev,
                            Some(TokenKind::LBrace | TokenKind::RBrace | TokenKind::Semi)
                        );
                    self.flush_blank_lines();
                    if can_merge {
                        self.trim_to_prev_line_end();
                    }
                    if !self.at_line_start {
                        self.space();
                    } else {
                        self.indent();
                    }
                    self.write(body);
                    self.nl();
                    self.set_prev(TokenKind::CommentLine);
                }

                // ── Block comment ─────────────────────────────────────────────
                TokenKind::CommentBlock => {
                    self.flush_blank_lines();
                    let was_at_line_start = self.at_line_start;
                    if !was_at_line_start {
                        self.space();
                    } else {
                        self.indent();
                    }
                    let target_indent_col = self.line_indent_col;
                    // Normalize newlines in the block comment body.
                    let nl = self.config.newline_str();
                    let normalized = tok
                        .lexeme
                        .replace("\r\n", "\n")
                        .replace('\r', "\n")
                        .replace('\n', nl);
                    // Ensure the closing `*/` has a leading space when it sits
                    // flush at the start of a line (e.g. `\n*/` → `\n */`).
                    // When the config option is enabled, rewrite a bare `*/`
                    // closing line to ` */` to match ` *`-continuation style.
                    // Exception: SQLite-style `**`-continuation comments already
                    // have `*/` at column 0 — don't add a spurious space there.
                    let uses_double_star = normalized
                        .split(nl)
                        .skip(1)
                        .any(|line| line.starts_with("**"));
                    let normalized = if self.config.comments.normalize_block_comment_closing
                        && !uses_double_star
                        && normalized.contains(&format!("{nl}*/"))
                    {
                        normalized.replace(&format!("{nl}*/"), &format!("{nl} */"))
                    } else {
                        normalized
                    };
                    // When indent style is spaces, expand leading tabs in
                    // continuation lines to spaces.  The leading whitespace
                    // before the `*` on each continuation line is indentation,
                    // not comment content, and should follow the indent style.
                    let normalized = if self.config.indent.style == IndentStyle::Spaces {
                        let tab_w = self.config.indent.width as usize;
                        let mut out = String::with_capacity(normalized.len());
                        let mut first = true;
                        for line in normalized.split(nl) {
                            if !first {
                                out.push_str(nl);
                            }
                            first = false;
                            // Expand each leading tab to tab_w spaces.
                            let non_tab = line.trim_start_matches('\t');
                            let n_tabs = line.len() - non_tab.len();
                            for _ in 0..n_tabs * tab_w {
                                out.push(' ');
                            }
                            out.push_str(non_tab);
                        }
                        out
                    } else {
                        normalized
                    };
                    // Strip trailing spaces from each line inside the comment.
                    let normalized = if normalized.contains(' ') || normalized.contains('\t') {
                        let mut out = String::with_capacity(normalized.len());
                        let mut first = true;
                        for line in normalized.split(nl) {
                            if !first {
                                out.push_str(nl);
                            }
                            first = false;
                            out.push_str(line.trim_end());
                        }
                        out
                    } else {
                        normalized
                    };
                    // Re-indent continuation lines to match the column of `/*`.
                    // When `add_braces_to_if/while/for` shifts a comment to a
                    // deeper indent, the continuation lines in the lexeme still
                    // carry their original (shallower) whitespace prefix.
                    //
                    // Strategy: for each continuation line, compute a per-line
                    // delta = target_indent - orig_indent and apply it to the
                    // line's own leading whitespace.  This preserves style-
                    // alignment characters (e.g. the ` ` before `*` in standard
                    // ` * foo` style) while shifting the indent portion.
                    //
                    // All measurements are in the unit appropriate for the indent
                    // style:
                    //   Spaces — expanded-space count (tabs expanded to tab_w)
                    //   Tabs   — raw character count (tabs counted as 1)
                    //
                    // We walk backward past any injected synthetic tokens (e.g.
                    // a `{` from add_braces_to_while) to find the real Whitespace
                    // token that preceded `/*` in the source.
                    let normalized = if was_at_line_start && normalized.contains(nl) {
                        let tab_w = self.config.indent.width as usize;
                        let use_spaces = self.config.indent.style == IndentStyle::Spaces;
                        // Measure a whitespace string in the current unit.
                        let measure_ws = |s: &str| -> usize {
                            if use_spaces {
                                s.chars().map(|c| if c == '\t' { tab_w } else { 1 }).sum()
                            } else {
                                s.chars().count()
                            }
                        };
                        // Target indent in the current unit.
                        let target: i64 = if use_spaces {
                            target_indent_col as i64
                        } else {
                            self.indent_level as i64
                        };
                        // Original indent of `/*` from the preceding Whitespace token.
                        let orig_indent: i64 = {
                            let comment_idx = self.pos - 1;
                            let mut ws_lexeme = "";
                            let mut j = comment_idx;
                            while j > 0 {
                                j -= 1;
                                match self.tokens[j].kind {
                                    TokenKind::Whitespace => {
                                        ws_lexeme = self.tokens[j].lexeme;
                                        break;
                                    }
                                    TokenKind::Newline => break,
                                    _ => {}
                                }
                            }
                            measure_ws(ws_lexeme) as i64
                        };
                        let delta: i64 = target - orig_indent;
                        let lines: Vec<&str> = normalized.split(nl).collect();
                        let mut out = String::with_capacity(normalized.len() + 16);
                        for (i, line) in lines.iter().enumerate() {
                            if i > 0 {
                                out.push_str(nl);
                                let own_ws = line.len() - line.trim_start().len();
                                let shifted = ((own_ws as i64) + delta).max(0) as usize;
                                // When the `/*` didn't move (delta==0), ensure lines at
                                // or below the `/*` column land at target+1 so content
                                // is visually inside the opener (uncrustify cmt_indent_multi).
                                // When delta!=0, the `/*` was shifted by add_braces or
                                // similar — keep proportional alignment, don't add +1.
                                let new_ws =
                                    if delta == 0 && (own_ws as i64) <= orig_indent && target > 0 {
                                        shifted.max(target as usize + 1)
                                    } else {
                                        shifted
                                    };
                                // Build new prefix: indent_level tabs/spaces + any
                                // remaining style-alignment spaces. Blank lines
                                // inside block comments get no trailing whitespace.
                                let trimmed = line.trim_start();
                                if !trimmed.is_empty() {
                                    if use_spaces {
                                        for _ in 0..new_ws {
                                            out.push(' ');
                                        }
                                    } else {
                                        let tabs = (self.indent_level as usize).min(new_ws);
                                        for _ in 0..tabs {
                                            out.push('\t');
                                        }
                                        for _ in 0..new_ws.saturating_sub(tabs) {
                                            out.push(' ');
                                        }
                                    }
                                    out.push_str(trimmed);
                                }
                            } else {
                                out.push_str(line);
                            }
                        }
                        out
                    } else {
                        normalized
                    };
                    self.write(&normalized);
                    // If the comment ends the line (next source token is a
                    // Newline), emit the newline now and mark it consumed so
                    // skip_ws() doesn't double-count it as a blank line.
                    // CommentLine does this implicitly because its lexeme
                    // includes the trailing \n; CommentBlock does not.
                    if matches!(self.tokens.get(self.pos), Some(t) if t.kind == TokenKind::Newline)
                    {
                        self.nl();
                        self.skip_next_newline = true;
                    }
                    self.set_prev(TokenKind::CommentBlock);
                }

                // ── Opening brace ─────────────────────────────────────────────
                TokenKind::LBrace => {
                    let ctx = self.infer_brace_ctx();
                    self.flush_blank_lines();

                    match ctx {
                        BraceCtx::Other => {
                            // Initializer list — stay on same line with a space
                            if self.needs_space(TokenKind::LBrace) {
                                self.space();
                            }
                            self.write("{");

                            // Small initializer: keep entirely on one line.
                            // Large flat initializer: also keep on one line when
                            // expand_large_initializers is disabled (uncrustify default).
                            let inline_end = self.small_initializer_end().or_else(|| {
                                if !self.config.braces.expand_large_initializers {
                                    self.large_flat_initializer_end()
                                } else {
                                    None
                                }
                            });
                            if let Some(end) = inline_end {
                                let content: Vec<(&str, TokenKind)> = self.tokens[self.pos..end]
                                    .iter()
                                    .filter(|t| {
                                        !matches!(
                                            t.kind,
                                            TokenKind::Whitespace | TokenKind::Newline
                                        )
                                    })
                                    .map(|t| (t.lexeme, t.kind))
                                    .collect();

                                if content.is_empty() {
                                    self.write("}");
                                } else {
                                    self.write(" ");
                                    let mut prev_kind = TokenKind::LBrace;
                                    let mut suppress = false;
                                    for (lex, kind) in content.iter() {
                                        // No space before . or -> only when it's member
                                        // access (prev ends an expression). Designated
                                        // initializer .field comes after , or { and
                                        // does need a space.
                                        let need_space = !(suppress
                                            || matches!(
                                                kind,
                                                TokenKind::Comma | TokenKind::RParen
                                            )
                                            || matches!(
                                                prev_kind,
                                                TokenKind::LBrace
                                                    | TokenKind::Dot
                                                    | TokenKind::Arrow
                                                    | TokenKind::LParen
                                            )
                                            || matches!(kind, TokenKind::Dot | TokenKind::Arrow)
                                                && prev_kind.ends_expr()
                                            || matches!(kind, TokenKind::LParen)
                                                && prev_kind.ends_expr());
                                        if need_space {
                                            self.write(" ");
                                        }
                                        suppress = false;
                                        self.write(lex);
                                        // Unary context: after any non-expression-ending
                                        // token (=, comma, {, (, etc.), - + * & are unary
                                        // — suppress space between operator and operand.
                                        if matches!(
                                            kind,
                                            TokenKind::Minus
                                                | TokenKind::Plus
                                                | TokenKind::Star
                                                | TokenKind::Amp
                                        ) && !prev_kind.ends_expr()
                                        {
                                            suppress = true;
                                        }
                                        prev_kind = *kind;
                                    }
                                    self.write(" }");
                                }
                                self.pos = end + 1;
                                self.set_prev(TokenKind::RBrace);
                                continue;
                            }
                        }
                        // extern "C" { } is a linkage specification. Placement is
                        // controlled by braces.extern_c_brace:
                        //   force_same_line — always K&R (Google/LLVM style)
                        //   preserve        — leave brace where source has it
                        BraceCtx::ExternC => {
                            match self.config.braces.extern_c_brace {
                                ExternCBrace::ForceSameLine => {
                                    if self.at_line_start {
                                        self.trim_to_prev_line_end();
                                    }
                                    self.space();
                                }
                                ExternCBrace::Preserve => {
                                    if self.at_line_start {
                                        // brace was already on its own line in source — keep it
                                    } else {
                                        self.space();
                                    }
                                }
                            }
                            self.write("{");
                        }
                        _ => match self.config.braces.style {
                            BraceStyle::Allman => {
                                self.ensure_own_line();
                                self.write("{");
                            }
                            BraceStyle::Kr | BraceStyle::Stroustrup => {
                                // fn_brace_newline: function-definition braces go on
                                // their own line even in KR mode.  Control-flow
                                // constructs (if/for/while/switch) always stay on the
                                // same line.
                                let fn_newline = ctx == BraceCtx::Function
                                    && self.config.braces.fn_brace_newline
                                    && !self.rparen_closes_ctrl_flow();
                                if fn_newline {
                                    self.ensure_own_line();
                                } else if self.at_line_start
                                    && self.prev == Some(TokenKind::PreprocLine)
                                {
                                    // Never pull `{` onto a preprocessor directive line
                                    // (e.g. `else\n#endif\n{`). Keep it on its own line.
                                    self.indent();
                                } else {
                                    if self.at_line_start {
                                        // Source had Allman-style brace; enforce KR.
                                        self.trim_to_prev_line_end();
                                    }
                                    self.space();
                                }
                                self.write("{");
                            }
                        },
                    }

                    // ── Empty-body collapse ───────────────────────────────────
                    // When collapse_empty_body is set and the only content between
                    // `{` and `}` is whitespace, emit `{}` on the same line.
                    if self.config.braces.collapse_empty_body {
                        let mut look = self.pos;
                        while look < self.tokens.len()
                            && matches!(
                                self.tokens[look].kind,
                                TokenKind::Whitespace | TokenKind::Newline
                            )
                        {
                            look += 1;
                        }
                        if self.tokens.get(look).map(|t| t.kind) == Some(TokenKind::RBrace) {
                            // Consume whitespace + the `}` token.
                            self.pos = look + 1;
                            self.write("}");

                            // Replicate the post-`}` newline/spacing decisions from
                            // the RBrace arm so callers see the same output shape.
                            let mut after = self.pos;
                            while after < self.tokens.len()
                                && matches!(
                                    self.tokens[after].kind,
                                    TokenKind::Whitespace | TokenKind::Newline
                                )
                            {
                                after += 1;
                            }
                            let next_kind = self.tokens.get(after).map(|t| t.kind);

                            self.emit_post_brace_spacing(ctx, next_kind, tok.span.line);

                            self.pending_switch = false;
                            self.pending_type = false;
                            self.pending_extern_c = false;
                            self.set_prev(TokenKind::RBrace);
                            continue;
                        }
                    }

                    if ctx == BraceCtx::Switch {
                        self.switch_depth += 1;
                        self.case_body_stack.push(false);
                    }
                    if ctx == BraceCtx::Type {
                        self.class_depth += 1;
                    }
                    // A Block at global scope (brace_stack currently empty) is a
                    // macro-defined function body (e.g. SM_STATE(...) { ... }).
                    // Treat it like a Function for the var-decl blank-line rule.
                    let is_func_like = ctx == BraceCtx::Function
                        || (ctx == BraceCtx::Block && self.brace_stack.is_empty());
                    if is_func_like && self.config.newlines.blank_line_after_var_decl_block {
                        self.in_var_decl_block = true;
                        self.at_func_stmt_start = true;
                        self.saw_func_decl = false;
                    }
                    self.pending_switch = false;
                    self.pending_type = false;
                    self.pending_extern_c = false;
                    // `{` takes over indentation; = alignment no longer applies.
                    self.assign_col = None;
                    let is_large_init = ctx == BraceCtx::Other
                        && self.config.braces.expand_large_initializers
                        && self.large_flat_initializer_end().is_some();
                    self.brace_stack.push(ctx);
                    self.large_init_stack.push(is_large_init);
                    // `case X: {` — the case-label colon already incremented
                    // indent_level (via indent_switch_case). The block `{` should
                    // take over that indent slot rather than adding a new one, so the
                    // body sits at case_body_level + 1 (not + 2).
                    if ctx == BraceCtx::Block
                        && self.last_was_case_colon
                        && self.config.indent.indent_switch_case
                    {
                        // Undo the case-body indent; the block's own +1 below
                        // restores the same net level.
                        self.indent_level = self.indent_level.saturating_sub(1);
                        if let Some(active) = self.case_body_stack.last_mut() {
                            *active = false;
                        }
                    }
                    if ctx != BraceCtx::ExternC {
                        self.indent_level += 1;
                    }
                    self.nl();
                    self.skip_next_newline = true;
                    if self.config.newlines.blank_line_after_open_brace
                        && matches!(ctx, BraceCtx::Function | BraceCtx::Block)
                    {
                        self.blank_lines = self.blank_lines.max(1);
                    }
                    self.set_prev(TokenKind::LBrace);
                }

                // ── Closing brace ─────────────────────────────────────────────
                TokenKind::RBrace => {
                    let closing_ctx = self.brace_stack.last().copied().unwrap_or(BraceCtx::Other);
                    // When indent_switch_case is on, an active case body adds an
                    // extra indent level that must be unwound before the `}`.
                    if closing_ctx == BraceCtx::Switch && self.config.indent.indent_switch_case {
                        if let Some(true) = self.case_body_stack.last() {
                            self.indent_level = self.indent_level.saturating_sub(1);
                        }
                        self.case_body_stack.pop();
                    }
                    if closing_ctx != BraceCtx::ExternC && self.indent_level > 0 {
                        self.indent_level -= 1;
                    }
                    // An `=` inside the brace body (e.g. the last enum value without a
                    // trailing comma) leaves assign_col set. Clear it so `indent()` uses
                    // normal indentation rather than aligning to the `=` column.
                    self.assign_col = None;
                    self.flush_blank_lines();
                    self.ensure_own_line();
                    self.write("}");

                    let ctx = self.brace_stack.pop().unwrap_or(BraceCtx::Other);
                    self.large_init_stack.pop();

                    if ctx == BraceCtx::Switch {
                        self.switch_depth = self.switch_depth.saturating_sub(1);
                    }
                    if ctx == BraceCtx::Type {
                        self.class_depth = self.class_depth.saturating_sub(1);
                    }
                    // brace_stack was already popped; a Block that left the stack
                    // empty was a top-level macro-function body (see LBrace logic).
                    let was_func_like = ctx == BraceCtx::Function
                        || (ctx == BraceCtx::Block && self.brace_stack.is_empty());
                    if was_func_like {
                        self.in_var_decl_block = false;
                        self.at_func_stmt_start = false;
                        self.force_blank_after_decls = false;
                    }

                    // Semicolon required after type definitions and namespace
                    let needs_semi = matches!(ctx, BraceCtx::Type);

                    // Peek: is the next token `;`?
                    let mut look = self.pos;
                    while look < self.tokens.len()
                        && matches!(
                            self.tokens[look].kind,
                            TokenKind::Whitespace | TokenKind::Newline
                        )
                    {
                        look += 1;
                    }
                    let next_kind = self.tokens.get(look).map(|t| t.kind);

                    if needs_semi && next_kind != Some(TokenKind::Semi) {
                        // The struct/class/enum definition has no trailing `;` —
                        // we must not add one ourselves (the source might be a forward
                        // decl without one, which is fine). Just emit the brace.
                    }

                    self.emit_post_brace_spacing(ctx, next_kind, tok.span.line);

                    self.set_prev(TokenKind::RBrace);
                }

                // ── Semicolon ─────────────────────────────────────────────────
                TokenKind::Semi => {
                    self.flush_blank_lines();
                    self.pending_type = false;
                    self.pending_extern_c = false;
                    self.write(";");
                    // Don't emit newline if we're inside parens (for-loop header).
                    if self.paren_depth == 0 {
                        self.assign_col = None;
                        // If a trailing inline comment follows on the same source
                        // line, let the CommentLine handler close the line instead.
                        if self.peek_inline_comment(tok.span.line) {
                            // nothing — CommentLine will emit the trailing \n
                        } else {
                            self.nl();
                            self.skip_next_newline = true;
                        }
                        // Signal that the next token starts a new statement so the
                        // var-decl-block state machine can evaluate it.
                        if self.in_var_decl_block {
                            let top = self.brace_stack.last();
                            let is_func_top = top == Some(&BraceCtx::Function)
                                || (top == Some(&BraceCtx::Block) && self.brace_stack.len() == 1);
                            if is_func_top {
                                self.at_func_stmt_start = true;
                            }
                        }
                    }
                    self.set_prev(TokenKind::Semi);
                }

                // ── Paren depth tracking ──────────────────────────────────────
                TokenKind::LParen => {
                    self.flush_blank_lines();
                    let is_cast = self.next_is_type_kw()
                        && !self.prev_is_sizeof_like()
                        && !self.prev.is_some_and(|p| p.is_control_kw());
                    self.cast_paren_stack.push(is_cast);
                    if self.at_line_start {
                        self.indent();
                    } else if self.needs_space(TokenKind::LParen) {
                        self.space();
                    } else if self.next_is_fn_ptr_declarator()
                        && matches!(
                            self.prev,
                            Some(TokenKind::Keyword | TokenKind::Ident | TokenKind::RParen)
                        )
                    {
                        // function-pointer declarator: `void (*Fn)(...)` needs space before `(`
                        self.space();
                    }
                    self.write("(");
                    self.paren_depth += 1;
                    // When space_inside_parens is Add (or Preserve with source space),
                    // the first argument starts one column later; include that offset
                    // so continuation lines align with it rather than the bare `(`.
                    let extra = match self.config.spacing.space_inside_parens {
                        SpaceOption::Add => 1,
                        SpaceOption::Remove => 0,
                        // Peek at the raw next token to see if source has a space.
                        SpaceOption::Preserve => usize::from(matches!(
                            self.tokens.get(self.pos),
                            Some(t) if t.kind == TokenKind::Whitespace
                        )),
                    };
                    self.paren_col_stack.push(self.current_col + extra);
                    // Precompute the EOL continuation column: base indent of
                    // this line plus one indent width per paren opened on it.
                    let indent_width = self.config.indent.width as usize;
                    let parens_on_line = (self.paren_depth - self.line_start_paren_depth) as usize;
                    let eol_col = self.line_indent_col + parens_on_line * indent_width;
                    self.paren_eol_stack.push((false, eol_col));
                    self.set_prev(TokenKind::LParen);
                }
                TokenKind::RParen => {
                    self.flush_blank_lines();
                    if self.at_line_start {
                        self.align_to_paren();
                    } else {
                        let want = match self.config.spacing.space_inside_parens {
                            SpaceOption::Add => true,
                            SpaceOption::Remove => false,
                            SpaceOption::Preserve => self.src_had_inline_ws,
                        };
                        if want {
                            self.space();
                        }
                    }
                    self.write(")");
                    self.paren_depth = self.paren_depth.saturating_sub(1);
                    self.paren_col_stack.pop();
                    self.paren_eol_stack.pop();
                    let is_cast_close = self.cast_paren_stack.pop().unwrap_or(false);
                    // A pointer declarator `*`/`&` immediately before `)` (e.g.
                    // `sizeof(char *)`) sets suppress_next_space, but that flag
                    // must not leak out of the closing paren into subsequent tokens.
                    self.suppress_next_space = false;
                    self.prev = Some(TokenKind::RParen);
                    self.last_was_template_close = false;
                    self.last_was_cast_close = is_cast_close;
                }

                // ── Bracket depth tracking ────────────────────────────────────
                TokenKind::LBracket => {
                    self.flush_blank_lines();
                    self.write("[");
                    self.bracket_depth += 1;
                    self.set_prev(TokenKind::LBracket);
                }
                TokenKind::RBracket => {
                    self.flush_blank_lines();
                    if !self.at_line_start {
                        let want = match self.config.spacing.space_inside_brackets {
                            SpaceOption::Add => true,
                            SpaceOption::Remove => false,
                            SpaceOption::Preserve => self.src_had_inline_ws,
                        };
                        if want {
                            self.space();
                        }
                    }
                    self.write("]");
                    self.bracket_depth = self.bracket_depth.saturating_sub(1);
                    self.set_prev(TokenKind::RBracket);
                }

                // ── Colon after case / default / access specifier / ternary ──
                TokenKind::Colon => {
                    self.flush_blank_lines();
                    // Ternary `:` gets a space before it; case/label/access/goto do not.
                    if self.ternary_depth > 0
                        && !self.in_case_label
                        && !self.in_access_label
                        && !self.in_goto_label
                    {
                        self.ternary_depth = self.ternary_depth.saturating_sub(1);
                        if !self.at_line_start {
                            self.space();
                        }
                    } else if !self.in_case_label
                        && !self.in_access_label
                        && !self.in_goto_label
                        && self.prev == Some(TokenKind::Ident)
                        && self.peek_non_ws_kind() == Some(TokenKind::LitInt)
                        && self
                            .brace_stack
                            .last()
                            .is_some_and(|ctx| *ctx == BraceCtx::Type)
                    {
                        // Bitfield colon: `field:N` → `field : N`
                        if !self.at_line_start {
                            self.space();
                        }
                    }
                    self.write(":");
                    let is_case_colon = self.in_case_label;
                    if self.in_case_label {
                        self.in_case_label = false;
                        self.nl();
                        self.skip_next_newline = true;
                        if self.config.indent.indent_switch_case {
                            self.indent_level += 1;
                            if let Some(active) = self.case_body_stack.last_mut() {
                                *active = true;
                            }
                        }
                    } else if self.in_access_label {
                        self.in_access_label = false;
                        self.nl();
                        self.skip_next_newline = true;
                    } else if self.in_goto_label {
                        self.in_goto_label = false;
                        self.nl();
                        self.skip_next_newline = true;
                    }
                    self.set_prev(TokenKind::Colon);
                    // Set AFTER set_prev() so set_prev() doesn't clear it.
                    self.last_was_case_colon = is_case_colon;
                }

                // ── switch keyword — arm to set pending_switch ────────────────
                TokenKind::KwSwitch => {
                    self.flush_blank_lines();
                    if self.at_line_start {
                        self.indent();
                    } else if self.needs_space(tok.kind) {
                        self.space();
                    }
                    self.pending_switch = true;
                    self.write(tok.lexeme);
                    self.set_prev(tok.kind);
                }

                // ── case / default labels ─────────────────────────────────────
                TokenKind::KwCase | TokenKind::KwDefault => {
                    self.flush_blank_lines();
                    if self.at_line_start {
                        if self.config.indent.indent_switch_case {
                            // Undo any prior case-body extra indent, then print
                            // at the switch-body level (no additional dedent).
                            if let Some(active) = self.case_body_stack.last_mut() {
                                if *active {
                                    self.indent_level = self.indent_level.saturating_sub(1);
                                    *active = false;
                                }
                            }
                            self.indent();
                        } else {
                            // Dedent one level relative to the switch body.
                            let saved = self.indent_level;
                            if self.switch_depth > 0 && self.indent_level > 0 {
                                self.indent_level -= 1;
                            }
                            self.indent();
                            self.indent_level = saved;
                        }
                    } else if self.needs_space(tok.kind) {
                        self.space();
                    }
                    self.in_case_label = true;
                    self.write(tok.lexeme);
                    self.set_prev(tok.kind);
                }

                // ── Access specifiers — dedented to class body level ──────────
                TokenKind::KwPublic | TokenKind::KwPrivate | TokenKind::KwProtected => {
                    self.flush_blank_lines();
                    if self.at_line_start && self.class_depth > 0 {
                        let saved = self.indent_level;
                        if self.indent_level > 0 {
                            self.indent_level -= 1;
                        }
                        self.indent();
                        self.indent_level = saved;
                        self.in_access_label = true;
                    } else if !self.at_line_start && self.needs_space(tok.kind) {
                        self.space();
                    }
                    self.write(tok.lexeme);
                    self.set_prev(tok.kind);
                }

                // ── Template angle brackets ───────────────────────────────────
                TokenKind::Lt
                    if matches!(
                        self.prev,
                        Some(
                            TokenKind::Ident
                                | TokenKind::KwTemplate
                                | TokenKind::Gt
                                | TokenKind::ColonColon
                        )
                    ) && self.looks_like_template_open() =>
                {
                    self.flush_blank_lines();
                    // No space between the name and `<`: `vector<int>` not `vector <int>`.
                    if self.at_line_start {
                        self.indent();
                    }
                    self.write("<");
                    self.template_depth += 1;
                    if self.config.spacing.space_inside_angle_brackets {
                        self.space();
                    }
                    self.set_prev(TokenKind::Lt);
                }

                TokenKind::Gt if self.template_depth > 0 => {
                    self.flush_blank_lines();
                    if self.config.spacing.space_inside_angle_brackets && !self.at_line_start {
                        self.space();
                    } else if self.at_line_start {
                        self.indent();
                    }
                    self.write(">");
                    self.template_depth -= 1;
                    self.prev = Some(TokenKind::Gt);
                    self.last_was_template_close = true;
                }

                // `>>` closing two nested template levels: `vector<vector<int>>`
                TokenKind::GtGt if self.template_depth >= 2 => {
                    self.flush_blank_lines();
                    if self.config.spacing.space_inside_angle_brackets && !self.at_line_start {
                        self.space();
                    } else if self.at_line_start {
                        self.indent();
                    }
                    self.write(">>");
                    self.template_depth -= 2;
                    self.prev = Some(TokenKind::Gt);
                    self.last_was_template_close = true;
                }

                // ── Pointer / reference declarator ───────────────────────────
                TokenKind::Star | TokenKind::Amp if self.is_ptr_decl_context() => {
                    self.flush_blank_lines();
                    match self.config.spacing.pointer_align {
                        PointerAlign::Middle => {
                            // Same as binary-op: space on both sides.
                            if self.at_line_start {
                                self.indent();
                            } else if self.needs_space(tok.kind) {
                                self.space();
                            }
                        }
                        PointerAlign::Type => {
                            // Star/amp attached to the type — no space before.
                            if self.at_line_start {
                                self.indent();
                            }
                            // Deliberately no space() call here.
                        }
                        PointerAlign::Name => {
                            // Star/amp attached to the name — space before (only
                            // between type and first star; consecutive stars/amps
                            // stay together), suppress space after.
                            if self.at_line_start {
                                self.indent();
                            } else if !matches!(self.prev, Some(TokenKind::Star | TokenKind::Amp)) {
                                self.space();
                            }
                            self.suppress_next_space = true;
                        }
                    }
                    self.write(tok.lexeme);
                    self.set_prev(tok.kind);
                }

                // ── Unary / binary * & + - (non-declarator) ─────────────────
                // In unary context, suppress the space after the op so `*ptr`,
                // `&x`, `-1`, `+x` stay compact.
                TokenKind::Star | TokenKind::Amp | TokenKind::Plus | TokenKind::Minus => {
                    self.flush_blank_lines();
                    // At line start (e.g. `*ptr = ...` after a standalone block
                    // comment) the operator is always unary, never binary — even
                    // if the previous emitted token was a CommentBlock which
                    // satisfies ends_expr().
                    // After a cast-close `)`, * and & are always unary (dereference /
                    // address-of), never binary multiplication / bitwise-and.
                    let is_binary = !self.at_line_start
                        && !self.last_was_cast_close
                        && self.prev.is_some_and(|p| p.ends_expr());
                    if self.at_line_start {
                        self.indent();
                    } else if self.needs_space(tok.kind) {
                        self.space();
                    }
                    if !is_binary {
                        self.suppress_next_space = true;
                    }
                    self.write(tok.lexeme);
                    self.set_prev(tok.kind);
                }

                // ── Type keywords — mark pending_type for brace context ──────
                TokenKind::KwClass
                | TokenKind::KwStruct
                | TokenKind::KwUnion
                | TokenKind::KwEnum => {
                    self.flush_blank_lines();
                    if self.at_line_start {
                        self.indent();
                    } else if self.needs_space(tok.kind) {
                        self.space();
                    }
                    self.pending_type = true;
                    self.write(tok.lexeme);
                    self.set_prev(tok.kind);
                }

                // ── Comma — newline after each element in large initializers ──
                TokenKind::Comma => {
                    self.flush_blank_lines();
                    if self.at_line_start {
                        self.indent();
                    }
                    self.write(",");
                    // A comma at statement level ends the current assignment expression.
                    if self.paren_depth == 0 && self.bracket_depth == 0 {
                        self.assign_col = None;
                    }
                    if (self.large_init_stack.last() == Some(&true)
                        || self
                            .brace_stack
                            .last()
                            .is_some_and(|ctx| *ctx == BraceCtx::Type))
                        && self.paren_depth == 0
                    {
                        // If a trailing line comment follows on the same source line,
                        // let the CommentLine handler close the line instead.
                        if self.peek_inline_line_comment(tok.span.line) {
                            // nothing — CommentLine will emit the trailing \n
                        } else {
                            self.nl();
                            self.skip_next_newline = true;
                        }
                    }
                    self.set_prev(TokenKind::Comma);
                }

                // ── Assignment operators — track RHS column for continuation ──
                TokenKind::Eq
                | TokenKind::PlusEq
                | TokenKind::MinusEq
                | TokenKind::StarEq
                | TokenKind::SlashEq
                | TokenKind::PercentEq
                | TokenKind::AmpEq
                | TokenKind::PipeEq
                | TokenKind::CaretEq
                | TokenKind::LtLtEq
                | TokenKind::GtGtEq => {
                    self.flush_blank_lines();
                    if self.at_line_start {
                        self.indent();
                    } else if self.needs_space(tok.kind) {
                        self.space();
                    }
                    self.write(tok.lexeme);
                    if self.paren_depth == 0 && self.bracket_depth == 0 && self.assign_col.is_none()
                    {
                        let space = usize::from(self.config.spacing.space_around_binary_ops);
                        self.assign_col = Some(self.current_col + space);
                        self.assign_eol = false;
                    }
                    self.set_prev(tok.kind);
                }

                // ── Goto labels: `identifier:` at statement level ─────────────
                TokenKind::Ident
                    if !self.config.indent.indent_goto_labels
                        && self.at_line_start
                        && self.paren_depth == 0
                        && self.bracket_depth == 0
                        && self.template_depth == 0
                        && self.ternary_depth == 0
                        && !self.in_case_label
                        && !self.in_access_label
                        && !self
                            .brace_stack
                            .last()
                            .is_some_and(|ctx| *ctx == BraceCtx::Type)
                        && self.peek_non_ws_kind() == Some(TokenKind::Colon) =>
                {
                    self.flush_blank_lines();
                    // Emit at column 0 — no indentation call.
                    self.write(tok.lexeme);
                    self.in_goto_label = true;
                    self.set_prev(tok.kind);
                }

                // ── Arrow `->`: return type vs member access ──────────────────
                TokenKind::Arrow => {
                    self.flush_blank_lines();
                    let is_return_type = {
                        let before_rparen = self.prev == Some(TokenKind::RParen);
                        let mut i = self.pos;
                        // skip whitespace — find the first real token after `->`
                        while i < self.tokens.len()
                            && matches!(
                                self.tokens[i].kind,
                                TokenKind::Whitespace | TokenKind::Newline
                            )
                        {
                            i += 1;
                        }
                        // That first real token should be a type-like identifier or keyword.
                        let first = self.tokens.get(i);
                        let first_is_type = first.map_or(false, |t| {
                            matches!(t.kind, TokenKind::Ident | TokenKind::Keyword)
                        });
                        // Look ahead past the type to see if the next token is `{` (function body).
                        let mut j = i + 1;
                        while j < self.tokens.len()
                            && matches!(
                                self.tokens[j].kind,
                                TokenKind::Whitespace | TokenKind::Newline
                            )
                        {
                            j += 1;
                        }
                        let after_is_lbrace = self
                            .tokens
                            .get(j)
                            .map_or(false, |t| t.kind == TokenKind::LBrace);
                        before_rparen && first_is_type && after_is_lbrace
                    };
                    if !self.at_line_start && is_return_type {
                        self.space();
                    }
                    self.write(tok.lexeme);
                    if is_return_type {
                        self.after_rparen_arrow = true;
                    }
                    self.set_prev(TokenKind::Arrow);
                }

                // ── `else` — keep `else if` on one line ─────────────────────
                TokenKind::KwElse => {
                    self.flush_blank_lines();

                    if self.at_line_start {
                        self.indent();
                    } else if self.needs_space(tok.kind) {
                        self.space();
                    }

                    self.write(tok.lexeme);
                    self.set_prev(tok.kind);

                    // If followed by `if`, skip the next newline so `else if`
                    // stays on one line regardless of source formatting.
                    if self.peek_non_ws_kind() == Some(TokenKind::KwIf) {
                        self.skip_next_newline = true;
                    }
                }

                // ── Everything else ───────────────────────────────────────────
                _ => {
                    self.flush_blank_lines();

                    if self.at_line_start {
                        self.indent();
                    } else if self.needs_space(tok.kind) {
                        self.space();
                    }

                    // Track `extern "C"` sequence for ExternC brace context.
                    // Keep the flag alive across the LitStr (`"C"`); set it on `extern`;
                    // clear it on anything else that breaks the sequence.
                    if !(tok.kind == TokenKind::LitStr && self.pending_extern_c) {
                        self.pending_extern_c =
                            tok.kind == TokenKind::Keyword && tok.lexeme == "extern";
                    }

                    self.write(tok.lexeme);
                    self.set_prev(tok.kind);
                    // `operator` keyword — suppress spacing before the overloaded symbol.
                    if tok.kind == TokenKind::Keyword && tok.lexeme == "operator" {
                        self.after_operator_kw = true;
                    }
                    // Ternary `?` — track depth so the matching `:` gets spaces.
                    if tok.kind == TokenKind::Question {
                        self.ternary_depth += 1;
                    }
                }
            }
        }

        // Normalise any \r\n or \r remaining in the output to the configured style.
        let nl = self.config.newline_str();
        if nl != "\n" {
            let output = self.output.replace("\r\n", "\n").replace('\r', "\n");
            self.output = output.replace('\n', nl);
        }

        Ok(self.output)
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn format<'src>(tokens: &[Token<'src>], config: &Config) -> Result<String, FunkyError> {
    let injected;
    let tokens: &[Token<'src>] = if config.braces.add_braces_to_if
        || config.braces.add_braces_to_while
        || config.braces.add_braces_to_for
    {
        injected = inject_braces_pass(tokens, config);
        &injected
    } else {
        tokens
    };
    let output = Fmt::new(config, tokens).format()?;
    let nl = config.newline_str();
    let output = if config.spacing.align_right_cmt_span > 0 {
        let normalize_single = config.spacing.align_right_cmt_style == AlignCmtStyle::All;
        align_trailing_comments(
            &output,
            nl,
            config.spacing.align_right_cmt_gap.max(1),
            normalize_single,
            config.spacing.align_on_tabstop,
            config.indent.width as usize,
            config.spacing.align_right_cmt_span,
        )
    } else {
        output
    };
    let output = if config.spacing.align_enum_equ_span > 0 {
        align_enum_equals(
            &output,
            nl,
            config.spacing.align_on_tabstop,
            config.indent.width as usize,
        )
    } else {
        output
    };
    let output = if config.spacing.align_doxygen_cmt_span > 0 {
        align_doxygen_comments(
            &output,
            nl,
            config.spacing.align_on_tabstop,
            config.indent.width as usize,
        )
    } else {
        output
    };
    Ok(output)
}

/// Normalizes the whitespace between `#endif` and a trailing `/*` comment to
/// exactly `spaces` spaces. Lines with no `/*` are returned unchanged.

/// Scan `s` and insert a space after every `,` that is not already followed
/// by whitespace and is not inside a string/char literal or comment.
fn add_space_after_comma(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            // String literal — copy verbatim.
            '"' => {
                out.push('"');
                loop {
                    match chars.next() {
                        None => break,
                        Some('\\') => {
                            out.push('\\');
                            if let Some(esc) = chars.next() {
                                out.push(esc);
                            }
                        }
                        Some('"') => {
                            out.push('"');
                            break;
                        }
                        Some(ch) => out.push(ch),
                    }
                }
            }
            // Char literal — copy verbatim.
            '\'' => {
                out.push('\'');
                loop {
                    match chars.next() {
                        None => break,
                        Some('\\') => {
                            out.push('\\');
                            if let Some(esc) = chars.next() {
                                out.push(esc);
                            }
                        }
                        Some('\'') => {
                            out.push('\'');
                            break;
                        }
                        Some(ch) => out.push(ch),
                    }
                }
            }
            '/' => {
                out.push('/');
                match chars.peek() {
                    // Block comment — copy verbatim.
                    Some('*') => {
                        out.push('*');
                        chars.next();
                        let mut prev = ' ';
                        loop {
                            match chars.next() {
                                None => break,
                                Some(ch) => {
                                    if prev == '*' && ch == '/' {
                                        out.push(ch);
                                        break;
                                    }
                                    out.push(ch);
                                    prev = ch;
                                }
                            }
                        }
                    }
                    // Line comment — copy rest of string verbatim.
                    Some('/') => {
                        for ch in chars.by_ref() {
                            out.push(ch);
                        }
                    }
                    _ => {}
                }
            }
            ',' => {
                out.push(',');
                // Insert a space unless next char is already whitespace.
                if !matches!(
                    chars.peek(),
                    None | Some(' ') | Some('\t') | Some('\n') | Some('\r')
                ) {
                    out.push(' ');
                }
            }
            ch => out.push(ch),
        }
    }
    out
}

/// Apply `space_around_binary_ops` formatting to the condition part of a
/// `#if` or `#elif` directive line.  The input `line` is the full directive
/// (starting with `#`, ending with the newline string `nl`).  Returns the
/// reformatted directive, or the original if it doesn't match.
fn format_preproc_if_condition(line: &str, nl: &str) -> String {
    // Multi-line conditions (backslash continuations) can't be safely
    // reformatted — pass through verbatim.
    if line.contains("\\\n") || line.contains("\\\r\n") || line.contains("\\\r") {
        return line.to_string();
    }

    // Split off the trailing newline (or newline sequence).
    let (body, trailing_nl) = if let Some(b) = line.strip_suffix(nl) {
        (b, nl)
    } else if let Some(b) = line.strip_suffix('\n') {
        (b, "\n")
    } else {
        (line, "")
    };

    // Parse `# <ws>* (if|elif) <ws>+` prefix.
    let after_hash = match body.strip_prefix('#') {
        Some(s) => s,
        None => return line.to_string(),
    };
    let after_hash = after_hash.trim_start_matches(|c: char| c == ' ' || c == '\t');
    let (kw, rest) = if after_hash.starts_with("elif")
        && !after_hash[4..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
    {
        ("elif", &after_hash[4..])
    } else if after_hash.starts_with("if")
        && !after_hash[2..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
    {
        ("if", &after_hash[2..])
    } else {
        return line.to_string();
    };

    let condition_and_rest = rest.trim_start_matches(|c: char| c == ' ' || c == '\t');

    // Separate any trailing `/* comment */` from the condition.
    let (condition_raw, trailing_comment) = if let Some(pos) = condition_and_rest.find("/*") {
        (
            condition_and_rest[..pos].trim_end(),
            &condition_and_rest[pos..],
        )
    } else {
        (condition_and_rest.trim_end(), "")
    };

    let formatted = format_preproc_expr(condition_raw);

    if trailing_comment.is_empty() {
        format!("#{kw} {formatted}{trailing_nl}")
    } else {
        format!("#{kw} {formatted} {trailing_comment}{trailing_nl}")
    }
}

/// Operator-only predicate: returns true for characters that begin/continue
/// an operator token in a preprocessor condition.
fn is_preproc_op_char(c: char) -> bool {
    matches!(
        c,
        '>' | '<' | '!' | '=' | '&' | '|' | '+' | '-' | '*' | '/' | '%' | '~' | '^'
    )
}

/// Reformat a preprocessor condition expression with spaces around binary
/// operators.  Identifiers, numbers, and parens are kept as-is.
fn format_preproc_expr(expr: &str) -> String {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum TK {
        Ident,
        Number,
        Op,
        LParen,
        RParen,
        Comma,
    }

    let mut toks: Vec<(TK, &str)> = Vec::new();
    let bytes = expr.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = expr[i..].chars().next().unwrap();
        match c {
            c if c.is_whitespace() => {
                i += c.len_utf8();
            }
            '(' => {
                toks.push((TK::LParen, &expr[i..i + 1]));
                i += 1;
            }
            ')' => {
                toks.push((TK::RParen, &expr[i..i + 1]));
                i += 1;
            }
            ',' => {
                toks.push((TK::Comma, &expr[i..i + 1]));
                i += 1;
            }
            // Character and string literals — scan to the closing quote,
            // respecting backslash escapes, and emit the whole thing as Ident
            // so the reconstruction logic never splits them or adds spaces inside.
            '\'' | '"' => {
                let quote = c;
                let start = i;
                i += 1;
                while i < expr.len() {
                    let nc = expr[i..].chars().next().unwrap();
                    i += nc.len_utf8();
                    if nc == '\\' {
                        // Skip escaped character.
                        if i < expr.len() {
                            let escaped = expr[i..].chars().next().unwrap();
                            i += escaped.len_utf8();
                        }
                    } else if nc == quote {
                        break;
                    }
                }
                toks.push((TK::Ident, &expr[start..i]));
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                i += c.len_utf8();
                while i < expr.len() {
                    let nc = expr[i..].chars().next().unwrap();
                    if nc.is_alphanumeric() || nc == '_' {
                        i += nc.len_utf8();
                    } else {
                        break;
                    }
                }
                toks.push((TK::Ident, &expr[start..i]));
            }
            c if c.is_ascii_digit() => {
                let start = i;
                i += 1;
                while i < expr.len() {
                    let nc = expr[i..].chars().next().unwrap();
                    if nc.is_alphanumeric() || nc == '_' || nc == '.' {
                        i += nc.len_utf8();
                    } else {
                        break;
                    }
                }
                toks.push((TK::Number, &expr[start..i]));
            }
            c if is_preproc_op_char(c) => {
                let start = i;
                i += 1;
                // Two-char operators: >=, <=, ==, !=, &&, ||, <<, >>
                if i < expr.len() {
                    let nc = expr[i..].chars().next().unwrap();
                    if is_preproc_op_char(nc)
                        && matches!(
                            (&expr[start..i], nc),
                            (">" | "<" | "!" | "=" | "&" | "|", '=')
                                | ("&", '&')
                                | ("|", '|')
                                | ("<", '<')
                                | (">", '>')
                        )
                    {
                        i += nc.len_utf8();
                    }
                }
                toks.push((TK::Op, &expr[start..i]));
            }
            c => {
                // Pass through anything else (e.g. `\` in edge cases)
                toks.push((TK::Ident, &expr[i..i + c.len_utf8()]));
                i += c.len_utf8();
            }
        }
    }

    // Reconstruct with spacing.
    let mut out = String::with_capacity(expr.len() + 16);
    for (idx, &(tk, s)) in toks.iter().enumerate() {
        let prev_tk = idx.checked_sub(1).map(|j| toks[j].0);
        let is_unary = tk == TK::Op
            && matches!(
                prev_tk,
                None | Some(TK::LParen) | Some(TK::Op) | Some(TK::Comma)
            );
        match tk {
            TK::LParen => {
                // No space before `(` after ident/number (function-call style).
                let no_space_before = matches!(prev_tk, Some(TK::Ident) | Some(TK::RParen));
                if !no_space_before && !out.is_empty() {
                    out.push(' ');
                }
                out.push('(');
            }
            TK::RParen => {
                out.push(')');
            }
            TK::Comma => {
                out.push(',');
            }
            TK::Op if is_unary => {
                // Unary: space before (if needed) but NOT after.
                if !out.is_empty() && !matches!(prev_tk, Some(TK::LParen)) {
                    // No space after `(` — but after other tokens, add space.
                    match prev_tk {
                        Some(TK::Op) if out.ends_with(|c: char| !c.is_whitespace()) => {
                            // Another op just before — keep as-is (e.g. `!!`)
                        }
                        _ => {}
                    }
                }
                out.push_str(s);
            }
            TK::Op => {
                // Binary: space before and after.
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(s);
                out.push(' ');
                // Don't push the trailing space yet — we'll push it as the
                // "needs_space" of the NEXT token, to avoid trailing spaces.
                // Actually just push it; if the next token is `)` we'll get
                // `x > )` which is wrong.  Handle specially:
                // Actually: we pushed the space unconditionally. The next
                // iteration will decide whether to add space too. Since `)` and
                // `,` don't check prev_space, they'll just push without extra.
                // The space we just added will stay.  That's fine for binary
                // op followed by `)` which shouldn't occur in valid C anyway.
            }
            TK::Ident | TK::Number => {
                // Space before if previous was a value token or `)`.
                let needs_space = matches!(
                    prev_tk,
                    Some(TK::Ident) | Some(TK::Number) | Some(TK::RParen)
                );
                if needs_space || (out.ends_with(' ') && false) {
                    // `out.ends_with(' ')` is already handled by Op above.
                }
                if needs_space {
                    out.push(' ');
                }
                out.push_str(s);
            }
        }
    }

    // Trim trailing space left by binary op at end (shouldn't happen, but safe).
    out.trim_end().to_string()
}

fn normalize_endif_spacing(line: &str, spaces: u32) -> String {
    if let Some(pos) = line.find("/*") {
        let before = line[..pos].trim_end();
        let gap = " ".repeat(spaces as usize);
        format!("{before}{gap}{}", &line[pos..])
    } else {
        line.to_string()
    }
}

/// Round `n` up to the next multiple of `step` (or `n` itself if already a multiple).
fn round_up_to_multiple(n: usize, step: usize) -> usize {
    if step == 0 {
        return n;
    }
    n.div_ceil(step) * step
}

/// Returns the byte index of the `//` or `/*` that starts a trailing inline
/// comment on `line`, or `None` if the line has no trailing comment (standalone
/// comment lines and blank lines also return `None`).
///
/// A `/* */` comment is only considered trailing when nothing non-whitespace
/// follows its closing `*/` — this prevents mid-expression block comments like
/// `2 /* two bytes */ +` from being treated as trailing comments.
fn trailing_comment_col(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("/*") {
        return None;
    }
    // Preprocessor lines (#endif, #else, #ifdef, etc.) have their own spacing
    // rules and must not be included in trailing-comment alignment groups.
    if trimmed.starts_with('#') {
        return None;
    }
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'/' && (bytes[i + 1] == b'/' || bytes[i + 1] == b'*') {
            // Skip /**< — those are Doxygen member comments handled by their own pass.
            if bytes[i + 1] == b'*'
                && bytes.get(i + 2) == Some(&b'*')
                && bytes.get(i + 3) == Some(&b'<')
            {
                i += 1;
                continue;
            }
            // Skip `://` — part of a URL (e.g. `https://`), not a comment.
            if bytes[i + 1] == b'/' && i > 0 && bytes[i - 1] == b':' {
                i += 1;
                continue;
            }
            let before = &line[..i];
            if before.bytes().any(|b| b != b' ' && b != b'\t') {
                if bytes[i + 1] == b'/' {
                    // `//` extends to end of line — always trailing.
                    return Some(i);
                }
                // `/* */` — only trailing when nothing non-whitespace follows `*/`.
                let mut j = i + 2;
                while j + 1 < bytes.len() {
                    if bytes[j] == b'*' && bytes[j + 1] == b'/' {
                        let after = &line[j + 2..];
                        if !after.bytes().any(|b| b != b' ' && b != b'\t') {
                            return Some(i);
                        }
                        break; // code after `*/` — not a trailing comment
                    }
                    j += 1;
                }
            }
        }
        i += 1;
    }
    None
}

/// Align trailing `//` comments within groups of lines carrying trailing
/// comments.  Lines without a comment are allowed inside a group when they
/// are no more than `span` non-commented lines away from the next commented
/// line (matches uncrustify's `align_right_cmt_span` semantics).
/// `min_gap` is the minimum number of spaces between code end and comment.
fn align_trailing_comments(
    output: &str,
    nl: &str,
    min_gap: usize,
    normalize_single: bool,
    on_tabstop: bool,
    tab_width: usize,
    span: usize,
) -> String {
    let lines: Vec<&str> = output.split(nl).collect();
    let n = lines.len();
    let cols: Vec<Option<usize>> = lines.iter().map(|l| trailing_comment_col(l)).collect();
    let mut result: Vec<String> = lines.iter().map(|l| l.to_string()).collect();

    let mut i = 0;
    while i < n {
        if cols[i].is_some() {
            // Extend the group as long as the next commented line is within
            // `span` non-commented lines of the last commented line found.
            let mut last_cmt = i;
            let mut scan = i + 1;
            loop {
                // Find the next commented line from `scan`.
                let next = (scan..n).find(|&k| cols[k].is_some());
                match next {
                    Some(k) if k - last_cmt < span => {
                        last_cmt = k;
                        scan = k + 1;
                    }
                    _ => break,
                }
            }
            let j = last_cmt + 1; // exclusive end of group

            let commented_in_group = (i..j).filter(|&k| cols[k].is_some()).count();
            let is_single = commented_in_group == 1;
            // Skip single-line groups unless normalize_single is set.
            if is_single && !normalize_single {
                i = j;
                continue;
            }
            let max_code_len = (i..j)
                .filter(|&k| cols[k].is_some())
                .map(|k| lines[k][..cols[k].unwrap()].trim_end().len())
                .max()
                .unwrap();
            let raw_target = max_code_len + min_gap;
            let target = if on_tabstop && tab_width > 0 {
                round_up_to_multiple(raw_target, tab_width)
            } else {
                raw_target
            };
            for k in i..j {
                if let Some(col) = cols[k] {
                    let code = lines[k][..col].trim_end();
                    let comment = &lines[k][col..];
                    let pad = target.max(code.len() + 1) - code.len();
                    result[k] = format!("{}{}{}", code, " ".repeat(pad), comment);
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }

    result.join(nl)
}

/// Returns the byte index of the `=` assignment operator in an enum value line
/// (e.g. `    FOO = 3,`), or `None` if the line isn't an enum value with `=`.
///
/// Requires the line to end with `,` to avoid false-positives inside function
/// bodies. Skips compound operators (`==`, `!=`, `<=`, `>=`, `+=`, …).
fn enum_eq_col(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with(|c: char| c.is_alphabetic() || c == '_') {
        return None;
    }
    // Enum members end with `,` (non-last) or with an alphanumeric/`_`/`)` (last
    // member).  Reject anything that ends with `;`, `{`, `}`, etc. to avoid
    // false-positives on declarations or initialiser lines.
    let last = trimmed.trim_end().chars().last().unwrap_or(' ');
    if !matches!(last, ',' | ')') && !last.is_alphanumeric() && last != '_' {
        return None;
    }
    let bytes = line.as_bytes();
    let mut in_string = false;
    let mut in_char = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if in_string || in_char => {
                i += 2; // skip escaped character
                continue;
            }
            b'"' if !in_char => {
                in_string = !in_string;
            }
            b'\'' if !in_string => {
                in_char = !in_char;
            }
            b'=' if !in_string && !in_char => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                    i += 2; // skip both `=` of `==`
                    continue;
                }
                if i > 0
                    && matches!(
                        bytes[i - 1],
                        b'!' | b'<' | b'>' | b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^'
                    )
                {
                    i += 1;
                    continue; // compound op
                }
                return Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// True when `line` is a comment-only line that should not break an enum
/// alignment group.  Matches single-line `//` comments and single-line
/// `/* ... */` block comments (including `/** ... */` doc comments).
fn is_enum_comment_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("//") || (trimmed.starts_with("/*") && trimmed.trim_end().ends_with("*/"))
}

/// True when `line` looks like a bare enum member with no explicit value
/// (e.g. `    RED,` or `    RED, // comment`).  Used to let bare members
/// act as transparent connectors within an alignment group so that
/// `RED, GREEN = 5, BLUE, YELLOW = 10` all align their `=` signs together.
fn is_bare_enum_member(line: &str) -> bool {
    let trimmed = line.trim_start();
    if !trimmed.starts_with(|c: char| c.is_alphabetic() || c == '_') {
        return false;
    }
    if !trimmed.contains(',') {
        return false;
    }
    // No bare `=` — those are captured by enum_eq_col instead.
    !trimmed.contains('=')
}

/// Align `=` signs within groups of consecutive enum value lines.
/// Bare enum members (no explicit value) are transparent within a group —
/// they don't break alignment but are left unchanged themselves.
fn align_enum_equals(output: &str, nl: &str, on_tabstop: bool, tab_width: usize) -> String {
    let lines: Vec<&str> = output.split(nl).collect();
    let n = lines.len();
    let cols: Vec<Option<usize>> = lines.iter().map(|l| enum_eq_col(l)).collect();
    let mut result: Vec<String> = lines.iter().map(|l| l.to_string()).collect();

    let mut i = 0;
    while i < n {
        if cols[i].is_some() {
            // Extend the group through bare members, blank lines, and preprocessor
            // directives (#ifdef/#endif/#else) as well as `=` members.  All of these
            // are transparent connectors — they don't break the alignment group.
            // This matches uncrustify's behavior of aligning enum members across
            // conditional-compilation blocks.
            let mut j = i + 1;
            while j < n
                && (cols[j].is_some()
                    || is_bare_enum_member(lines[j])
                    || lines[j].trim().is_empty()
                    || lines[j].trim_start().starts_with('#')
                    || is_enum_comment_line(lines[j]))
            {
                j += 1;
            }
            // Trim trailing blank/bare lines so they don't become orphaned group members.
            while j > i + 1 && cols[j - 1].is_none() {
                j -= 1;
            }
            // Collect indices of only the `=`-bearing lines in this group.
            let eq_indices: Vec<usize> = (i..j).filter(|&k| cols[k].is_some()).collect();
            if eq_indices.len() > 1 {
                let max_name_len = eq_indices
                    .iter()
                    .map(|&k| lines[k][..cols[k].unwrap()].trim_end().len())
                    .max()
                    .unwrap();
                let raw_target = max_name_len + 1;
                let target = if on_tabstop && tab_width > 0 {
                    round_up_to_multiple(raw_target, tab_width)
                } else {
                    raw_target
                };
                for k in eq_indices {
                    let col = cols[k].unwrap();
                    let name = lines[k][..col].trim_end();
                    let rest = &lines[k][col..]; // starts with `= …`
                    let pad = target - name.len();
                    result[k] = format!("{}{}{}", name, " ".repeat(pad), rest);
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }

    result.join(nl)
}

/// Returns the byte index of the `/**<` that starts a Doxygen member comment
/// on `line`, or `None` if there is no such trailing comment.  Standalone
/// comment lines (where `/**<` is the first non-whitespace) return `None`.
fn trailing_doxygen_col(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with("/**<") {
        return None;
    }
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i] == b'/' && bytes[i + 1] == b'*' && bytes[i + 2] == b'*' && bytes[i + 3] == b'<'
        {
            let before = &line[..i];
            if before.bytes().any(|b| b != b' ' && b != b'\t') {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// True for a struct/class member line that has no `/**<` comment but should not
/// break an alignment group — blank lines and closing-brace lines do break it.
fn is_transparent_doxygen_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    !trimmed.is_empty() && !line.starts_with('}')
}

/// Align trailing `/**<` Doxygen member comments within groups of consecutive
/// lines that all carry such a comment.  Comment-less member lines (e.g. a field
/// with no doc) are transparent: they extend the group without being rewritten
/// themselves.  Blank lines and closing-brace lines break the group.
fn align_doxygen_comments(output: &str, nl: &str, on_tabstop: bool, tab_width: usize) -> String {
    let lines: Vec<&str> = output.split(nl).collect();
    let n = lines.len();
    let cols: Vec<Option<usize>> = lines.iter().map(|l| trailing_doxygen_col(l)).collect();
    let mut result: Vec<String> = lines.iter().map(|l| l.to_string()).collect();

    let mut i = 0;
    while i < n {
        if cols[i].is_some() {
            // Extend the group through comment-less member lines as well.
            let mut j = i + 1;
            while j < n && (cols[j].is_some() || is_transparent_doxygen_line(lines[j])) {
                j += 1;
            }
            // Trim trailing transparent lines that have no /**< after them.
            while j > i + 1 && cols[j - 1].is_none() {
                j -= 1;
            }
            let commented: Vec<usize> = (i..j).filter(|&k| cols[k].is_some()).collect();
            if commented.len() > 1 {
                let max_code_len = commented
                    .iter()
                    .map(|&k| lines[k][..cols[k].unwrap()].trim_end().len())
                    .max()
                    .unwrap();
                let raw_target = max_code_len + 1;
                let computed_min = if on_tabstop && tab_width > 0 {
                    round_up_to_multiple(raw_target, tab_width)
                } else {
                    raw_target
                };
                let max_existing = commented.iter().map(|&k| cols[k].unwrap()).max().unwrap();
                let target = computed_min.max(max_existing);
                for k in commented {
                    let col = cols[k].unwrap();
                    let code = lines[k][..col].trim_end();
                    let comment = &lines[k][col..];
                    let pad = target.max(code.len() + 1) - code.len();
                    result[k] = format!("{}{}{}", code, " ".repeat(pad), comment);
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }

    result.join(nl)
}
