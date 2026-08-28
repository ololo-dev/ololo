//! Task and test adaptation subsystem.
//!
//! Provides LLM-backed rewriting of source tasks and generation of test
//! templates for registered agents. The subsystem is split into:
//!
//! - `service`: public trait + request/response/error types
//! - `live`: production LLM-backed implementation
//! - `loop_task`: background channel receiver + retry loop
//! - `fake`: deterministic test implementation with DB writes
//!
//! Production code wires `LiveAdaptationService`; tests inject
//! `FakeAdaptationService` via the trait boundary.
pub mod command_policy;
pub mod fake;
pub mod live;
pub mod loop_task;
pub mod metrics;
pub mod passthrough;
pub mod service;
