use super::*;

// Inner `tests` module inside a file already named tests.rs — kept for the
// existing test-path names; the inception is deliberate.
#[allow(clippy::module_inception)]
mod tests {
    use super::*;

    #[test]
    fn name_validation_trims_and_bounds() {
        assert!(validate_name("").is_err());
        assert!(validate_name("   ").is_err());
        assert!(validate_name(&"x".repeat(121)).is_err());
        assert_eq!(validate_name("  hi  ").unwrap(), "hi");
        assert_eq!(validate_name(&"x".repeat(120)).unwrap().len(), 120);
    }

    #[test]
    fn transitions_match_contract_fr_009() {
        assert!(validate_transition(&SessionStatus::Lobby, &SessionStatus::Running).is_ok());
        assert!(validate_transition(&SessionStatus::Lobby, &SessionStatus::Cancelled).is_ok());
        assert!(validate_transition(&SessionStatus::Running, &SessionStatus::Finished).is_ok());
        assert!(validate_transition(&SessionStatus::Running, &SessionStatus::Cancelled).is_ok());
        assert!(validate_transition(&SessionStatus::Lobby, &SessionStatus::Lobby).is_ok());
        assert!(matches!(
            validate_transition(&SessionStatus::Running, &SessionStatus::Lobby),
            Err(SessionError::InvalidTransition)
        ));
        assert!(matches!(
            validate_transition(&SessionStatus::Finished, &SessionStatus::Running),
            Err(SessionError::InvalidTransition)
        ));
        assert!(matches!(
            validate_transition(&SessionStatus::Cancelled, &SessionStatus::Lobby),
            Err(SessionError::InvalidTransition)
        ));
    }

    #[test]
    fn transitions_paused_edges() {
        assert!(validate_transition(&SessionStatus::Running, &SessionStatus::Paused).is_ok());
        assert!(validate_transition(&SessionStatus::Paused, &SessionStatus::Running).is_ok());
        assert!(validate_transition(&SessionStatus::Paused, &SessionStatus::Cancelled).is_ok());
    }

    #[test]
    fn transitions_paused_to_finished_rejected() {
        assert!(matches!(
            validate_transition(&SessionStatus::Paused, &SessionStatus::Finished),
            Err(SessionError::InvalidTransition)
        ));
    }

    #[test]
    fn transitions_paused_is_terminal_for_non_resume_non_cancel() {
        assert!(matches!(
            validate_transition(&SessionStatus::Paused, &SessionStatus::Lobby),
            Err(SessionError::InvalidTransition)
        ));
        assert!(matches!(
            validate_transition(&SessionStatus::Paused, &SessionStatus::Paused),
            Ok(())
        ));
    }
}
