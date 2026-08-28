//! Test-only helpers shared across crate test modules.
//!
//! Kept behind `#[cfg(test)]` so nothing ships in the binary.

#[cfg(test)]
pub(crate) mod test_util {
    use std::sync::Mutex;

    /// Serializes HOME mutation across tests so two tests do not race on the
    /// process-global `HOME` variable.
    pub static HOME_LOCK: Mutex<()> = Mutex::new(());

    /// Restores `HOME` to its previous value on drop, so mutating tests do not
    /// leak state into following tests.
    pub struct HomeGuard {
        prev: Option<String>,
    }

    impl HomeGuard {
        pub fn set(new: &str) -> Self {
            let prev = std::env::var("HOME").ok();
            unsafe { std::env::set_var("HOME", new) };
            Self { prev }
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => unsafe { std::env::set_var("HOME", v) },
                None => unsafe { std::env::remove_var("HOME") },
            }
        }
    }
}
