//! Single integration-test binary for `arena-core`.
//!
//! Every module here used to be its own `tests/*.rs` target — ~40
//! separate binaries each linking the full dependency stack, which dominated
//! CI compile time. One binary links once; nextest still runs the tests in
//! parallel (one process per test).

mod common;
mod entities;
mod git_store;
mod ids_ord;
mod join_code;
mod judging_agent_setup;
mod judging_appraisal;
mod judging_criteria_run;
mod judging_defs;
mod judging_dossier;
mod judging_evidence;
mod judging_gate;
mod judging_programs;
mod judging_run;
mod judging_run_errors;
mod judging_session_scope;
mod judging_status_clears_score;
mod judging_tools;
mod llm;
mod probe_engine_answer;
mod probe_engine_fixtures;
mod probe_engine_js;
mod protocol_agent;
mod protocol_arena;
mod protocol_methods;
mod protocol_player;
mod sandbox_self_check;
mod scoring_aggregates;
mod scoring_board;
mod scoring_winners;
mod session_completion;
mod session_status;
mod settings_encryption;
mod structured_markdown_tests;
mod util_username_gen;
mod validation_judge_results;
mod validation_judges;
mod validation_tags;
mod validation_test_template;
mod validation_username;
