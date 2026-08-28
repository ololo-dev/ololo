use arena_core::session_status::SessionStatus;

pub enum ProbeAction {
    Dispatch,
    Pause,
    Exit,
}

pub fn decide_probe_action(status: SessionStatus) -> ProbeAction {
    match status {
        SessionStatus::Running => ProbeAction::Dispatch,
        SessionStatus::Paused => ProbeAction::Pause,
        SessionStatus::Lobby => ProbeAction::Pause,
        SessionStatus::Finished | SessionStatus::Cancelled => ProbeAction::Exit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arena_core::session_status::SessionStatus;

    #[test]
    fn running_dispatches() {
        assert!(matches!(
            decide_probe_action(SessionStatus::Running),
            ProbeAction::Dispatch
        ));
    }

    #[test]
    fn paused_and_lobby_pause() {
        assert!(matches!(
            decide_probe_action(SessionStatus::Paused),
            ProbeAction::Pause
        ));
        assert!(matches!(
            decide_probe_action(SessionStatus::Lobby),
            ProbeAction::Pause
        ));
    }

    #[test]
    fn finished_and_cancelled_exit() {
        assert!(matches!(
            decide_probe_action(SessionStatus::Finished),
            ProbeAction::Exit
        ));
        assert!(matches!(
            decide_probe_action(SessionStatus::Cancelled),
            ProbeAction::Exit
        ));
    }
}
