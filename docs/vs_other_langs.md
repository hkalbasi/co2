# CO2 vs other system languages and similar tools

## CO2 vs Carbon

Carbon is Google's experimental successor to C++, designed for large-scale migration from C++ codebases.
There is a huge intersection between Carbon and CO2 goals, the major difference is that Carbon is focused
on C++ but CO2 targets C.

We can [imagine a CO2++ language](./vision/lingua_franca.md) which is backward compatible with C++,
and has similar goals to CO2 but with C++ instead of C as the base language.
Comparing CO2++ with Carbon is more apple to apple.

Carbon wants to be a new language, which is (somehow?) memory safe and is free of "decades of technical dept",
while maintaining interoperability with the old C++. CO2++ does not do anything on making the C++ better,
it sticks to the C++ syntax and semantics, and is backward compatible to the C++ so it can't do anything
about safety or fixing C++ technical depts. But if you consider Rust and CO2++ as a package,
they can compete with Carbon. Rust is the memory safe and modern language part,
while CO2++ acts as the interop and migration layer.

Benefits of the Rust/CO2++ combo over Carbon:
* A solid safety story
  * Rust is fully safe, CO2++ is fully unsafe (but can declare its functions as safe) and for more safety you can migrate more code to Rust.
  * Carbon safety is under design as of writing, with major parts (like the borrow checker like system) remaining unknown.
* Ability of using the mature Rust and C++ ecosystems
  * Carbon have access to the C++ ecosystem, but for achieving its goals maximally, it needs safe and modern Carbon libraries.
    But building Carbon libraries for everything needs a huge amount of time and resources (see [Network Effect](#network-effect-for-programming-languages))
  * CO2++ can easily use Rust crates and `#include` C++ libraries. And there is no preference for CO2++ libraries against Rust libraries.
* CO2++ syntax is just C++
  * Target audience of Carbon already know C++, but they should learn another very different syntax.

## CO2 vs C2Rust

C2Rust is a tool that automatically translates C code into unsafe Rust,
with the goal of helping projects migrate away from C entirely. It converts C syntax into equivalent Rust,
producing `.rs` files that can be compiled with `rustc`. Once translated,
the idea is that developers gradually refactor the generated unsafe Rust into safe, idiomatic Rust.

CO2 also enables you to migrate C projects to Rust, but instead of a machine generated C like Rust,
you can structure your code as CO2 crates and rewrite them in Rust one crate at a time.

Benefits of CO2 over C2Rust:
* Working on files with C-like syntax, not a alien and machine generated unsafe Rust.
* Clear boundaries over what is real Rust and what is C in the Rust skin.
* C2Rust makes `unsafe` keyword become a noise and lose its value.
* C2Rust is only for Rust migration, but CO2 code is usable on its own.

Benefits of C2Rust over CO2:
* CO2 does not support all Rust tooling currently, specially third party ones.

## CO2 vs bindge, cbindge, Zngur, CXX, Crubit, ...

There are other family of tools for interoperability between C/C++ and Rust.
These tools are binding generators: They generate some code in both language to bridge things.
Some of these tools are C++ only, which we compare against the [imaginary CO2++](./vision/lingua_franca.md)
language instead of CO2 itself.

Benefits of CO2 over bindgen and cbindgen:
* Unlike CO2, they limit Rust side to a narrow C-like API, so the Rust side needs a wrapper.
* C side is always unsafe to call in those tools, but CO2 gives the C author the control to declare their functions as either safe or unsafe. 
  CO2 functions can also receive things like references and smart pointers to ensure the API is safe to call.
* Miri works with CO2 and can detect UB in the mixed code.
* A CO2 code is probably faster than a bindgen bridged code:
  * Lack of wrapper overhead
  * Better optimizations. There is a cross language LTO, but is hard to setup and limited.
Benefits of bindgen and cbindgen over CO2:
* With these tools, the C side remains almost untouched.
* You have more options on C compiler and build systems.
* CO2 has a higher upfront adoption cost.

For C++, binding generator tools do more job. Unlike C, Rust doesn't natively handle C++ types.
So C++ binding generators generate more code to handle more C++ and Rust types, including templates/generics and smart pointers.
CO2++ currently doesn't exist, but we can expect it to have these pros and cons:

Benefits of CO2++ over Zngur, CXX, Crubit, ...:
* These tools either giveup on templates/generics, or require the user to specify monomorphizations they want.
  CO2++ (like CO2 itself) can use generics of other crates without any problem.
* Some of these tools require heavy and complex integration with the build system. CO2++ wants Cargo as the build system,
  but it makes very simple Cargo projects with no build scripts or complex build pipeline.
* Some of these tools require declaring the boundary in something similar to a C header file.
  CO2++ crates does not need any interface declaration, and can export items directly from the crate.
* Miri works with CO2++ and can detect UB in the mixed code.
* A CO2 code is probably faster than a bindgen bridged code:
  * Lack of wrapper overhead. These C++ binding generators specially generate some invisible code (unlike C ones)
    which may have non trivial performance impact. Frequent cross language calls are discouraged in those tools due this reason.
  * Better optimizations. There is a cross language LTO, but is hard to setup and limited.
Benefits of Zngur, CXX, Crubit, ... over CO2++:
* You have more options on C++ compiler and build systems.
* CO2++ has a higher upfront adoption cost.

## CO2 vs Zig, Hare, C3, Odin, ...

See [CO2 as a C successor language](./c_successor.md).

## CO2 vs D's importC


## Network effect for programming languages

A network effect occurs when a product or system becomes more valuable as more people use it.
Telephones are the classic example: a single phone is useless,
but every new phone added to the network makes the entire network more valuable for everyone already on it.
Social networks, marketplaces, and communication protocols all exhibit network effects:
the more participants, the stronger the pull to join and stay.

Programming languages are subject to strong network effects. The value of a language is not just its design,
but the size and quality of its ecosystem: libraries, tooling, documentation, community, and the pool of developers who know it.
New languages face a steep uphill battle because early adopters must build everything from scratch while
the incumbent languages already have mature solutions for common problems.

CO2 is different from other new system programming languages, in that it borrows the C syntax and is able to use both C libraries
and Rust crates. So by losing the ability to innovate over syntax and similar things in the PL design,
it overcomes the network effect issue in both libraries and developer familiarity.
