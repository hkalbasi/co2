# Preproccessor

This part of code is adopted from [Claude's C Compiler](https://github.com/anthropics/claudes-c-compiler) which is generated
using LLM without human supervision. Before CCC, we were using `gcc -E`, and we had problems in matching spans.
So we needed our own preprocessor. But implementing a preprocessor is hard, and at that moment,
CCC had the most complete Rust based preprocessor.

The code is changed very much from the original CCC implementation, but you can still feel the LLM base of it.
I would happily accept a human rewrite of this crate, as long as it passes all tests, improves the code quality,
and doesn't create a performance regression. Please coordinate with me in advance if you want to do this,
since it is a huge amount of work.
