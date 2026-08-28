//! Raise the process file-descriptor limit at startup.
//!
//! Docker 25+ stopped overriding the container `nofile` ulimit, so both
//! binaries inherit the kernel default: **soft 1024**, hard 524288. A game
//! server day — WebSocket observers, Traefik keep-alive pools, git
//! `http-backend` CGI children, a PG pool — sits routinely in the hundreds
//! of descriptors, and on 2026-08-17 the dev `server` crossed 1024: `accept`
//! failed with EMFILE every second, the health check (which also needs an
//! accepted socket) went red, Traefik dropped the route, and the whole API
//! served the SPA's 404 page until a manual container restart. Raising soft
//! to hard at startup is the standard fix (what nginx, envoy and postgres
//! all do) and turns a hard evening outage into half a million descriptors
//! of headroom.

/// Raise `RLIMIT_NOFILE`'s soft limit to the hard limit. Returns
/// `(before, after)` soft values on success. Never fatal: a process that
/// cannot raise its limit still runs — it just keeps the old ceiling.
#[cfg(unix)]
// `rlim_t` is u64 on Linux/macOS but not guaranteed to be on every libc
// target — the casts are for the platforms where it isn't.
#[allow(clippy::unnecessary_cast)]
pub fn raise_nofile_limit() -> Option<(u64, u64)> {
    // SAFETY: plain libc getrlimit/setrlimit calls on a zeroed struct; no
    // pointers outlive the calls.
    unsafe {
        let mut lim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) != 0 {
            return None;
        }
        let before = lim.rlim_cur as u64;
        if lim.rlim_cur >= lim.rlim_max {
            return Some((before, before));
        }
        lim.rlim_cur = lim.rlim_max;
        if libc::setrlimit(libc::RLIMIT_NOFILE, &lim) != 0 {
            return None;
        }
        Some((before, lim.rlim_cur as u64))
    }
}

#[cfg(not(unix))]
pub fn raise_nofile_limit() -> Option<(u64, u64)> {
    None
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn soft_limit_ends_up_at_hard_limit() {
        let (before, after) = super::raise_nofile_limit().expect("rlimit readable");
        assert!(after >= before, "raising never lowers the limit");
        // After the call the soft limit equals the hard limit.
        unsafe {
            let mut lim = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            assert_eq!(libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim), 0);
            assert_eq!(lim.rlim_cur, lim.rlim_max);
            #[allow(clippy::unnecessary_cast)]
            {
                assert_eq!(after, lim.rlim_cur as u64);
            }
        }
    }
}
