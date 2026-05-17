# Developer Log: CoreFn Translation & Interpreter Implementation

## Overview
This log summarizes the development of the code generation and execution pipeline for the PureScript Alexandrite compiler, moving it from a static analyzer to a functional compiler and interpreter.

## 1. Architectural Design
The project was extended to support a standard functional compiler pipeline:
- **CoreFn IR**: A minimal, typed lambda calculus representing the desugared program.
- **Evidence Recording**: A mechanism in the type-checker to track how constraints (like `Eq a`) are solved.
- **Elaboration**: The process of injecting dictionaries and desugaring high-level PureScript constructs (like `if-then-else`) into CoreFn.
- **Interpreter**: A tree-walking evaluator that executes CoreFn.

## 2. New Crates
### `compiler-core/corefn`
- Defines the IR: `Expr`, `Binder`, `Literal`, `Var`, and `Declaration`.
- Supports serialization/deserialization for snapshot testing.
- Uses a minimal set of primitives suitable for both interpretation and future JS/WASM backends.

### `compiler-core/elaborating`
- Translates `LoweredModule` + `CheckedModule` -> `CoreFnModule`.
- **Key Logic**:
  - Preserves local names for variables and let-bindings.
  - Injects "evidence" (dictionaries) into applications of constrained functions.
  - Desugars `IfThenElse` into CoreFn `Case` expressions.
  - Supports `Lambda`, `Let`, `Array`, `Record`, and `Constructor` expressions.

### `compiler-core/evaluating`
- The execution engine.
- **Value System**: Supports integers, strings, booleans, closures, and data constructors.
- **Environment**: Manages lexical scoping.
- **Recursion Support**: Uses a shared `Arc<RwLock>` module environment, allowing closures to resolve top-level names across the entire module.
- **FFI**: A native Rust function bridge (`Value::Foreign`) for host-provided logic.

## 3. Integration & Testing
- **New Test Categories**: `elaborating` (for IR verification) and `evaluating` (for execution verification).
- **FFI Mocking**: The test runner was enhanced to dynamically register common FFIs:
  - `log`: Captures output in a shared buffer and executes as a PureScript `Effect`.
  - `add`, `sub`, `eq`: Provides basic arithmetic and comparison.

### Verified Fixtures:
1. `01_simple_let`: Complex local scoping and lambdas.
2. `02_simple_eq`: Basic typeclass dictionary injection.
3. `03_ffi_log`: PureScript `Effect` handling and side-effect capturing.
4. `04_recursion`: A recursive Fibonacci implementation (`fib 5` -> `5`).

## 4. Key Milestones
- **IR & Evidence**: Established the "plumbing" between type-checking and code-gen.
- **Literal Values**: Fixed a major gap in the lowering phase where literal values were being discarded.
- **Recursive Scoping**: Solved the "chicken-and-egg" problem of module-level recursion in the interpreter.
- **Effect Execution**: Correctly implemented the PureScript `Effect` pattern (thunking side effects).

## Next Steps
- **Desugaring Phase**: Move `do` and `ado` blocks from elaboration into a dedicated desugaring pass.
- **Full Dictionary Injection**: Complete the mapping of instance members into dictionary records.
- **Standard Library**: Register more FFIs to support the standard `Prelude`.
