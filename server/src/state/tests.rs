#[cfg(test)]
mod player_registry_tests {

    use crate::state::{PlayerChannel, PlayerRegistry};
    use dashmap::DashMap;
    use std::sync::Arc;

    #[test]
    fn player_registry_is_correct_type() {
        let registry: PlayerRegistry = Arc::new(DashMap::new());
        assert!(registry.is_empty());
        let (tx, _rx) = tokio::sync::broadcast::channel::<crate::protocol::PlayerFrame>(16);
        let channel = Arc::new(PlayerChannel::new(tx));
        let id = uuid::Uuid::nil();
        registry.insert(id, channel);
        assert!(!registry.is_empty());
        registry.remove(&id);
        assert!(registry.is_empty());
    }
}

#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {

    use crate::state::CliAuthSession;
    use dashmap::DashMap;
    use std::sync::Arc;

    #[test]
    fn cli_auth_session_claim_hit_and_miss() {
        let sessions: Arc<DashMap<String, CliAuthSession>> = Arc::new(DashMap::new());
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        sessions.insert(
            "test_token".to_string(),
            CliAuthSession {
                tx,
                created_at: std::time::Instant::now(),
            },
        );
        // First remove succeeds
        assert!(sessions.remove("test_token").is_some());
        // Second remove returns None (atomic, first-claimer-wins)
        assert!(sessions.remove("test_token").is_none());
    }
}
