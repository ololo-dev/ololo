use crate::state::{GameServerState, PlayerAgentRegistry};
use arena_core::protocol::PlayerAgentFrame;
use tokio::sync::mpsc;
use uuid::Uuid;

/// RAII guard that removes the player's agent channel from the registry
/// when the socket handler returns (success or error).
///
/// The registry is keyed by `player_id` alone, so a reconnecting player briefly
/// has two live handlers. The guard therefore removes the entry only while it
/// still holds *this* handler's channel: an unconditional remove let the older
/// handler's teardown delete the newer one's registration, silently cutting
/// `JudgeScored` delivery to a connection that was very much alive.
///
/// When the removal actually happens — i.e. this handler was still the
/// player's current connection — the guard also flips the player's persisted
/// agent presence to disconnected. A superseded handler's teardown does
/// neither, so a reconnect never marks the new, live connection offline.
pub struct RegistryGuard {
    registry: PlayerAgentRegistry,
    player_id: Uuid,
    own: mpsc::Sender<PlayerAgentFrame>,
    /// `Some` once the handler has announced the connection: carries what the
    /// disconnect side needs (db + publisher + join_code).
    presence: Option<(GameServerState, String)>,
}

impl RegistryGuard {
    pub fn new(
        registry: PlayerAgentRegistry,
        player_id: Uuid,
        own: mpsc::Sender<PlayerAgentFrame>,
    ) -> Self {
        Self {
            registry,
            player_id,
            own,
            presence: None,
        }
    }

    /// Arm the disconnect side: from now on, tearing down while still the
    /// current connection also records the player as offline.
    pub fn arm_presence(&mut self, state: GameServerState, join_code: String) {
        self.presence = Some((state, join_code));
    }
}

impl Drop for RegistryGuard {
    fn drop(&mut self) {
        let own = self.own.clone();
        let removed = self
            .registry
            .remove_if(&self.player_id, move |_, current| {
                current.same_channel(&own)
            })
            .is_some();
        if !removed {
            return; // superseded: the newer handler owns presence now
        }
        if let Some((state, join_code)) = self.presence.take() {
            let player_id = self.player_id;
            let registry = self.registry.clone();
            // Drop cannot await; the write is fire-and-forget. Re-check the
            // registry inside the task: if a reconnect registered between our
            // removal and this write, the new handler already (or will)
            // record connected=true, and racing a stale `false` over it would
            // mark a live client offline.
            tokio::spawn(async move {
                if registry.contains_key(&player_id) {
                    return;
                }
                super::presence::set_agent_presence(&state, &join_code, player_id, false).await;
            });
        }
    }
}

/// Whether `own` is still the channel registered for `player_id` — i.e. whether
/// this handler is the player's current connection.
///
/// A newer connection overwrites the registry entry, and the older handler has
/// no other way to learn it was superseded. Two handlers dispatching probes for
/// one player send every probe twice; the duplicates then execute concurrently
/// on the player's machine and race over the shared `.ololo/tmp` state, so
/// answers cross between them. That corrupts scoring in both directions —
/// duplicated passes double the reward, crossed answers invent failures.
pub fn is_current_connection(
    registry: &PlayerAgentRegistry,
    player_id: Uuid,
    own: &mpsc::Sender<PlayerAgentFrame>,
) -> bool {
    registry
        .get(&player_id)
        .map(|entry| entry.value().same_channel(own))
        .unwrap_or(false)
}
