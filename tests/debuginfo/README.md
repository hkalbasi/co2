# Debuginfo tests

Verify debug info emission by compiling with `-g`, running under GDB,
and checking debugger output against `//@ gdb-check` patterns with `[...]` wildcards.

Inspired by Rust's debuginfo tests.

## Directives

```
//@ gdb-command:<command>   — debugger command (run, print x, continue, whatis x, …)
//@ gdb-check:<pattern>     — expected debugger output line (with [...] wildcard matching)
```

Commands and checks are paired in order. Each check is matched against the next
unmatched line of debugger stdout. `[...]` matches any substring (zero or more chars).

## Breakpoints

Place `// #break` on the same line as a function call that should stop the debugger.
Breakpoints are set on all `// #break` lines before execution.
