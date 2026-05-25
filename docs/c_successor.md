# CO2 as a C successor language

## What people want from a C successor which CO2 has

### Modern package manager and build system

CO2 uses Cargo as the build system, which solves these problems with C system:
* Managing dependencies per project, having multiple versions of a dependency in a single project
* Cross compilation
* Link time optimization between dependencies
* Minimal build system configuration, but maximal features (testing, benchmarking, ...)

C can not have a Cargo-like build system with all features, because you will hit duplicate symbols with any non trivial dependency tree.
CO2 supports functions with mangled symbols, so you can use cargo with arbitrary complex trees.
Also there is a habit of distributing C libraries in binary form, limiting cross compilation and link time optimizations.
Other C successors may have fixed the language side problem, but they will need to use these binary distributed C libs,
losing the build system benefit. CO2 projects can use the entire ~250K Rust crates in crates.io, so it doesn't have this problem.

### Safety

CO2 is not a safe language, but it has some safety features, like most other C successors.
Some of these features are:
* Opt-in fat pointers which have bound checking
* Smart pointers like `Box<T>`, `Arc<T>`, ... with manual `drop`
* Borrow checker which warns (not error) about some use after free, missing `drop`, ...

More importantly, CO2 enables rewriting the safety critical parts of your project in Rust,
which is a memory safe language, and using memory safe dependencies.

### Getting rid of header files

Header files are a nice and simple way to manage code in multiple files and translation units,
but they are dreaded due to manual duplicate definitions, which may desync,
having unwanted implications on inlining and similar things, and their impact on compile times.

CO2 provide the same module/crate system of Rust, so you can avoid writing header files.

### Compatibility with C

Most C successors have a good story on compatibility with C, and it is a defining feature of a C successor.
C successors expect some C project to migrate to them,
and compatibility with C ensures that the migration is possible in an incremental way.

CO2 goes further than most other C successors,
because it is designed as a superset of C rather than a completely separate language.

## What people want from a C successor which CO2 has not

### Implementation simplicity

C is a simple language, and you can implement a compiler for a great subset of it in ~10K lines of code.
While CO2 repository has not much more codes than this, it is tightly coupled with the Rust compiler.
Writing a standalone CO2 compiler probably needs millions of lines of code. Some of the C successors,
like Hare, have implementation simplicity as a goal.

But implementation simplicity is not important on its own, and there are secondary goals which this is a proxy for them,
and CO2 achives some of these by other means:
* Portability: It is easy to port a simple compiler and language to other niche systems and archituctures,
  and C is the most portable language partly due to this. While porting Rust/CO2 toolchain is not an easy task by any definition of easy,
  massive amount of work is done on the Rust toolchain to make it portable. The Rust compiler supports many backends,
  including llvm, gcc, cranelift, and a C generating backend. CO2 inherits all of these,
  so it is much more portable than many of the simpler C successors.
* Learning curve: If a language is easy to implement, there is more chance that it is also easier to learn.
  CO2 is a bit special in this regard. It expects the target audience to know C, and if they know C they already know most of the CO2.
  There are some new concepts and syntaxs, but the learning curve is much simpler than a from zero language.
* Bootstrapping: Compilers are usually implemented in their own language, so for building them you need a version of them.
  The process of building a version of a self-host compiler from zero is called bootstrapping.
  Rust is not an easy language to bootstrap, but it is not very hard either and there are good tools like `mrustc` to help with this.
  CO2 is just as hard as Rust (you just need to build a Rust compiler since CO2 compiler is not self host and is written in Rust) but
  language with simpler implementations are usually easier to bootstrap.

### Modern macro/meta programming system

Like header files, preprocessor directives are a nice and simple macro system, but they have major shortcomings and footguns.
C successors usually have some interesting features in this area, which enables things like converting a struct to json.

While CO2 could use Rust macro system or invent its own system, it only supports the C preprocessor.
CO2 needed the C preprocessor for backward compatibility, and it is enough for some common usecases.
If you need more, instead of incorporating Rust macros, CO2 enables you to write that part of your code in Rust,
and use it in the rest of your CO2 project.
