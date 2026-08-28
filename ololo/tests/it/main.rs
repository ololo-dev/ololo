//! Single integration-test binary for `ololo`.
//!
//! Every module here used to be its own `tests/*.rs` target — ~3
//! separate binaries each linking the full dependency stack, which dominated
//! CI compile time. One binary links once; nextest still runs the tests in
//! parallel (one process per test).

mod cli_smoke;
mod nc1_gate_probe;
