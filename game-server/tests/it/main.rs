//! Single integration-test binary for `game-server`.
//!
//! Every module here used to be its own `tests/*.rs` target — ~22
//! separate binaries each linking the full dependency stack, which dominated
//! CI compile time. One binary links once; nextest still runs the tests in
//! parallel (one process per test).

mod api_judge_run_tests;
mod heartbeat_tests;
mod idle_sweep_tests;
mod judge_execution;
mod judge_registrar_tests;
mod judge_vision_images;
mod open_ended_lifecycle_tests;
mod player_agent_single_flight;
mod player_agent_tests;
mod player_agent_ws_auth_tests;
mod presence_tests;
mod probe_exec_tests;
mod probe_scheduler_tests;
mod recovery_integration_tests;
mod recovery_tests;
mod router_tests;
mod scheduler_multi_section_tests;
mod scheduler_player_scoping_tests;
mod session_completion_tests;
mod session_lifecycle_tests;
mod state_tests;
mod zmq_pub_tests;
mod zmq_pub_trait;
